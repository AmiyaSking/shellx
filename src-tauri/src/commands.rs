use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::models::{
    AuditEntry, AuditSource, CommandOutcome, CommandRequest, Connection, ConnectionInput,
    HostKeyTrust, McpServerInfo, PolicyDecision, SessionInfo,
};
use crate::pty::{PtyManager, PtyOpenError, PtyOpened, SessionMeta};
use crate::ssh;
use crate::ssh_config::{self, SshConfigPreview};
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

fn open_error(message: impl Into<String>) -> PtyOpenError {
    PtyOpenError {
        kind: "other".into(),
        message: message.into(),
        host_key: None,
    }
}

#[tauri::command]
pub async fn pty_open(
    app: AppHandle,
    state: State<'_, AppState>,
    pty: State<'_, PtyManager>,
    connection_id: String,
    cols: u32,
    rows: u32,
    request_id: String,
    trusted_host_keys: Option<Vec<HostKeyTrust>>,
) -> Result<PtyOpened, PtyOpenError> {
    let (alias, params) = state
        .pty_params(
            &connection_id,
            cols,
            rows,
            trusted_host_keys.unwrap_or_default(),
        )
        .map_err(open_error)?;
    let target = format!("{}@{}:{}", params.username, params.host, params.port);
    let cancel = pty.register_cancel(&request_id).map_err(open_error)?;
    let session_id = Uuid::new_v4().to_string();
    let meta = SessionMeta {
        connection_id: connection_id.clone(),
        connection_alias: alias.clone(),
    };

    let result = pty
        .open(app, session_id.clone(), meta, params, cancel)
        .await;
    pty.clear_cancel(&request_id);

    match result {
        Ok(host_key) => {
            state.record_pty_event(
                Some(&connection_id),
                format!(
                    "PTY 会话已建立：{target}，主机密钥{}（{}）",
                    if host_key.known {
                        "已信任"
                    } else {
                        "首次记录"
                    },
                    host_key.fingerprint
                ),
            );
            Ok(PtyOpened {
                session_id,
                alias,
                host_key,
            })
        }
        Err(error) => {
            state.record_pty_event(
                Some(&connection_id),
                format!("PTY 连接失败：{target}：{error}"),
            );
            Err(PtyOpenError::from_ssh(&error))
        }
    }
}

#[tauri::command]
pub fn pty_cancel(pty: State<'_, PtyManager>, request_id: String) -> bool {
    pty.cancel(&request_id)
}

#[tauri::command]
pub fn forget_host_key(
    state: State<'_, AppState>,
    host: String,
    port: u16,
) -> Result<bool, String> {
    let removed = ssh::forget_known_host(&state.known_hosts_path(), &host, port)?;
    state.record_pty_event(
        None,
        format!("主机密钥信任已移除：{host}:{port}（移除 {removed} 条）"),
    );
    Ok(removed)
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
pub async fn pty_close(
    state: State<'_, AppState>,
    pty: State<'_, PtyManager>,
    session_id: String,
) -> Result<bool, String> {
    let meta = pty.close(&session_id).await?;
    if let Some(meta) = &meta {
        state.record_pty_event(
            Some(&meta.connection_id),
            format!("PTY 会话已关闭：{}", meta.connection_alias),
        );
    }
    Ok(meta.is_some())
}

#[tauri::command]
pub fn pty_close_all(
    state: State<'_, AppState>,
    pty: State<'_, PtyManager>,
) -> Result<usize, String> {
    let count = pty.shutdown();
    if count > 0 {
        state.record_pty_event(None, format!("已关闭全部 PTY 会话（{count} 个）"));
    }
    Ok(count)
}

#[tauri::command]
pub fn pty_active_sessions(pty: State<'_, PtyManager>) -> usize {
    pty.active_count()
}

#[tauri::command]
pub fn set_session_logging(pty: State<'_, PtyManager>, enabled: bool) {
    pty.set_session_logging(enabled);
}

#[tauri::command]
pub fn preview_ssh_config() -> Result<SshConfigPreview, String> {
    ssh_config::preview_default()
}
