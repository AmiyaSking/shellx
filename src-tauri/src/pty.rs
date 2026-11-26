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
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use encoding_rs::{Decoder, Encoding, UTF_8};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinHandle;

use crate::encoding::base64_encode;
use crate::ssh::{ConnectParams, HostKey, PtyConn, PtyEvent, SshError};

pub const EVENT_OUTPUT: &str = "pty:output";
pub const EVENT_EXIT: &str = "pty:exit";
pub const EVENT_CLOSED: &str = "pty:closed";

/// How long a TCP connect + handshake + auth may take before we give up.
/// Keyboard-interactive/2FA waits for the user, so this is generous; the UI
/// can cancel at any time.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

/// Coalesce output for about half a 60 Hz frame. A busy command produces
/// thousands of tiny SSH packets per second; sending one IPC event per packet
/// swamps the webview and makes xterm.js re-render needlessly.
const OUTPUT_BATCH_WINDOW: Duration = Duration::from_millis(8);
/// Upper bound for one output event, so a single IPC message stays small.
const OUTPUT_BATCH_MAX_BYTES: usize = 64 * 1024;
/// Unacknowledged bytes allowed in flight to the webview. When the terminal
/// falls behind, the pump stops reading; russh's bounded channel fills and TCP
/// backpressure eventually throttles the remote process instead of growing an
/// unbounded queue in the UI.
const OUTPUT_IN_FLIGHT_LIMIT: usize = 1024 * 1024;

/// Byte-budget flow control between the SSH reader and xterm.
///
/// The pump reserves bytes before emitting an event; the frontend returns them
/// with `pty_ack` once xterm has parsed that batch. A slow renderer therefore
/// applies real backpressure all the way to the remote PTY.
struct OutputFlow {
    in_flight: AtomicUsize,
    wake: Notify,
    closed: AtomicBool,
}

impl OutputFlow {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            wake: Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// Wait until `bytes` fit in the budget, then reserve them.
    async fn reserve(&self, bytes: usize) {
        let bytes = bytes.min(OUTPUT_IN_FLIGHT_LIMIT);
        if bytes == 0 {
            return;
        }
        loop {
            // Register before re-checking the budget: `notify_one` stores a
            // permit, so an ACK racing with this wait is not lost.
            let notified = self.wake.notified();
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            let current = self.in_flight.load(Ordering::Acquire);
            if current + bytes <= OUTPUT_IN_FLIGHT_LIMIT
                && self
                    .in_flight
                    .compare_exchange_weak(
                        current,
                        current + bytes,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
            {
                return;
            }
            notified.await;
        }
    }

    /// Return `bytes` to the budget after the terminal consumed them.
    fn ack(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let bytes = bytes.min(OUTPUT_IN_FLIGHT_LIMIT);
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            let next = current.saturating_sub(bytes);
            match self.in_flight.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        self.wake.notify_one();
    }

    /// Release a pump blocked on the budget when its session goes away.
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.wake.notify_one();
    }
}

/// Output accumulated for the next IPC event.
struct OutputBatch {
    data: Vec<u8>,
    deadline: Option<tokio::time::Instant>,
}

impl OutputBatch {
    fn new() -> Self {
        Self {
            data: Vec::with_capacity(OUTPUT_BATCH_MAX_BYTES),
            deadline: None,
        }
    }

    /// Append decoded bytes. Returns true when the batch should be flushed now.
    fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        self.data.extend_from_slice(bytes);
        if self.deadline.is_none() {
            self.deadline = Some(tokio::time::Instant::now() + OUTPUT_BATCH_WINDOW);
        }
        self.data.len() >= OUTPUT_BATCH_MAX_BYTES
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        self.deadline
    }

    /// Reserve flow-control budget, base64-encode and emit one batch.
    /// Returns false when the window is gone and the pump should stop.
    async fn flush(
        &mut self,
        app: &AppHandle,
        session_id: &str,
        flow: &OutputFlow,
    ) -> bool {
        if self.data.is_empty() {
            self.deadline = None;
            return true;
        }
        flow.reserve(self.data.len()).await;
        let payload = PtyOutputPayload {
            session_id: session_id.to_string(),
            data: base64_encode(&self.data),
        };
        self.data.clear();
        self.deadline = None;
        app.emit(EVENT_OUTPUT, payload).is_ok()
    }
}

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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub connection_id: String,
    pub connection_alias: String,
}

/// The fingerprint dialog needs structured data, not a formatted error string.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOpenError {
    /// `host_key_unknown`, `host_key_changed` or `other`.
    pub kind: String,
    pub message: String,
    pub host_key: Option<HostKeyPrompt>,
}

impl PtyOpenError {
    pub fn from_ssh(error: &SshError) -> Self {
        match error {
            SshError::HostKeyUnknown {
                host,
                port,
                algorithm,
                fingerprint,
            } => Self {
                kind: "host_key_unknown".into(),
                message: error.to_string(),
                host_key: Some(HostKeyPrompt {
                    host: host.clone(),
                    port: *port,
                    algorithm: algorithm.clone(),
                    fingerprint: fingerprint.clone(),
                }),
            },
            SshError::HostKeyChanged {
                host,
                port,
                algorithm,
                fingerprint,
                ..
            } => Self {
                kind: "host_key_changed".into(),
                message: error.to_string(),
                host_key: Some(HostKeyPrompt {
                    host: host.clone(),
                    port: *port,
                    algorithm: algorithm.clone(),
                    fingerprint: fingerprint.clone(),
                }),
            },
            other => Self {
                kind: "other".into(),
                message: other.to_string(),
                host_key: None,
            },
        }
    }
}

struct PtyEntry {
    conn: Arc<PtyConn>,
    task: JoinHandle<()>,
    meta: SessionMeta,
    encoding: &'static Encoding,
    flow: Arc<OutputFlow>,
}

/// Holds every live PTY session and the in-flight connection attempts that can
/// still be cancelled from the UI.
#[derive(Default)]
pub struct PtyManager {
    entries: Mutex<HashMap<String, PtyEntry>>,
    cancellations: Mutex<HashMap<String, Arc<Notify>>>,
    session_logging: Arc<AtomicBool>,
}

fn close_session_log(log: &mut Option<BufWriter<File>>) {
    if let Some(mut writer) = log.take() {
        let _ = writeln!(writer, "\n# closed at {}", Utc::now().to_rfc3339());
        let _ = writer.flush();
    }
}

impl PtyManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_count(&self) -> usize {
        self.entries.lock().map(|guard| guard.len()).unwrap_or(0)
    }

    pub fn set_session_logging(&self, enabled: bool) {
        self.session_logging.store(enabled, Ordering::Relaxed);
    }

    #[allow(dead_code)] // used by tests; useful for a future status indicator
    pub fn session_logging_enabled(&self) -> bool {
        self.session_logging.load(Ordering::Relaxed)
    }

    #[allow(dead_code)] // kept for the verification harness and future session UI
    pub fn is_open(&self, session_id: &str) -> bool {
        self.entries
            .lock()
            .map(|guard| guard.contains_key(session_id))
            .unwrap_or(false)
    }

    /// Register an in-flight connection so `pty_cancel` can abort it.
    pub fn register_cancel(&self, request_id: &str) -> Result<Arc<Notify>, String> {
        let notify = Arc::new(Notify::new());
        lock(&self.cancellations)?.insert(request_id.to_string(), notify.clone());
        Ok(notify)
    }

    /// Returns `true` when a matching in-flight attempt was signalled.
    pub fn cancel(&self, request_id: &str) -> bool {
        let notify = self
            .cancellations
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(request_id));
        if let Some(notify) = notify {
            // notify_one stores a permit when nobody is waiting yet, so a
            // cancel racing with the start of the connect still takes effect.
            notify.notify_one();
            true
        } else {
            false
        }
    }

    pub fn clear_cancel(&self, request_id: &str) {
        if let Ok(mut guard) = self.cancellations.lock() {
            guard.remove(request_id);
        }
    }

    /// Connect, open the shell and start pumping output to the frontend.
    pub async fn open(
        &self,
        app: AppHandle,
        session_id: String,
        meta: SessionMeta,
        params: ConnectParams,
        cancel: Arc<Notify>,
    ) -> Result<HostKey, SshError> {
        let encoding = resolve_encoding(&params.encoding);
        let log_path: Option<PathBuf> = app.path().app_data_dir().ok().map(|dir| {
            dir.join("sessions")
                .join(Utc::now().format("%Y-%m-%d").to_string())
                .join(format!("{session_id}.log"))
        });
        let log_header = format!(
            "# ShellX terminal session\n# id: {session_id}\n# connection: {} ({})\n# target: {}@{}:{}\n# started: {}\n",
            meta.connection_alias,
            meta.connection_id,
            params.username,
            params.host,
            params.port,
            Utc::now().to_rfc3339()
        );
        let logging = self.session_logging.clone();

        let (conn, mut reader) = run_ssh(async move {
            tokio::select! {
                result = tokio::time::timeout(OPEN_TIMEOUT, PtyConn::open(params)) => {
                    match result {
                        Ok(result) => result,
                        Err(_) => Err(SshError::Connect(format!(
                            "连接超时（{} 秒），已中止",
                            OPEN_TIMEOUT.as_secs()
                        ))),
                    }
                }
                _ = cancel.notified() => Err(SshError::Connect("连接已取消".into())),
            }
        })
        .await?;

        let conn = Arc::new(conn);
        let host_key = conn.host_key().clone();
        let flow = Arc::new(OutputFlow::new());

        let pump_id = session_id.clone();
        let pump_app = app.clone();
        let pump_flow = flow.clone();
        let runtime = runtime().map_err(SshError::Connect)?;
        let task = runtime.spawn(async move {
            let mut log: Option<BufWriter<File>> = None;
            let mut log_attempted = false;
            let mut decoder: Option<Decoder> = if std::ptr::eq(encoding, UTF_8) {
                None
            } else {
                Some(encoding.new_decoder())
            };
            let mut batch = OutputBatch::new();

            loop {
                let event = match batch.deadline() {
                    Some(deadline) => {
                        tokio::select! {
                            event = reader.next_event() => event,
                            _ = tokio::time::sleep_until(deadline) => {
                                if !batch.flush(&pump_app, &pump_id, &pump_flow).await {
                                    break;
                                }
                                continue;
                            }
                        }
                    }
                    None => reader.next_event().await,
                };

                match event {
                    PtyEvent::Data(bytes) => {
                        if logging.load(Ordering::Relaxed) && !log_attempted {
                            log_attempted = true;
                            if let Some(path) = &log_path {
                                if let Some(parent) = path.parent() {
                                    let _ = fs::create_dir_all(parent);
                                }
                                match OpenOptions::new()
                                    .create(true)
                                    .append(true)
                                    .open(path)
                                {
                                    Ok(mut file) => {
                                        let _ = writeln!(file, "{log_header}");
                                        log = Some(BufWriter::new(file));
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "[shellx] 无法创建会话日志 {}: {error}",
                                            path.display()
                                        );
                                    }
                                }
                            }
                        }
                        if let Some(writer) = log.as_mut() {
                            let _ = writer.write_all(&bytes);
                        }

                        let output = if let Some(decoder) = decoder.as_mut() {
                            decode_chunk(decoder, &bytes)
                        } else {
                            bytes
                        };
                        if batch.push(&output)
                            && !batch.flush(&pump_app, &pump_id, &pump_flow).await
                        {
                            break;
                        }
                    }
                    PtyEvent::Exit(exit_code) => {
                        append_decoder_tail(&mut batch, decoder.as_mut());
                        if !batch.flush(&pump_app, &pump_id, &pump_flow).await {
                            break;
                        }
                        let _ = pump_app.emit(
                            EVENT_EXIT,
                            PtyExitPayload {
                                session_id: pump_id.clone(),
                                exit_code: Some(exit_code),
                            },
                        );
                        close_session_log(&mut log);
                    }
                    PtyEvent::Closed => {
                        append_decoder_tail(&mut batch, decoder.as_mut());
                        // A failed emit means the window is gone; there is
                        // still nothing left to deliver on close.
                        let _ = batch.flush(&pump_app, &pump_id, &pump_flow).await;
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

            pump_flow.close();
            close_session_log(&mut log);
        });

        let mut entries = lock(&self.entries).map_err(SshError::Connect)?;
        if let Some(previous) = entries.insert(
            session_id,
            PtyEntry {
                conn,
                task,
                meta,
                encoding,
                flow,
            },
        ) {
            previous.flow.close();
            previous.task.abort();
        }
        Ok(host_key)
    }

    pub async fn write(&self, session_id: &str, data: Vec<u8>) -> Result<(), String> {
        let (conn, encoding) = {
            let guard = lock(&self.entries)?;
            let entry = guard
                .get(session_id)
                .ok_or_else(|| format!("会话 {session_id} 不存在或已断开"))?;
            (entry.conn.clone(), entry.encoding)
        };
        let data = encode_input(encoding, data);
        run(async move { conn.write(&data).await }).await
    }

    /// Return consumed output bytes to the session's flow-control budget.
    /// Unknown sessions are ignored because an ACK can race with `pty_close`.
    pub fn ack_output(&self, session_id: &str, bytes: usize) -> bool {
        let flow = self
            .entries
            .lock()
            .ok()
            .and_then(|guard| guard.get(session_id).map(|entry| entry.flow.clone()));
        match flow {
            Some(flow) => {
                flow.ack(bytes);
                true
            }
            None => false,
        }
    }

    pub async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let conn = self.conn(session_id)?;
        run(async move { conn.resize(cols, rows).await }).await
    }

    pub async fn signal(&self, session_id: &str, signal: String) -> Result<(), String> {
        let conn = self.conn(session_id)?;
        run(async move { conn.signal(&signal).await }).await
    }

    /// Close a session and return the metadata the caller can audit.
    pub async fn close(&self, session_id: &str) -> Result<Option<SessionMeta>, String> {
        let entry = lock(&self.entries)?.remove(session_id);
        let Some(entry) = entry else {
            return Ok(None);
        };
        let conn = entry.conn.clone();
        let _ = run(async move {
            conn.close().await;
            Ok::<(), SshError>(())
        })
        .await;
        entry.flow.close();
        entry.task.abort();
        Ok(Some(entry.meta))
    }

    /// Close everything on app exit. Synchronous: the process is going away and
    /// there is no runtime left to await on.
    pub fn shutdown(&self) -> usize {
        let drained: Vec<(String, PtyEntry)> = self
            .entries
            .lock()
            .map(|mut guard| guard.drain().collect())
            .unwrap_or_default();
        let count = drained.len();
        for (_, entry) in drained {
            entry.flow.close();
            entry.task.abort();
            drop(entry.conn);
        }
        if let Ok(mut guard) = self.cancellations.lock() {
            for notify in guard.drain().map(|(_, notify)| notify) {
                notify.notify_one();
            }
        }
        count
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
/// Port forwarding reuses it for its listeners and tunnel tasks.
pub(crate) fn runtime() -> Result<&'static Runtime, String> {
    static CELL: Mutex<Option<&'static Runtime>> = Mutex::new(None);
    let mut guard = CELL.lock().map_err(|_| "PTY 运行时锁不可用".to_string())?;
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

/// Run one `russh` operation on the PTY runtime and preserve its typed error.
async fn run_ssh<T, F>(future: F) -> Result<T, SshError>
where
    F: std::future::Future<Output = Result<T, SshError>> + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    let runtime = runtime().map_err(SshError::Connect)?;
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    match receiver.await {
        Ok(result) => result,
        Err(_) => Err(SshError::Connect("PTY 后台任务已退出".into())),
    }
}

/// String-flavoured wrapper for commands that do not need structured errors.
async fn run<T, F>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, SshError>> + Send + 'static,
    T: Send + 'static,
{
    run_ssh(future).await.map_err(|error| error.to_string())
}

fn resolve_encoding(label: &str) -> &'static Encoding {
    Encoding::for_label(label.trim().as_bytes()).unwrap_or(UTF_8)
}

fn encode_input(encoding: &'static Encoding, data: Vec<u8>) -> Vec<u8> {
    if std::ptr::eq(encoding, UTF_8) {
        return data;
    }
    let text = String::from_utf8_lossy(&data);
    let (encoded, _, _) = encoding.encode(&text);
    encoded.into_owned()
}

fn decode_chunk(decoder: &mut Decoder, bytes: &[u8]) -> Vec<u8> {
    // encoding_rs writes into the String's spare capacity; an empty String has
    // none. Reserve for the worst case of three UTF-8 bytes per input byte
    // (unmappable/invalid input becomes U+FFFD).
    let mut text = String::with_capacity(bytes.len() * 4 + 16);
    let (_result, _read, _replaced) =
        decoder.decode_to_string(bytes, &mut text, false);
    text.into_bytes()
}

fn flush_decoder(decoder: &mut Decoder) -> Vec<u8> {
    let mut text = String::with_capacity(16);
    let (_result, _read, _replaced) = decoder.decode_to_string(&[], &mut text, true);
    text.into_bytes()
}

fn append_decoder_tail(batch: &mut OutputBatch, decoder: Option<&mut Decoder>) {
    let Some(decoder) = decoder else {
        return;
    };
    let tail = flush_decoder(decoder);
    if !tail.is_empty() {
        // The batch is flushed immediately afterwards. If this happens to
        // exceed the soft cap, one event simply carries a few extra bytes.
        let _ = batch.push(&tail);
    }
}

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
        let error = manager.conn("missing").err().unwrap();
        assert!(error.contains("missing"));
    }

    #[tokio::test]
    async fn closing_an_unknown_session_is_not_an_error() {
        let manager = PtyManager::new();
        assert!(manager.close("ghost").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancellation_is_consumed_once() {
        let manager = PtyManager::new();
        let notify = manager.register_cancel("request-1").unwrap();
        let waiter = notify.notified();
        assert!(manager.cancel("request-1"));
        waiter.await;
        assert!(!manager.cancel("request-1"));
        manager.clear_cancel("request-1");
    }

    #[test]
    fn shutdown_drains_sessions_and_cancellations() {
        let manager = PtyManager::new();
        manager.register_cancel("pending-1").unwrap();
        assert_eq!(manager.shutdown(), 0);
        assert!(!manager.cancel("pending-1"));
    }

    #[test]
    fn gbk_input_and_output_roundtrip_across_chunks() {
        let encoding = resolve_encoding("gbk");
        let utf8 = "中文".as_bytes().to_vec();
        let encoded = encode_input(encoding, utf8.clone());
        assert_ne!(encoded, utf8, "GBK bytes differ from UTF-8 bytes");

        let mut decoder = encoding.new_decoder();
        let mut output = decode_chunk(&mut decoder, &encoded[..1]);
        output.extend(decode_chunk(&mut decoder, &encoded[1..]));
        output.extend(flush_decoder(&mut decoder));
        assert_eq!(String::from_utf8(output).unwrap(), "中文");
    }

    #[test]
    fn unknown_encoding_falls_back_to_utf8() {
        assert!(std::ptr::eq(resolve_encoding("not-a-real-encoding"), UTF_8));
        let input = "hello".as_bytes().to_vec();
        assert_eq!(encode_input(resolve_encoding("not-a-real-encoding"), input.clone()), input);
    }

    #[test]
    fn session_logging_toggle_is_shared_with_sessions() {
        let manager = PtyManager::new();
        assert!(!manager.session_logging_enabled());
        manager.set_session_logging(true);
        assert!(manager.session_logging_enabled());
        manager.set_session_logging(false);
        assert!(!manager.session_logging_enabled());
    }

    #[test]
    fn output_batches_coalesce_until_the_frame_deadline() {
        let mut batch = OutputBatch::new();
        assert!(!batch.push(b"a"));
        assert!(batch.deadline().is_some());
        assert!(!batch.push(b"bc"));
        assert_eq!(batch.data, b"abc");

        // Reaching the soft cap asks the pump to flush immediately.
        assert!(batch.push(&vec![0u8; OUTPUT_BATCH_MAX_BYTES]));
        assert_eq!(batch.data.len(), OUTPUT_BATCH_MAX_BYTES + 3);
    }

    #[test]
    fn empty_output_does_not_start_a_batch() {
        let mut batch = OutputBatch::new();
        assert!(!batch.push(b""));
        assert!(batch.deadline().is_none());
        assert!(batch.data.is_empty());
    }

    #[tokio::test]
    async fn flow_control_blocks_until_the_terminal_acks() {
        let flow = Arc::new(OutputFlow::new());
        flow.reserve(OUTPUT_IN_FLIGHT_LIMIT).await;

        let waiter = {
            let flow = flow.clone();
            tokio::spawn(async move { flow.reserve(1024).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished(), "budget is exhausted");

        flow.ack(1024);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("reservation should unblock after an ACK")
            .expect("reserve task should not panic");
    }

    #[tokio::test]
    async fn duplicate_acks_do_not_underflow_the_budget() {
        let flow = OutputFlow::new();
        flow.reserve(2048).await;
        flow.ack(4096);
        assert_eq!(flow.in_flight.load(Ordering::Acquire), 0);

        // The full budget is available again.
        flow.reserve(OUTPUT_IN_FLIGHT_LIMIT).await;
        assert_eq!(
            flow.in_flight.load(Ordering::Acquire),
            OUTPUT_IN_FLIGHT_LIMIT
        );
    }

    #[tokio::test]
    async fn closing_the_flow_releases_a_blocked_pump() {
        let flow = Arc::new(OutputFlow::new());
        flow.reserve(OUTPUT_IN_FLIGHT_LIMIT).await;

        let waiter = {
            let flow = flow.clone();
            tokio::spawn(async move { flow.reserve(1).await })
        };
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        flow.close();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("close should wake a blocked reservation")
            .expect("reserve task should not panic");
    }

    #[test]
    fn ack_for_an_unknown_session_is_ignored() {
        let manager = PtyManager::new();
        assert!(!manager.ack_output("missing", 1024));
    }

    #[test]
    fn host_key_unknown_error_carries_the_fingerprint() {
        let error = SshError::HostKeyUnknown {
            host: "example.com".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:abc".into(),
        };
        let open_error = PtyOpenError::from_ssh(&error);
        assert_eq!(open_error.kind, "host_key_unknown");
        let prompt = open_error.host_key.expect("prompt payload");
        assert_eq!(prompt.host, "example.com");
        assert_eq!(prompt.port, 22);
        assert_eq!(prompt.fingerprint, "SHA256:abc");
    }
}
