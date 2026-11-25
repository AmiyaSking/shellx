use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::models::{
    AuditEntry, AuditSource, CommandOutcome, CommandRequest, Connection, ConnectionInput,
    McpServerInfo, PolicyDecision, SessionInfo,
};
use crate::pty::{PtyManager, PtyOpened};
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

// ---------------------------------------------------------------------------
// Interactive terminal (real SSH PTY, as opposed to the command/RPC path above)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn pty_open(
    app: AppHandle,
    state: State<'_, AppState>,
    pty: State<'_, PtyManager>,
    connection_id: String,
    cols: u32,
    rows: u32,
) -> Result<PtyOpened, String> {
    let (alias, params) = state.pty_params(&connection_id, cols, rows)?;
    let session_id = Uuid::new_v4().to_string();
    let host_key = pty.open(app, session_id.clone(), params).await?;
    Ok(PtyOpened {
        session_id,
        alias,
        host_key,
    })
}

#[tauri::command]
pub async fn pty_write(
    pty: State<'_, PtyManager>,
    session_id: String,
    data: String,
) -> Result<(), String> {
    pty.write(&session_id, data.into_bytes()).await
}

#[tauri::command]
pub async fn pty_resize(
    pty: State<'_, PtyManager>,
    session_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    pty.resize(&session_id, cols.max(1), rows.max(1)).await
}

#[tauri::command]
pub async fn pty_signal(
    pty: State<'_, PtyManager>,
    session_id: String,
    signal: String,
) -> Result<(), String> {
    pty.signal(&session_id, signal).await
}

#[tauri::command]
pub async fn pty_close(pty: State<'_, PtyManager>, session_id: String) -> Result<bool, String> {
    pty.close(&session_id).await
}

#[tauri::command]
pub async fn pty_close_all(pty: State<'_, PtyManager>) -> Result<usize, String> {
    pty.close_all().await
}

#[tauri::command]
pub fn pty_active_sessions(pty: State<'_, PtyManager>) -> usize {
    pty.active_count()
}
