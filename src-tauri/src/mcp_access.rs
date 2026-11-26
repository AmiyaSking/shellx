//! Versioned, configurable MCP access policy.
//!
//! The old model hard-coded an environment matrix (dev/staging/prod) in
//! `policy.rs`. This module replaces it with a resource tree:
//!
//! ```text
//! global defaults -> group rule -> connection rule -> capability ceiling
//! ```
//!
//! Each node can override the access ceiling and the confirmation threshold.
//! Visibility is configured separately. Critical commands and other hard
//! protections are never relaxed by this policy.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{Connection, Environment, McpAccessMode, RiskLevel};

pub const MCP_POLICY_VERSION: u32 = 2;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    #[default]
    None,
    Read,
    Write,
    Full,
}

impl AccessLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "不可访问",
            Self::Read => "只读",
            Self::Write => "读写",
            Self::Full => "完全访问",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmLevel {
    Off,
    Medium,
    #[default]
    High,
}

impl ConfirmLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "关闭",
            Self::Medium => "中风险起",
            Self::High => "高风险起",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPolicyDefaults {
    #[serde(default)]
    pub access: AccessLevel,
    #[serde(default)]
    pub confirm: ConfirmLevel,
}

impl Default for McpPolicyDefaults {
    fn default() -> Self {
        Self {
            access: AccessLevel::Read,
            confirm: ConfirmLevel::High,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityMode {
    #[default]
    All,
    Selected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpRuleTarget {
    Group { name: String },
    Connection { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPolicyRule {
    pub target: McpRuleTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<AccessLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<ConfirmLevel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilityOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sftp: Option<AccessLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarding: Option<AccessLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPolicy {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub defaults: McpPolicyDefaults,
    #[serde(default)]
    pub visibility: VisibilityMode,
    #[serde(default)]
    pub rules: Vec<McpPolicyRule>,
    /// Per-connection ceilings that can only tighten the connection access.
    #[serde(default)]
    pub capabilities: HashMap<String, McpCapabilityOverrides>,
    /// Tool name -> enabled. Missing entries are enabled.
    #[serde(default)]
    pub tools: HashMap<String, bool>,
}

fn default_version() -> u32 {
    MCP_POLICY_VERSION
}

impl Default for McpPolicy {
    fn default() -> Self {
        Self {
            version: MCP_POLICY_VERSION,
            defaults: McpPolicyDefaults::default(),
            visibility: VisibilityMode::All,
            rules: Vec::new(),
            capabilities: HashMap::new(),
            tools: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessSource {
    Global,
    Group,
    Connection,
}

impl AccessSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "全局",
            Self::Group => "分组",
            Self::Connection => "连接",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Exec,
    Sftp,
    Forwarding,
    Pty,
}

/// Effective values for one connection after inheritance.
#[derive(Debug, Clone)]
pub struct EffectiveAccess {
    pub access: AccessLevel,
    pub confirm: ConfirmLevel,
    pub visible: bool,
    pub source: AccessSource,
    pub confirm_source: AccessSource,
    pub group_name: Option<String>,
    pub sftp: AccessLevel,
    pub forwarding: AccessLevel,
}

impl EffectiveAccess {
    pub fn level_for(&self, capability: Capability) -> AccessLevel {
        match capability {
            Capability::Exec | Capability::Pty => self.access,
            Capability::Sftp => self.sftp,
            Capability::Forwarding => self.forwarding,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionAccess {
    pub connection_id: String,
    pub alias: String,
    pub group: String,
    pub environment: Environment,
    pub visible: bool,
    pub access: AccessLevel,
    pub confirm: ConfirmLevel,
    pub source: AccessSource,
    pub confirm_source: AccessSource,
    pub group_name: Option<String>,
    pub sftp_access: AccessLevel,
    pub forwarding_access: AccessLevel,
    pub pty_enabled: bool,
    pub pty_allowed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PolicyVerdict {
    pub allowed: bool,
    pub needs_confirmation: bool,
    pub visible: bool,
    pub effective: AccessLevel,
    pub source: AccessSource,
    pub confirm: ConfirmLevel,
    pub confirm_source: AccessSource,
}

impl McpPolicy {
    /// Load a policy from disk. A missing file returns `None`; a corrupt or
    /// future-version file returns a locked policy so a bad file never widens
    /// access.
    pub fn load(path: &Path) -> Result<Option<Self>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => {
                return Err(format!("读取 MCP 权限策略失败：{error}"))
            }
        };
        let policy = match serde_json::from_str::<Self>(&text) {
            Ok(policy) if policy.version <= MCP_POLICY_VERSION => policy.normalized(),
            Ok(policy) => {
                eprintln!(
                    "[shellx] MCP 权限策略版本 {} 高于当前支持的 {}，已锁定",
                    policy.version, MCP_POLICY_VERSION
                );
                Self::locked()
            }
            Err(error) => {
                eprintln!("[shellx] MCP 权限策略解析失败，已锁定：{error}");
                Self::locked()
            }
        };
        Ok(Some(policy))
    }

    pub fn persist(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建 MCP 权限目录：{error}"))?;
        }
        let json = serde_json::to_string_pretty(&self.normalized())
            .map_err(|error| format!("序列化 MCP 权限策略失败：{error}"))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json)
            .map_err(|error| format!("写入 MCP 权限策略失败：{error}"))?;
        std::fs::rename(&temp, path)
            .map_err(|error| format!("替换 MCP 权限策略失败：{error}"))
    }

    pub fn locked() -> Self {
        Self {
            version: MCP_POLICY_VERSION,
            defaults: McpPolicyDefaults {
                access: AccessLevel::None,
                confirm: ConfirmLevel::High,
            },
            visibility: VisibilityMode::Selected,
            rules: Vec::new(),
            capabilities: HashMap::new(),
            tools: HashMap::new(),
        }
    }

    /// Build a policy that preserves the legacy environment matrix for every
    /// existing connection.
    pub fn from_connections(connections: &[Connection]) -> Self {
        let mut policy = Self::default();
        for connection in connections {
            let (visible, access, confirm) = legacy_access(connection);
            // Skip rules that would merely restate the global defaults; this
            // keeps the exceptions-only view meaningful after migration.
            if visible
                && access == policy.defaults.access
                && confirm == policy.defaults.confirm
            {
                continue;
            }
            policy.apply_legacy_connection(connection);
        }
        policy
    }

    pub fn normalized(&self) -> Self {
        let mut policy = self.clone();
        policy.version = MCP_POLICY_VERSION;
        let mut rules: Vec<McpPolicyRule> = Vec::with_capacity(policy.rules.len());
        for rule in policy.rules.drain(..) {
            if let Some(existing) = rules
                .iter_mut()
                .find(|existing| existing.target == rule.target)
            {
                *existing = rule;
            } else {
                rules.push(rule);
            }
        }
        policy.rules = rules;
        policy
    }

    /// Update the connection rule for the legacy `mcpAccess` quick preset while
    /// preserving capability overrides.
    pub fn apply_legacy_connection(&mut self, connection: &Connection) {
        let (visible, access, confirm) = legacy_access(connection);
        let rule = self
            .rules
            .iter_mut()
            .find(|rule| {
                rule.target
                    == McpRuleTarget::Connection {
                        id: connection.id.clone(),
                    }
            });
        match rule {
            Some(rule) => {
                rule.visible = Some(visible);
                rule.access = Some(access);
                rule.confirm = Some(confirm);
            }
            None => self.rules.push(McpPolicyRule {
                target: McpRuleTarget::Connection {
                    id: connection.id.clone(),
                },
                visible: Some(visible),
                access: Some(access),
                confirm: Some(confirm),
            }),
        }
    }

    pub fn remove_connection(&mut self, connection_id: &str) {
        self.rules.retain(|rule| {
            rule.target
                != McpRuleTarget::Connection {
                    id: connection_id.to_string(),
                }
        });
        self.capabilities.remove(connection_id);
    }

    pub fn tool_enabled(&self, name: &str) -> bool {
        self.tools.get(name).copied().unwrap_or(true)
    }

    pub fn effective(&self, connection: &Connection) -> EffectiveAccess {
        let mut access = self.defaults.access;
        let mut confirm = self.defaults.confirm;
        let mut source = AccessSource::Global;
        let mut confirm_source = AccessSource::Global;
        let mut group_name = None;
        let mut visible_override = None;

        let group = connection.group.trim();
        if let Some(rule) = self.rules.iter().find(|rule| {
            rule.target
                == McpRuleTarget::Group {
                    name: group.to_string(),
                }
        }) {
            if let Some(value) = rule.access {
                access = value;
                source = AccessSource::Group;
                group_name = Some(group.to_string());
            }
            if let Some(value) = rule.confirm {
                confirm = value;
                confirm_source = AccessSource::Group;
            }
            if let Some(value) = rule.visible {
                visible_override = Some(value);
            }
        }

        if let Some(rule) = self.rules.iter().find(|rule| {
            rule.target
                == McpRuleTarget::Connection {
                    id: connection.id.clone(),
                }
        }) {
            if let Some(value) = rule.access {
                access = value;
                source = AccessSource::Connection;
                group_name = None;
            }
            if let Some(value) = rule.confirm {
                confirm = value;
                confirm_source = AccessSource::Connection;
            }
            if let Some(value) = rule.visible {
                visible_override = Some(value);
            }
        }

        let visible = match self.visibility {
            VisibilityMode::All => visible_override.unwrap_or(true),
            VisibilityMode::Selected => visible_override.unwrap_or(false),
        } && access != AccessLevel::None;

        let caps = self.capabilities.get(&connection.id);
        let sftp = min_level(
            access,
            caps.and_then(|caps| caps.sftp).unwrap_or(AccessLevel::Full),
        );
        let forwarding = min_level(
            access,
            caps.and_then(|caps| caps.forwarding)
                .unwrap_or(AccessLevel::Full),
        );

        EffectiveAccess {
            access,
            confirm,
            visible,
            source,
            confirm_source,
            group_name,
            sftp,
            forwarding,
        }
    }

    pub fn describe(&self, connection: &Connection) -> McpConnectionAccess {
        let effective = self.effective(connection);
        McpConnectionAccess {
            connection_id: connection.id.clone(),
            alias: connection.alias.clone(),
            group: connection.group.clone(),
            environment: connection.environment,
            visible: effective.visible,
            access: effective.access,
            confirm: effective.confirm,
            source: effective.source,
            confirm_source: effective.confirm_source,
            group_name: effective.group_name,
            sftp_access: effective.sftp,
            forwarding_access: effective.forwarding,
            pty_enabled: connection.mcp_pty_enabled,
            pty_allowed: effective.visible
                && effective.access >= AccessLevel::Full
                && connection.mcp_pty_enabled,
        }
    }

    /// Check a call that requires `required`. Critical commands and hard
    /// protections are handled by the caller before this method.
    pub fn check(
        &self,
        connection: &Connection,
        capability: Capability,
        required: AccessLevel,
    ) -> PolicyVerdict {
        let effective = self.effective(connection);
        let allowed_level = effective.level_for(capability);
        let allowed = effective.visible
            && effective.access != AccessLevel::None
            && required <= allowed_level;
        // Interactive PTY is gated by the explicit per-connection opt-in; once
        // enabled it must remain usable from stdio, which has no approval UI.
        let needs_confirmation = allowed
            && capability != Capability::Pty
            && needs_confirmation(required, effective.confirm);
        PolicyVerdict {
            allowed,
            needs_confirmation,
            visible: effective.visible,
            effective: allowed_level,
            source: effective.source,
            confirm: effective.confirm,
            confirm_source: effective.confirm_source,
        }
    }
}

pub fn legacy_access(connection: &Connection) -> (bool, AccessLevel, ConfirmLevel) {
    match connection.mcp_access {
        McpAccessMode::Disabled => (false, AccessLevel::None, ConfirmLevel::High),
        McpAccessMode::ReadOnly => (true, AccessLevel::Read, ConfirmLevel::High),
        McpAccessMode::StandardControlled => match connection.environment {
            Environment::Dev => (true, AccessLevel::Full, ConfirmLevel::High),
            Environment::Staging | Environment::Prod => {
                (true, AccessLevel::Full, ConfirmLevel::Medium)
            }
        },
    }
}

pub fn required_level_for_risk(risk: RiskLevel) -> Option<AccessLevel> {
    match risk {
        RiskLevel::Low => Some(AccessLevel::Read),
        RiskLevel::Medium => Some(AccessLevel::Write),
        RiskLevel::High => Some(AccessLevel::Full),
        RiskLevel::Critical => None,
    }
}

pub fn needs_confirmation(required: AccessLevel, confirm: ConfirmLevel) -> bool {
    match confirm {
        ConfirmLevel::Off => false,
        ConfirmLevel::Medium => required >= AccessLevel::Write,
        ConfirmLevel::High => required >= AccessLevel::Full,
    }
}

fn min_level(left: AccessLevel, right: AccessLevel) -> AccessLevel {
    if left < right {
        left
    } else {
        right
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AuthMethod, Connection};

    fn connection(id: &str, group: &str, environment: Environment, access: McpAccessMode) -> Connection {
        Connection {
            id: id.to_string(),
            alias: id.to_string(),
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "tester".to_string(),
            group: group.to_string(),
            tags: Vec::new(),
            environment,
            auth: AuthMethod::default(),
            mcp_access: access,
            mcp_pty_enabled: false,
            jump_host: None,
            remark: String::new(),
            term: "xterm-256color".to_string(),
            encoding: "utf-8".to_string(),
            startup_command: String::new(),
            legacy_algorithms: false,
            forward_agent: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn legacy_matrix_is_preserved() {
        let dev = connection("dev", "g", Environment::Dev, McpAccessMode::StandardControlled);
        let staging = connection("stg", "g", Environment::Staging, McpAccessMode::StandardControlled);
        let read_only = connection("ro", "g", Environment::Dev, McpAccessMode::ReadOnly);
        let disabled = connection("off", "g", Environment::Dev, McpAccessMode::Disabled);
        let policy = McpPolicy::from_connections(&[dev.clone(), staging.clone(), read_only.clone(), disabled.clone()]);

        let dev_access = policy.effective(&dev);
        assert_eq!(dev_access.access, AccessLevel::Full);
        assert_eq!(dev_access.confirm, ConfirmLevel::High);
        assert!(dev_access.visible);

        let staging_access = policy.effective(&staging);
        assert_eq!(staging_access.access, AccessLevel::Full);
        assert_eq!(staging_access.confirm, ConfirmLevel::Medium);

        let read_only_access = policy.effective(&read_only);
        assert_eq!(read_only_access.access, AccessLevel::Read);
        assert!(read_only_access.visible);

        let disabled_access = policy.effective(&disabled);
        assert_eq!(disabled_access.access, AccessLevel::None);
        assert!(!disabled_access.visible);
    }

    #[test]
    fn connection_overrides_group_and_inherits_confirm() {
        let conn = connection("c", "g", Environment::Prod, McpAccessMode::StandardControlled);
        let mut policy = McpPolicy::default();
        policy.rules.push(McpPolicyRule {
            target: McpRuleTarget::Group { name: "g".into() },
            visible: Some(true),
            access: Some(AccessLevel::Full),
            confirm: Some(ConfirmLevel::Medium),
        });
        policy.rules.push(McpPolicyRule {
            target: McpRuleTarget::Connection { id: "c".into() },
            visible: None,
            access: Some(AccessLevel::Read),
            confirm: None,
        });
        let effective = policy.effective(&conn);
        assert_eq!(effective.access, AccessLevel::Read);
        assert_eq!(effective.source, AccessSource::Connection);
        assert_eq!(effective.confirm, ConfirmLevel::Medium);
        assert_eq!(effective.confirm_source, AccessSource::Group);
    }

    #[test]
    fn ungrouped_connections_can_have_a_group_rule() {
        let conn = connection("c", "", Environment::Dev, McpAccessMode::StandardControlled);
        let mut policy = McpPolicy::default();
        policy.rules.push(McpPolicyRule {
            target: McpRuleTarget::Group { name: "".into() },
            visible: Some(true),
            access: Some(AccessLevel::Write),
            confirm: None,
        });
        assert_eq!(policy.effective(&conn).access, AccessLevel::Write);
        assert!(policy.effective(&conn).visible);
    }

    #[test]
    fn selected_visibility_requires_an_explicit_rule() {
        let conn = connection("c", "g", Environment::Dev, McpAccessMode::StandardControlled);
        let mut policy = McpPolicy::from_connections(std::slice::from_ref(&conn));
        policy.visibility = VisibilityMode::Selected;
        // from_connections sets visible=true on the connection rule.
        assert!(policy.effective(&conn).visible);

        policy.rules.clear();
        assert!(!policy.effective(&conn).visible);
    }

    #[test]
    fn capability_ceilings_can_only_tighten() {
        let conn = connection("c", "g", Environment::Dev, McpAccessMode::StandardControlled);
        let mut policy = McpPolicy::from_connections(std::slice::from_ref(&conn));
        policy.capabilities.insert(
            "c".into(),
            McpCapabilityOverrides {
                sftp: Some(AccessLevel::Read),
                forwarding: Some(AccessLevel::Full),
            },
        );
        let effective = policy.effective(&conn);
        assert_eq!(effective.access, AccessLevel::Full);
        assert_eq!(effective.sftp, AccessLevel::Read);
        assert_eq!(effective.forwarding, AccessLevel::Full);
    }

    #[test]
    fn confirmation_thresholds_match_the_old_matrix() {
        assert!(needs_confirmation(AccessLevel::Write, ConfirmLevel::Medium));
        assert!(needs_confirmation(AccessLevel::Full, ConfirmLevel::High));
        assert!(!needs_confirmation(AccessLevel::Write, ConfirmLevel::High));
        assert!(!needs_confirmation(AccessLevel::Full, ConfirmLevel::Off));
    }

    #[test]
    fn future_policy_versions_are_locked() {
        let locked = McpPolicy::locked();
        let conn = connection("c", "g", Environment::Dev, McpAccessMode::StandardControlled);
        assert!(!locked.effective(&conn).visible);
        assert_eq!(locked.effective(&conn).access, AccessLevel::None);
    }

    #[test]
    fn corrupt_policy_file_locks_access_instead_of_widening_it() {
        let dir = std::env::temp_dir().join(format!(
            "shellx-policy-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("mcp_policy.json");
        std::fs::write(&path, "{not valid json").expect("write corrupt policy");
        let policy = McpPolicy::load(&path)
            .expect("load should not fail")
            .expect("file exists");
        let conn = connection("c", "g", Environment::Dev, McpAccessMode::StandardControlled);
        assert!(!policy.effective(&conn).visible);
        assert_eq!(policy.effective(&conn).access, AccessLevel::None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tools_are_enabled_unless_explicitly_disabled() {
        let mut policy = McpPolicy::default();
        assert!(policy.tool_enabled("exec_command"));
        policy.tools.insert("exec_command".into(), false);
        assert!(!policy.tool_enabled("exec_command"));
    }
}
