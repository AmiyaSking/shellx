use serde_json::{json, Value};

use crate::models::{AuditSource, CommandRequest, McpToolInfo, RiskLevel};
use crate::state::AppState;

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
        description: "在已授权服务器上执行命令，受策略引擎与审批流约束",
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

/// JSON manifest for `tools/list`. Consumed once the stdio JSON-RPC loop is
/// wired in phase 1; kept here so the catalog has a single source of truth.
#[allow(dead_code)]
pub fn tool_manifest() -> Value {
    let tools: Vec<Value> = TOOL_CATALOG
        .iter()
        .map(|tool| {
            let schema = match tool.name {
                "exec_command" => json!({
                    "type": "object",
                    "properties": {
                        "connection_id": { "type": "string", "description": "已授权连接 ID" },
                        "command": { "type": "string", "description": "要执行的命令" },
                        "timeout_secs": { "type": "integer", "minimum": 1 }
                    },
                    "required": ["connection_id", "command"],
                    "additionalProperties": false
                }),
                "read_output" => json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" },
                        "offset": { "type": "integer", "minimum": 0 }
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
        .collect();

    json!({
        "protocolVersion": "2025-06-18",
        "tools": tools
    })
}

/// Skeleton dispatcher for MCP `tools/call`. The stdio transport will call this
/// once `--mcp-stdio` is implemented; policy and audit already apply because
/// every execution goes through `AppState::run_command`.
#[allow(dead_code)]
pub fn dispatch_tool(
    state: &AppState,
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
            let connection_id = arguments
                .get("connection_id")
                .or_else(|| arguments.get("connectionId"))
                .and_then(Value::as_str)
                .ok_or_else(|| "缺少 connection_id 参数".to_string())?;
            let command = arguments
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| "缺少 command 参数".to_string())?;
            let timeout_secs = arguments
                .get("timeout_secs")
                .or_else(|| arguments.get("timeoutSecs"))
                .and_then(Value::as_u64);

            let outcome = state.run_command(CommandRequest {
                connection_id: connection_id.to_string(),
                command: command.to_string(),
                source: AuditSource::Agent,
                timeout_secs,
            })?;
            serde_json::to_value(outcome).map_err(|error| error.to_string())
        }
        "read_output" | "send_signal" | "close_session" => {
            Err(format!("MCP 工具 {name} 将在第一阶段接入"))
        }
        other => Err(format!("未知 MCP 工具: {other}")),
    }
}
