//! In-process SSH server used by `ssh.rs` / `sftp.rs` integration tests.
//!
//! It speaks real SSH over loopback so host-key trust, password and
//! keyboard-interactive authentication, PTY echo, direct-tcpip forwarding and
//! SFTP file operations can be exercised without Docker or an external server.

use std::collections::HashMap;
use std::io::{Read as _, Seek, SeekFrom, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::server::{self, Auth, Msg, Response, Server as _, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Sig};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
    Version,
};
use russh_sftp::server::StatusReply;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::models::HostKeyTrust;
use crate::ssh::{connect_authenticated, ConnectParams, Credential, SshError};

#[derive(Clone)]
pub(crate) enum TestAuthMode {
    Password(String),
    KeyboardInteractive(String),
}

#[derive(Clone)]
struct TestServer {
    auth: TestAuthMode,
    /// When set, `sftp` subsystem requests are served from this directory.
    sftp_root: Option<PathBuf>,
    channels: Arc<Mutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl server::Server for TestServer {
    type Handler = TestServer;

    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        Self {
            auth: self.auth.clone(),
            sftp_root: self.sftp_root.clone(),
            channels: Default::default(),
        }
    }
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        _user: &str,
        password: &str,
    ) -> Result<Auth, Self::Error> {
        match &self.auth {
            TestAuthMode::Password(expected) if expected == password => Ok(Auth::Accept),
            _ => Ok(Auth::reject()),
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let TestAuthMode::KeyboardInteractive(expected) = &self.auth else {
            return Ok(Auth::reject());
        };

        match response {
            None => Ok(Auth::Partial {
                name: "OTP".into(),
                instructions: "Enter the one-time code".into(),
                prompts: vec![("OTP code: ".into(), false)].into(),
            }),
            Some(mut response) => {
                let answer = response
                    .next()
                    .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
                    .unwrap_or_default();
                if answer == *expected {
                    Ok(Auth::Accept)
                } else {
                    Ok(Auth::Reject {
                        proceed_with_methods: Some(MethodSet::from(
                            &[MethodKind::KeyboardInteractive][..],
                        )),
                        partial_success: false,
                    })
                }
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        // Keep the channel so `subsystem_request` can turn it into a stream.
        self.channels
            .lock()
            .expect("test server channel table")
            .insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _host_to_connect: &str,
        _port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let id = channel.id();
        let handle = session.handle();
        reply.accept().await;
        // The confirmation is processed after this handler returns, so send
        // from a task rather than queuing on the not-yet-open channel.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = handle.data(id, b"forwarded".to_vec()).await;
            let _ = handle.eof(id).await;
        });
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(root) = self.sftp_root.clone().filter(|_| name == "sftp") else {
            let _ = session.channel_failure(channel_id);
            return Ok(());
        };
        let channel = self
            .channels
            .lock()
            .expect("test server channel table")
            .remove(&channel_id);
        let Some(channel) = channel else {
            let _ = session.channel_failure(channel_id);
            return Ok(());
        };

        let _ = session.channel_success(channel_id);
        russh_sftp::server::run(channel.into_stream(), TestSftpSession::new(root)).await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: russh::ChannelId,
        _term: &str,
        _col_width: u32,
        _row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: russh::ChannelId,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let _ = session.channel_success(channel);
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).trim().to_string();
        let _ = session.channel_success(channel);

        // `sleep` never answers so the client-side timeout can be exercised.
        if command == "sleep" {
            return Ok(());
        }

        let (output, status) = if let Some(rest) = command.strip_prefix("echo ") {
            (format!("{rest}\n"), 0)
        } else if command == "exit 7" {
            (String::new(), 7)
        } else {
            (format!("ran: {command}\n"), 0)
        };
        let _ = session.data(channel, output.into_bytes());
        let _ = session.exit_status_request(channel, status);
        let _ = session.eof(channel);
        let _ = session.close(channel);
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        // Turn the signal into an exit status so clients can observe that it
        // reached the remote command. This mirrors a shell reporting 130/143.
        let status = match signal {
            Sig::INT => 130,
            Sig::TERM => 143,
            Sig::KILL => 137,
            _ => 1,
        };
        let _ = session.exit_status_request(channel, status);
        let _ = session.eof(channel);
        let _ = session.close(channel);
        Ok(())
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        // Only shell channels echo. Subsystem channels are removed from the
        // table and consumed by the SFTP server; echoing their binary packets
        // would corrupt the SFTP stream.
        let is_shell = self
            .channels
            .lock()
            .expect("test server channel table")
            .contains_key(&channel);
        if is_shell {
            session.data(channel, data.to_vec())?;
        }
        Ok(())
    }
}

/// Minimal filesystem-backed SFTP server rooted at a test directory.
///
/// It implements just the operations ShellX uses: realpath, directory listing,
/// open/read/write/close, stat, mkdir/rmdir/remove and rename.
struct TestSftpSession {
    root: PathBuf,
    next_handle: u64,
    handles: HashMap<Vec<u8>, TestHandle>,
}

enum TestHandle {
    File(std::fs::File),
    Dir {
        entries: Vec<(russh_sftp::protocol::SftpString, FileAttributes)>,
        sent: bool,
    },
}

impl TestSftpSession {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            next_handle: 0,
            handles: HashMap::new(),
        }
    }

    fn new_handle(&mut self, handle: TestHandle) -> Vec<u8> {
        self.next_handle += 1;
        let token = format!("h{}", self.next_handle).into_bytes();
        self.handles.insert(token.clone(), handle);
        token
    }

    /// Map a remote POSIX path onto the sandbox directory, refusing `..`.
    fn resolve(&self, path: &[u8]) -> Result<PathBuf, StatusReply> {
        let text = String::from_utf8_lossy(path);
        let mut resolved = self.root.clone();
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    return Err(StatusReply::new(StatusCode::PermissionDenied)
                        .with_message("path escapes the SFTP test root"))
                }
                other => resolved.push(other),
            }
        }
        Ok(resolved)
    }

    /// Collapse a client path into the virtual absolute path shown in listings.
    fn virtual_path(path: &[u8]) -> String {
        let text = String::from_utf8_lossy(path);
        let parts: Vec<&str> = text
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect();
        format!("/{}", parts.join("/"))
    }

    fn dir_entries(
        dir: &Path,
    ) -> Result<Vec<(russh_sftp::protocol::SftpString, FileAttributes)>, StatusReply> {
        let mut entries: Vec<(russh_sftp::protocol::SftpString, FileAttributes)> =
            Vec::new();
        for entry in std::fs::read_dir(dir).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(io_error)?;
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push((name, FileAttributes::from(&metadata)));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }
}

fn io_error(error: std::io::Error) -> StatusReply {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        std::io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    };
    StatusReply::new(code).with_message(error.to_string())
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

impl russh_sftp::server::Handler for TestSftpSession {
    type Error = StatusReply;

    fn unimplemented(&self) -> Self::Error {
        StatusReply::new(StatusCode::OpUnsupported)
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }

    async fn realpath(
        &mut self,
        id: u32,
        path: russh_sftp::protocol::SftpString,
    ) -> Result<Name, Self::Error> {
        let real = self.resolve(path.as_bytes())?;
        std::fs::metadata(&real).map_err(io_error)?;
        Ok(Name {
            id,
            files: vec![File::dummy(Self::virtual_path(path.as_bytes()))],
        })
    }

    async fn stat(
        &mut self,
        id: u32,
        path: russh_sftp::protocol::SftpString,
    ) -> Result<Attrs, Self::Error> {
        let real = self.resolve(path.as_bytes())?;
        let metadata = std::fs::metadata(&real).map_err(io_error)?;
        Ok(Attrs {
            id,
            attrs: FileAttributes::from(&metadata),
        })
    }

    async fn opendir(
        &mut self,
        id: u32,
        path: russh_sftp::protocol::SftpString,
    ) -> Result<Handle, Self::Error> {
        let real = self.resolve(path.as_bytes())?;
        if !real.is_dir() {
            return Err(StatusReply::new(StatusCode::NoSuchFile)
                .with_message(format!("not a directory: {path}")));
        }
        let entries = Self::dir_entries(&real)?;
        let handle = self.new_handle(TestHandle::Dir {
            entries,
            sent: false,
        });
        Ok(Handle {
            id,
            handle: handle.into(),
        })
    }

    async fn readdir(
        &mut self,
        id: u32,
        handle: russh_sftp::protocol::SftpHandle,
    ) -> Result<Name, Self::Error> {
        let Some(TestHandle::Dir { entries, sent }) =
            self.handles.get_mut(handle.as_bytes())
        else {
            return Err(StatusReply::new(StatusCode::Failure)
                .with_message("unknown directory handle"));
        };
        if *sent {
            return Err(StatusCode::Eof.into());
        }
        *sent = true;
        Ok(Name {
            id,
            files: std::mem::take(entries)
                .into_iter()
                .map(|(name, attrs)| File::new(name, attrs))
                .collect(),
        })
    }

    async fn open(
        &mut self,
        id: u32,
        filename: russh_sftp::protocol::SftpString,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let real = self.resolve(filename.as_bytes())?;
        let options: std::fs::OpenOptions = pflags.into();
        let file = options.open(&real).map_err(io_error)?;
        let handle = self.new_handle(TestHandle::File(file));
        Ok(Handle {
            id,
            handle: handle.into(),
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: russh_sftp::protocol::SftpHandle,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let Some(TestHandle::File(file)) = self.handles.get_mut(handle.as_bytes()) else {
            return Err(StatusReply::new(StatusCode::Failure)
                .with_message("unknown file handle"));
        };
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        let mut buffer = vec![0u8; (len as usize).min(256 * 1024)];
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            return Err(StatusCode::Eof.into());
        }
        buffer.truncate(read);
        Ok(Data { id, data: buffer })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: russh_sftp::protocol::SftpHandle,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let Some(TestHandle::File(file)) = self.handles.get_mut(handle.as_bytes()) else {
            return Err(StatusReply::new(StatusCode::Failure)
                .with_message("unknown file handle"));
        };
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        file.write_all(&data).map_err(io_error)?;
        Ok(ok(id))
    }

    async fn close(
        &mut self,
        id: u32,
        handle: russh_sftp::protocol::SftpHandle,
    ) -> Result<Status, Self::Error> {
        self.handles.remove(handle.as_bytes());
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: russh_sftp::protocol::SftpString,
        _attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        let real = self.resolve(path.as_bytes())?;
        std::fs::create_dir(&real).map_err(io_error)?;
        Ok(ok(id))
    }

    async fn rmdir(
        &mut self,
        id: u32,
        path: russh_sftp::protocol::SftpString,
    ) -> Result<Status, Self::Error> {
        let real = self.resolve(path.as_bytes())?;
        std::fs::remove_dir(&real).map_err(io_error)?;
        Ok(ok(id))
    }

    async fn remove(
        &mut self,
        id: u32,
        filename: russh_sftp::protocol::SftpString,
    ) -> Result<Status, Self::Error> {
        let real = self.resolve(filename.as_bytes())?;
        std::fs::remove_file(&real).map_err(io_error)?;
        Ok(ok(id))
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: russh_sftp::protocol::SftpString,
        newpath: russh_sftp::protocol::SftpString,
    ) -> Result<Status, Self::Error> {
        let old = self.resolve(oldpath.as_bytes())?;
        let new = self.resolve(newpath.as_bytes())?;
        std::fs::rename(&old, &new).map_err(io_error)?;
        Ok(ok(id))
    }
}

pub(crate) struct TestSshServer {
    pub port: u16,
    task: JoinHandle<()>,
}

impl Drop for TestSshServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn start_test_server(auth: TestAuthMode) -> TestSshServer {
    start_server(auth, None).await
}

/// Start an SSH server whose `sftp` subsystem is rooted at `root`.
pub(crate) async fn start_sftp_test_server(
    auth: TestAuthMode,
    root: PathBuf,
) -> TestSshServer {
    start_server(auth, Some(root)).await
}

async fn start_server(auth: TestAuthMode, sftp_root: Option<PathBuf>) -> TestSshServer {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind test SSH server");
    let port = listener.local_addr().expect("test server address").port();

    let mut config = server::Config {
        auth_rejection_time: Duration::from_millis(1),
        auth_rejection_time_initial: Some(Duration::from_millis(1)),
        inactivity_timeout: Some(Duration::from_secs(30)),
        ..Default::default()
    };
    config.keys = vec![russh::keys::PrivateKey::random(
        &mut rand::rng(),
        russh::keys::Algorithm::Ed25519,
    )
    .expect("generate test host key")];
    config.methods = match &auth {
        TestAuthMode::Password(_) => {
            MethodSet::from(&[MethodKind::Password][..])
        }
        TestAuthMode::KeyboardInteractive(_) => {
            MethodSet::from(&[MethodKind::KeyboardInteractive][..])
        }
    };
    let config = Arc::new(config);

    let task = tokio::spawn(async move {
        let mut server = TestServer {
            auth,
            sftp_root,
            channels: Default::default(),
        };
        let _ = server.run_on_socket(config, &listener).await;
    });

    TestSshServer { port, task }
}

/// Temp directory plus the `known_hosts` path the client should use.
pub(crate) fn temp_known_hosts() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("shellx-ssh-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp SSH state dir");
    let path = dir.join("known_hosts");
    (dir, path)
}

/// Connect once to learn the server fingerprint, then return parameters that
/// trust it. This mirrors the GUI's first-connection confirmation.
pub(crate) async fn trusted_params(
    server: &TestSshServer,
    known_hosts: &Path,
    credential: Credential,
) -> ConnectParams {
    let mut params = ConnectParams::new("127.0.0.1", server.port, "tester");
    params.credential = credential;
    params.known_hosts_path = known_hosts.to_path_buf();

    let error = connect_authenticated(&params)
        .await
        .err()
        .expect("unknown host key must be rejected");
    let fingerprint = match error {
        SshError::HostKeyUnknown { fingerprint, .. } => fingerprint,
        other => panic!("expected HostKeyUnknown, got {other:?}"),
    };
    params.trusted_host_keys = vec![HostKeyTrust {
        host: "127.0.0.1".into(),
        port: server.port,
        fingerprint,
    }];
    params
}
