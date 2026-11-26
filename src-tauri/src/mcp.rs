use serde_json::{json, Value};
use uuid::Uuid;

use crate::encoding::base64_decode;
use crate::models::{
    AuditSource, CommandOutcome, CommandRequest, McpToolInfo, RiskLevel,
};
use crate::sftp::SftpManager;
use crate::state::{AppState, CommandStart};

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub risk: RiskLevel,
}

/// Tools exposed by the local MCP server. `list_servers` only returns safe
/// metadata; secrets and private-key paths never leave the Rust core.
pub const TOOL_CATALOG: &[ToolDef] = &[
    ToolDef {
        name: "list_servers",
        description: "列出已授权给 MCP 的服务器元信息（不含凭据）",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "exec_command",
        description: "在已授权服务器上执行命令，受策略引擎与审批流约束；wait=false 时返回会话 ID 供轮询或中断",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "read_output",
        description: "分页读取会话输出",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "send_signal",
        description: "向会话发送中断信号",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "close_session",
        description: "关闭指定会话",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "sftp_open",
        description: "打开 SFTP 文件会话（连接须已授权给 MCP；主机密钥须已在桌面端确认）",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "sftp_close",
        description: "关闭 SFTP 文件会话",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "sftp_list",
        description: "列出远端目录",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "sftp_stat",
        description: "读取远端文件或目录的元信息",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "sftp_read_file",
        description: "读取远端文件内容（默认最多 64KiB，UTF-8 或 base64）",
        risk: RiskLevel::Low,
    },
    ToolDef {
        name: "sftp_write_file",
        description: "创建或覆盖远端文件（dev 环境受控连接可写，staging/prod 需审批）",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "sftp_mkdir",
        description: "创建远端目录",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "sftp_remove",
        description: "删除远端文件或目录（目录需 recursive=true）",
        risk: RiskLevel::Medium,
    },
    ToolDef {
        name: "sftp_rename",
        description: "重命名远端文件或目录",
        risk: RiskLevel::Medium,
    },
];

pub fn tool_catalog() -> Vec<McpToolInfo> {
    TOOL_CATALOG
        .iter()
        .map(|tool| McpToolInfo {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            risk: tool.risk,
        })
        .collect()
}

/// JSON definitions returned by `tools/list`.
pub fn tool_definitions() -> Vec<Value> {
    TOOL_CATALOG
        .iter()
        .map(|tool| {
            let schema = match tool.name {
                "exec_command" => json!({
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "已授权连接 ID" },
                        "command": { "type": "string", "description": "要执行的命令" },
                        "timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 600,
                            "description": "执行超时（秒），默认 30"
                        },
                        "wait": {
                            "type": "boolean",
                            "description": "true（默认）等待命令结束并返回输出；false 立即返回 session_id，可配合 read_output / send_signal / close_session 使用"
                        }
                    },
                    "required": ["connection_id", "command"],
                    "additionalProperties": false
                }),
                "read_output" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "offset": { "type": "integer", "minimum": 0 },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 500,
                            "description": "本页最大行数，默认 200"
                        }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                "send_signal" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "signal": { "type": "string" }
                    },
                    "required": ["session_id", "signal"],
                    "additionalProperties": false
                }),
                "close_session" => json!({
                    "type": "object",
                    "properties": { "session_id": { "type": "string" } },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                "sftp_open" => json!({
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "已授权连接 ID" }
                    },
                    "required": ["connection_id"],
                    "additionalProperties": false
                }),
                "sftp_close" => json!({
                    "type": "object",
                    "properties": { "session_id": { "type": "string" } },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                "sftp_list" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string", "description": "远端目录，默认 ." }
                    },
                    "required": ["session_id"],
                    "additionalProperties": false
                }),
                "sftp_stat" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string" }
                    },
                    "required": ["session_id", "path"],
                    "additionalProperties": false
                }),
                "sftp_read_file" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string" },
                        "max_bytes": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 524288,
                            "description": "最多读取字节数，默认 65536"
                        }
                    },
                    "required": ["session_id", "path"],
                    "additionalProperties": false
                }),
                "sftp_write_file" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string" },
                        "content": { "type": "string" },
                        "encoding": {
                            "type": "string",
                            "enum": ["utf-8", "base64"],
                            "description": "默认 utf-8"
                        }
                    },
                    "required": ["session_id", "path", "content"],
                    "additionalProperties": false
                }),
                "sftp_mkdir" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string" }
                    },
                    "required": ["session_id", "path"],
                    "additionalProperties": false
                }),
                "sftp_remove" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "path": { "type": "string" },
                        "is_dir": { "type": "boolean", "description": "目标是目录" },
                        "recursive": {
                            "type": "boolean",
                            "description": "删除目录时必须为 true，表示递归删除"
                        }
                    },
                    "required": ["session_id", "path"],
                    "additionalProperties": false
                }),
                "sftp_rename" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "old_path": { "type": "string" },
                        "new_path": { "type": "string" }
                    },
                    "required": ["session_id", "old_path", "new_path"],
                    "additionalProperties": false
                }),
                _ => json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
            };
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": schema
            })
        })
        .collect()
}

pub const RESOURCE_SESSIONS: &str = "shellx://sessions";
pub const RESOURCE_AUDIT: &str = "shellx://audit";
pub const SESSION_OUTPUT_TEMPLATE: &str = "shellx://sessions/{sessionId}/output";

pub fn session_output_uri(session_id: &str) -> String {
    format!("shellx://sessions/{session_id}/output")
}

/// Read-only context resources advertised to MCP clients. Output is already
/// redacted when it enters the session ring, so no extra masking is needed.
pub fn resource_definitions(state: &AppState) -> Result<Vec<Value>, String> {
    let mut resources = vec![
        json!({
            "uri": RESOURCE_SESSIONS,
            "name": "sessions",
            "title": "命令会话",
            "description": "当前命令会话的元信息（JSON）",
            "mimeType": "application/json"
        }),
        json!({
            "uri": RESOURCE_AUDIT,
            "name": "audit",
            "title": "审计日志",
            "description": "最近的审计记录（JSON，最多 100 条）",
            "mimeType": "application/json"
        }),
    ];
    for session in state.list_sessions()? {
        resources.push(json!({
            "uri": session_output_uri(&session.id),
            "name": format!("session-{}-output", session.id),
            "title": format!("{} 输出", session.connection_alias),
            "description": "会话缓冲输出（纯文本）",
            "mimeType": "text/plain"
        }));
    }
    Ok(resources)
}

pub fn resource_templates() -> Vec<Value> {
    vec![json!({
        "uriTemplate": SESSION_OUTPUT_TEMPLATE,
        "name": "session-output",
        "title": "会话输出",
        "description": "按会话 ID 读取缓冲输出",
        "mimeType": "text/plain"
    })]
}

/// Resolve a `resources/read` URI. Unknown URIs are reported as resource
/// errors by the transport.
pub fn read_resource(state: &AppState, uri: &str) -> Result<Value, String> {
    if uri == RESOURCE_SESSIONS {
        let sessions = state.list_sessions()?;
        let text = serde_json::to_string_pretty(&json!({ "sessions": sessions }))
            .map_err(|error| error.to_string())?;
        return Ok(resource_text(uri, "application/json", text));
    }
    if uri == RESOURCE_AUDIT {
        let entries = state.list_audit(100);
        let text = serde_json::to_string_pretty(&json!({ "entries": entries }))
            .map_err(|error| error.to_string())?;
        return Ok(resource_text(uri, "application/json", text));
    }
    if let Some(rest) = uri.strip_prefix("shellx://sessions/") {
        let Some(session_id) = rest.strip_suffix("/output") else {
            return Err(format!("未知资源：{uri}"));
        };
        if session_id.is_empty() || session_id.contains('/') {
            return Err(format!("无效的会话资源：{uri}"));
        }
        let text = state.session_output_text(session_id)?;
        return Ok(resource_text(uri, "text/plain", text));
    }
    Err(format!("未知资源：{uri}"))
}

fn resource_text(uri: &str, mime_type: &str, text: String) -> Value {
    json!({
        "contents": [{
            "uri": uri,
            "mimeType": mime_type,
            "text": text
        }]
    })
}

/// Dispatch one MCP `tools/call`. Policy, approval and audit always apply
/// because execution goes through `AppState::run_command`.
///
/// The stdio transport has no dialog, so commands that need human approval are
/// rejected (and audited) instead of hanging forever.
pub async fn dispatch_tool(
    state: &AppState,
    sftp: &SftpManager,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    match name {
        "list_servers" => {
            let servers: Vec<Value> = state
                .list_connections()?
                .into_iter()
                .filter(|connection| connection.mcp_access.is_authorized())
                .map(|connection| {
                    json!({
                        "id": connection.id,
                        "alias": connection.alias,
                        "group": connection.group,
                        "environment": connection.environment,
                        "mcpAccess": connection.mcp_access,
                        "tags": connection.tags
                    })
                })
                .collect();
            Ok(json!({ "servers": servers }))
        }
        "exec_command" => {
            let connection_id = string_arg(&arguments, "connection_id", "connectionId")?;
            let command = string_arg(&arguments, "command", "command")?;
            let timeout_secs = arguments
                .get("timeout_secs")
                .or_else(|| arguments.get("timeoutSecs"))
                .and_then(Value::as_u64);
            let wait = arguments
                .get("wait")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let start = state
                .start_command(CommandRequest {
                    connection_id: connection_id.to_string(),
                    command: command.to_string(),
                    source: AuditSource::Agent,
                    timeout_secs,
                })
                .await?;
            match start {
                CommandStart::Ready(outcome) => finish_outcome(state, outcome).await,
                CommandStart::Started {
                    session_id,
                    completion,
                } => {
                    if !wait {
                        return Ok(json!({
                            "sessionId": session_id,
                            "running": true
                        }));
                    }
                    let outcome = completion
                        .await
                        .map_err(|_| "命令后台任务意外退出".to_string())??;
                    finish_outcome(state, outcome).await
                }
            }
        }
        "read_output" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let offset = arguments
                .get("offset")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(200) as usize;
            let page = state.read_session_output(session_id, offset, limit)?;
            Ok(json!({
                "sessionId": session_id,
                "offset": offset,
                "nextOffset": page.next_offset,
                "eof": page.eof,
                "running": page.running,
                "exitCode": page.exit_code,
                "output": page.text
            }))
        }
        "send_signal" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let signal = string_arg(&arguments, "signal", "signal")?;
            state.send_signal(session_id, signal).await?;
            Ok(json!({
                "sessionId": session_id,
                "signal": signal,
                "sent": true
            }))
        }
        "close_session" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            if state.close_session(session_id).await? {
                Ok(json!({ "sessionId": session_id, "closed": true }))
            } else {
                Err(format!("会话 {session_id} 不存在"))
            }
        }
        "sftp_open" => {
            let connection_id = string_arg(&arguments, "connection_id", "connectionId")?;
            let connection =
                state.authorize_sftp(connection_id, false, "打开 SFTP 会话")?;
            let params = state.sftp_params(connection_id)?;
            let session_id = Uuid::new_v4().to_string();
            let opened = sftp
                .open(session_id.clone(), connection_id.to_string(), params)
                .await?;
            state.record_agent_event(
                Some(&connection.id),
                format!(
                    "MCP 打开 SFTP 会话（{}，{} 个条目）",
                    opened.listing.path,
                    opened.listing.entries.len()
                ),
            );
            serde_json::to_value(opened).map_err(|error| error.to_string())
        }
        "sftp_close" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let connection_id = sftp.connection_id(session_id);
            let closed = sftp.close(session_id).await;
            if closed {
                if let Some(connection_id) = connection_id {
                    state.record_agent_event(
                        Some(&connection_id),
                        format!("MCP 关闭 SFTP 会话 {session_id}"),
                    );
                }
            }
            Ok(json!({ "sessionId": session_id, "closed": closed }))
        }
        "sftp_list" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(&connection_id, false, "列出 SFTP 目录")?;
            let path = arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".")
                .to_string();
            let listing = sftp.list(session_id, path).await?;
            serde_json::to_value(listing).map_err(|error| error.to_string())
        }
        "sftp_stat" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(&connection_id, false, "读取 SFTP 元信息")?;
            let path = string_arg(&arguments, "path", "path")?.to_string();
            let entry = sftp.stat(session_id, path).await?;
            serde_json::to_value(entry).map_err(|error| error.to_string())
        }
        "sftp_read_file" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(&connection_id, false, "读取 SFTP 文件")?;
            let path = string_arg(&arguments, "path", "path")?.to_string();
            let max_bytes = arguments
                .get("max_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(64 * 1024)
                .clamp(1, 512 * 1024) as usize;
            let file = sftp.read_file(session_id, path.clone(), max_bytes).await?;
            state.record_agent_event(
                Some(&connection_id),
                format!(
                    "MCP 读取 SFTP 文件 {path}（{} 字节{}）",
                    file.size,
                    if file.truncated { "，已截断" } else { "" }
                ),
            );
            serde_json::to_value(file).map_err(|error| error.to_string())
        }
        "sftp_write_file" => {
            const MAX_WRITE_BYTES: usize = 1024 * 1024;
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let path = string_arg(&arguments, "path", "path")?.to_string();
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(
                &connection_id,
                true,
                &format!("写入 SFTP 文件 {path}"),
            )?;
            let content = string_arg(&arguments, "content", "content")?;
            let encoding = arguments
                .get("encoding")
                .and_then(Value::as_str)
                .unwrap_or("utf-8")
                .to_ascii_lowercase();
            let data = match encoding.as_str() {
                "utf-8" | "utf8" | "text" => content.as_bytes().to_vec(),
                "base64" => base64_decode(content)?,
                other => return Err(format!("不支持的 encoding：{other}")),
            };
            if data.len() > MAX_WRITE_BYTES {
                return Err(format!(
                    "写入内容 {} 字节，超过 {} 字节上限",
                    data.len(),
                    MAX_WRITE_BYTES
                ));
            }
            let bytes = sftp.write_file(session_id, path.clone(), data).await?;
            state.record_agent_event(
                Some(&connection_id),
                format!("MCP 写入 SFTP 文件 {path}（{bytes} 字节）"),
            );
            Ok(json!({
                "sessionId": session_id,
                "path": path,
                "bytesWritten": bytes
            }))
        }
        "sftp_mkdir" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let path = string_arg(&arguments, "path", "path")?.to_string();
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(
                &connection_id,
                true,
                &format!("创建 SFTP 目录 {path}"),
            )?;
            sftp.create_dir(session_id, path.clone()).await?;
            state.record_agent_event(
                Some(&connection_id),
                format!("MCP 创建 SFTP 目录 {path}"),
            );
            Ok(json!({ "sessionId": session_id, "path": path, "created": true }))
        }
        "sftp_remove" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let path = string_arg(&arguments, "path", "path")?.to_string();
            let is_dir = arguments
                .get("is_dir")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let recursive = arguments
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_dir && !recursive {
                return Err("删除目录需要显式传入 recursive=true".into());
            }
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(
                &connection_id,
                true,
                &format!("删除 SFTP 路径 {path}"),
            )?;
            sftp.remove(session_id, path.clone(), is_dir).await?;
            state.record_agent_event(
                Some(&connection_id),
                format!(
                    "MCP 删除 SFTP {} {path}",
                    if is_dir { "目录" } else { "文件" }
                ),
            );
            Ok(json!({ "sessionId": session_id, "path": path, "removed": true }))
        }
        "sftp_rename" => {
            let session_id = string_arg(&arguments, "session_id", "sessionId")?;
            let old_path = string_arg(&arguments, "old_path", "oldPath")?.to_string();
            let new_path = string_arg(&arguments, "new_path", "newPath")?.to_string();
            let connection_id = sftp_connection(sftp, session_id)?;
            state.authorize_sftp(
                &connection_id,
                true,
                &format!("重命名 SFTP 路径 {old_path} -> {new_path}"),
            )?;
            sftp.rename(session_id, old_path.clone(), new_path.clone())
                .await?;
            state.record_agent_event(
                Some(&connection_id),
                format!("MCP 重命名 SFTP 路径 {old_path} -> {new_path}"),
            );
            Ok(json!({
                "sessionId": session_id,
                "oldPath": old_path,
                "newPath": new_path,
                "renamed": true
            }))
        }
        other => Err(format!("未知 MCP 工具: {other}")),
    }
}

/// Turn a policy outcome into a tool result. Approval is impossible over
/// stdio, so pending requests are denied (and audited) instead of hanging.
async fn finish_outcome(
    state: &AppState,
    outcome: CommandOutcome,
) -> Result<Value, String> {
    match outcome {
        CommandOutcome::Completed { result } => {
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        CommandOutcome::Blocked {
            message, audit_id, ..
        } => Err(format!(
            "命令被策略引擎拦截：{message}（审计 ID：{audit_id}）"
        )),
        CommandOutcome::PendingApproval { approval } => {
            let reason = "MCP stdio 模式没有人工审批界面，已自动拒绝";
            let _ = state.deny_approval(&approval.id, reason).await?;
            Err(format!(
                "命令需要人工审批（{}），但 stdio 传输无法弹出审批窗口，已自动拒绝。\
                 请在桌面端确认后执行，或改用低风险命令。",
                approval.reasons.join("；")
            ))
        }
    }
}

fn sftp_connection(sftp: &SftpManager, session_id: &str) -> Result<String, String> {
    sftp.connection_id(session_id)
        .ok_or_else(|| format!("SFTP 会话 {session_id} 不存在或已关闭"))
}

fn string_arg<'a>(
    arguments: &'a Value,
    name: &str,
    alias: &str,
) -> Result<&'a str, String> {
    arguments
        .get(name)
        .or_else(|| arguments.get(alias))
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少 {name} 参数"))
}
