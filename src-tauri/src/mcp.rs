use serde_json::{json, Value};

use crate::models::{
    AuditSource, CommandOutcome, CommandRequest, McpToolInfo, RiskLevel,
};
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

/// Dispatch one MCP `tools/call`. Policy, approval and audit always apply
/// because execution goes through `AppState::run_command`.
///
/// The stdio transport has no dialog, so commands that need human approval are
/// rejected (and audited) instead of hanging forever.
pub async fn dispatch_tool(
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
