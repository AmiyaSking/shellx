use serde::{Deserialize, Serialize};

/// Target environment. Production is always treated with the strictest policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Dev,
    Staging,
    Prod,
}

impl Default for Environment {
    fn default() -> Self {
        Self::Dev
    }
}

impl Environment {
    pub fn label(self) -> &'static str {
        match self {
            Environment::Dev => "dev",
            Environment::Staging => "staging",
            Environment::Prod => "prod",
        }
    }
}

/// What an MCP agent is allowed to do on a connection. Manual GUI sessions are
/// not constrained by this setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAccessMode {
    Disabled,
    ReadOnly,
    StandardControlled,
}

impl Default for McpAccessMode {
    fn default() -> Self {
        Self::Disabled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Password,
    PrivateKey,
    Agent,
}

impl Default for AuthKind {
    fn default() -> Self {
        Self::Agent
    }
}

/// Authentication metadata only. Real secrets are held by the vault and are
/// referenced by `secret_ref`; they are never persisted in connections.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    #[serde(default)]
    pub kind: AuthKind,
    #[serde(default)]
    pub secret_ref: Option<String>,
    #[serde(default)]
    pub private_key_path: Option<String>,
}

fn default_term() -> String {
    "xterm-256color".to_string()
}

fn default_encoding() -> String {
    "utf-8".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub environment: Environment,
    #[serde(default)]
    pub auth: AuthMethod,
    pub mcp_access: McpAccessMode,
    /// Allow MCP clients to open a full interactive PTY. Off by default
    /// because an interactive shell bypasses command-level policy.
    #[serde(default)]
    pub mcp_pty_enabled: bool,
    #[serde(default)]
    pub jump_host: Option<String>,
    #[serde(default)]
    pub remark: String,
    #[serde(default = "default_term")]
    pub term: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub startup_command: String,
    #[serde(default)]
    pub legacy_algorithms: bool,
    #[serde(default)]
    pub forward_agent: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    #[serde(default)]
    pub id: Option<String>,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub environment: Environment,
    #[serde(default)]
    pub auth: AuthMethod,
    #[serde(default)]
    pub mcp_access: McpAccessMode,
    #[serde(default)]
    pub mcp_pty_enabled: bool,
    #[serde(default)]
    pub jump_host: Option<String>,
    #[serde(default)]
    pub remark: String,
    #[serde(default = "default_term")]
    pub term: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default)]
    pub startup_command: String,
    #[serde(default)]
    pub legacy_algorithms: bool,
    #[serde(default)]
    pub forward_agent: bool,
    /// Plaintext secrets from the form. They are written to the OS credential
    /// store and never persisted in connections.json. An empty value leaves the
    /// previously saved secret untouched.
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub passphrase: Option<String>,
}

/// A host key the user explicitly confirmed in the fingerprint dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyTrust {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

impl HostKeyTrust {
    pub fn fingerprint_for(&self, host: &str, port: u16) -> Option<&str> {
        if self.port == port && self.host.eq_ignore_ascii_case(host) {
            Some(self.fingerprint.as_str())
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl Default for RiskLevel {
    fn default() -> Self {
        Self::Medium
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Allow,
    RequireApproval,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyDecision {
    pub action: PolicyAction,
    pub risk: RiskLevel,
    pub reasons: Vec<String>,
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSource {
    User,
    Agent,
    System,
}

impl Default for AuditSource {
    fn default() -> Self {
        Self::User
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRequest {
    pub connection_id: String,
    pub command: String,
    #[serde(default)]
    pub source: AuditSource,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub id: String,
    pub session_id: String,
    pub connection_id: String,
    pub connection_alias: String,
    pub environment: Environment,
    pub command: String,
    pub exit_code: Option<i32>,
    pub output: String,
    pub truncated: bool,
    pub risk: RiskLevel,
    pub policy_action: PolicyAction,
    pub decision_reasons: Vec<String>,
    pub duration_ms: u64,
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub connection_id: String,
    pub connection_alias: String,
    pub environment: Environment,
    pub command: String,
    pub risk: RiskLevel,
    pub reasons: Vec<String>,
    pub source: AuditSource,
    pub requested_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommandOutcome {
    Completed {
        result: CommandResult,
    },
    PendingApproval {
        approval: ApprovalRequest,
    },
    Blocked {
        decision: PolicyDecision,
        #[serde(rename = "auditId")]
        audit_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: String,
    pub source: AuditSource,
    pub connection_id: Option<String>,
    pub connection_alias: Option<String>,
    pub environment: Option<Environment>,
    pub command: Option<String>,
    pub risk: Option<RiskLevel>,
    pub action: PolicyAction,
    pub approval: Option<ApprovalDecision>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Closed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub connection_id: String,
    pub connection_alias: String,
    pub created_at: String,
    pub last_active_at: String,
    pub status: SessionStatus,
    pub output_lines: usize,
    pub output_bytes: usize,
    /// A command is currently executing in this session.
    pub running: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub transport: String,
    pub command: String,
    pub status: String,
    pub tools: Vec<McpToolInfo>,
    pub enabled_tools: usize,
    pub authorized_connections: usize,
    pub active_sessions: usize,
}
