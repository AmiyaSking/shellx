use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::Utc;
use uuid::Uuid;

use crate::audit::AuditLogger;
use crate::connections::ConnectionStore;
use crate::forwards::{ForwardRule, ForwardRuleInput, ForwardStore};
use crate::mcp;
use crate::models::{
    ApprovalDecision, ApprovalRequest, AuditEntry, AuditSource, AuthKind, CommandOutcome,
    CommandRequest, CommandResult, Connection, ConnectionInput, HostKeyTrust, McpServerInfo,
    PolicyAction, PolicyDecision, RiskLevel, SessionInfo,
};
use crate::policy::PolicyEngine;
use crate::redaction::Redactor;
use crate::sessions::SessionManager;
use crate::ssh::{self, ConnectParams, Credential, JumpHost};
use crate::vault::SecretVault;

struct PendingApproval {
    request: ApprovalRequest,
    command: String,
    source: AuditSource,
    timeout_secs: Option<u64>,
}

/// Default and maximum wall-clock budget for one agent command.
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 30;
const MAX_COMMAND_TIMEOUT_SECS: u64 = 600;
/// Upper bound on the output returned to an agent or kept in the session ring.
const MAX_COMMAND_OUTPUT_BYTES: usize = 256 * 1024;

/// Core services shared by the Tauri commands and the (future) MCP transport.
pub struct AppState {
    data_dir: PathBuf,
    connections: Mutex<ConnectionStore>,
    forwards: Mutex<ForwardStore>,
    sessions: Mutex<SessionManager>,
    approvals: Mutex<HashMap<String, PendingApproval>>,
    vault: SecretVault,
    audit: AuditLogger,
    policy: PolicyEngine,
    redactor: Redactor,
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
            sessions: Mutex::new(SessionManager::default()),
            approvals: Mutex::new(HashMap::new()),
            vault: SecretVault::new(),
            audit: AuditLogger::new(&data_dir),
            policy: PolicyEngine,
            redactor: Redactor::new(),
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

    pub async fn run_command(&self, request: CommandRequest) -> Result<CommandOutcome, String> {
        let connection = self.get_connection(&request.connection_id)?;
        let decision = self.policy.evaluate(
            connection.environment,
            connection.mcp_access,
            request.source,
            &request.command,
        );

        match decision.action {
            PolicyAction::Allow => {
                self.execute_command(
                    &connection,
                    &request.command,
                    request.source,
                    decision,
                    None,
                    request.timeout_secs,
                )
                .await
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
                Ok(CommandOutcome::PendingApproval { approval })
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
                Ok(CommandOutcome::Blocked {
                    decision,
                    audit_id: entry.id,
                    message,
                })
            }
        }
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
        self.execute_command(
            &connection,
            &command,
            pending.source,
            decision,
            Some(ApprovalDecision::Approved),
            pending.timeout_secs,
        )
        .await
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

    pub fn list_audit(&self, limit: usize) -> Vec<AuditEntry> {
        self.audit.recent(limit)
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, String> {
        Ok(lock(&self.sessions)?.list())
    }

    pub fn close_session(&self, id: &str) -> Result<bool, String> {
        Ok(lock(&self.sessions)?.close(id))
    }

    /// Paginated access to a command session's output ring. Returns the text,
    /// the offset to pass next time, and whether the buffer is exhausted.
    pub fn read_session_output(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(String, usize, bool), String> {
        lock(&self.sessions)?.read_output(session_id, offset, limit)
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

    /// Execute one command over a dedicated SSH `exec` channel.
    ///
    /// The host key must already be trusted (confirmed in the terminal), so an
    /// agent can never silently accept a new key. Policy, approval and audit
    /// have already run by the time this is called.
    async fn execute_command(
        &self,
        connection: &Connection,
        command: &str,
        source: AuditSource,
        decision: PolicyDecision,
        approval: Option<ApprovalDecision>,
        timeout_secs: Option<u64>,
    ) -> Result<CommandOutcome, String> {
        let started = Instant::now();
        let (_, mut params) = self.pty_params(&connection.id, 80, 24, Vec::new())?;
        params.agent_forwarding = false;
        let timeout = Duration::from_secs(
            timeout_secs
                .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS)
                .clamp(1, MAX_COMMAND_TIMEOUT_SECS),
        );

        let exec = match ssh::exec_command(&params, command, timeout).await {
            Ok(exec) => exec,
            Err(error) => {
                let detail = format!("命令执行失败：{error}");
                let _ = self.audit.log(make_audit(
                    source,
                    Some(connection),
                    Some(command.to_string()),
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

        let mut output = exec.output;
        let truncated = output.len() > MAX_COMMAND_OUTPUT_BYTES;
        if truncated {
            let mut cutoff = MAX_COMMAND_OUTPUT_BYTES;
            while cutoff > 0 && !output.is_char_boundary(cutoff) {
                cutoff -= 1;
            }
            output.truncate(cutoff);
            output.push_str("\n[输出已截断]");
        }
        let output = self.redactor.redact(&output);
        let exit_code = exec.exit_status.map(|status| status as i32);
        let duration_ms = started.elapsed().as_millis() as u64;

        let session_id = {
            let mut sessions = lock(&self.sessions)?;
            let id = {
                let session = sessions.get_or_create(&connection.id, &connection.alias);
                session.id.clone()
            };
            sessions.append_output(&id, &format!("$ {command}\n{output}"));
            id
        };

        let result = CommandResult {
            id: Uuid::new_v4().to_string(),
            session_id,
            connection_id: connection.id.clone(),
            connection_alias: connection.alias.clone(),
            environment: connection.environment,
            command: command.to_string(),
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
        self.audit.log(make_audit(
            source,
            Some(connection),
            Some(command.to_string()),
            Some(decision.risk),
            PolicyAction::Allow,
            approval,
            exit_code,
            Some(duration_ms),
            detail,
        ))?;

        Ok(CommandOutcome::Completed { result })
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
