//! SFTP file browser sessions.
//!
//! Each session owns one authenticated SSH connection and one SFTP subsystem.
//! Listing is read-only; upload/download stream through tokio files so large
//! files are not buffered into memory.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use russh_sftp::client::SftpSession;
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

use crate::ssh::{self, AuthenticatedSession, ConnectParams};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpEntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<String>,
    pub permissions: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpListing {
    pub path: String,
    pub entries: Vec<SftpEntryInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpOpened {
    pub session_id: String,
    pub listing: SftpListing,
}

struct SftpEntry {
    sftp: Arc<SftpSession>,
    /// Keeps the SSH connection (and its jump-host transport) alive.
    _ssh: AuthenticatedSession,
    connection_id: String,
}

#[derive(Default)]
pub struct SftpManager {
    sessions: Mutex<HashMap<String, Arc<SftpEntry>>>,
}

impl SftpManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn open(
        &self,
        session_id: String,
        connection_id: String,
        params: ConnectParams,
    ) -> Result<SftpOpened, String> {
        let entry = on_runtime(async move {
            let ssh = match tokio::time::timeout(
                Duration::from_secs(20),
                ssh::connect_authenticated(&params),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => return Err("连接 SSH 服务器超时".to_string()),
            };

            let channel = ssh
                .handle
                .channel_open_session()
                .await
                .map_err(|error| format!("无法打开 SFTP 通道：{error}"))?;
            channel
                .request_subsystem(true, "sftp")
                .await
                .map_err(|error| format!("服务器拒绝 SFTP 子系统：{error}"))?;
            let sftp = SftpSession::new(channel.into_stream())
                .await
                .map_err(|error| format!("SFTP 初始化失败：{error}"))?;
            let path = sftp
                .canonicalize(".")
                .await
                .map_err(|error| format!("无法读取 SFTP 主目录：{error}"))?;
            let entries = list_entries(&sftp, &path).await?;

            Ok((
                SftpEntry {
                    sftp: Arc::new(sftp),
                    _ssh: ssh,
                    connection_id,
                },
                SftpListing { path, entries },
            ))
        })
        .await?;

        let (entry, listing) = entry;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "SFTP 会话表内部状态损坏".to_string())?;
        sessions.insert(session_id.clone(), Arc::new(entry));
        Ok(SftpOpened {
            session_id,
            listing,
        })
    }

    pub async fn list(&self, session_id: &str, path: String) -> Result<SftpListing, String> {
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            let path = if path.trim().is_empty() {
                ".".to_string()
            } else {
                path
            };
            let canonical = sftp
                .canonicalize(path)
                .await
                .map_err(|error| format!("无法打开目录：{error}"))?;
            let entries = list_entries(&sftp, &canonical).await?;
            Ok(SftpListing {
                path: canonical,
                entries,
            })
        })
        .await
    }

    pub async fn download(
        &self,
        session_id: &str,
        remote_path: String,
        local_path: String,
    ) -> Result<u64, String> {
        if local_path.trim().is_empty() {
            return Err("本地保存路径不能为空".into());
        }
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            let mut remote = sftp
                .open(remote_path)
                .await
                .map_err(|error| format!("无法打开远端文件：{error}"))?;
            let mut local = tokio::fs::File::create(&local_path)
                .await
                .map_err(|error| format!("无法创建本地文件：{error}"))?;
            let bytes = tokio::io::copy(&mut remote, &mut local)
                .await
                .map_err(|error| format!("下载失败：{error}"))?;
            let _ = local.flush().await;
            let _ = remote.close().await;
            Ok(bytes)
        })
        .await
    }

    pub async fn upload(
        &self,
        session_id: &str,
        local_path: String,
        remote_path: String,
    ) -> Result<u64, String> {
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            let mut local = tokio::fs::File::open(&local_path)
                .await
                .map_err(|error| format!("无法读取本地文件：{error}"))?;
            let mut remote = sftp
                .create(remote_path)
                .await
                .map_err(|error| format!("无法创建远端文件：{error}"))?;
            let bytes = tokio::io::copy(&mut local, &mut remote)
                .await
                .map_err(|error| format!("上传失败：{error}"))?;
            let _ = remote.close().await;
            Ok(bytes)
        })
        .await
    }

    pub async fn close(&self, session_id: &str) -> bool {
        let entry = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(session_id));
        let Some(entry) = entry else {
            return false;
        };
        let sftp = entry.sftp.clone();
        let _ = on_runtime(async move {
            sftp.close()
                .await
                .map_err(|error| format!("关闭 SFTP 会话失败：{error}"))
        })
        .await;
        true
    }

    pub fn connection_id(&self, session_id: &str) -> Option<String> {
        self.sessions
            .lock()
            .ok()
            .and_then(|guard| guard.get(session_id).map(|entry| entry.connection_id.clone()))
    }

    pub fn shutdown(&self) -> usize {
        let count = self
            .sessions
            .lock()
            .map(|mut guard| guard.drain().count())
            .unwrap_or(0);
        count
    }

    fn sftp(&self, session_id: &str) -> Result<Arc<SftpSession>, String> {
        self.sessions
            .lock()
            .map_err(|_| "SFTP 会话表内部状态损坏".to_string())?
            .get(session_id)
            .map(|entry| entry.sftp.clone())
            .ok_or_else(|| format!("SFTP 会话 {session_id} 不存在或已关闭"))
    }
}

async fn list_entries(
    sftp: &SftpSession,
    path: &str,
) -> Result<Vec<SftpEntryInfo>, String> {
    let read_dir = sftp
        .read_dir(path)
        .await
        .map_err(|error| format!("读取目录失败：{error}"))?;
    let mut entries: Vec<SftpEntryInfo> = read_dir
        .filter_map(|entry| {
            let file_type = entry.file_type();
            let metadata = entry.metadata();
            let name = entry.file_name();
            if name.is_empty() {
                return None;
            }
            Some(SftpEntryInfo {
                name,
                path: entry.path(),
                is_dir: file_type.is_dir(),
                is_symlink: file_type.is_symlink(),
                size: metadata.len(),
                modified: metadata.modified().ok().map(format_time),
                permissions: metadata.permissions.map(|permissions| permissions & 0o777),
            })
        })
        .collect();
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn format_time(time: SystemTime) -> String {
    DateTime::<Utc>::from(time).to_rfc3339()
}

async fn on_runtime<T, F>(future: F) -> Result<T, String>
where
    F: Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    let runtime = crate::pty::runtime()?;
    runtime.spawn(async move {
        let _ = sender.send(future.await);
    });
    match receiver.await {
        Ok(result) => result,
        Err(_) => Err("SFTP 后台任务已退出".to_string()),
    }
}
