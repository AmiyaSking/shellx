use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use chrono::Utc;
use uuid::Uuid;

use crate::audit::AuditLogger;
use crate::connections::ConnectionStore;
use crate::mcp;
use crate::models::{
    ApprovalDecision, ApprovalRequest, AuditEntry, AuditSource, AuthKind, CommandOutcome,
    CommandRequest, CommandResult, Connection, ConnectionInput, McpServerInfo, PolicyAction,
    PolicyDecision, RiskLevel, SessionInfo,
};
use crate::policy::PolicyEngine;
use crate::redaction::Redactor;
use crate::sessions::SessionManager;
use crate::ssh::{self, ConnectParams, Credential};
use crate::vault::SecretVault;

struct PendingApproval {
    request: ApprovalRequest,
    command: String,
    source: AuditSource,
}

/// Core services shared by the Tauri commands and the (future) MCP transport.
pub struct AppState {
    data_dir: PathBuf,
    connections: Mutex<ConnectionStore>,
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
            sessions: Mutex::new(SessionManager::default()),
            approvals: Mutex::new(HashMap::new()),
            vault: SecretVault::default(),
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
            "ShellX 核心启动（脚手架模式）",
        ));
        Ok(state)
    }

    pub fn list_connections(&self) -> Result<Vec<Connection>, String> {
        Ok(lock(&self.connections)?.list())
    }

    pub fn save_connection(&self, input: ConnectionInput) -> Result<Connection, String> {
        let password = input.password.clone();
        let connection = {
            let mut store = lock(&self.connections)?;
            store.upsert(input)?
        };
        if let Some(secret) = password {
            if !secret.is_empty() {
                self.vault.set(vault_key(&connection.id), secret);
            }
        }
        Ok(connection)
    }

    pub fn delete_connection(&self, id: &str) -> Result<(), String> {
        lock(&self.connections)?.remove(id)?;
        self.vault.remove(&vault_key(id));
        lock(&self.sessions)?.close_for_connection(id);
        Ok(())
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

    pub fn run_command(&self, request: CommandRequest) -> Result<CommandOutcome, String> {
        let _timeout_secs = request.timeout_secs; // wired with the real SSH channel in phase 1
        let connection = self.get_connection(&request.connection_id)?;
        let decision = self.policy.evaluate(
            connection.environment,
            connection.mcp_access,
            request.source,
            &request.command,
        );

        match decision.action {
            PolicyAction::Allow => self.execute_command(
                &connection,
                &request.command,
                request.source,
                decision,
                None,
            ),
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

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        approve: bool,
        command_override: Option<String>,
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
            let reason = "用户拒绝了该命令".to_string();
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
        )
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

    pub fn mcp_info(&self) -> Result<McpServerInfo, String> {
        let authorized_connections = lock(&self.connections)?.authorized_count();
        let active_sessions = lock(&self.sessions)?.active_count();
        Ok(McpServerInfo {
            transport: "stdio".into(),
            command: "shellx --mcp-stdio".into(),
            status: "scaffold".into(),
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
    ) -> Result<(String, ConnectParams), String> {
        let connection = self.get_connection(connection_id)?;
        let credential = match connection.auth.kind {
            AuthKind::Password => {
                let secret = self.vault.get(&vault_key(&connection.id)).ok_or_else(|| {
                    "该连接还没有保存口令（凭据只存在内存里，重启后需要重新填写）".to_string()
                })?;
                Credential::Password(secret)
            }
            AuthKind::PrivateKey => {
                let path = connection
                    .auth
                    .private_key_path
                    .clone()
                    .ok_or_else(|| "该连接未配置私钥文件路径".to_string())?;
                Credential::PrivateKey {
                    path: PathBuf::from(path),
                    passphrase: self.vault.get(&passphrase_key(&connection.id)),
                }
            }
            AuthKind::Agent => Credential::Agent,
        };

        Ok((
            connection.alias.clone(),
            ConnectParams {
                host: connection.host.clone(),
                port: connection.port,
                username: connection.username.clone(),
                credential,
                known_hosts_path: self.known_hosts_path(),
                term: ssh::DEFAULT_TERM.to_string(),
                cols: cols.max(1),
                rows: rows.max(1),
            },
        ))
    }

    fn get_connection(&self, id: &str) -> Result<Connection, String> {
        lock(&self.connections)?
            .get(id)
            .ok_or_else(|| "连接不存在".to_string())
    }

    fn execute_command(
        &self,
        connection: &Connection,
        command: &str,
        source: AuditSource,
        decision: PolicyDecision,
        approval: Option<ApprovalDecision>,
    ) -> Result<CommandOutcome, String> {
        let started = std::time::Instant::now();
        let (raw_output, exit_code) = simulate_command(connection, command);
        let output = self.redactor.redact(&raw_output);
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
            exit_code: Some(exit_code),
            output,
            truncated: false,
            risk: decision.risk,
            policy_action: decision.action,
            decision_reasons: decision.reasons.clone(),
            duration_ms,
            simulated: true,
        };

        let detail = if approval.is_some() {
            "审批通过后执行完成（模拟执行器）"
        } else {
            "命令执行完成（模拟执行器）"
        };
        self.audit.log(make_audit(
            source,
            Some(connection),
            Some(command.to_string()),
            Some(decision.risk),
            PolicyAction::Allow,
            approval,
            Some(exit_code),
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

/// Scaffold executor. Keeps policy, approval and audit flows fully functional
/// while the real SSH transport is being implemented.
fn simulate_command(connection: &Connection, command: &str) -> (String, i32) {
    let trimmed = command.trim();
    let lower = trimmed.to_lowercase();

    let output = if lower.starts_with("echo ") {
        trimmed.get(5..).unwrap_or_default().to_string()
    } else if lower == "whoami" {
        connection.username.clone()
    } else if lower == "hostname" {
        connection.alias.clone()
    } else if lower == "pwd" {
        format!("/home/{}", connection.username)
    } else if lower == "date" {
        Utc::now().to_rfc2822()
    } else if lower == "uptime" {
        " 14:32:01 up 42 days,  3:17,  2 users,  load average: 0.08, 0.12, 0.09".to_string()
    } else if lower == "ls" || lower.starts_with("ls ") || lower == "ll" {
        "app  config  logs  scripts  README.md".to_string()
    } else if lower.starts_with("df") {
        "Filesystem      Size  Used Avail Use% Mounted on\n/dev/vda1        80G   31G   45G  41% /"
            .to_string()
    } else if lower.starts_with("systemctl status") {
        format!(
            "● {}.service - simulated\n     Loaded: loaded\n     Active: active (running)",
            lower
                .split_whitespace()
                .nth(2)
                .unwrap_or("unknown")
        )
    } else {
        format!(
            "[shellx] 脚手架模拟执行：命令已通过策略引擎，但尚未接入真实 SSH 通道。\n\
             [shellx] target={}@{}:{}\n\
             [shellx] TODO(phase-1): 使用 russh 建立连接并回传真实输出。",
            connection.username, connection.host, connection.port
        )
    };

    (output, 0)
}
