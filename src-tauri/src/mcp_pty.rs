//! Headless interactive PTY sessions for the MCP stdio server.
//!
//! The GUI's `PtyManager` emits Tauri events and uses ACK-based backpressure,
//! which does not exist in the headless process. These sessions reuse the same
//! `ssh::PtyConn` transport but buffer a bounded tail of raw output that MCP
//! tools can poll. The feature is opt-in per connection because an interactive
//! shell bypasses command-level policy.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::task::JoinHandle;

use crate::ssh::{ConnectParams, HostKey, PtyConn, PtyEvent};

/// Upper bound on concurrent MCP terminals per stdio process.
const MAX_SESSIONS: usize = 8;
/// Raw terminal output kept per session (ANSI bytes included).
const MAX_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOpened {
    pub session_id: String,
    pub connection_id: String,
    pub cols: u32,
    pub rows: u32,
    pub host_key: HostKey,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutput {
    pub session_id: String,
    /// Decoded with the connection's configured encoding; ANSI escapes are
    /// preserved so agents can see exactly what a terminal would show.
    pub output: String,
    pub running: bool,
    /// Older output has been dropped from the buffer (or this page skipped it).
    pub truncated: bool,
    pub received_bytes: u64,
}

pub struct McpPtyManager {
    sessions: Mutex<HashMap<String, Arc<PtySession>>>,
}

struct PtySession {
    connection_id: String,
    encoding: String,
    conn: PtyConn,
    output: Arc<Mutex<OutputBuffer>>,
    exited: Arc<AtomicBool>,
    received: Arc<AtomicU64>,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Default)]
struct OutputBuffer {
    bytes: VecDeque<u8>,
    truncated: bool,
}

impl OutputBuffer {
    fn push(&mut self, data: &[u8]) {
        self.bytes.extend(data.iter().copied());
        if self.bytes.len() > MAX_BUFFER_BYTES {
            let excess = self.bytes.len() - MAX_BUFFER_BYTES;
            self.bytes.drain(..excess);
            self.truncated = true;
        }
    }
}

impl Default for McpPtyManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

impl McpPtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn open(
        &self,
        session_id: String,
        connection_id: String,
        params: ConnectParams,
    ) -> Result<TerminalOpened, String> {
        {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "终端会话表内部状态损坏".to_string())?;
            if sessions.len() >= MAX_SESSIONS {
                return Err(format!(
                    "MCP 交互式终端数量已达上限（{MAX_SESSIONS}），请先关闭其他会话"
                ));
            }
        }

        let cols = params.cols;
        let rows = params.rows;
        let encoding = params.encoding.clone();
        let (conn, mut reader) = PtyConn::open(params)
            .await
            .map_err(|error| format!("打开交互式终端失败：{error}"))?;
        let host_key = conn.host_key().clone();

        let output = Arc::new(Mutex::new(OutputBuffer::default()));
        let exited = Arc::new(AtomicBool::new(false));
        let received = Arc::new(AtomicU64::new(0));
        let task = tokio::spawn({
            let output = output.clone();
            let exited = exited.clone();
            let received = received.clone();
            async move {
                while let PtyEvent::Data(bytes) = reader.next_event().await {
                    received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    if let Ok(mut buffer) = output.lock() {
                        buffer.push(&bytes);
                    }
                }
                exited.store(true, Ordering::Relaxed);
            }
        });

        let session = Arc::new(PtySession {
            connection_id: connection_id.clone(),
            encoding,
            conn,
            output,
            exited,
            received,
            task: Mutex::new(Some(task)),
        });
        self.sessions
            .lock()
            .map_err(|_| "终端会话表内部状态损坏".to_string())?
            .insert(session_id.clone(), session);

        Ok(TerminalOpened {
            session_id,
            connection_id,
            cols,
            rows,
            host_key,
        })
    }

    /// Send already-encoded bytes (control characters, escape sequences).
    pub async fn write(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        let session = self.session(session_id)?;
        if session.exited.load(Ordering::Relaxed) {
            return Err("交互式终端已经结束".into());
        }
        session
            .conn
            .write(&data)
            .await
            .map_err(|error| error.to_string())
    }

    /// Encode text with the connection's configured encoding and send it.
    pub async fn write_text(&self, session_id: &str, text: &str) -> Result<(), String> {
        let session = self.session(session_id)?;
        if session.exited.load(Ordering::Relaxed) {
            return Err("交互式终端已经结束".into());
        }
        let encoding = encoding_rs::Encoding::for_label(session.encoding.as_bytes())
            .unwrap_or(encoding_rs::UTF_8);
        let (encoded, _, _) = encoding.encode(text);
        session
            .conn
            .write(&encoded)
            .await
            .map_err(|error| error.to_string())
    }

    pub fn read(&self, session_id: &str, max_bytes: usize) -> Result<TerminalOutput, String> {
        let session = self.session(session_id)?;
        let max_bytes = max_bytes.clamp(1, MAX_BUFFER_BYTES);
        let buffer = session
            .output
            .lock()
            .map_err(|_| "终端输出缓冲不可用".to_string())?;
        let skip = buffer.bytes.len().saturating_sub(max_bytes);
        let bytes: Vec<u8> = buffer.bytes.iter().skip(skip).copied().collect();
        let truncated = buffer.truncated || skip > 0;
        drop(buffer);

        let encoding = encoding_rs::Encoding::for_label(session.encoding.as_bytes())
            .unwrap_or(encoding_rs::UTF_8);
        let (text, _, _) = encoding.decode(&bytes);
        Ok(TerminalOutput {
            session_id: session_id.to_string(),
            output: text.into_owned(),
            running: !session.exited.load(Ordering::Relaxed),
            truncated,
            received_bytes: session.received.load(Ordering::Relaxed),
        })
    }

    pub async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let session = self.session(session_id)?;
        session
            .conn
            .resize(cols, rows)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn signal(&self, session_id: &str, signal: &str) -> Result<(), String> {
        let session = self.session(session_id)?;
        session
            .conn
            .signal(signal)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn close(&self, session_id: &str) -> bool {
        let session = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(session_id));
        let Some(session) = session else {
            return false;
        };
        if let Ok(mut task) = session.task.lock() {
            if let Some(task) = task.take() {
                task.abort();
            }
        }
        session.exited.store(true, Ordering::Relaxed);
        session.conn.close().await;
        true
    }

    pub fn connection_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(session_id)
                    .map(|session| session.connection_id.clone())
            })
    }

    pub fn shutdown(&self) -> usize {
        let sessions: Vec<Arc<PtySession>> = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.drain().map(|(_, session)| session).collect())
            .unwrap_or_default();
        for session in &sessions {
            if let Ok(mut task) = session.task.lock() {
                if let Some(task) = task.take() {
                    task.abort();
                }
            }
            session.exited.store(true, Ordering::Relaxed);
        }
        sessions.len()
    }

    fn session(&self, session_id: &str) -> Result<Arc<PtySession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "终端会话表内部状态损坏".to_string())?
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("终端会话 {session_id} 不存在或已关闭"))
    }
}
