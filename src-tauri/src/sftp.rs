//! SFTP file browser sessions.
//!
//! Each session owns one authenticated SSH connection and one SFTP subsystem.
//! Listing is read-only; upload/download stream through tokio files so large
//! files are not buffered into memory.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use russh_sftp::client::SftpSession;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot;

use crate::ssh::{self, AuthenticatedSession, ConnectParams};

pub const EVENT_PROGRESS: &str = "sftp:progress";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SftpProgressEvent {
    pub session_id: String,
    pub direction: String,
    pub transferred: u64,
    pub total: u64,
}

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
        app: AppHandle,
        session_id: &str,
        remote_path: String,
        local_path: String,
    ) -> Result<u64, String> {
        if local_path.trim().is_empty() {
            return Err("本地保存路径不能为空".into());
        }
        let sftp = self.sftp(session_id)?;
        let session_id = session_id.to_string();
        on_runtime(async move {
            let total = sftp
                .metadata(remote_path.clone())
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let mut remote = sftp
                .open(remote_path)
                .await
                .map_err(|error| format!("无法打开远端文件：{error}"))?;
            let mut local = tokio::fs::File::create(&local_path)
                .await
                .map_err(|error| format!("无法创建本地文件：{error}"))?;
            let bytes = copy_with_progress(
                &mut remote,
                &mut local,
                total,
                &app,
                &session_id,
                "download",
            )
            .await?;
            let _ = remote.close().await;
            Ok(bytes)
        })
        .await
    }

    pub async fn upload(
        &self,
        app: AppHandle,
        session_id: &str,
        local_path: String,
        remote_path: String,
    ) -> Result<u64, String> {
        let sftp = self.sftp(session_id)?;
        let session_id = session_id.to_string();
        on_runtime(async move {
            let total = tokio::fs::metadata(&local_path)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let mut local = tokio::fs::File::open(&local_path)
                .await
                .map_err(|error| format!("无法读取本地文件：{error}"))?;
            let mut remote = sftp
                .create(remote_path)
                .await
                .map_err(|error| format!("无法创建远端文件：{error}"))?;
            let bytes = copy_with_progress(
                &mut local,
                &mut remote,
                total,
                &app,
                &session_id,
                "upload",
            )
            .await?;
            let _ = remote.close().await;
            Ok(bytes)
        })
        .await
    }

    pub async fn create_dir(&self, session_id: &str, path: String) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("目录路径不能为空".into());
        }
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            sftp.create_dir(path)
                .await
                .map_err(|error| format!("创建目录失败：{error}"))
        })
        .await
    }

    pub async fn remove(
        &self,
        session_id: &str,
        path: String,
        is_dir: bool,
    ) -> Result<(), String> {
        if path.trim().is_empty() {
            return Err("路径不能为空".into());
        }
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            if is_dir {
                sftp.remove_dir(path)
                    .await
                    .map_err(|error| format!("删除目录失败（目录必须为空）：{error}"))
            } else {
                sftp.remove_file(path)
                    .await
                    .map_err(|error| format!("删除文件失败：{error}"))
            }
        })
        .await
    }

    pub async fn rename(
        &self,
        session_id: &str,
        old_path: String,
        new_path: String,
    ) -> Result<(), String> {
        if old_path.trim().is_empty() || new_path.trim().is_empty() {
            return Err("路径不能为空".into());
        }
        let sftp = self.sftp(session_id)?;
        on_runtime(async move {
            sftp.rename(old_path, new_path)
                .await
                .map_err(|error| format!("重命名失败：{error}"))
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

async fn copy_with_progress<R, W>(
    reader: &mut R,
    writer: &mut W,
    total: u64,
    app: &AppHandle,
    session_id: &str,
    direction: &str,
) -> Result<u64, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0u8; 64 * 1024];
    let mut transferred = 0u64;
    let mut last_emit = Instant::now();
    emit_progress(app, session_id, direction, 0, total);

    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("传输读取失败：{error}"))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("传输写入失败：{error}"))?;
        transferred += read as u64;
        if last_emit.elapsed() >= Duration::from_millis(120) || transferred == total {
            emit_progress(app, session_id, direction, transferred, total);
            last_emit = Instant::now();
        }
    }

    let _ = writer.flush().await;
    emit_progress(app, session_id, direction, transferred, total);
    Ok(transferred)
}

fn emit_progress(
    app: &AppHandle,
    session_id: &str,
    direction: &str,
    transferred: u64,
    total: u64,
) {
    let _ = app.emit(
        EVENT_PROGRESS,
        SftpProgressEvent {
            session_id: session_id.to_string(),
            direction: direction.to_string(),
            transferred,
            total,
        },
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{connect_authenticated, Credential};
    use crate::ssh_test_server::{
        start_sftp_test_server, temp_known_hosts, trusted_params, TestAuthMode,
        TestSshServer,
    };
    use std::path::PathBuf;

    fn seed_root() -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "shellx-sftp-it-{}",
            uuid::Uuid::new_v4()
        ));
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("sub")).expect("create SFTP root");
        std::fs::write(root.join("hello.txt"), b"hello").expect("seed hello.txt");
        std::fs::write(root.join("sub").join("nested.txt"), b"nested")
            .expect("seed nested.txt");
        (dir, root)
    }

    async fn open_session(
        manager: &SftpManager,
        server: &TestSshServer,
        known_hosts: &std::path::Path,
        session_id: &str,
    ) -> SftpOpened {
        let params = trusted_params(
            server,
            known_hosts,
            Credential::Password("secret".into()),
        )
        .await;
        manager
            .open(session_id.into(), "conn-1".into(), params)
            .await
            .expect("open SFTP session")
    }

    fn entry<'a>(listing: &'a SftpListing, name: &str) -> &'a SftpEntryInfo {
        listing
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "missing entry {name}, got {:?}",
                    listing
                        .entries
                        .iter()
                        .map(|entry| entry.name.as_str())
                        .collect::<Vec<_>>()
                )
            })
    }

    #[tokio::test]
    async fn real_sftp_lists_and_mutates_remote_files() {
        let (dir, root) = seed_root();
        let server =
            start_sftp_test_server(TestAuthMode::Password("secret".into()), root.clone())
                .await;
        let (known_dir, known_hosts) = temp_known_hosts();
        let manager = SftpManager::new();

        let opened = open_session(&manager, &server, &known_hosts, "sftp-it-1").await;
        assert_eq!(opened.listing.path, "/");
        let hello = entry(&opened.listing, "hello.txt");
        assert!(!hello.is_dir);
        assert_eq!(hello.size, 5);
        assert_eq!(hello.path, "/hello.txt");
        let sub = entry(&opened.listing, "sub");
        assert!(sub.is_dir);
        assert_eq!(sub.path, "/sub");

        let nested = manager
            .list("sftp-it-1", "/sub".into())
            .await
            .expect("list sub directory");
        assert_eq!(nested.path, "/sub");
        assert_eq!(entry(&nested, "nested.txt").size, 6);

        manager
            .create_dir("sftp-it-1", "/created".into())
            .await
            .expect("create directory");
        assert!(root.join("created").is_dir());

        manager
            .rename("sftp-it-1", "/created".into(), "/renamed".into())
            .await
            .expect("rename directory");
        assert!(root.join("renamed").is_dir());

        manager
            .remove("sftp-it-1", "/renamed".into(), true)
            .await
            .expect("remove directory");
        assert!(!root.join("renamed").exists());

        manager
            .remove("sftp-it-1", "/hello.txt".into(), false)
            .await
            .expect("remove file");
        assert!(!root.join("hello.txt").exists());

        assert!(manager.close("sftp-it-1").await);
        assert!(
            manager.list("sftp-it-1", "/".into()).await.is_err(),
            "closed session must no longer be usable"
        );

        drop(server);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&known_dir).ok();
    }

    #[tokio::test]
    async fn real_sftp_round_trips_file_contents() {
        let (dir, root) = seed_root();
        let server =
            start_sftp_test_server(TestAuthMode::Password("secret".into()), root.clone())
                .await;
        let (known_dir, known_hosts) = temp_known_hosts();
        let params = trusted_params(
            &server,
            &known_hosts,
            Credential::Password("secret".into()),
        )
        .await;

        // Drive the same client session type the manager uses, so the
        // streaming upload/download data path is exercised over real SSH.
        let ssh = connect_authenticated(&params)
            .await
            .expect("connect SFTP transport");
        let channel = ssh
            .handle
            .channel_open_session()
            .await
            .expect("open session channel");
        channel
            .request_subsystem(true, "sftp")
            .await
            .expect("request sftp subsystem");
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .expect("start SFTP session");

        // A payload that spans several 32 KiB SFTP packets.
        let payload: Vec<u8> = (0..256 * 1024).map(|index| (index % 251) as u8).collect();

        let mut remote = sftp.create("/payload.bin").await.expect("create file");
        remote.write_all(&payload).await.expect("write file");
        remote.close().await.expect("close file");
        assert_eq!(
            std::fs::read(root.join("payload.bin")).expect("read uploaded file"),
            payload
        );

        let mut remote = sftp.open("/payload.bin").await.expect("open file");
        let mut read_back = Vec::new();
        remote
            .read_to_end(&mut read_back)
            .await
            .expect("read file back");
        assert_eq!(read_back, payload);
        remote.close().await.expect("close file");

        let listing: Vec<_> = sftp
            .read_dir("/")
            .await
            .expect("list root")
            .collect();
        let uploaded = listing
            .iter()
            .find(|entry| entry.file_name() == "payload.bin")
            .expect("uploaded file is listed");
        assert_eq!(uploaded.metadata().len() as usize, payload.len());

        sftp.close().await.expect("close SFTP session");
        drop(ssh);
        drop(server);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&known_dir).ok();
    }
}
