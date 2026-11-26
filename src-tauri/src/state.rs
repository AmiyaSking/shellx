use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::audit::AuditLogger;
use crate::connections::ConnectionStore;
use crate::forwards::{ForwardRule, ForwardRuleInput, ForwardStore};
use crate::mcp;
use crate::models::{
    ApprovalDecision, ApprovalRequest, AuditEntry, AuditSource, AuthKind, CommandOutcome,
    CommandRequest, CommandResult, Connection, ConnectionInput, Environment, HostKeyTrust,
    McpAccessMode, McpServerInfo, PolicyAction, PolicyDecision, RiskLevel, SessionInfo,
};
use crate::policy::PolicyEngine;
use crate::redaction::Redactor;
use crate::sessions::{SessionManager, SessionOutputPage};
use crate::ssh::{self, ConnectParams, Credential, JumpHost};
use crate::vault::SecretVault;

struct PendingApproval {
    request: ApprovalRequest,
    command: String,
    source: AuditSource,
    timeout_secs: Option<u64>,
}

struct RunningCommand {
    connection_id: String,
    control: ssh::ExecControl,
}

/// Reservation held while a command is connecting, so two concurrent starts
/// on the same connection cannot race into one session.
struct StartGuard {
    starting: Arc<Mutex<HashSet<String>>>,
    connection_id: String,
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        if let Ok(mut starting) = self.starting.lock() {
            starting.remove(&self.connection_id);
        }
    }
}

/// Default and maximum wall-clock budget for one agent command.
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 30;
const MAX_COMMAND_TIMEOUT_SECS: u64 = 600;
/// Upper bound on the output returned to an agent or kept in the session ring.
const MAX_COMMAND_OUTPUT_BYTES: usize = 256 * 1024;

/// Result of starting a command.
pub enum CommandStart {
    /// Policy already produced a final answer (blocked or awaiting approval).
    Ready(CommandOutcome),
    /// The command is running; `completion` resolves when it finishes.
    Started {
        session_id: String,
        completion: oneshot::Receiver<Result<CommandOutcome, String>>,
    },
}

struct StartedExecution {
    session_id: String,
    completion: oneshot::Receiver<Result<CommandOutcome, String>>,
}

/// Core services shared by the Tauri commands and the MCP transport.
pub struct AppState {
    data_dir: PathBuf,
    connections: Mutex<ConnectionStore>,
    forwards: Mutex<ForwardStore>,
    /// Shared with background command tasks, which outlive one request.
    sessions: Arc<Mutex<SessionManager>>,
    approvals: Mutex<HashMap<String, PendingApproval>>,
    running: Arc<Mutex<HashMap<String, RunningCommand>>>,
    starting: Arc<Mutex<HashSet<String>>>,
    vault: SecretVault,
    audit: Arc<AuditLogger>,
    policy: PolicyEngine,
    redactor: Arc<Redactor>,
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, String> {
    mutex.lock().map_err(|_| "内部状态锁不可用".to_string())
}

impl AppState {
    pub fn load(data_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let state = Self {
            data_dir: data_dir.clone(),
            connections: Mutex::new(ConnectionStore::load(&data_dir)),
            forwards: Mutex::new(ForwardStore::load(&data_dir)),
            sessions: Arc::new(Mutex::new(SessionManager::default())),
            approvals: Mutex::new(HashMap::new()),
            running: Arc::new(Mutex::new(HashMap::new())),
            starting: Arc::new(Mutex::new(HashSet::new())),
            vault: SecretVault::new(),
            audit: Arc::new(AuditLogger::new(&data_dir)),
            policy: PolicyEngine,
            redactor: Arc::new(Redactor::new()),
        };
        let _ = state.audit.log(make_audit(
            AuditSource::System,
            None,
            None,
            None,
            PolicyAction::Allow,
            None,
            None,
            None,
            "ShellX 核心启动",
        ));
        Ok(state)
    }

    pub fn list_connections(&self) -> Result<Vec<Connection>, String> {
        Ok(lock(&self.connections)?.list())
    }

    pub fn save_connection(&self, input: ConnectionInput) -> Result<Connection, String> {
        let password = input.password.clone();
        let passphrase = input.passphrase.clone();
        let connection = {
            let mut store = lock(&self.connections)?;
            store.upsert(input)?
        };
        if let Some(secret) = password {
            if !secret.is_empty() {
                if let Err(error) = self.vault.set(&vault_key(&connection.id), &secret) {
                    eprintln!("[shellx] 口令未能写入系统凭据存储：{error}");
                }
            }
        }
        if let Some(secret) = passphrase {
            if !secret.is_empty() {
                if let Err(error) = self.vault.set(&passphrase_key(&connection.id), &secret) {
                    eprintln!("[shellx] 私钥口令未能写入系统凭据存储：{error}");
                }
            }
        }
        Ok(connection)
    }

    pub fn delete_connection(&self, id: &str) -> Result<(), String> {
        lock(&self.connections)?.remove(id)?;
        self.vault.remove(&vault_key(id));
        self.vault.remove(&passphrase_key(id));
        lock(&self.sessions)?.close_for_connection(id);
        Ok(())
    }

    /// Record a connection lifecycle event in the audit log. Interactive PTY
    /// input is not captured, but connect/disconnect/failure are.
    pub fn record_pty_event(&self, connection_id: Option<&str>, detail: impl Into<String>) {
        let connection = connection_id.and_then(|id| self.get_connection(id).ok());
        let _ = self.audit.log(make_audit(
            AuditSource::User,
            connection.as_ref(),
            None,
            None,
            PolicyAction::Allow,
            None,
            None,
            None,
            detail,
        ));
    }

    /// Record an MCP-originated SFTP action in the audit log.
    pub fn record_agent_event(&self, connection_id: Option<&str>, detail: impl Into<String>) {
        let connection = connection_id.and_then(|id| self.get_connection(id).ok());
        let _ = self.audit.log(make_audit(
            AuditSource::Agent,
            connection.as_ref(),
            None,
            None,
            PolicyAction::Allow,
            None,
            None,
            None,
            detail,
        ));
    }

    /// SFTP permission derived from the connection's MCP access and
    /// environment. Reads are low risk; writes/deletes follow the command
    /// matrix (staging/prod would need a human approval, which stdio cannot
    /// provide).
    pub fn authorize_sftp(
        &self,
        connection_id: &str,
        write: bool,
        action: &str,
    ) -> Result<Connection, String> {
        let connection = self.get_connection(connection_id)?;
        let denial = match connection.mcp_access {
            McpAccessMode::Disabled => Some("该连接未授权给 MCP".to_string()),
            McpAccessMode::ReadOnly if write => {
                Some("该连接对 MCP 为只读模式，不允许 SFTP 写入或删除".to_string())
            }
            McpAccessMode::StandardControlled
                if write && connection.environment != Environment::Dev =>
            {
                Some(format!(
                    "{} 环境的 SFTP 写操作需要人工审批，stdio 模式无法审批，已自动拒绝",
                    connection.environment.label()
                ))
            }
            _ => None,
        };
        if let Some(reason) = denial {
            let _ = self.audit.log(make_audit(
                AuditSource::Agent,
                Some(&connection),
                None,
                None,
                PolicyAction::Block,
                None,
                None,
                None,
                format!("{action} 被拒绝：{reason}"),
            ));
            return Err(reason);
        }
        Ok(connection)
    }

    pub fn preview_command(
        &self,
        connection_id: &str,
        command: &str,
        source: AuditSource,
    ) -> Result<PolicyDecision, String> {
        let connection = self.get_connection(connection_id)?;
        Ok(self.policy.evaluate(
            connection.environment,
            connection.mcp_access,
            source,
            command,
        ))
    }

    /// Evaluate policy and, if allowed, start the command. Returning a session
    /// id before the command finishes lets MCP clients read output or send a
    /// signal without holding the tool call open.
    pub async fn start_command(
        &self,
        request: CommandRequest,
    ) -> Result<CommandStart, String> {
        let connection = self.get_connection(&request.connection_id)?;
        let decision = self.policy.evaluate(
            connection.environment,
            connection.mcp_access,
            request.source,
            &request.command,
        );

        match decision.action {
            PolicyAction::Allow => {
                let started = self
                    .begin_execution(
                        connection,
                        request.command,
                        request.source,
                        decision,
                        None,
                        request.timeout_secs,
                    )
                    .await?;
                Ok(CommandStart::Started {
                    session_id: started.session_id,
                    completion: started.completion,
                })
            }
            PolicyAction::RequireApproval => {
                let approval = ApprovalRequest {
                    id: Uuid::new_v4().to_string(),
                    connection_id: connection.id.clone(),
                    connection_alias: connection.alias.clone(),
                    environment: connection.environment,
                    command: request.command.clone(),
                    risk: decision.risk,
                    reasons: decision.reasons.clone(),
                    source: request.source,
                    requested_at: Utc::now().to_rfc3339(),
                };
                self.audit.log(make_audit(
                    request.source,
                    Some(&connection),
                    Some(request.command.clone()),
                    Some(decision.risk),
                    PolicyAction::RequireApproval,
                    None,
                    None,
                    None,
                    "等待人工审批",
                ))?;
                lock(&self.approvals)?.insert(
                    approval.id.clone(),
                    PendingApproval {
                        request: approval.clone(),
                        command: request.command.clone(),
                        source: request.source,
                        timeout_secs: request.timeout_secs,
                    },
                );
                Ok(CommandStart::Ready(CommandOutcome::PendingApproval {
                    approval,
                }))
            }
            PolicyAction::Block => {
                let message = decision.reasons.join("；");
                let entry = self.audit.log(make_audit(
                    request.source,
                    Some(&connection),
                    Some(request.command.clone()),
                    Some(decision.risk),
                    PolicyAction::Block,
                    None,
                    None,
                    None,
                    message.clone(),
                ))?;
                Ok(CommandStart::Ready(CommandOutcome::Blocked {
                    decision,
                    audit_id: entry.id,
                    message,
                }))
            }
        }
    }

    pub async fn run_command(&self, request: CommandRequest) -> Result<CommandOutcome, String> {
        match self.start_command(request).await? {
            CommandStart::Ready(outcome) => Ok(outcome),
            CommandStart::Started { completion, .. } => completion
                .await
                .map_err(|_| "命令后台任务意外退出".to_string())?,
        }
    }

    /// Send a signal to a command that is still running.
    pub async fn send_signal(
        &self,
        session_id: &str,
        signal: &str,
    ) -> Result<(), String> {
        let control = {
            lock(&self.running)?
                .get(session_id)
                .map(|running| running.control.clone())
                .ok_or_else(|| {
                    format!("会话 {session_id} 当前没有正在运行的命令")
                })?
        };
        control.signal(signal).await
    }

    pub async fn resolve_approval(
        &self,
        approval_id: &str,
        approve: bool,
        command_override: Option<String>,
    ) -> Result<CommandOutcome, String> {
        self.resolve_approval_inner(approval_id, approve, command_override, "用户拒绝了该命令")
            .await
    }

    /// Reject a pending approval without a human in the loop. Used by the
    /// headless stdio transport, which has no approval dialog.
    pub async fn deny_approval(
        &self,
        approval_id: &str,
        reason: &str,
    ) -> Result<CommandOutcome, String> {
        self.resolve_approval_inner(approval_id, false, None, reason)
            .await
    }

    async fn resolve_approval_inner(
        &self,
        approval_id: &str,
        approve: bool,
        command_override: Option<String>,
        rejection_reason: &str,
    ) -> Result<CommandOutcome, String> {
        let pending = lock(&self.approvals)?
            .remove(approval_id)
            .ok_or_else(|| "审批请求不存在或已处理".to_string())?;
        let connection = self.get_connection(&pending.request.connection_id)?;
        let command = command_override
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| pending.command.clone());

        if !approve {
            let reason = rejection_reason.to_string();
            let entry = self.audit.log(make_audit(
                pending.source,
                Some(&connection),
                Some(command),
                Some(pending.request.risk),
                PolicyAction::Block,
                Some(ApprovalDecision::Rejected),
                None,
                None,
                reason.clone(),
            ))?;
            let decision = PolicyDecision {
                action: PolicyAction::Block,
                risk: pending.request.risk,
                reasons: vec![reason.clone()],
                rule_id: Some("approval-rejected".into()),
            };
            return Ok(CommandOutcome::Blocked {
                decision,
                audit_id: entry.id,
                message: reason,
            });
        }

        let mut decision = self.policy.evaluate(
            connection.environment,
            connection.mcp_access,
            pending.source,
            &command,
        );
        if decision.action == PolicyAction::Block {
            let message = decision.reasons.join("；");
            let entry = self.audit.log(make_audit(
                pending.source,
                Some(&connection),
                Some(command),
                Some(decision.risk),
                PolicyAction::Block,
                Some(ApprovalDecision::Approved),
                None,
                None,
                format!("审批通过，但重新判定后仍被拦截：{message}"),
            ))?;
            return Ok(CommandOutcome::Blocked {
                decision,
                audit_id: entry.id,
                message,
            });
        }

        // The user explicitly approved this exact command.
        decision.action = PolicyAction::Allow;
        let started = self
            .begin_execution(
                connection,
                command,
                pending.source,
                decision,
                Some(ApprovalDecision::Approved),
                pending.timeout_secs,
            )
            .await?;
        started
            .completion
            .await
            .map_err(|_| "命令后台任务意外退出".to_string())?
    }

    pub fn list_forward_rules(&self) -> Result<Vec<ForwardRule>, String> {
        Ok(lock(&self.forwards)?.list())
    }

    pub fn get_forward_rule(&self, id: &str) -> Result<ForwardRule, String> {
        lock(&self.forwards)?
            .get(id)
            .ok_or_else(|| "转发规则不存在".to_string())
    }

    pub fn save_forward_rule(
        &self,
        input: ForwardRuleInput,
    ) -> Result<ForwardRule, String> {
        self.get_connection(&input.connection_id)?;
        lock(&self.forwards)?.upsert(input)
    }

    pub fn delete_forward_rule(&self, id: &str) -> Result<(), String> {
        lock(&self.forwards)?.remove(id)
    }

    /// Connection parameters for a forwarding-only SSH session. The host key
    /// must already be trusted (confirmed by a previous terminal connection).
    pub fn forward_params(&self, connection_id: &str) -> Result<ConnectParams, String> {
        let (_, mut params) = self.pty_params(connection_id, 80, 24, Vec::new())?;
        // Agent channels are only requested by interactive terminal sessions.
        params.agent_forwarding = false;
        Ok(params)
    }

    /// Connection parameters for a file-transfer-only SSH session.
    pub fn sftp_params(&self, connection_id: &str) -> Result<ConnectParams, String> {
        self.forward_params(connection_id)
    }

    pub fn list_audit(&self, limit: usize) -> Vec<AuditEntry> {
        self.audit.recent(limit)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        Ok(lock(&self.sessions)?.list())
    }

    /// Close a command session. A command that is still running is killed
    /// first; its output is still recorded when the executor finishes.
    pub async fn close_session(&self, id: &str) -> Result<bool, String> {
        let control = {
            lock(&self.running)?
                .get(id)
                .map(|running| running.control.clone())
        };
        if let Some(control) = control {
            let _ = control.signal("KILL").await;
        }
        Ok(lock(&self.sessions)?.close(id))
    }

    /// Paginated access to a command session's output ring.
    pub fn read_session_output(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SessionOutputPage, String> {
        lock(&self.sessions)?.read_output(session_id, offset, limit)
    }

    /// Full buffered output for the MCP resources interface.
    pub fn session_output_text(&self, session_id: &str) -> Result<String, String> {
        lock(&self.sessions)?.output_text(session_id)
    }

    pub fn mcp_info(&self) -> Result<McpServerInfo, String> {
        let authorized_connections = lock(&self.connections)?.authorized_count();
        let active_sessions = lock(&self.sessions)?.active_count();
        Ok(McpServerInfo {
            transport: "stdio".into(),
            command: "shellx --mcp-stdio".into(),
            status: "ready".into(),
            tools: mcp::tool_catalog(),
            authorized_connections,
            active_sessions,
        })
    }

    /// Our own known_hosts file, deliberately separate from
    /// `~/.ssh/known_hosts` so ShellX never rewrites OpenSSH's trust state
    /// behind the user's back.
    pub fn known_hosts_path(&self) -> PathBuf {
        self.data_dir.join("known_hosts")
    }

    /// Everything the SSH transport needs to open a real interactive shell.
    pub fn pty_params(
        &self,
        connection_id: &str,
        cols: u32,
        rows: u32,
        trusted_host_keys: Vec<HostKeyTrust>,
    ) -> Result<(String, ConnectParams), String> {
        let connection = self.get_connection(connection_id)?;
        let credential = self.credential_for(&connection)?;

        let jump = match connection.jump_host.as_deref() {
            Some(jump_id) => {
                let jump = self
                    .get_connection(jump_id)
                    .map_err(|_| "连接配置的跳板机不存在".to_string())?;
                if jump.jump_host.is_some() {
                    return Err("暂不支持多级跳板机（跳板机本身又配置了跳板机）".into());
                }
                Some(JumpHost {
                    host: jump.host.clone(),
                    port: jump.port,
                    username: jump.username.clone(),
                    credential: self.credential_for(&jump)?,
                })
            }
            None => None,
        };

        Ok((
            connection.alias.clone(),
            ConnectParams {
                host: connection.host.clone(),
                port: connection.port,
                username: connection.username.clone(),
                credential,
                jump,
                auth_prompter: None,
                remote_forwards: Vec::new(),
                trusted_host_keys,
                known_hosts_path: self.known_hosts_path(),
                term: if connection.term.trim().is_empty() {
                    ssh::DEFAULT_TERM.to_string()
                } else {
                    connection.term.trim().to_string()
                },
                encoding: connection.encoding.clone(),
                startup_command: connection.startup_command.clone(),
                legacy_algorithms: connection.legacy_algorithms,
                agent_forwarding: connection.forward_agent,
                cols: cols.max(1),
                rows: rows.max(1),
            },
        ))
    }

    fn credential_for(&self, connection: &Connection) -> Result<Credential, String> {
        match connection.auth.kind {
            AuthKind::Password => {
                let secret = self.vault.get(&vault_key(&connection.id)).ok_or_else(|| {
                    format!(
                        "连接「{}」还没有保存口令，请编辑连接填写密码（会存入系统凭据存储）",
                        connection.alias
                    )
                })?;
                Ok(Credential::Password(secret))
            }
            AuthKind::PrivateKey => {
                let configured = connection
                    .auth
                    .private_key_path
                    .clone()
                    .ok_or_else(|| "该连接未配置私钥文件路径".to_string())?;
                let path = expand_home(&configured);
                if !path.is_file() {
                    return Err(format!("私钥文件不存在：{}", path.display()));
                }
                Ok(Credential::PrivateKey {
                    path,
                    passphrase: self.vault.get(&passphrase_key(&connection.id)),
                })
            }
            AuthKind::Agent => Ok(Credential::Agent),
        }
    }

    /// Seed a password in the in-memory vault for integration tests, without
    /// creating a real OS credential entry.
    #[cfg(test)]
    pub(crate) fn cache_password_for_test(&self, connection_id: &str, password: &str) {
        self.vault.cache_only(&vault_key(connection_id), password);
    }

    fn get_connection(&self, id: &str) -> Result<Connection, String> {
        lock(&self.connections)?
            .get(id)
            .ok_or_else(|| "连接不存在".to_string())
    }

    /// Reserve a connection for a command that is still connecting.
    fn reserve_connection(&self, connection_id: &str) -> Result<StartGuard, String> {
        let mut starting = lock(&self.starting)?;
        if !starting.insert(connection_id.to_string()) {
            return Err("该连接已有命令正在执行，请等待结束或先发送信号".into());
        }
        Ok(StartGuard {
            starting: self.starting.clone(),
            connection_id: connection_id.to_string(),
        })
    }

    /// Start one command over a dedicated SSH `exec` channel and return a
    /// completion handle. The command keeps running in a background task so
    /// `send_signal` and `read_output` can reach it from later requests.
    ///
    /// The host key must already be trusted (confirmed in the terminal), so an
    /// agent can never silently accept a new key. Policy, approval and audit
    /// have already run by the time this is called.
    async fn begin_execution(
        &self,
        connection: Connection,
        command: String,
        source: AuditSource,
        decision: PolicyDecision,
        approval: Option<ApprovalDecision>,
        timeout_secs: Option<u64>,
    ) -> Result<StartedExecution, String> {
        // Command sessions are per connection; refuse a second concurrent
        // command instead of mixing two output streams in one ring buffer.
        let _start_guard = self.reserve_connection(&connection.id)?;
        if lock(&self.running)?
            .values()
            .any(|running| running.connection_id == connection.id)
        {
            return Err("该连接已有命令正在执行，请等待结束或先发送信号".into());
        }

        let started = Instant::now();
        let (_, mut params) = self.pty_params(&connection.id, 80, 24, Vec::new())?;
        params.agent_forwarding = false;
        let timeout = Duration::from_secs(
            timeout_secs
                .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS)
                .clamp(1, MAX_COMMAND_TIMEOUT_SECS),
        );

        let (running, control) = match ssh::start_exec(&params, &command).await {
            Ok(started) => started,
            Err(error) => {
                let detail = format!("命令执行失败：{error}");
                let _ = self.audit.log(make_audit(
                    source,
                    Some(&connection),
                    Some(command.clone()),
                    Some(decision.risk),
                    PolicyAction::Block,
                    approval,
                    None,
                    Some(started.elapsed().as_millis() as u64),
                    detail.clone(),
                ));
                return Err(detail);
            }
        };

        let session_id = {
            let mut sessions = lock(&self.sessions)?;
            let id = sessions
                .get_or_create(&connection.id, &connection.alias)
                .id
                .clone();
            sessions.mark_running(&id);
            id
        };
        lock(&self.running)?.insert(
            session_id.clone(),
            RunningCommand {
                connection_id: connection.id.clone(),
                control,
            },
        );

        let (completion_tx, completion) = oneshot::channel();
        let sessions = self.sessions.clone();
        let running_map = self.running.clone();
        let audit = self.audit.clone();
        let redactor = self.redactor.clone();
        let task_session_id = session_id.clone();
        tokio::spawn(async move {
            let exec = running.collect_exec(timeout).await;
            if let Ok(mut map) = running_map.lock() {
                map.remove(&task_session_id);
            }
            let duration_ms = started.elapsed().as_millis() as u64;
            let outcome = match exec {
                Ok(exec) => {
                    let exit_code = exec.exit_status.map(|status| status as i32);
                    let (output, truncated) =
                        finalize_output(exec.output, redactor.as_ref());
                    if let Ok(mut sessions) = sessions.lock() {
                        sessions.finish_command(
                            &task_session_id,
                            &format!("$ {command}\n{output}"),
                            exit_code,
                        );
                    }
                    let result = CommandResult {
                        id: Uuid::new_v4().to_string(),
                        session_id: task_session_id,
                        connection_id: connection.id.clone(),
                        connection_alias: connection.alias.clone(),
                        environment: connection.environment,
                        command: command.clone(),
                        exit_code,
                        output,
                        truncated,
                        risk: decision.risk,
                        policy_action: decision.action,
                        decision_reasons: decision.reasons.clone(),
                        duration_ms,
                        simulated: false,
                    };
                    let detail = if approval.is_some() {
                        "审批通过后执行完成（SSH exec）"
                    } else {
                        "命令执行完成（SSH exec）"
                    };
                    let _ = audit.log(make_audit(
                        source,
                        Some(&connection),
                        Some(command),
                        Some(decision.risk),
                        PolicyAction::Allow,
                        approval,
                        exit_code,
                        Some(duration_ms),
                        detail,
                    ));
                    Ok(CommandOutcome::Completed { result })
                }
                Err(error) => {
                    let detail = format!("命令执行失败：{error}");
                    if let Ok(mut sessions) = sessions.lock() {
                        sessions.finish_command(
                            &task_session_id,
                            &format!("$ {command}\n{detail}"),
                            None,
                        );
                    }
                    let _ = audit.log(make_audit(
                        source,
                        Some(&connection),
                        Some(command),
                        Some(decision.risk),
                        PolicyAction::Block,
                        approval,
                        None,
                        Some(duration_ms),
                        detail.clone(),
                    ));
                    Err(detail)
                }
            };
            let _ = completion_tx.send(outcome);
        });

        Ok(StartedExecution {
            session_id,
            completion,
        })
    }
}

fn vault_key(connection_id: &str) -> String {
    format!("connection:{connection_id}:password")
}

fn passphrase_key(connection_id: &str) -> String {
    format!("connection:{connection_id}:passphrase")
}

/// Expand `~`, `~/...` and `~\...` using the current user's home directory.
/// Other paths are returned unchanged.
fn expand_home(path: &str) -> PathBuf {
    let rest = if path == "~" {
        Some("")
    } else if let Some(rest) = path.strip_prefix("~/") {
        Some(rest)
    } else if let Some(rest) = path.strip_prefix("~\\") {
        Some(rest)
    } else {
        None
    };
    let Some(rest) = rest else {
        return PathBuf::from(path);
    };
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return PathBuf::from(path);
    };
    if rest.is_empty() {
        PathBuf::from(home)
    } else {
        PathBuf::from(home).join(rest)
    }
}

/// Truncate oversized output and mask secrets before it leaves the core.
fn finalize_output(mut output: String, redactor: &Redactor) -> (String, bool) {
    let truncated = output.len() > MAX_COMMAND_OUTPUT_BYTES;
    if truncated {
        let mut cutoff = MAX_COMMAND_OUTPUT_BYTES;
        while cutoff > 0 && !output.is_char_boundary(cutoff) {
            cutoff -= 1;
        }
        output.truncate(cutoff);
        output.push_str("\n[输出已截断]");
    }
    (redactor.redact(&output), truncated)
}

#[allow(clippy::too_many_arguments)]
fn make_audit(
    source: AuditSource,
    connection: Option<&Connection>,
    command: Option<String>,
    risk: Option<RiskLevel>,
    action: PolicyAction,
    approval: Option<ApprovalDecision>,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    detail: impl Into<String>,
) -> AuditEntry {
    AuditEntry {
        id: String::new(),
        timestamp: String::new(),
        source,
        connection_id: connection.map(|item| item.id.clone()),
        connection_alias: connection.map(|item| item.alias.clone()),
        environment: connection.map(|item| item.environment),
        command,
        risk,
        action,
        approval,
        exit_code,
        duration_ms,
        detail: detail.into(),
    }
}
