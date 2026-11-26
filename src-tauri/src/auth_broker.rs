//! Global broker for keyboard-interactive / 2FA prompts.
//!
//! Terminal, forwarding and SFTP sessions all create their own authenticated
//! SSH connections. They share this broker so the frontend can show one global
//! authentication dialog regardless of which subsystem started the connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use crate::ssh::{AuthPromptInfo, AuthPrompter, SshError};

pub const EVENT_AUTH_PROMPT: &str = "auth:prompt";
pub const EVENT_AUTH_CLOSED: &str = "auth:closed";
const AUTH_PROMPT_TIMEOUT: Duration = Duration::from_secs(110);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthPromptPayload {
    pub prompt_id: String,
    pub host: String,
    pub name: String,
    pub instructions: String,
    pub prompts: Vec<AuthPromptInfo>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthPromptClosed {
    prompt_id: String,
}

struct PendingAuth {
    sender: oneshot::Sender<Result<Vec<String>, String>>,
}

#[derive(Default)]
struct Inner {
    pending: Mutex<HashMap<String, PendingAuth>>,
}

/// Removes a pending prompt when its waiting future is dropped (connection
/// cancelled, tab closed, ...), so stale prompts do not linger for the timeout.
struct PendingGuard {
    inner: Arc<Inner>,
    app: AppHandle,
    prompt_id: String,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.inner.pending.lock() {
            guard.remove(&self.prompt_id);
        }
        let _ = self.app.emit(
            EVENT_AUTH_CLOSED,
            AuthPromptClosed {
                prompt_id: self.prompt_id.clone(),
            },
        );
    }
}

#[derive(Clone, Default)]
pub struct AuthBroker {
    inner: Arc<Inner>,
}

impl AuthBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn prompter(&self, app: AppHandle) -> AuthPrompter {
        let inner = self.inner.clone();
        AuthPrompter::new(move |host, name, instructions, prompts| {
            let app = app.clone();
            let inner = inner.clone();
            Box::pin(async move {
                let prompt_id = uuid::Uuid::new_v4().to_string();
                let (sender, receiver) = oneshot::channel();
                {
                    let mut guard = inner.pending.lock().map_err(|_| {
                        SshError::Unsupported("认证提示状态锁不可用".to_string())
                    })?;
                    guard.insert(prompt_id.clone(), PendingAuth { sender });
                }
                let _pending = PendingGuard {
                    inner: inner.clone(),
                    app: app.clone(),
                    prompt_id: prompt_id.clone(),
                };

                let payload = AuthPromptPayload {
                    prompt_id: prompt_id.clone(),
                    host,
                    name,
                    instructions,
                    prompts,
                };
                if app.emit(EVENT_AUTH_PROMPT, payload).is_err() {
                    return Err(SshError::Unsupported(
                        "无法在界面中显示认证提示".to_string(),
                    ));
                }

                match tokio::time::timeout(AUTH_PROMPT_TIMEOUT, receiver).await {
                    Ok(Ok(Ok(answers))) => Ok(answers),
                    Ok(Ok(Err(message))) => Err(SshError::Unsupported(message)),
                    Ok(Err(_)) => {
                        Err(SshError::Unsupported("认证请求已取消".to_string()))
                    }
                    Err(_) => Err(SshError::Unsupported(
                        "交互式认证等待超时".to_string(),
                    )),
                }
            })
        })
    }

    pub fn respond(&self, prompt_id: &str, responses: Vec<String>) -> bool {
        let entry = self
            .inner
            .pending
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(prompt_id));
        match entry {
            Some(entry) => entry.sender.send(Ok(responses)).is_ok(),
            None => false,
        }
    }

    pub fn cancel(&self, prompt_id: &str) -> bool {
        let entry = self
            .inner
            .pending
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(prompt_id));
        match entry {
            Some(entry) => {
                let _ = entry.sender.send(Err("用户取消了认证".to_string()));
                true
            }
            None => false,
        }
    }

    pub fn shutdown(&self) -> usize {
        let count = self
            .inner
            .pending
            .lock()
            .map(|mut guard| guard.drain().count())
            .unwrap_or(0);
        count
    }
}
