//! SSH transport: connection, host-key verification and an interactive PTY channel.
//!
//! This module deliberately has **no dependency on tauri**, so that
//! `.dev/verify-harness` can compile and test it against a real server.
//! Everything above it (event delivery, the session table, the approval flow)
//! lives in `pty.rs`.
//!
//! Host-key policy: trust-on-first-use *with* recording ("accept-new").
//! The first time a host is seen its fingerprint is appended to our own
//! `known_hosts` file; afterwards a changed key is a hard failure, never a prompt
//! we silently accept.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Msg};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::agent::AgentIdentity;
use russh::keys::{self as rkeys, ssh_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{
    Channel, ChannelMsg, ChannelOpenFailure, ChannelReadHalf, ChannelWriteHalf,
    Disconnect, Sig,
};

use crate::models::HostKeyTrust;

/// The terminal type we advertise to the server.
pub const DEFAULT_TERM: &str = "xterm-256color";

/// How we prove our identity to the server.
#[derive(Clone)]
pub enum Credential {
    Password(String),
    PrivateKey {
        path: PathBuf,
        passphrase: Option<String>,
    },
    /// Local SSH agent (OpenSSH agent / Pageant). The agent signs the
    /// authentication request; private key material never enters ShellX.
    Agent,
}

// Hand-written so a password can never leak into a log line through `{:?}`.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credential::Password(_) => f.write_str("Password(<redacted>)"),
            Credential::PrivateKey { path, passphrase } => f
                .debug_struct("PrivateKey")
                .field("path", path)
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
            Credential::Agent => f.write_str("Agent"),
        }
    }
}

/// Optional one-level jump host (ProxyJump equivalent).
#[derive(Clone, Debug)]
pub struct JumpHost {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
}

/// A remote (-R) forward: the server listens and sends channels to a local
/// target through this connection.
#[derive(Clone, Debug)]
pub struct RemoteForward {
    pub remote_address: String,
    pub remote_port: u32,
    pub local_host: String,
    pub local_port: u16,
}

#[derive(Clone, Debug)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
    pub jump: Option<JumpHost>,
    /// Remote forwarding targets for server-initiated channels.
    pub remote_forwards: Vec<RemoteForward>,
    /// Fingerprints the user already confirmed in the host-key dialog. A first
    /// connection without a matching entry fails with `HostKeyUnknown`.
    pub trusted_host_keys: Vec<HostKeyTrust>,
    /// Where our own known_hosts file lives (not ~/.ssh/known_hosts: we keep a
    /// separate file so ShellX never silently rewrites the user's OpenSSH state).
    pub known_hosts_path: PathBuf,
    pub term: String,
    /// Remote character encoding label understood by `encoding_rs`.
    pub encoding: String,
    /// Optional command typed into the shell right after login.
    pub startup_command: String,
    pub cols: u32,
    pub rows: u32,
}

impl ConnectParams {
    #[allow(dead_code)] // convenience constructor used by the verification harness
    pub fn new(host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            credential: Credential::Agent,
            jump: None,
            remote_forwards: Vec::new(),
            trusted_host_keys: Vec::new(),
            known_hosts_path: PathBuf::from("known_hosts"),
            term: DEFAULT_TERM.to_string(),
            encoding: "utf-8".to_string(),
            startup_command: String::new(),
            cols: 80,
            rows: 24,
        }
    }
}

fn trusted_fingerprint(trusted: &[HostKeyTrust], host: &str, port: u16) -> Option<String> {
    trusted
        .iter()
        .find_map(|entry| entry.fingerprint_for(host, port).map(str::to_string))
}

/// What we learned about the server's key during the handshake.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HostKey {
    /// e.g. `ssh-ed25519`
    pub algorithm: String,
    /// `SHA256:...` — the same form OpenSSH prints.
    pub fingerprint: String,
    /// `true` when the key was already in our known_hosts file.
    pub known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshError {
    Connect(String),
    HostKeyChanged {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
        detail: String,
    },
    HostKeyUnknown {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
    },
    KnownHosts(String),
    KeyFile(String),
    Auth {
        username: String,
        host: String,
        detail: String,
    },
    Channel(String),
    Unsupported(String),
}

impl fmt::Display for SshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SshError::Connect(detail) => write!(f, "无法连接到服务器：{detail}"),
            SshError::HostKeyChanged {
                host,
                port,
                fingerprint,
                detail,
                ..
            } => write!(
                f,
                "主机密钥已变更（{host}:{port}，当前指纹 {fingerprint}）：{detail}。为防中间人攻击已中止连接。"
            ),
            SshError::HostKeyUnknown {
                host,
                port,
                algorithm,
                fingerprint,
            } => write!(
                f,
                "首次连接 {host}:{port}，主机密钥（{algorithm}，{fingerprint}）尚未被信任，请确认指纹后重试。"
            ),
            SshError::KnownHosts(detail) => write!(f, "读取 known_hosts 失败：{detail}"),
            SshError::KeyFile(detail) => write!(f, "读取私钥失败：{detail}"),
            SshError::Auth {
                username,
                host,
                detail,
            } => write!(f, "{username}@{host} 认证失败：{detail}"),
            SshError::Channel(detail) => write!(f, "打开终端通道失败：{detail}"),
            SshError::Unsupported(detail) => write!(f, "暂不支持：{detail}"),
        }
    }
}

impl std::error::Error for SshError {}

/// One thing the remote side did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    Data(Vec<u8>),
    Exit(u32),
    Closed,
}

struct HostKeyState {
    host: String,
    port: u16,
    known_hosts_path: PathBuf,
    /// Fingerprint the user confirmed for this attempt, if any.
    expected_fingerprint: Option<String>,
    remote_forwards: Arc<Vec<RemoteForward>>,
    host_key: Option<HostKey>,
    /// Set when we refuse the server, so the real error survives `connect()`.
    rejection: Option<SshError>,
}

/// `russh` handler whose only job is to verify the server's key.
#[derive(Clone)]
pub struct HostKeyHandler {
    state: Arc<Mutex<HostKeyState>>,
}

impl HostKeyHandler {
    fn new(
        host: String,
        port: u16,
        known_hosts_path: PathBuf,
        expected_fingerprint: Option<String>,
        remote_forwards: Vec<RemoteForward>,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(HostKeyState {
                host,
                port,
                known_hosts_path,
                expected_fingerprint,
                remote_forwards: Arc::new(remote_forwards),
                host_key: None,
                rejection: None,
            })),
        }
    }

    fn snapshot(&self) -> (Option<HostKey>, Option<SshError>) {
        match self.state.lock() {
            Ok(guard) => (guard.host_key.clone(), guard.rejection.clone()),
            Err(_) => (
                None,
                Some(SshError::KnownHosts("主机密钥状态锁已失效".into())),
            ),
        }
    }
}

impl client::Handler for HostKeyHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            // The host-certificate path hands back the raw key data.
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let fingerprint = key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
        let algorithm = key.algorithm().to_string();

        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(false),
        };

        let known = match rkeys::known_hosts::check_known_hosts_path(
            &state.host,
            state.port,
            &key,
            &state.known_hosts_path,
        ) {
            Ok(known) => known,
            Err(error) => {
                // A recorded key with a different value: refuse, loudly. This
                // path is never overridable from the fingerprint dialog.
                state.rejection = Some(SshError::HostKeyChanged {
                    host: state.host.clone(),
                    port: state.port,
                    algorithm: algorithm.clone(),
                    fingerprint: fingerprint.clone(),
                    detail: error.to_string(),
                });
                return Ok(false);
            }
        };

        if !known {
            // Trust-on-first-use is only allowed after the UI showed this exact
            // fingerprint and the user confirmed it.
            let confirmed = state.expected_fingerprint.as_deref() == Some(fingerprint.as_str());
            if !confirmed {
                state.rejection = Some(SshError::HostKeyUnknown {
                    host: state.host.clone(),
                    port: state.port,
                    algorithm: algorithm.clone(),
                    fingerprint: fingerprint.clone(),
                });
                return Ok(false);
            }
            if let Err(error) = rkeys::known_hosts::learn_known_hosts_path(
                &state.host,
                state.port,
                &key,
                &state.known_hosts_path,
            ) {
                state.rejection = Some(SshError::KnownHosts(error.to_string()));
                return Ok(false);
            }
        }

        state.host_key = Some(HostKey {
            algorithm,
            fingerprint,
            known,
        });
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), russh::Error> {
        let target = match self.state.lock() {
            Ok(state) => state
                .remote_forwards
                .iter()
                .find(|forward| {
                    forward.remote_port == connected_port
                        && (forward.remote_address == connected_address
                            || forward.remote_address == "0.0.0.0"
                            || forward.remote_address == "::"
                            || connected_address == "0.0.0.0"
                            || connected_address == "::")
                })
                .or_else(|| {
                    state
                        .remote_forwards
                        .iter()
                        .find(|forward| forward.remote_port == connected_port)
                })
                .cloned(),
            Err(_) => None,
        };

        let Some(target) = target else {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        };

        reply.accept().await;
        let mut stream = channel.into_stream();
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect((
                target.local_host.as_str(),
                target.local_port,
            ))
            .await
            {
                Ok(mut socket) => {
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                }
                Err(error) => {
                    eprintln!(
                        "[shellx] 远程转发无法连接本地目标 {}:{}：{error}",
                        target.local_host, target.local_port
                    );
                }
            }
        });
        Ok(())
    }
}

/// Write side of an open PTY: cheap to share, used by the Tauri commands.
pub struct PtyConn {
    host_key: HostKey,
    write: ChannelWriteHalf<Msg>,
    session: Mutex<Option<client::Handle<HostKeyHandler>>>,
    /// Keeps the jump-host transport (and its direct-tcpip channel) alive for
    /// as long as the target session exists.
    _jump_session: Option<client::Handle<HostKeyHandler>>,
}

/// Read side of an open PTY: owns the receive half, driven by one task.
pub struct PtyReader {
    read: ChannelReadHalf,
}

/// An authenticated SSH connection without a terminal channel. Port
/// forwarding keeps this handle alive for the lifetime of its listeners.
pub struct AuthenticatedSession {
    pub handle: client::Handle<HostKeyHandler>,
    pub host_key: HostKey,
    pub jump_handle: Option<client::Handle<HostKeyHandler>>,
}

/// Connect, verify the host key and authenticate, without opening a shell.
pub async fn connect_authenticated(
    params: &ConnectParams,
) -> Result<AuthenticatedSession, SshError> {
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let target_handler = HostKeyHandler::new(
        params.host.clone(),
        params.port,
        params.known_hosts_path.clone(),
        trusted_fingerprint(&params.trusted_host_keys, &params.host, params.port),
        params.remote_forwards.clone(),
    );

    let mut jump_session: Option<client::Handle<HostKeyHandler>> = None;

    let connect_result = if let Some(jump) = &params.jump {
        let jump_handler = HostKeyHandler::new(
            jump.host.clone(),
            jump.port,
            params.known_hosts_path.clone(),
            trusted_fingerprint(&params.trusted_host_keys, &jump.host, jump.port),
            Vec::new(),
        );
        let mut jump_handle = match client::connect(
            config.clone(),
            (jump.host.as_str(), jump.port),
            jump_handler.clone(),
        )
        .await
        {
            Ok(handle) => handle,
            Err(error) => {
                let (_, rejection) = jump_handler.snapshot();
                return Err(rejection.unwrap_or_else(|| {
                    SshError::Connect(format!(
                        "无法连接跳板机 {}:{}：{error}",
                        jump.host, jump.port
                    ))
                }));
            }
        };

        let (_, rejection) = jump_handler.snapshot();
        if let Some(error) = rejection {
            return Err(error);
        }

        authenticate(
            &mut jump_handle,
            &jump.username,
            &jump.host,
            &jump.credential,
        )
        .await
        .map_err(|error| match error {
            SshError::Auth { detail, .. } => SshError::Auth {
                username: jump.username.clone(),
                host: jump.host.clone(),
                detail: format!("跳板机认证失败：{detail}"),
            },
            other => other,
        })?;

        let channel = jump_handle
            .channel_open_direct_tcpip(params.host.clone(), params.port as u32, "127.0.0.1", 0)
            .await
            .map_err(|error| {
                SshError::Channel(format!(
                    "无法通过跳板机打开到 {}:{} 的通道：{error}",
                    params.host, params.port
                ))
            })?;
        let stream = channel.into_stream();
        jump_session = Some(jump_handle);
        client::connect_stream(config.clone(), stream, target_handler.clone()).await
    } else {
        client::connect(
            config.clone(),
            (params.host.as_str(), params.port),
            target_handler.clone(),
        )
        .await
    };

    let mut session = match connect_result {
        Ok(session) => session,
        Err(error) => {
            // Prefer the reason we recorded over russh's generic version.
            let (_, rejection) = target_handler.snapshot();
            return Err(rejection.unwrap_or_else(|| SshError::Connect(error.to_string())));
        }
    };

    let (host_key, rejection) = target_handler.snapshot();
    if let Some(error) = rejection {
        return Err(error);
    }
    let host_key =
        host_key.ok_or_else(|| SshError::Connect("握手完成但没有收到主机公钥".to_string()))?;

    authenticate(
        &mut session,
        &params.username,
        &params.host,
        &params.credential,
    )
    .await?;

    Ok(AuthenticatedSession {
        handle: session,
        host_key,
        jump_handle: jump_session,
    })
}

impl PtyConn {
    /// Connect, authenticate and open an interactive shell with a PTY.
    pub async fn open(params: ConnectParams) -> Result<(PtyConn, PtyReader), SshError> {
        let AuthenticatedSession {
            handle: session,
            host_key,
            jump_handle,
        } = connect_authenticated(&params).await?;

        let channel = session
            .channel_open_session()
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        let (read, write) = channel.split();

        write
            .request_pty(false, &params.term, params.cols, params.rows, 0, 0, &[])
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        write
            .request_shell(true)
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;

        if !params.startup_command.trim().is_empty() {
            let encoding = encoding_rs::Encoding::for_label(
                params.encoding.trim().as_bytes(),
            )
            .unwrap_or(encoding_rs::UTF_8);
            let (encoded, _, _) = encoding.encode(params.startup_command.trim());
            let mut data = encoded.into_owned();
            data.push(b'\r');
            write
                .data_bytes(data)
                .await
                .map_err(|error| SshError::Channel(error.to_string()))?;
        }

        Ok((
            PtyConn {
                host_key,
                write,
                session: Mutex::new(Some(session)),
                _jump_session: jump_handle,
            },
            PtyReader { read },
        ))
    }

    pub fn host_key(&self) -> &HostKey {
        &self.host_key
    }

    /// Feed user input (already encoded, including control characters).
    pub async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        if data.is_empty() {
            return Ok(());
        }
        self.write
            .data_bytes(data.to_vec())
            .await
            .map_err(|error| SshError::Channel(error.to_string()))
    }

    /// Tell the server the window size changed. Without this `vim`, `htop` and
    /// `less` keep drawing at the initial 80x24 forever.
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        self.write
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|error| SshError::Channel(error.to_string()))
    }

    /// Send a signal to the remote process group, e.g. `"INT"` for Ctrl-C.
    pub async fn signal(&self, name: &str) -> Result<(), SshError> {
        let signal = parse_signal(name)?;
        self.write
            .signal(signal)
            .await
            .map_err(|error| SshError::Channel(error.to_string()))
    }

    /// Half-close and disconnect. Safe to call more than once.
    pub async fn close(&self) {
        let _ = self.write.eof().await;
        let session = match self.session.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        if let Some(session) = session {
            let _ = session
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

impl PtyReader {
    /// Next meaningful event from the server.
    ///
    /// Bookkeeping messages (`WindowAdjusted`, `Success`, ...) are skipped so
    /// callers only ever see data, an exit status or a close.
    pub async fn next_event(&mut self) -> PtyEvent {
        loop {
            match self.read.wait().await {
                Some(ChannelMsg::Data { data }) => return PtyEvent::Data(data.to_vec()),
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    return PtyEvent::Data(data.to_vec())
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => return PtyEvent::Exit(exit_status),
                Some(ChannelMsg::ExitSignal { .. }) => return PtyEvent::Exit(1),
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => return PtyEvent::Closed,
                Some(_) => continue,
            }
        }
    }
}

async fn authenticate(
    session: &mut client::Handle<HostKeyHandler>,
    username: &str,
    host: &str,
    credential: &Credential,
) -> Result<(), SshError> {
    let auth_failed = |detail: String| SshError::Auth {
        username: username.to_string(),
        host: host.to_string(),
        detail,
    };

    let result = match credential {
        Credential::Password(password) => session
            .authenticate_password(username.to_string(), password.clone())
            .await
            .map_err(|error| auth_failed(error.to_string()))?,
        Credential::PrivateKey { path, passphrase } => {
            let key = rkeys::load_secret_key(path, passphrase.as_deref())
                .map_err(|error| SshError::KeyFile(format!("{}: {error}", path.display())))?;
            // RSA keys need a modern rsa-sha2-* signature; otherwise newer
            // servers reject the legacy ssh-rsa (SHA-1) algorithm.
            let hash_alg = if matches!(key.public_key().algorithm(), ssh_key::Algorithm::Rsa { .. })
            {
                session
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|error| auth_failed(format!("查询 RSA 签名算法失败：{error}")))?
                    .flatten()
            } else {
                None
            };
            session
                .authenticate_publickey(
                    username.to_string(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg),
                )
                .await
                .map_err(|error| auth_failed(error.to_string()))?
        }
        Credential::Agent => return authenticate_agent(session, username, host).await,
    };

    if result.success() {
        Ok(())
    } else {
        Err(auth_failed(
            "服务器拒绝了该凭据（口令/私钥错误，或该用户不允许此认证方式）".to_string(),
        ))
    }
}

/// A boxed agent client so the named-pipe, Pageant and Unix-socket transports
/// can share one authentication routine.
type DynAgent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

#[cfg(windows)]
async fn connect_agent() -> Result<DynAgent, SshError> {
    let pipe_error = match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
        Ok(agent) => return Ok(agent.dynamic()),
        Err(error) => error.to_string(),
    };
    match AgentClient::connect_pageant().await {
        Ok(agent) => Ok(agent.dynamic()),
        Err(error) => Err(SshError::Unsupported(format!(
            "未检测到可用的 SSH Agent（OpenSSH Agent: {pipe_error}；Pageant: {error}）。\
             请启动 Windows OpenSSH Authentication Agent 服务，或改用密码/私钥认证。"
        ))),
    }
}

#[cfg(unix)]
async fn connect_agent() -> Result<DynAgent, SshError> {
    match AgentClient::connect_env().await {
        Ok(agent) => Ok(agent.dynamic()),
        Err(error) => Err(SshError::Unsupported(format!(
            "未检测到可用的 SSH Agent（SSH_AUTH_SOCK）：{error}。请启动 ssh-agent 或改用密码/私钥认证。"
        ))),
    }
}

#[cfg(not(any(unix, windows)))]
async fn connect_agent() -> Result<DynAgent, SshError> {
    Err(SshError::Unsupported(
        "当前平台不支持 SSH Agent 认证".into(),
    ))
}

async fn authenticate_agent(
    session: &mut client::Handle<HostKeyHandler>,
    username: &str,
    host: &str,
) -> Result<(), SshError> {
    let auth_failed = |detail: String| SshError::Auth {
        username: username.to_string(),
        host: host.to_string(),
        detail,
    };

    let mut agent = connect_agent().await?;

    // OpenSSH agents with destination constraints need the session binding
    // before they reveal or use keys. Older agents simply reject the extension,
    // which is fine.
    if let Some(binding) = session.agent_session_binding().cloned() {
        let _ = agent.bind_session(&binding, false).await;
    }

    let identities = agent
        .request_identities()
        .await
        .map_err(|error| auth_failed(format!("SSH Agent 读取身份失败：{error}")))?;
    if identities.is_empty() {
        return Err(auth_failed("SSH Agent 中没有可用身份".to_string()));
    }

    let mut last_error = "SSH Agent 中的身份均被服务器拒绝".to_string();
    for identity in identities {
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                let hash_alg = if matches!(key.algorithm(), ssh_key::Algorithm::Rsa { .. }) {
                    session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|error| auth_failed(format!("查询 RSA 签名算法失败：{error}")))?
                        .flatten()
                } else {
                    None
                };
                session
                    .authenticate_publickey_with(username.to_string(), key, hash_alg, &mut agent)
                    .await
                    .map(|result| result.success())
                    .map_err(|error| error.to_string())
            }
            AgentIdentity::Certificate { certificate, .. } => {
                let hash_alg = if matches!(
                    certificate.public_key().algorithm(),
                    ssh_key::Algorithm::Rsa { .. }
                ) {
                    session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|error| auth_failed(format!("查询 RSA 签名算法失败：{error}")))?
                        .flatten()
                } else {
                    None
                };
                session
                    .authenticate_certificate_with(
                        username.to_string(),
                        certificate,
                        hash_alg,
                        &mut agent,
                    )
                    .await
                    .map(|result| result.success())
                    .map_err(|error| error.to_string())
            }
        };

        match result {
            Ok(true) => return Ok(()),
            Ok(false) => {
                last_error = "SSH Agent 中的身份被服务器拒绝".to_string();
            }
            Err(error) => last_error = format!("SSH Agent 签名失败：{error}"),
        }
    }

    Err(auth_failed(last_error))
}

/// Remove the stored key for `host:port` from ShellX's own known_hosts file.
///
/// Used by the "host key changed" UI so the user can forget an old key and
/// reconnect deliberately. Hashed entries written by OpenSSH are left alone.
pub fn forget_known_host(known_hosts_path: &Path, host: &str, port: u16) -> Result<bool, String> {
    let content = match fs::read_to_string(known_hosts_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("读取 known_hosts 失败：{error}")),
    };

    let port22 = port == 22;
    let bracketed = format!("[{host}]:{port}");
    let mut removed = 0usize;
    let mut kept = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push_str(line);
            kept.push('\n');
            continue;
        }
        let Some(patterns) = trimmed.split_whitespace().next() else {
            continue;
        };
        let matches_host = patterns.split(',').any(|pattern| {
            if pattern.starts_with("|1|") {
                // Hashed OpenSSH entries cannot be matched without the salt
                // hash; leave them untouched rather than risk deleting others.
                return false;
            }
            let candidate = if port22 { host } else { bracketed.as_str() };
            pattern.eq_ignore_ascii_case(candidate)
        });
        if matches_host {
            removed += 1;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }

    if removed == 0 {
        return Ok(false);
    }

    let temp = known_hosts_path.with_extension("known_hosts.tmp");
    fs::write(&temp, kept).map_err(|error| format!("写入 known_hosts 失败：{error}"))?;
    fs::rename(&temp, known_hosts_path)
        .map_err(|error| format!("替换 known_hosts 失败：{error}"))?;
    Ok(true)
}

fn parse_signal(name: &str) -> Result<Sig, SshError> {
    let signal = match name.trim().to_ascii_uppercase().as_str() {
        "INT" | "SIGINT" | "2" => Sig::INT,
        "TERM" | "SIGTERM" | "15" => Sig::TERM,
        "HUP" | "SIGHUP" | "1" => Sig::HUP,
        "QUIT" | "SIGQUIT" | "3" => Sig::QUIT,
        "KILL" | "SIGKILL" | "9" => Sig::KILL,
        "USR1" | "SIGUSR1" => Sig::USR1,
        // `russh::Sig` only enumerates the RFC 4254 names; anything else is
        // passed through verbatim, which is what the protocol allows.
        "USR2" | "SIGUSR2" => Sig::Custom("USR2".to_string()),
        other => Sig::Custom(other.trim_start_matches("SIG").to_string()),
    };
    Ok(signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_never_appears_in_debug_output() {
        let credential = Credential::Password("hunter2".to_string());
        let rendered = format!("{credential:?}");
        assert!(!rendered.contains("hunter2"), "got {rendered}");
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn private_key_path_stays_visible_but_passphrase_does_not() {
        let credential = Credential::PrivateKey {
            path: PathBuf::from("C:/keys/id_ed25519"),
            passphrase: Some("s3cret".to_string()),
        };
        let rendered = format!("{credential:?}");
        assert!(rendered.contains("id_ed25519"));
        assert!(!rendered.contains("s3cret"));
    }

    #[test]
    fn signals_are_parsed_in_both_spellings() {
        // `russh::Sig` does not implement PartialEq; compare its Debug form.
        assert_eq!(format!("{:?}", parse_signal("INT").unwrap()), "INT");
        assert_eq!(format!("{:?}", parse_signal("sigterm").unwrap()), "TERM");
        assert_eq!(format!("{:?}", parse_signal("9").unwrap()), "KILL");
        // Not in the RFC 4254 enumeration: forwarded as a custom name.
        assert_eq!(
            format!("{:?}", parse_signal("USR2").unwrap()),
            "Custom(\"USR2\")"
        );
        assert_eq!(
            format!("{:?}", parse_signal("sigstop").unwrap()),
            "Custom(\"STOP\")"
        );
    }

    #[test]
    fn host_key_change_message_mentions_the_fingerprint() {
        let error = SshError::HostKeyChanged {
            host: "example.com".into(),
            port: 22,
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:abc".into(),
            detail: "key changed at line 3".into(),
        };
        let rendered = error.to_string();
        assert!(rendered.contains("SHA256:abc"));
        assert!(rendered.contains("example.com"));
    }

    #[test]
    fn defaults_fit_a_standard_terminal() {
        let params = ConnectParams::new("host", 22, "user");
        assert_eq!(params.cols, 80);
        assert_eq!(params.rows, 24);
        assert_eq!(params.term, DEFAULT_TERM);
        assert!(params.jump.is_none());
        assert!(params.trusted_host_keys.is_empty());
    }

    #[test]
    fn trusted_fingerprint_is_scoped_by_host_and_port() {
        let trusted = vec![HostKeyTrust {
            host: "example.com".into(),
            port: 22,
            fingerprint: "SHA256:abc".into(),
        }];
        assert_eq!(
            trusted_fingerprint(&trusted, "EXAMPLE.com", 22).as_deref(),
            Some("SHA256:abc")
        );
        assert!(trusted_fingerprint(&trusted, "example.com", 2222).is_none());
        assert!(trusted_fingerprint(&trusted, "other.com", 22).is_none());
    }

    #[test]
    fn forget_known_host_removes_only_the_requested_entry() {
        let dir = std::env::temp_dir().join(format!(
            "shellx-known-hosts-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("known_hosts");
        std::fs::write(
            &path,
            "example.com ssh-ed25519 AAAA\n\
             [example.com]:2222 ssh-rsa BBBB\n\
             other.com ssh-ed25519 CCCC\n",
        )
        .unwrap();

        assert!(forget_known_host(&path, "example.com", 22).unwrap());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("AAAA"));
        assert!(content.contains("BBBB"));
        assert!(content.contains("CCCC"));

        assert!(forget_known_host(&path, "example.com", 2222).unwrap());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("BBBB"));
        assert!(content.contains("CCCC"));
        assert!(!forget_known_host(&path, "example.com", 22).unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }
}
