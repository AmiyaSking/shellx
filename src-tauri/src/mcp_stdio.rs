//! Line-delimited JSON-RPC 2.0 transport for the local MCP server.
//!
//! Started with `shellx --mcp-stdio`. The protocol requires stdout to carry
//! only JSON-RPC messages, so diagnostics go to stderr. Each request is handled
//! in order; `tools/call` reuses the same `AppState`, policy engine, approval
//! store and audit log as the GUI.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::mcp;
use crate::mcp::McpServices;
use crate::state::AppState;

/// Latest MCP revision we implement.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];

const SERVER_INSTRUCTIONS: &str = "ShellX 本地终端服务。命令执行、SFTP 与端口转发受连接级授权、\
环境分级与策略引擎约束；stdio 模式没有人工审批窗口，需要审批的操作会被自动拒绝并写入审计日志。\
交互式终端只有在连接显式开启 MCP 终端后才可用（危险：会绕过命令级策略）。";

/// Entry point used by `main` when `--mcp-stdio` is present.
pub fn run() -> Result<(), String> {
    let data_dir = data_dir_from_args(std::env::args().skip(1))?;
    let state = AppState::load(data_dir)
        .map_err(|error| format!("无法初始化 ShellX 数据目录：{error}"))?;
    let services = McpServices::new();
    let result = serve(&state, &services);
    services.pty.shutdown();
    services.forwards.shutdown();
    services.sftp.shutdown();
    result
}

/// Read requests from stdin and write responses to stdout until EOF.
pub fn serve(state: &AppState, services: &McpServices) -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve_io(state, services, stdin.lock(), stdout.lock())
}

/// Transport loop with injectable IO, used by tests.
pub fn serve_io<R: BufRead, W: Write>(
    state: &AppState,
    services: &McpServices,
    mut reader: R,
    mut writer: W,
) -> Result<(), String> {
    let runtime = crate::pty::runtime()?;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => return Err(format!("读取 stdin 失败：{error}")),
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(response) = runtime.block_on(handle_line(state, services, trimmed)) {
            let encoded = serde_json::to_string(&response)
                .map_err(|error| format!("序列化 MCP 响应失败：{error}"))?;
            writeln!(writer, "{encoded}").map_err(|error| {
                format!("写入 stdout 失败（MCP 客户端可能已断开）：{error}")
            })?;
            writer
                .flush()
                .map_err(|error| format!("刷新 stdout 失败：{error}"))?;
        }
    }
    Ok(())
}

/// Parse and handle one line. Returns `None` for notifications.
pub async fn handle_line(
    state: &AppState,
    services: &McpServices,
    line: &str,
) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return Some(error_response(
                Value::Null,
                -32700,
                format!("JSON 解析失败：{error}"),
            ))
        }
    };
    handle_request(state, services, request).await
}

/// Handle one JSON-RPC message. Notifications (no `id`) produce no response.
pub async fn handle_request(
    state: &AppState,
    services: &McpServices,
    request: Value,
) -> Option<Value> {
    if !request.is_object() {
        return Some(error_response(
            Value::Null,
            -32600,
            "请求必须是 JSON 对象".into(),
        ));
    }
    let id = request.get("id").cloned();
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return id.map(|id| error_response(id, -32600, "缺少 method 字段".into()));
    };
    // Notifications carry no id and never get a response.
    let id = id?;

    match method {
        "initialize" => Some(ok_response(id, initialize_result(&request))),
        "ping" => Some(ok_response(id, json!({}))),
        "tools/list" => Some(ok_response(id, json!({ "tools": mcp::tool_definitions() }))),
        "tools/call" => {
            Some(call_tool(state, services, id, request.get("params")).await)
        }
        "resources/list" => match mcp::resource_definitions(state) {
            Ok(resources) => Some(ok_response(id, json!({ "resources": resources }))),
            Err(error) => Some(error_response(id, -32603, error)),
        },
        "resources/templates/list" => Some(ok_response(
            id,
            json!({ "resourceTemplates": mcp::resource_templates() }),
        )),
        "resources/read" => Some(read_resource(state, id, request.get("params"))),
        other => Some(error_response(
            id,
            -32601,
            format!("未实现的方法：{other}"),
        )),
    }
}

fn initialize_result(request: &Value) -> Value {
    let requested = request
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol_version = requested
        .filter(|version| SUPPORTED_VERSIONS.contains(version))
        .unwrap_or(PROTOCOL_VERSION);

    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false }
        },
        "serverInfo": {
            "name": "shellx",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": SERVER_INSTRUCTIONS
    })
}

fn read_resource(state: &AppState, id: Value, params: Option<&Value>) -> Value {
    let Some(uri) = params
        .and_then(|params| params.get("uri"))
        .and_then(Value::as_str)
    else {
        return error_response(id, -32602, "resources/read 缺少 uri 参数".into());
    };
    match mcp::read_resource(state, uri) {
        Ok(result) => ok_response(id, result),
        Err(message) => error_response(id, -32002, message),
    }
}

async fn call_tool(
    state: &AppState,
    services: &McpServices,
    id: Value,
    params: Option<&Value>,
) -> Value {
    let Some(name) = params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
    else {
        return error_response(id, -32602, "tools/call 缺少 name 参数".into());
    };
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    // Pick up GUI changes (MCP permission, PTY opt-in, new rules) without
    // requiring an MCP client restart.
    if let Err(error) = state.refresh_from_disk() {
        return error_response(id, -32603, format!("无法刷新本地配置：{error}"));
    }

    match mcp::dispatch_tool(state, services, name, arguments).await {
        Ok(value) => tool_result(id, value, false),
        Err(message) => tool_result(id, Value::String(message), true),
    }
}

fn tool_result(id: Value, value: Value, is_error: bool) -> Value {
    let text = match &value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    let mut result = json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    });
    if !is_error && value.is_object() {
        result["structuredContent"] = value;
    }
    ok_response(id, result)
}

fn ok_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Resolve the data directory shared with the GUI.
///
/// `--data-dir <path>` wins, then `SHELLX_DATA_DIR`, then the platform default
/// that matches Tauri's `app_data_dir` for the `com.shellx.app` identifier.
pub fn data_dir_from_args<I>(args: I) -> Result<PathBuf, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter().peekable();
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            let value = args
                .next()
                .ok_or_else(|| "--data-dir 需要一个路径参数".to_string())?;
            return Ok(PathBuf::from(value));
        }
        if let Some(value) = arg.strip_prefix("--data-dir=") {
            if value.is_empty() {
                return Err("--data-dir 需要一个路径参数".to_string());
            }
            return Ok(PathBuf::from(value));
        }
    }

    if let Some(path) = std::env::var_os("SHELLX_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }

    let base = platform_data_dir()
        .ok_or_else(|| "无法确定应用数据目录，请使用 --data-dir 或 SHELLX_DATA_DIR".to_string())?;
    Ok(base.join("com.shellx.app"))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_data_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forwards::{ForwardKind, ForwardRuleInput};
    use crate::models::ConnectionInput;
    use crate::ssh::{connect_authenticated, Credential};
    use crate::ssh_test_server::{
        start_sftp_test_server, start_test_server, trusted_params, TestAuthMode,
    };
    use std::time::Duration;

    struct TestState {
        dir: PathBuf,
        state: AppState,
        services: McpServices,
    }

    impl Drop for TestState {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn test_state() -> TestState {
        let dir = std::env::temp_dir().join(format!(
            "shellx-mcp-it-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create MCP test data dir");
        let state = AppState::load(dir.clone()).expect("load app state");
        TestState {
            dir,
            state,
            services: McpServices::new(),
        }
    }

    fn save_connection(
        state: &AppState,
        host: &str,
        port: u16,
        environment: &str,
        mcp_access: &str,
    ) -> String {
        let input: ConnectionInput = serde_json::from_value(json!({
            "alias": format!("{environment}-box"),
            "host": host,
            "port": port,
            "username": "tester",
            "environment": environment,
            "mcpAccess": mcp_access,
            "auth": { "kind": "password" }
        }))
        .expect("build connection input");
        state.save_connection(input).expect("save connection").id
    }

    fn enable_pty(state: &AppState, id: &str) {
        let connection = state
            .list_connections()
            .expect("list connections")
            .into_iter()
            .find(|connection| connection.id == id)
            .expect("connection exists");
        let input: ConnectionInput = serde_json::from_value(json!({
            "id": connection.id,
            "alias": connection.alias,
            "host": connection.host,
            "port": connection.port,
            "username": connection.username,
            "group": connection.group,
            "tags": connection.tags,
            "environment": connection.environment,
            "auth": connection.auth,
            "mcpAccess": connection.mcp_access,
            "mcpPtyEnabled": true,
            "jumpHost": connection.jump_host,
            "remark": connection.remark,
            "term": connection.term,
            "encoding": connection.encoding,
            "startupCommand": connection.startup_command,
            "legacyAlgorithms": connection.legacy_algorithms,
            "forwardAgent": connection.forward_agent
        }))
        .expect("build connection input");
        state.save_connection(input).expect("enable MCP PTY");
    }

    fn tool_text(response: &Value) -> &str {
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text")
    }

    #[tokio::test]
    async fn initialize_reports_capabilities_and_protocol() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        )
        .await
        .expect("initialize response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], "shellx");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_object());
    }

    #[tokio::test]
    async fn older_protocol_version_is_echoed_when_supported() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#,
        )
        .await
        .expect("initialize response");
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
    }

    #[tokio::test]
    async fn notifications_get_no_response() {
        let fixture = test_state();
        assert!(
            handle_line(
                &fixture.state, &fixture.services,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            )
            .await
            .is_none()
        );
    }

    #[tokio::test]
    async fn tools_list_exposes_the_catalog() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
        )
        .await
        .expect("tools/list response");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(tools.len(), mcp::TOOL_CATALOG.len());
        for tool in tools {
            assert!(tool["inputSchema"]["type"] == "object");
        }
    }

    #[tokio::test]
    async fn resources_expose_sessions_audit_and_templates() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":20,"method":"resources/list"}"#,
        )
        .await
        .expect("resources/list response");
        let resources = response["result"]["resources"]
            .as_array()
            .expect("resources array");
        let uris: Vec<&str> = resources
            .iter()
            .filter_map(|resource| resource["uri"].as_str())
            .collect();
        assert!(uris.contains(&"shellx://sessions"));
        assert!(uris.contains(&"shellx://audit"));

        let templates = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":21,"method":"resources/templates/list"}"#,
        )
        .await
        .expect("resources/templates/list response");
        assert_eq!(
            templates["result"]["resourceTemplates"][0]["uriTemplate"],
            "shellx://sessions/{sessionId}/output"
        );

        let read = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":22,"method":"resources/read","params":{"uri":"shellx://sessions"}}"#,
        )
        .await
        .expect("read sessions resource");
        assert_eq!(read["result"]["contents"][0]["mimeType"], "application/json");
        let body: Value =
            serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap())
                .expect("parse sessions JSON");
        assert!(body["sessions"].is_array());

        let audit = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":23,"method":"resources/read","params":{"uri":"shellx://audit"}}"#,
        )
        .await
        .expect("read audit resource");
        let body: Value =
            serde_json::from_str(audit["result"]["contents"][0]["text"].as_str().unwrap())
                .expect("parse audit JSON");
        assert!(
            !body["entries"].as_array().unwrap().is_empty(),
            "startup audit entry should be present"
        );
    }

    #[tokio::test]
    async fn unknown_resource_returns_resource_error() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":24,"method":"resources/read","params":{"uri":"shellx://nope"}}"#,
        )
        .await
        .expect("error response");
        assert_eq!(response["error"]["code"], -32002);

        let missing = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":25,"method":"resources/read","params":{}}"#,
        )
        .await
        .expect("missing uri response");
        assert_eq!(missing["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn list_servers_only_returns_authorized_connections() {
        let fixture = test_state();
        let authorized = save_connection(
            &fixture.state,
            "127.0.0.1",
            22,
            "dev",
            "standard_controlled",
        );
        save_connection(&fixture.state, "127.0.0.1", 22, "dev", "disabled");

        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_servers","arguments":{}}}"#,
        )
        .await
        .expect("tools/call response");
        assert_eq!(response["result"]["isError"], false);
        let servers: Value =
            serde_json::from_str(tool_text(&response)).expect("parse tool JSON");
        let servers = servers["servers"].as_array().expect("servers array");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["id"], authorized);
    }

    #[tokio::test]
    async fn tool_calls_reload_connection_settings_from_disk() {
        let fixture = test_state();
        save_connection(
            &fixture.state,
            "127.0.0.1",
            22,
            "dev",
            "standard_controlled",
        );

        let listed = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":28,"method":"tools/call","params":{"name":"list_servers","arguments":{}}}"#,
        )
        .await
        .expect("list servers");
        let body: Value = serde_json::from_str(tool_text(&listed)).expect("parse servers");
        assert_eq!(body["servers"].as_array().unwrap().len(), 1);

        // Simulate the GUI revoking MCP access while the stdio process runs.
        let path = fixture.dir.join("connections.json");
        let text = std::fs::read_to_string(&path).expect("read connections.json");
        std::fs::write(&path, text.replace("standard_controlled", "disabled"))
            .expect("rewrite connections.json");

        let listed = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":29,"method":"tools/call","params":{"name":"list_servers","arguments":{}}}"#,
        )
        .await
        .expect("list servers after reload");
        let body: Value = serde_json::from_str(tool_text(&listed)).expect("parse servers");
        assert!(
            body["servers"].as_array().unwrap().is_empty(),
            "MCP must pick up permission changes from disk"
        );
    }

    #[tokio::test]
    async fn disabled_connection_is_blocked_before_any_network_use() {
        let fixture = test_state();
        let connection = save_connection(&fixture.state, "127.0.0.1", 9, "dev", "disabled");
        let response = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "exec_command",
                    "arguments": { "connection_id": connection, "command": "uptime" }
                }
            })
            .to_string(),
        )
        .await
        .expect("tools/call response");
        assert_eq!(response["result"]["isError"], true);
        assert!(
            tool_text(&response).contains("拦截"),
            "got {}",
            tool_text(&response)
        );
    }

    #[tokio::test]
    async fn approval_required_command_is_auto_rejected_and_audited() {
        let fixture = test_state();
        let connection = save_connection(&fixture.state, "127.0.0.1", 9, "prod", "standard_controlled");
        let response = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 6,
                "method": "tools/call",
                "params": {
                    "name": "exec_command",
                    "arguments": { "connection_id": connection, "command": "rm -rf /var/www" }
                }
            })
            .to_string(),
        )
        .await
        .expect("tools/call response");
        assert_eq!(response["result"]["isError"], true);
        assert!(
            tool_text(&response).contains("自动拒绝"),
            "got {}",
            tool_text(&response)
        );

        let audit = fixture.state.list_audit(10);
        assert!(
            audit
                .iter()
                .any(|entry| entry.detail.contains("stdio 模式没有人工审批界面")),
            "approval rejection must be audited: {:?}",
            audit.iter().map(|entry| &entry.detail).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn exec_command_runs_over_real_ssh_and_output_is_paged() {
        let server = start_test_server(TestAuthMode::Password("secret".into())).await;
        let fixture = test_state();
        let connection = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "standard_controlled",
        );
        fixture
            .state
            .cache_password_for_test(&connection, "secret");

        // Confirm the host key the same way the desktop UI does: the second
        // connection records it in the known_hosts file shared by both modes.
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        let trusted = connect_authenticated(&trust)
            .await
            .expect("record the trusted host key");
        drop(trusted);

        let response = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": {
                    "name": "exec_command",
                    "arguments": {
                        "connection_id": connection,
                        "command": "echo hello-from-mcp",
                        "timeout_secs": 5
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("tools/call response");
        assert_eq!(response["result"]["isError"], false, "{}", tool_text(&response));
        let result: Value =
            serde_json::from_str(tool_text(&response)).expect("parse command result");
        assert_eq!(result["exitCode"], 0);
        assert_eq!(result["simulated"], false);
        assert!(result["output"].as_str().unwrap().contains("hello-from-mcp"));
        let session_id = result["sessionId"].as_str().expect("session id").to_string();

        let page = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 8,
                "method": "tools/call",
                "params": {
                    "name": "read_output",
                    "arguments": { "session_id": session_id, "offset": 0 }
                }
            })
            .to_string(),
        )
        .await
        .expect("read_output response");
        assert_eq!(page["result"]["isError"], false);
        let page: Value = serde_json::from_str(tool_text(&page)).expect("parse page");
        assert!(page["output"].as_str().unwrap().contains("hello-from-mcp"));
        assert_eq!(page["eof"], true);

        let expected_uri = format!("shellx://sessions/{session_id}/output");
        let resources = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":26,"method":"resources/list"}"#,
        )
        .await
        .expect("resources/list response");
        assert!(
            resources["result"]["resources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|resource| resource["uri"].as_str() == Some(expected_uri.as_str())),
            "session output must be listed as a resource"
        );

        let resource = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 27,
                "method": "resources/read",
                "params": { "uri": expected_uri }
            })
            .to_string(),
        )
        .await
        .expect("resources/read response");
        assert_eq!(resource["result"]["contents"][0]["mimeType"], "text/plain");
        assert!(resource["result"]["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello-from-mcp"));

        let closed = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "tools/call",
                "params": {
                    "name": "close_session",
                    "arguments": { "session_id": page["sessionId"] }
                }
            })
            .to_string(),
        )
        .await
        .expect("close_session response");
        assert_eq!(closed["result"]["isError"], false);
    }

    #[tokio::test]
    async fn background_command_can_be_polled_and_signaled() {
        let server = start_test_server(TestAuthMode::Password("secret".into())).await;
        let fixture = test_state();
        let connection = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "standard_controlled",
        );
        fixture
            .state
            .cache_password_for_test(&connection, "secret");
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        drop(connect_authenticated(&trust).await.expect("trust host key"));

        // Start without waiting: the call returns as soon as the remote
        // command is running.
        let started = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 11,
                "method": "tools/call",
                "params": {
                    "name": "exec_command",
                    "arguments": {
                        "connection_id": connection,
                        "command": "sleep",
                        "timeout_secs": 30,
                        "wait": false
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("start response");
        assert_eq!(started["result"]["isError"], false, "{}", tool_text(&started));
        let started: Value = serde_json::from_str(tool_text(&started)).expect("parse start");
        let session_id = started["sessionId"].as_str().expect("session id").to_string();
        assert_eq!(started["running"], true);

        let page = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 12,
                "method": "tools/call",
                "params": {
                    "name": "read_output",
                    "arguments": { "session_id": session_id }
                }
            })
            .to_string(),
        )
        .await
        .expect("read running session");
        let page: Value = serde_json::from_str(tool_text(&page)).expect("parse page");
        assert_eq!(page["running"], true);
        assert_eq!(page["exitCode"], Value::Null);

        let signal = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 13,
                "method": "tools/call",
                "params": {
                    "name": "send_signal",
                    "arguments": { "session_id": session_id, "signal": "INT" }
                }
            })
            .to_string(),
        )
        .await
        .expect("send_signal response");
        assert_eq!(signal["result"]["isError"], false, "{}", tool_text(&signal));

        let mut finished = None;
        for _ in 0..50 {
            let response = handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 14,
                    "method": "tools/call",
                    "params": {
                        "name": "read_output",
                        "arguments": { "session_id": session_id }
                    }
                })
                .to_string(),
            )
            .await
            .expect("poll session");
            let page: Value =
                serde_json::from_str(tool_text(&response)).expect("parse page");
            if page["running"] == false {
                finished = Some(page);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let finished = finished.expect("signalled command should finish");
        assert_eq!(finished["exitCode"], 130);

        let closed = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 15,
                "method": "tools/call",
                "params": {
                    "name": "close_session",
                    "arguments": { "session_id": session_id }
                }
            })
            .to_string(),
        )
        .await
        .expect("close_session response");
        assert_eq!(closed["result"]["isError"], false);
    }

    #[tokio::test]
    async fn sftp_read_write_round_trip_over_mcp() {
        let fixture = test_state();
        let root = fixture.dir.join("sftp-root");
        std::fs::create_dir_all(&root).expect("create sftp root");
        std::fs::write(root.join("hello.txt"), b"hello").expect("seed file");

        let server =
            start_sftp_test_server(TestAuthMode::Password("secret".into()), root.clone())
                .await;
        let connection = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "standard_controlled",
        );
        fixture.state.cache_password_for_test(&connection, "secret");
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        drop(connect_authenticated(&trust).await.expect("trust host key"));

        let opened = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 30,
                "method": "tools/call",
                "params": {
                    "name": "sftp_open",
                    "arguments": { "connection_id": connection }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_open response");
        assert_eq!(opened["result"]["isError"], false, "{}", tool_text(&opened));
        let opened: Value = serde_json::from_str(tool_text(&opened)).expect("parse open");
        let session_id = opened["sessionId"].as_str().expect("session id").to_string();
        assert!(opened["listing"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == "hello.txt"));

        let read = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 31,
                "method": "tools/call",
                "params": {
                    "name": "sftp_read_file",
                    "arguments": { "session_id": session_id, "path": "/hello.txt" }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_read_file response");
        assert_eq!(read["result"]["isError"], false, "{}", tool_text(&read));
        let file: Value = serde_json::from_str(tool_text(&read)).expect("parse file");
        assert_eq!(file["content"], "hello");
        assert_eq!(file["encoding"], "utf-8");
        assert_eq!(file["size"], 5);

        let write = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 32,
                "method": "tools/call",
                "params": {
                    "name": "sftp_write_file",
                    "arguments": {
                        "session_id": session_id,
                        "path": "/agent.txt",
                        "content": "written by agent\n"
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_write_file response");
        assert_eq!(write["result"]["isError"], false, "{}", tool_text(&write));
        assert_eq!(
            std::fs::read_to_string(root.join("agent.txt")).expect("read written file"),
            "written by agent\n"
        );

        let stat = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 33,
                "method": "tools/call",
                "params": {
                    "name": "sftp_stat",
                    "arguments": { "session_id": session_id, "path": "/agent.txt" }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_stat response");
        let stat: Value = serde_json::from_str(tool_text(&stat)).expect("parse stat");
        assert_eq!(stat["size"], 17);
        assert_eq!(stat["isDir"], false);

        let mkdir = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 34,
                "method": "tools/call",
                "params": {
                    "name": "sftp_mkdir",
                    "arguments": { "session_id": session_id, "path": "/agent-dir" }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_mkdir response");
        assert_eq!(mkdir["result"]["isError"], false, "{}", tool_text(&mkdir));
        assert!(root.join("agent-dir").is_dir());

        let renamed = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 35,
                "method": "tools/call",
                "params": {
                    "name": "sftp_rename",
                    "arguments": {
                        "session_id": session_id,
                        "old_path": "/agent.txt",
                        "new_path": "/agent-dir/agent.txt"
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_rename response");
        assert_eq!(renamed["result"]["isError"], false, "{}", tool_text(&renamed));
        assert!(root.join("agent-dir").join("agent.txt").is_file());

        let removed_file = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 36,
                "method": "tools/call",
                "params": {
                    "name": "sftp_remove",
                    "arguments": {
                        "session_id": session_id,
                        "path": "/agent-dir/agent.txt"
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_remove file response");
        assert_eq!(
            removed_file["result"]["isError"], false,
            "{}",
            tool_text(&removed_file)
        );
        assert!(!root.join("agent-dir").join("agent.txt").exists());

        let removed_dir = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 37,
                "method": "tools/call",
                "params": {
                    "name": "sftp_remove",
                    "arguments": {
                        "session_id": session_id,
                        "path": "/agent-dir",
                        "is_dir": true,
                        "recursive": true
                    }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_remove dir response");
        assert_eq!(
            removed_dir["result"]["isError"], false,
            "{}",
            tool_text(&removed_dir)
        );
        assert!(!root.join("agent-dir").exists());

        let closed = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 38,
                "method": "tools/call",
                "params": {
                    "name": "sftp_close",
                    "arguments": { "session_id": session_id }
                }
            })
            .to_string(),
        )
        .await
        .expect("sftp_close response");
        assert_eq!(closed["result"]["isError"], false);

        let audit = fixture.state.list_audit(50);
        assert!(
            audit
                .iter()
                .any(|entry| entry.detail.contains("MCP 写入 SFTP 文件 /agent.txt")),
            "SFTP writes must be audited"
        );

        drop(server);
    }

    #[tokio::test]
    async fn sftp_write_permissions_follow_mcp_access_and_environment() {
        let fixture = test_state();
        let root = fixture.dir.join("sftp-root");
        std::fs::create_dir_all(&root).expect("create sftp root");
        std::fs::write(root.join("hello.txt"), b"hello").expect("seed file");
        let server =
            start_sftp_test_server(TestAuthMode::Password("secret".into()), root.clone())
                .await;
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        drop(connect_authenticated(&trust).await.expect("trust host key"));

        for (index, (environment, access, expected)) in [
            ("dev", "read_only", "只读"),
            ("staging", "standard_controlled", "自动拒绝"),
        ]
        .into_iter()
        .enumerate()
        {
            let connection = save_connection(
                &fixture.state,
                "127.0.0.1",
                server.port,
                environment,
                access,
            );
            fixture.state.cache_password_for_test(&connection, "secret");

            let opened = handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 40 + index,
                    "method": "tools/call",
                    "params": {
                        "name": "sftp_open",
                        "arguments": { "connection_id": connection }
                    }
                })
                .to_string(),
            )
            .await
            .expect("sftp_open response");
            assert_eq!(opened["result"]["isError"], false, "{}", tool_text(&opened));
            let opened: Value = serde_json::from_str(tool_text(&opened)).expect("parse open");
            let session_id = opened["sessionId"].as_str().expect("session id").to_string();

            // Reads are low risk and allowed for both read-only and controlled
            // connections, regardless of environment.
            let read = handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 45 + index,
                    "method": "tools/call",
                    "params": {
                        "name": "sftp_read_file",
                        "arguments": { "session_id": session_id, "path": "/hello.txt" }
                    }
                })
                .to_string(),
            )
            .await
            .expect("read response");
            assert_eq!(read["result"]["isError"], false, "{}", tool_text(&read));

            let denied = handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 50 + index,
                    "method": "tools/call",
                    "params": {
                        "name": "sftp_write_file",
                        "arguments": {
                            "session_id": session_id,
                            "path": "/denied.txt",
                            "content": "nope"
                        }
                    }
                })
                .to_string(),
            )
            .await
            .expect("denied write response");
            assert_eq!(denied["result"]["isError"], true);
            assert!(
                tool_text(&denied).contains(expected),
                "expected {expected}, got {}",
                tool_text(&denied)
            );
            assert!(!root.join("denied.txt").exists());

            handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 60 + index,
                    "method": "tools/call",
                    "params": {
                        "name": "sftp_close",
                        "arguments": { "session_id": session_id }
                    }
                })
                .to_string(),
            )
            .await
            .expect("sftp_close response");
        }

        let disabled = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "disabled",
        );
        let denied = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 70,
                "method": "tools/call",
                "params": {
                    "name": "sftp_open",
                    "arguments": { "connection_id": disabled }
                }
            })
            .to_string(),
        )
        .await
        .expect("disabled open response");
        assert_eq!(denied["result"]["isError"], true);
        assert!(tool_text(&denied).contains("未授权"));

        drop(server);
    }

    #[tokio::test]
    async fn forward_rules_can_be_started_and_stopped_over_mcp() {
        let server = start_test_server(TestAuthMode::Password("secret".into())).await;
        let fixture = test_state();
        let connection = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "standard_controlled",
        );
        fixture.state.cache_password_for_test(&connection, "secret");
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        drop(connect_authenticated(&trust).await.expect("trust host key"));

        let rule = fixture
            .state
            .save_forward_rule(ForwardRuleInput {
                id: None,
                connection_id: connection.clone(),
                kind: ForwardKind::Local,
                bind_host: "127.0.0.1".into(),
                bind_port: 0,
                target_host: "127.0.0.1".into(),
                target_port: 9,
                remark: "mcp forward".into(),
            })
            .expect("save forward rule");

        let listed = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":80,"method":"tools/call","params":{"name":"list_forward_rules","arguments":{}}}"#,
        )
        .await
        .expect("list rules response");
        assert_eq!(listed["result"]["isError"], false, "{}", tool_text(&listed));
        let body: Value = serde_json::from_str(tool_text(&listed)).expect("parse rules");
        let listed_rule = body["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == rule.id)
            .expect("rule is listed");
        assert_eq!(listed_rule["running"], false);

        let started = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 81,
                "method": "tools/call",
                "params": {
                    "name": "start_forward",
                    "arguments": { "rule_id": rule.id }
                }
            })
            .to_string(),
        )
        .await
        .expect("start forward response");
        assert_eq!(started["result"]["isError"], false, "{}", tool_text(&started));
        let status: Value = serde_json::from_str(tool_text(&started)).expect("parse status");
        assert_eq!(status["running"], true);
        assert!(status["boundPort"].as_u64().unwrap_or(0) > 0);

        let stopped = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 82,
                "method": "tools/call",
                "params": {
                    "name": "stop_forward",
                    "arguments": { "rule_id": rule.id }
                }
            })
            .to_string(),
        )
        .await
        .expect("stop forward response");
        assert_eq!(stopped["result"]["isError"], false, "{}", tool_text(&stopped));
        let stopped: Value = serde_json::from_str(tool_text(&stopped)).expect("parse stop");
        assert_eq!(stopped["stopped"], true);

        let unsafe_rule = fixture
            .state
            .save_forward_rule(ForwardRuleInput {
                id: None,
                connection_id: connection,
                kind: ForwardKind::Local,
                bind_host: "0.0.0.0".into(),
                bind_port: 0,
                target_host: "127.0.0.1".into(),
                target_port: 9,
                remark: "non-loopback".into(),
            })
            .expect("save unsafe rule");
        let denied = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 83,
                "method": "tools/call",
                "params": {
                    "name": "start_forward",
                    "arguments": { "rule_id": unsafe_rule.id }
                }
            })
            .to_string(),
        )
        .await
        .expect("unsafe forward response");
        assert_eq!(denied["result"]["isError"], true);
        assert!(
            tool_text(&denied).contains("回环"),
            "got {}",
            tool_text(&denied)
        );

        drop(server);
    }

    #[tokio::test]
    async fn interactive_terminal_requires_opt_in_and_echoes_input() {
        let server = start_test_server(TestAuthMode::Password("secret".into())).await;
        let fixture = test_state();
        let connection = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "standard_controlled",
        );
        fixture.state.cache_password_for_test(&connection, "secret");
        let trust = trusted_params(
            &server,
            &fixture.state.known_hosts_path(),
            Credential::Password("secret".into()),
        )
        .await;
        drop(connect_authenticated(&trust).await.expect("trust host key"));

        let denied = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 90,
                "method": "tools/call",
                "params": {
                    "name": "terminal_open",
                    "arguments": { "connection_id": connection }
                }
            })
            .to_string(),
        )
        .await
        .expect("denied terminal response");
        assert_eq!(denied["result"]["isError"], true);
        assert!(
            tool_text(&denied).contains("未开启"),
            "got {}",
            tool_text(&denied)
        );

        enable_pty(&fixture.state, &connection);

        let opened = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 91,
                "method": "tools/call",
                "params": {
                    "name": "terminal_open",
                    "arguments": { "connection_id": connection, "cols": 100, "rows": 40 }
                }
            })
            .to_string(),
        )
        .await
        .expect("open terminal response");
        assert_eq!(opened["result"]["isError"], false, "{}", tool_text(&opened));
        let opened: Value = serde_json::from_str(tool_text(&opened)).expect("parse terminal");
        let session_id = opened["sessionId"].as_str().expect("session id").to_string();
        assert_eq!(opened["cols"], 100);
        assert_eq!(opened["rows"], 40);

        let written = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 92,
                "method": "tools/call",
                "params": {
                    "name": "terminal_write",
                    "arguments": { "session_id": session_id, "data": "hello mcp\r" }
                }
            })
            .to_string(),
        )
        .await
        .expect("terminal_write response");
        assert_eq!(written["result"]["isError"], false, "{}", tool_text(&written));

        let mut echoed = None;
        for _ in 0..40 {
            let response = handle_line(
                &fixture.state, &fixture.services,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 93,
                    "method": "tools/call",
                    "params": {
                        "name": "terminal_read",
                        "arguments": { "session_id": session_id }
                    }
                })
                .to_string(),
            )
            .await
            .expect("terminal_read response");
            let output: Value =
                serde_json::from_str(tool_text(&response)).expect("parse output");
            if output["output"]
                .as_str()
                .is_some_and(|text| text.contains("hello mcp"))
            {
                echoed = Some(output);
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(echoed.is_some(), "terminal should echo the written input");

        let resized = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 94,
                "method": "tools/call",
                "params": {
                    "name": "terminal_resize",
                    "arguments": { "session_id": session_id, "cols": 120, "rows": 50 }
                }
            })
            .to_string(),
        )
        .await
        .expect("terminal_resize response");
        assert_eq!(resized["result"]["isError"], false, "{}", tool_text(&resized));
        assert_eq!(resized["result"]["structuredContent"]["cols"], 120);

        let closed = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 95,
                "method": "tools/call",
                "params": {
                    "name": "terminal_close",
                    "arguments": { "session_id": session_id }
                }
            })
            .to_string(),
        )
        .await
        .expect("terminal_close response");
        assert_eq!(closed["result"]["isError"], false, "{}", tool_text(&closed));

        let gone = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 96,
                "method": "tools/call",
                "params": {
                    "name": "terminal_read",
                    "arguments": { "session_id": session_id }
                }
            })
            .to_string(),
        )
        .await
        .expect("closed terminal response");
        assert_eq!(gone["result"]["isError"], true);

        let audit = fixture.state.list_audit(50);
        assert!(
            audit
                .iter()
                .any(|entry| entry.detail.contains("MCP 打开交互式终端")),
            "PTY opens must be audited"
        );

        // A read-only connection must never get an interactive shell, even if
        // the PTY flag was somehow persisted.
        let read_only = save_connection(
            &fixture.state,
            "127.0.0.1",
            server.port,
            "dev",
            "read_only",
        );
        fixture.state.cache_password_for_test(&read_only, "secret");
        enable_pty(&fixture.state, &read_only);
        let denied = handle_line(
            &fixture.state, &fixture.services,
            &json!({
                "jsonrpc": "2.0",
                "id": 97,
                "method": "tools/call",
                "params": {
                    "name": "terminal_open",
                    "arguments": { "connection_id": read_only }
                }
            })
            .to_string(),
        )
        .await
        .expect("read-only terminal response");
        assert_eq!(denied["result"]["isError"], true);
        assert!(
            tool_text(&denied).contains("受控"),
            "got {}",
            tool_text(&denied)
        );

        drop(server);
    }

    #[tokio::test]
    async fn malformed_json_returns_parse_error() {
        let fixture = test_state();
        let response = handle_line(&fixture.state, &fixture.services, "{not json")
            .await
            .expect("parse error response");
        assert_eq!(response["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let fixture = test_state();
        let response = handle_line(
            &fixture.state, &fixture.services,
            r#"{"jsonrpc":"2.0","id":10,"method":"bogus/method"}"#,
        )
        .await
        .expect("error response");
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn data_dir_flag_overrides_the_default() {
        let dir = data_dir_from_args(["--mcp-stdio".to_string(), "--data-dir".to_string(), "C:/tmp/x".to_string()])
            .expect("parse data dir");
        assert_eq!(dir, PathBuf::from("C:/tmp/x"));

        let dir = data_dir_from_args(["--data-dir=D:/shellx-data".to_string()])
            .expect("parse inline data dir");
        assert_eq!(dir, PathBuf::from("D:/shellx-data"));
    }

    #[test]
    fn stdio_loop_answers_each_request_line() {
        let fixture = test_state();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n"
        );
        let mut output = Vec::new();
        serve_io(&fixture.state, &fixture.services, input.as_bytes(), &mut output).expect("serve stdio");

        let text = String::from_utf8(output).expect("utf-8 output");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "notifications must not produce output");
        let first: Value = serde_json::from_str(lines[0]).expect("parse first response");
        let second: Value = serde_json::from_str(lines[1]).expect("parse second response");
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        assert!(second["result"]["tools"].as_array().unwrap().len() >= 5);
    }
}
