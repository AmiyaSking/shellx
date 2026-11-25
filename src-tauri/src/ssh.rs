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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Msg};
use russh::keys::{self as rkeys, ssh_key, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect, Sig};

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
    /// Not wired up yet: agent forwarding needs a named-pipe transport and a
    /// signer handshake. Kept as an explicit variant so the UI can say so
    /// instead of silently falling back to something weaker.
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

#[derive(Clone, Debug)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub credential: Credential,
    /// Where our own known_hosts file lives (not ~/.ssh/known_hosts: we keep a
    /// separate file so ShellX never silently rewrites the user's OpenSSH state).
    pub known_hosts_path: PathBuf,
    pub term: String,
    pub cols: u32,
    pub rows: u32,
}

impl ConnectParams {
    pub fn new(host: impl Into<String>, port: u16, username: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port,
            username: username.into(),
            credential: Credential::Agent,
            known_hosts_path: PathBuf::from("known_hosts"),
            term: DEFAULT_TERM.to_string(),
            cols: 80,
            rows: 24,
        }
    }
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
        fingerprint: String,
        detail: String,
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
            } => write!(
                f,
                "主机密钥已变更（{host}:{port}，当前指纹 {fingerprint}）：{detail}。为防中间人攻击已中止连接。"
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
    fn new(host: String, port: u16, known_hosts_path: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(HostKeyState {
                host,
                port,
                known_hosts_path,
                host_key: None,
                rejection: None,
            })),
        }
    }

    fn snapshot(&self) -> (Option<HostKey>, Option<SshError>) {
        match self.state.lock() {
            Ok(guard) => (guard.host_key.clone(), guard.rejection.clone()),
            Err(_) => (None, Some(SshError::KnownHosts("主机密钥状态锁已失效".into()))),
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
                // A recorded key with a different value: refuse, loudly.
                state.rejection = Some(SshError::HostKeyChanged {
                    host: state.host.clone(),
                    port: state.port,
                    fingerprint: fingerprint.clone(),
                    detail: error.to_string(),
                });
                return Ok(false);
            }
        };

        if !known {
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
}

/// Write side of an open PTY: cheap to share, used by the Tauri commands.
pub struct PtyConn {
    host_key: HostKey,
    write: ChannelWriteHalf<Msg>,
    session: Mutex<Option<client::Handle<HostKeyHandler>>>,
}

/// Read side of an open PTY: owns the receive half, driven by one task.
pub struct PtyReader {
    read: ChannelReadHalf,
}

impl PtyConn {
    /// Connect, authenticate and open an interactive shell with a PTY.
    pub async fn open(params: ConnectParams) -> Result<(PtyConn, PtyReader), SshError> {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        });
        let handler = HostKeyHandler::new(
            params.host.clone(),
            params.port,
            params.known_hosts_path.clone(),
        );

        let connect_result = client::connect(
            config,
            (params.host.as_str(), params.port),
            handler.clone(),
        )
        .await;

        let mut session = match connect_result {
            Ok(session) => session,
            Err(error) => {
                // Prefer the reason we recorded over russh's generic version.
                let (_, rejection) = handler.snapshot();
                return Err(rejection.unwrap_or_else(|| SshError::Connect(error.to_string())));
            }
        };

        let (host_key, rejection) = handler.snapshot();
        if let Some(error) = rejection {
            return Err(error);
        }
        let host_key = host_key.ok_or_else(|| {
            SshError::Connect("握手完成但没有收到主机公钥".to_string())
        })?;

        authenticate(&mut session, &params).await?;

        let channel = session
            .channel_open_session()
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        let (read, write) = channel.split();

        write
            .request_pty(
                false,
                &params.term,
                params.cols,
                params.rows,
                0,
                0,
                &[],
            )
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;
        write
            .request_shell(true)
            .await
            .map_err(|error| SshError::Channel(error.to_string()))?;

        Ok((
            PtyConn {
                host_key,
                write,
                session: Mutex::new(Some(session)),
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
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return PtyEvent::Closed
                }
                Some(_) => continue,
            }
        }
    }
}

async fn authenticate(
    session: &mut client::Handle<HostKeyHandler>,
    params: &ConnectParams,
) -> Result<(), SshError> {
    let auth_failed = |detail: String| SshError::Auth {
        username: params.username.clone(),
        host: params.host.clone(),
        detail,
    };

    let result = match &params.credential {
        Credential::Password(password) => session
            .authenticate_password(params.username.clone(), password.clone())
            .await
            .map_err(|error| auth_failed(error.to_string()))?,
        Credential::PrivateKey { path, passphrase } => {
            let key = rkeys::load_secret_key(path, passphrase.as_deref()).map_err(|error| {
                SshError::KeyFile(format!("{}: {error}", path.display()))
            })?;
            session
                .authenticate_publickey(
                    params.username.clone(),
                    PrivateKeyWithHashAlg::new(Arc::new(key), None),
                )
                .await
                .map_err(|error| auth_failed(error.to_string()))?
        }
        Credential::Agent => {
            return Err(SshError::Unsupported(
                "SSH Agent 认证尚未接入，请先用口令或私钥文件".to_string(),
            ))
        }
    };

    if result.success() {
        Ok(())
    } else {
        Err(auth_failed(
            "服务器拒绝了该凭据（口令/私钥错误，或该用户不允许此认证方式）".to_string(),
        ))
    }
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
        assert_eq!(parse_signal("INT").unwrap(), Sig::INT);
        assert_eq!(parse_signal("sigterm").unwrap(), Sig::TERM);
        assert_eq!(parse_signal("9").unwrap(), Sig::KILL);
        // Not in the RFC 4254 enumeration: forwarded as a custom name.
        assert_eq!(parse_signal("USR2").unwrap(), Sig::Custom("USR2".to_string()));
        assert_eq!(
            parse_signal("sigstop").unwrap(),
            Sig::Custom("STOP".to_string())
        );
    }

    #[test]
    fn host_key_change_message_mentions_the_fingerprint() {
        let error = SshError::HostKeyChanged {
            host: "example.com".into(),
            port: 22,
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
    }
}
