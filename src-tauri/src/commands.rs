use tauri::State;

use crate::models::{
    AuditEntry, AuditSource, CommandOutcome, CommandRequest, Connection, ConnectionInput,
    McpServerInfo, PolicyDecision, SessionInfo,
};
use crate::state::AppState;

#[tauri::command]
pub fn list_connections(state: State<'_, AppState>) -> Result<Vec<Connection>, String> {
    state.list_connections()
}

#[tauri::command]
pub fn save_connection(
    state: State<'_, AppState>,
    input: ConnectionInput,
) -> Result<Connection, String> {
    state.save_connection(input)
}

#[tauri::command]
pub fn delete_connection(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_connection(&id)
}

#[tauri::command]
pub fn preview_command(
    state: State<'_, AppState>,
    connection_id: String,
    command: String,
    source: Option<AuditSource>,
) -> Result<PolicyDecision, String> {
    state.preview_command(&connection_id, &command, source.unwrap_or_default())
}

#[tauri::command]
pub fn run_command(
    state: State<'_, AppState>,
    request: CommandRequest,
) -> Result<CommandOutcome, String> {
    state.run_command(request)
}

#[tauri::command]
pub fn resolve_approval(
    state: State<'_, AppState>,
    approval_id: String,
    approve: bool,
    command_override: Option<String>,
) -> Result<CommandOutcome, String> {
    state.resolve_approval(&approval_id, approve, command_override)
}

#[tauri::command]
pub fn list_audit_entries(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<AuditEntry>, String> {
    Ok(state.list_audit(limit.unwrap_or(100)))
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    state.list_sessions()
}

#[tauri::command]
pub fn close_session(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.close_session(&id)
}

#[tauri::command]
pub fn mcp_server_info(state: State<'_, AppState>) -> Result<McpServerInfo, String> {
    state.mcp_info()
}
