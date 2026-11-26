mod audit;
mod auth_broker;
mod commands;
mod connections;
mod encoding;
mod forwards;
mod mcp;
mod mcp_access;
mod mcp_pty;
mod mcp_stdio;
mod models;
mod policy;
mod pty;
mod redaction;
mod sessions;
mod sftp;
#[cfg(test)]
mod ssh_test_server;
mod shell;
mod ssh;
mod ssh_config;
mod state;
mod vault;

use tauri::Manager;

use state::AppState;

/// Headless MCP server over stdin/stdout (`shellx --mcp-stdio`).
///
/// It deliberately does not start Tauri or open a window: MCP clients spawn
/// this process and speak JSON-RPC over its standard streams.
pub fn run_mcp_stdio() -> Result<(), String> {
    mcp_stdio::run()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::load(data_dir)?;
            app.manage(state);
            app.manage(pty::PtyManager::new());
            app.manage(forwards::ForwardManager::new());
            app.manage(sftp::SftpManager::new());
            app.manage(auth_broker::AuthBroker::new());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_connections,
            commands::save_connection,
            commands::delete_connection,
            commands::preview_command,
            commands::run_command,
            commands::resolve_approval,
            commands::list_audit_entries,
            commands::list_sessions,
            commands::close_session,
            commands::mcp_server_info,
            commands::mcp_launch_info,
            commands::get_mcp_policy,
            commands::save_mcp_policy,
            commands::pty_open,
            commands::pty_cancel,
            commands::auth_respond,
            commands::auth_cancel,
            commands::pty_write,
            commands::pty_ack,
            commands::pty_resize,
            commands::pty_signal,
            commands::pty_close,
            commands::pty_close_all,
            commands::pty_active_sessions,
            commands::set_session_logging,
            commands::forget_host_key,
            commands::preview_ssh_config,
            commands::list_forward_rules,
            commands::save_forward_rule,
            commands::delete_forward_rule,
            commands::start_forward,
            commands::stop_forward,
            commands::list_forward_statuses,
            commands::sftp_open,
            commands::sftp_list,
            commands::sftp_download,
            commands::sftp_upload,
            commands::sftp_mkdir,
            commands::sftp_remove,
            commands::sftp_rename,
            commands::sftp_cancel,
            commands::sftp_close,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ShellX");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app_handle.state::<pty::PtyManager>().shutdown();
            app_handle.state::<forwards::ForwardManager>().shutdown();
            app_handle.state::<sftp::SftpManager>().shutdown();
            app_handle.state::<auth_broker::AuthBroker>().shutdown();
        }
    });
}
