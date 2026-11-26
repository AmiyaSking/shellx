// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // MCP clients launch the binary with `--mcp-stdio`; in that mode we speak
    // JSON-RPC on stdin/stdout and never initialize the GUI.
    if std::env::args().skip(1).any(|arg| arg == "--mcp-stdio") {
        if let Err(error) = shellx_lib::run_mcp_stdio() {
            eprintln!("[shellx] MCP stdio 退出：{error}");
            std::process::exit(1);
        }
        return;
    }

    shellx_lib::run()
}
