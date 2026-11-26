mod audit;
mod commands;
mod connections;
mod encoding;
mod mcp;
mod models;
mod policy;
mod pty;
mod redaction;
mod sessions;
mod shell;
mod ssh;
mod state;
mod vault;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let state = AppState::load(data_dir)?;
            app.manage(state);
            app.manage(pty::PtyManager::new());
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
            commands::pty_open,
            commands::pty_cancel,
            commands::pty_write,
            commands::pty_resize,
            commands::pty_signal,
            commands::pty_close,
            commands::pty_close_all,
            commands::pty_active_sessions,
            commands::set_session_logging,
            commands::forget_host_key,
        ])
        .build(tauri::generate_context!())
        .expect("error while building ShellX");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app_handle.state::<pty::PtyManager>().shutdown();
        }
    });
}
