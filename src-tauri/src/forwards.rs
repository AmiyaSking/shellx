//! Local (-L) and dynamic SOCKS5 (-D) port forwarding.
//!
//! Each running rule owns one authenticated SSH connection and a local TCP
//! listener. Remote forwarding (-R) needs a server-initiated channel callback
//! in `ssh.rs` and is not part of this module yet.

use std::collections::HashMap;
use std::future::Future;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, JoinSet};

use crate::ssh::{self, AuthenticatedSession, ConnectParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRule {
    pub id: String,
    pub connection_id: String,
    pub kind: ForwardKind,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub remark: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRuleInput {
    #[serde(default)]
    pub id: Option<String>,
    pub connection_id: String,
    pub kind: ForwardKind,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardStatus {
    pub rule_id: String,
    pub running: bool,
    pub bound_port: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ForwardFile {
    #[serde(default)]
    rules: Vec<ForwardRule>,
}

/// Persistent forwarding rules; runtime state lives in `ForwardManager`.
pub struct ForwardStore {
    path: PathBuf,
    rules: Vec<ForwardRule>,
}

impl ForwardStore {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("forwards.json");
        let rules = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<ForwardFile>(&text).ok())
            .map(|file| file.rules)
            .unwrap_or_default();
        Self { path, rules }
    }

    pub fn list(&self) -> Vec<ForwardRule> {
        self.rules.clone()
    }

    pub fn get(&self, id: &str) -> Option<ForwardRule> {
        self.rules.iter().find(|rule| rule.id == id).cloned()
    }

    pub fn upsert(&mut self, input: ForwardRuleInput) -> Result<ForwardRule, String> {
        if input.connection_id.trim().is_empty() {
            return Err("必须选择一个连接".into());
        }
        if input.bind_host.trim().is_empty() {
            return Err("监听地址不能为空".into());
        }
        if input.kind != ForwardKind::Dynamic {
            if input.target_host.trim().is_empty() {
                return Err("该转发类型必须填写目标主机".into());
            }
            if input.target_port == 0 {
                return Err("目标端口必须大于 0".into());
            }
        }

        let existing = input
            .id
            .as_deref()
            .and_then(|id| self.rules.iter().position(|rule| rule.id == id));
        let rule = ForwardRule {
            id: existing
                .map(|index| self.rules[index].id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            connection_id: input.connection_id.trim().to_string(),
            kind: input.kind,
            bind_host: input.bind_host.trim().to_string(),
            bind_port: input.bind_port,
            target_host: if input.kind != ForwardKind::Dynamic {
                input.target_host.trim().to_string()
            } else {
                String::new()
            },
            target_port: if input.kind != ForwardKind::Dynamic {
                input.target_port
            } else {
                0
            },
            remark: input.remark.trim().to_string(),
        };

        match existing {
            Some(index) => self.rules[index] = rule.clone(),
            None => self.rules.push(rule.clone()),
        }
        self.persist()?;
        Ok(rule)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let before = self.rules.len();
        self.rules.retain(|rule| rule.id != id);
        if self.rules.len() == before {
            return Err("转发规则不存在".into());
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建数据目录: {error}"))?;
        }
        let json = serde_json::to_string_pretty(&ForwardFile {
            rules: self.rules.clone(),
        })
        .map_err(|error| format!("序列化转发规则失败: {error}"))?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, json)
            .map_err(|error| format!("写入转发规则失败: {error}"))?;
        std::fs::rename(&temp, &self.path)
            .map_err(|error| format!("替换转发规则失败: {error}"))?;
        Ok(())
    }
}

struct RunningForward {
    task: JoinHandle<()>,
    bound_port: u16,
    error: Arc<Mutex<Option<String>>>,
}

#[derive(Default)]
pub struct ForwardManager {
    running: Mutex<HashMap<String, RunningForward>>,
}

impl ForwardManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, rule_id: &str) -> ForwardStatus {
        if let Ok(guard) = self.running.lock() {
            if let Some(running) = guard.get(rule_id) {
                let error = running
                    .error
                    .lock()
                    .ok()
                    .and_then(|error| error.clone());
                return ForwardStatus {
                    rule_id: rule_id.to_string(),
                    running: true,
                    bound_port: Some(running.bound_port),
                    error,
                };
            }
        }
        ForwardStatus {
            rule_id: rule_id.to_string(),
            running: false,
            bound_port: None,
            error: None,
        }
    }

    pub fn statuses(&self, rules: &[ForwardRule]) -> Vec<ForwardStatus> {
        rules.iter().map(|rule| self.status(&rule.id)).collect()
    }

    pub async fn start(
        &self,
        rule: ForwardRule,
        params: ConnectParams,
    ) -> Result<ForwardStatus, String> {
        let existing = self.status(&rule.id);
        if existing.running {
            return Ok(existing);
        }
        if rule.kind == ForwardKind::Remote {
            return self.start_remote(rule, params).await;
        }

        let bind_host = rule.bind_host.clone();
        let bind_port = rule.bind_port;
        let listener = on_runtime(async move {
            TcpListener::bind((bind_host.as_str(), bind_port))
                .await
                .map_err(|error| {
                    format!("无法监听 {bind_host}:{bind_port}：{error}")
                })
        })
        .await?;
        let bound_port = listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or(rule.bind_port);

        let session = on_runtime(async move {
            match tokio::time::timeout(
                Duration::from_secs(20),
                ssh::connect_authenticated(&params),
            )
            .await
            {
                Ok(Ok(session)) => Ok(session),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err("连接 SSH 服务器超时".to_string()),
            }
        })
        .await?;
        let session = Arc::new(session);

        let error_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let task_error = error_slot.clone();
        let rule_for_task = rule.clone();
        let runtime = crate::pty::runtime()?;
        let task = runtime.spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((socket, peer)) => {
                                let session = session.clone();
                                let rule = rule_for_task.clone();
                                connections.spawn(async move {
                                    if let Err(error) = handle_connection(session, socket, peer, &rule).await {
                                        eprintln!("[shellx] 转发连接失败（{}）：{error}", rule.id);
                                    }
                                });
                            }
                            Err(error) => {
                                if let Ok(mut slot) = task_error.lock() {
                                    *slot = Some(format!("监听失败：{error}"));
                                }
                                break;
                            }
                        }
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });

        let mut running = self
            .running
            .lock()
            .map_err(|_| "端口转发表内部状态损坏".to_string())?;
        if let Some(previous) = running.insert(
            rule.id.clone(),
            RunningForward {
                task,
                bound_port,
                error: error_slot,
            },
        ) {
            previous.task.abort();
        }

        Ok(ForwardStatus {
            rule_id: rule.id,
            running: true,
            bound_port: Some(bound_port),
            error: None,
        })
    }

    async fn start_remote(
        &self,
        rule: ForwardRule,
        mut params: ConnectParams,
    ) -> Result<ForwardStatus, String> {
        let remote_address = rule.bind_host.clone();
        let remote_port = rule.bind_port as u32;
        let rule_id = rule.id.clone();
        params.remote_forwards = vec![ssh::RemoteForward {
            remote_address: remote_address.clone(),
            remote_port,
            local_host: rule.target_host.clone(),
            local_port: rule.target_port,
        }];

        let (session, bound_port) = on_runtime(async move {
            let session = match tokio::time::timeout(
                Duration::from_secs(20),
                ssh::connect_authenticated(&params),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Ok(Err(error)) => return Err(error.to_string()),
                Err(_) => return Err("连接 SSH 服务器超时".to_string()),
            };
            let bound_port = session
                .handle
                .tcpip_forward(remote_address.clone(), remote_port)
                .await
                .map_err(|error| {
                    format!(
                        "服务器拒绝远程转发 {remote_address}:{remote_port}：{error}"
                    )
                })?;
            Ok((session, bound_port))
        })
        .await?;

        if bound_port > u16::MAX as u32 {
            return Err("服务器返回了无效的转发端口".into());
        }
        let bound_port = bound_port as u16;
        let error_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let runtime = crate::pty::runtime()?;
        let task = runtime.spawn(async move {
            // Keep the authenticated connection (and its remote forward) alive
            // until the rule is stopped or the app exits.
            let _session = session;
            std::future::pending::<()>().await;
        });

        let mut running = self
            .running
            .lock()
            .map_err(|_| "端口转发表内部状态损坏".to_string())?;
        if let Some(previous) = running.insert(
            rule_id.clone(),
            RunningForward {
                task,
                bound_port,
                error: error_slot,
            },
        ) {
            previous.task.abort();
        }

        Ok(ForwardStatus {
            rule_id,
            running: true,
            bound_port: Some(bound_port),
            error: None,
        })
    }

    pub fn stop(&self, rule_id: &str) -> bool {
        let entry = self
            .running
            .lock()
            .ok()
            .and_then(|mut guard| guard.remove(rule_id));
        if let Some(entry) = entry {
            entry.task.abort();
            true
        } else {
            false
        }
    }

    pub fn shutdown(&self) -> usize {
        let drained: Vec<(String, RunningForward)> = self
            .running
            .lock()
            .map(|mut guard| guard.drain().collect())
            .unwrap_or_default();
        let count = drained.len();
        for (_, entry) in drained {
            entry.task.abort();
        }
        count
    }
}

async fn handle_connection(
    session: Arc<AuthenticatedSession>,
    mut socket: TcpStream,
    peer: SocketAddr,
    rule: &ForwardRule,
) -> Result<(), String> {
    let (target_host, target_port, dynamic) = match rule.kind {
        ForwardKind::Remote => {
            return Err("远程转发由服务器发起，不使用本地监听".to_string())
        }
        ForwardKind::Local => (
            rule.target_host.clone(),
            rule.target_port,
            false,
        ),
        ForwardKind::Dynamic => {
            match tokio::time::timeout(
                Duration::from_secs(10),
                socks5_negotiate(&mut socket),
            )
            .await
            {
                Ok(Ok(target)) => (target.0, target.1, true),
                Ok(Err(error)) => {
                    let _ = socks5_reply(&mut socket, 0x01).await;
                    return Err(error);
                }
                Err(_) => {
                    let _ = socks5_reply(&mut socket, 0x01).await;
                    return Err("SOCKS5 握手超时".to_string());
                }
            }
        }
    };

    let channel = session
        .handle
        .channel_open_direct_tcpip(
            target_host.clone(),
            target_port as u32,
            peer.ip().to_string(),
            peer.port() as u32,
        )
        .await
        .map_err(|error| {
            format!("无法打开到 {target_host}:{target_port} 的通道：{error}")
        });

    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            if dynamic {
                let _ = socks5_reply(&mut socket, 0x05).await;
            }
            return Err(error);
        }
    };

    if dynamic {
        socks5_reply(&mut socket, 0x00)
            .await
            .map_err(|error| format!("SOCKS5 应答失败：{error}"))?;
    }

    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
    Ok(())
}

/// Parse a SOCKS5 greeting and CONNECT request without sending the final reply.
async fn socks5_negotiate(socket: &mut TcpStream) -> Result<(String, u16), String> {
    let mut greeting = [0u8; 2];
    socket
        .read_exact(&mut greeting)
        .await
        .map_err(|error| format!("读取 SOCKS5 握手失败：{error}"))?;
    if greeting[0] != 5 {
        return Err("仅支持 SOCKS5".to_string());
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    socket
        .read_exact(&mut methods)
        .await
        .map_err(|error| format!("读取 SOCKS5 认证方法失败：{error}"))?;
    if !methods.contains(&0x00) {
        socket.write_all(&[5, 0xff]).await.ok();
        return Err("客户端不支持无认证 SOCKS5".to_string());
    }
    socket
        .write_all(&[5, 0x00])
        .await
        .map_err(|error| format!("写入 SOCKS5 应答失败：{error}"))?;

    let mut request = [0u8; 4];
    socket
        .read_exact(&mut request)
        .await
        .map_err(|error| format!("读取 SOCKS5 请求失败：{error}"))?;
    if request[0] != 5 || request[1] != 0x01 {
        let _ = socks5_reply(socket, 0x07).await;
        return Err("仅支持 SOCKS5 CONNECT".to_string());
    }

    let host = match request[3] {
        0x01 => {
            let mut bytes = [0u8; 4];
            socket
                .read_exact(&mut bytes)
                .await
                .map_err(|error| format!("读取 IPv4 目标失败：{error}"))?;
            Ipv4Addr::from(bytes).to_string()
        }
        0x03 => {
            let mut length = [0u8; 1];
            socket
                .read_exact(&mut length)
                .await
                .map_err(|error| format!("读取域名长度失败：{error}"))?;
            let mut domain = vec![0u8; length[0] as usize];
            socket
                .read_exact(&mut domain)
                .await
                .map_err(|error| format!("读取域名失败：{error}"))?;
            String::from_utf8(domain)
                .map_err(|_| "SOCKS5 域名不是有效 UTF-8".to_string())?
        }
        0x04 => {
            let mut bytes = [0u8; 16];
            socket
                .read_exact(&mut bytes)
                .await
                .map_err(|error| format!("读取 IPv6 目标失败：{error}"))?;
            Ipv6Addr::from(bytes).to_string()
        }
        _ => {
            let _ = socks5_reply(socket, 0x08).await;
            return Err("不支持的 SOCKS5 地址类型".to_string());
        }
    };

    let mut port = [0u8; 2];
    socket
        .read_exact(&mut port)
        .await
        .map_err(|error| format!("读取目标端口失败：{error}"))?;
    Ok((host, u16::from_be_bytes(port)))
}

async fn socks5_reply(socket: &mut TcpStream, code: u8) -> std::io::Result<()> {
    let response = [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
    socket.write_all(&response).await?;
    socket.flush().await
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
        Err(_) => Err("端口转发后台任务已退出".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_validate_targets() {
        let mut store = ForwardStore {
            path: PathBuf::from("unused.json"),
            rules: Vec::new(),
        };
        let error = store
            .upsert(ForwardRuleInput {
                id: None,
                connection_id: "conn".into(),
                kind: ForwardKind::Local,
                bind_host: "127.0.0.1".into(),
                bind_port: 8080,
                target_host: String::new(),
                target_port: 80,
                remark: String::new(),
            })
            .unwrap_err();
        assert!(error.contains("目标主机"));
    }

    #[test]
    fn dynamic_rules_ignore_target_fields() {
        let path = std::env::temp_dir().join(format!(
            "shellx-forward-test-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut store = ForwardStore {
            path: path.clone(),
            rules: Vec::new(),
        };
        let rule = store
            .upsert(ForwardRuleInput {
                id: None,
                connection_id: "conn".into(),
                kind: ForwardKind::Dynamic,
                bind_host: "127.0.0.1".into(),
                bind_port: 1080,
                target_host: "ignored".into(),
                target_port: 80,
                remark: String::new(),
            })
            .expect("rule");
        assert!(rule.target_host.is_empty());
        assert_eq!(rule.target_port, 0);
        assert_eq!(store.list().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn manager_starts_empty_and_stops_unknown_rules() {
        let manager = ForwardManager::new();
        assert!(!manager.status("missing").running);
        assert!(!manager.stop("missing"));
    }
}
