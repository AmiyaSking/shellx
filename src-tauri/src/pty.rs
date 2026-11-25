//! PTY session registry and event bridge.
//!
//! This is the layer between the SSH transport (`ssh.rs`) and the Tauri
//! commands: it owns a small tokio runtime, keeps the live sessions, and pumps
//! remote output into frontend events.
//!
//! Why a private runtime instead of Tauri's: `russh` needs a reactor with
//! `net`/`io`/`time` enabled, and we must not depend on which tokio features
//! Tauri happens to build with today. The runtime is boxed and leaked on
//! purpose, so it is never dropped inside an async context.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::encoding::base64_encode;
use crate::ssh::{ConnectParams, HostKey, PtyConn, PtyEvent, SshError};

pub const EVENT_OUTPUT: &str = "pty:output";
pub const EVENT_EXIT: &str = "pty:exit";
pub const EVENT_CLOSED: &str = "pty:closed";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOutputPayload {
    pub session_id: String,
    /// base64 of the raw bytes; the terminal decodes them itself so a split
    /// multi-byte character is not mangled.
    pub data: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyExitPayload {
    pub session_id: String,
    pub exit_code: Option<u32>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyClosedPayload {
    pub session_id: String,
    pub reason: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOpened {
    pub session_id: String,
    pub alias: String,
    pub host_key: HostKey,
}

struct PtyEntry {
    conn: Arc<PtyConn>,
    task: JoinHandle<()>,
}

/// Holds every live PTY session.
#[derive(Default)]
pub struct PtyManager {
    entries: Mutex<HashMap<String, PtyEntry>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_count(&self) -> usize {
        self.entries.lock().map(|guard| guard.len()).unwrap_or(0)
    }

    pub fn is_open(&self, session_id: &str) -> bool {
        self.entries
            .lock()
            .map(|guard| guard.contains_key(session_id))
            .unwrap_or(false)
    }

    /// Connect, open the shell and start pumping output to the frontend.
    pub async fn open(
        &self,
        app: AppHandle,
        session_id: String,
        params: ConnectParams,
    ) -> Result<HostKey, String> {
        let (conn, mut reader) = run(async move { PtyConn::open(params).await }).await?;
        let conn = Arc::new(conn);
        let host_key = conn.host_key().clone();

        let pump_id = session_id.clone();
        let pump_app = app.clone();
        let task = runtime()?.spawn(async move {
            loop {
                match reader.next_event().await {
                    PtyEvent::Data(bytes) => {
                        let payload = PtyOutputPayload {
                            session_id: pump_id.clone(),
                            data: base64_encode(&bytes),
                        };
                        // A failed emit means the window is gone: stop reading.
                        if pump_app.emit(EVENT_OUTPUT, payload).is_err() {
                            break;
                        }
                    }
                    PtyEvent::Exit(exit_code) => {
                        let _ = pump_app.emit(
                            EVENT_EXIT,
                            PtyExitPayload {
                                session_id: pump_id.clone(),
                                exit_code: Some(exit_code),
                            },
                        );
                    }
                    PtyEvent::Closed => {
                        let _ = pump_app.emit(
                            EVENT_CLOSED,
                            PtyClosedPayload {
                                session_id: pump_id.clone(),
                                reason: "远端已关闭会话".to_string(),
                            },
                        );
                        break;
                    }
                }
            }
        });

        let mut entries = lock(&self.entries)?;
        if let Some(previous) = entries.insert(session_id, PtyEntry { conn, task }) {
            previous.task.abort();
        }
        Ok(host_key)
    }

    pub async fn write(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        let conn = self.conn(session_id)?;
        run(async move { conn.write(&data).await }).await
    }

    pub async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let conn = self.conn(session_id)?;
        run(async move { conn.resize(cols, rows).await }).await
    }

    pub async fn signal(&self, session_id: &str, signal: String) -> Result<(), String> {
        let conn = self.conn(session_id)?;
        run(async move { conn.signal(&signal).await }).await
    }

    /// Returns `true` when a session was actually closed.
    pub async fn close(&self, session_id: &str) -> Result<bool, String> {
        let entry = lock(&self.entries)?.remove(session_id);
        let Some(entry) = entry else {
            return Ok(false);
        };
        let conn = entry.conn.clone();
        let _ = run(async move {
            conn.close().await;
            Ok::<(), SshError>(())
        })
        .await;
        entry.task.abort();
        Ok(true)
    }

    /// Close everything on window close / app exit.
    pub async fn close_all(&self) -> Result<usize, String> {
        let drained: Vec<(String, PtyEntry)> = {
            let mut guard = lock(&self.entries)?;
            guard.drain().collect()
        };
        let count = drained.len();
        for (_, entry) in drained {
            let conn = entry.conn.clone();
            let _ = run(async move {
                conn.close().await;
                Ok::<(), SshError>(())
            })
            .await;
            entry.task.abort();
        }
        Ok(count)
    }

    fn conn(&self, session_id: &str) -> Result<Arc<PtyConn>, String> {
        lock(&self.entries)?
            .get(session_id)
            .map(|entry| entry.conn.clone())
            .ok_or_else(|| format!("会话 {session_id} 不存在或已断开"))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "PTY 会话表内部状态损坏".to_string())
}

/// The process-wide PTY runtime, leaked so it outlives every session.
fn runtime() -> Result<&'static Runtime, String> {
    static CELL: Mutex<Option<&'static Runtime>> = Mutex::new(None);
    let mut guard = CELL
        .lock()
        .map_err(|_| "PTY 运行时锁不可用".to_string())?;
    if let Some(existing) = *guard {
        return Ok(existing);
    }
    let created: &'static Runtime = Box::leak(Box::new(
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("shellx-pty")
            .enable_all()
            .build()
            .map_err(|error| format!("无法创建 PTY 运行时：{error}"))?,
    ));
    *guard = Some(created);
    Ok(created)
}

/// Run one `russh` operation on the PTY runtime and wait for its result.
///
/// Everything that touches the network goes through here, so the Tauri
/// command handlers never block the UI thread and never need `block_on`
/// inside an async context.
async fn run<T, F>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, SshError>> + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    runtime()?.spawn(async move {
        let _ = sender.send(future.await);
    });
    match receiver.await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => Err("PTY 后台任务已退出".to_string()),
    }
}

/// How long the UI waits for a socket close before giving up.
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_starts_empty() {
        let manager = PtyManager::new();
        assert_eq!(manager.active_count(), 0);
        assert!(!manager.is_open("nope"));
    }

    #[test]
    fn unknown_session_reports_a_readable_error() {
        let manager = PtyManager::new();
        let error = manager.conn("missing").unwrap_err();
        assert!(error.contains("missing"));
    }

    #[tokio::test]
    async fn closing_an_unknown_session_is_not_an_error() {
        let manager = PtyManager::new();
        assert!(!manager.close("ghost").await.unwrap());
    }
}
