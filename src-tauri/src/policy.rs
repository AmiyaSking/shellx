use crate::models::{
    AuditSource, Environment, McpAccessMode, PolicyAction, PolicyDecision, RiskLevel,
};

/// Deterministic command policy. This runs in the Rust core, never in the
/// frontend, so an AI agent cannot bypass it by prompt manipulation.
pub struct PolicyEngine;

impl PolicyEngine {
    pub fn evaluate(
        &self,
        environment: Environment,
        mcp_access: McpAccessMode,
        source: AuditSource,
        command: &str,
    ) -> PolicyDecision {
        let (risk, mut reasons) = classify(command);

        // Catastrophic commands are blocked for everyone, including manual use.
        if risk == RiskLevel::Critical {
            return policy(PolicyAction::Block, risk, reasons, "critical-destructive");
        }

        // MCP authorization gates only agent-originated calls.
        if source == AuditSource::Agent {
            match mcp_access {
                McpAccessMode::Disabled => {
                    reasons.push("该连接未授权给 MCP".into());
                    return policy(PolicyAction::Block, risk, reasons, "mcp-not-authorized");
                }
                McpAccessMode::ReadOnly if risk != RiskLevel::Low => {
                    reasons.push("该连接对 MCP 为只读模式，仅允许低风险命令".into());
                    return policy(PolicyAction::Block, risk, reasons, "mcp-read-only");
                }
                _ => {}
            }
        }

        let action = match environment {
            Environment::Dev => match risk {
                RiskLevel::Low | RiskLevel::Medium => PolicyAction::Allow,
                RiskLevel::High => PolicyAction::RequireApproval,
                RiskLevel::Critical => PolicyAction::Block,
            },
            Environment::Staging | Environment::Prod => match risk {
                RiskLevel::Low => PolicyAction::Allow,
                RiskLevel::Medium | RiskLevel::High => PolicyAction::RequireApproval,
                RiskLevel::Critical => PolicyAction::Block,
            },
        };

        if environment == Environment::Prod && risk >= RiskLevel::Medium {
            reasons.push("生产环境：中高风险命令强制人工审批".into());
        }

        PolicyDecision {
            action,
            risk,
            reasons,
            rule_id: Some(format!("env-{}", environment.label())),
        }
    }
}

fn policy(
    action: PolicyAction,
    risk: RiskLevel,
    reasons: Vec<String>,
    rule_id: &str,
) -> PolicyDecision {
    PolicyDecision {
        action,
        risk,
        reasons,
        rule_id: Some(rule_id.to_string()),
    }
}

fn classify(command: &str) -> (RiskLevel, Vec<String>) {
    let normalized = command.trim().to_lowercase();
    if normalized.is_empty() {
        return (RiskLevel::Medium, vec!["空命令".into()]);
    }
    if is_catastrophic(&normalized) {
        return (
            RiskLevel::Critical,
            vec!["命中极高危破坏性命令规则".into()],
        );
    }
    if let Some(reason) = high_risk_reason(&normalized) {
        return (RiskLevel::High, vec![reason.to_string()]);
    }
    if normalized.contains('>') {
        return (
            RiskLevel::Medium,
            vec!["包含输出重定向，可能写入文件".into()],
        );
    }
    if is_low_risk(&normalized) {
        return (RiskLevel::Low, vec!["只读检查类命令".into()]);
    }
    (
        RiskLevel::Medium,
        vec!["未命中只读白名单，按中风险处理".into()],
    )
}

/// Hard blocks: destructive actions that should never run, even with approval.
fn is_catastrophic(command: &str) -> bool {
    if command.contains(":(){ :|:& };:") || command.contains(":(){:|:&};:") {
        return true;
    }
    if command.starts_with("mkfs") || command.contains(" mkfs.") {
        return true;
    }
    if command.contains("of=/dev/sd")
        || command.contains("of=/dev/nvme")
        || command.contains("of=/dev/vd")
        || command.contains("> /dev/sd")
        || command.contains(">/dev/sd")
    {
        return true;
    }

    let mut tokens: Vec<&str> = command.split_whitespace().collect();
    if tokens.first() == Some(&"sudo") {
        tokens.remove(0);
    }
    if tokens.first() == Some(&"rm") {
        let recursive = tokens.iter().any(|token| {
            *token == "--recursive"
                || (token.starts_with('-') && !token.starts_with("--") && token.contains('r'))
        });
        let targets_root = tokens.iter().skip(1).any(|token| {
            let cleaned = token.trim_matches(|character| character == '"' || character == '\'');
            cleaned == "/" || cleaned == "/*"
        });
        if recursive && targets_root {
            return true;
        }
    }
    false
}

fn high_risk_reason(command: &str) -> Option<&'static str> {
    let mut split = command.split_whitespace();
    let first = split.next()?;
    let rest: Vec<&str> = split.collect();

    if first == "sudo" {
        return Some("请求提权执行");
    }
    if first == "su" {
        return Some("请求切换系统用户");
    }

    match first {
        "rm" | "rmdir" | "shred" => return Some("包含删除操作"),
        "chmod" | "chown" | "chgrp" => return Some("变更文件权限或所有者"),
        "kill" | "pkill" | "killall" => return Some("终止进程"),
        "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel" => {
            return Some("变更系统用户")
        }
        "passwd" | "visudo" => return Some("变更账户或提权配置"),
        "crontab" | "at" => return Some("变更计划任务"),
        "mount" | "umount" | "fdisk" | "parted" | "truncate" => {
            return Some("变更磁盘或文件系统")
        }
        "iptables" | "ip6tables" | "nft" | "ufw" | "firewall-cmd" => {
            return Some("变更防火墙规则")
        }
        "shutdown" | "reboot" | "poweroff" | "halt" | "init" => {
            return Some("变更主机运行状态")
        }
        "systemctl" => {
            if rest.first().map_or(false, |action| {
                matches!(
                    *action,
                    "restart"
                        | "stop"
                        | "start"
                        | "disable"
                        | "enable"
                        | "mask"
                        | "daemon-reload"
                )
            }) {
                return Some("变更系统服务状态");
            }
        }
        "service" => return Some("变更系统服务状态"),
        "docker" => {
            if rest.first().map_or(false, |action| {
                matches!(
                    *action,
                    "rm" | "rmi" | "stop" | "kill" | "prune" | "run" | "exec"
                )
            }) {
                return Some("变更容器或执行容器内命令");
            }
        }
        "kubectl" => {
            if rest.first().map_or(false, |action| {
                matches!(
                    *action,
                    "delete" | "apply" | "scale" | "rollout" | "edit" | "patch" | "exec"
                )
            }) {
                return Some("变更集群资源");
            }
        }
        "git" => {
            if rest.first() == Some(&"push") {
                return Some("推送远端仓库");
            }
            if rest.first() == Some(&"reset") && rest.contains(&"--hard") {
                return Some("重置工作区历史");
            }
            if rest.first() == Some(&"clean") {
                return Some("清理未跟踪文件");
            }
        }
        "sed" => {
            if rest
                .iter()
                .any(|argument| *argument == "--in-place" || argument.starts_with("-i"))
            {
                return Some("原地修改文件");
            }
        }
        "find" => {
            if rest
                .iter()
                .any(|argument| matches!(*argument, "-delete" | "-exec" | "-execdir"))
            {
                return Some("查找并删除或执行命令");
            }
        }
        _ => {}
    }

    if command.starts_with("apt ")
        || command.starts_with("apt-get ")
        || command.starts_with("yum ")
        || command.starts_with("dnf ")
        || command.starts_with("pacman ")
    {
        let mut words = command.split_whitespace();
        if words.any(|word| {
            matches!(
                word,
                "install" | "remove" | "purge" | "upgrade" | "dist-upgrade" | "erase"
            )
        }) {
            return Some("安装或删除系统软件包");
        }
    }
    if command.starts_with("pip ")
        || command.starts_with("pip3 ")
        || command.starts_with("npm ")
    {
        if command
            .split_whitespace()
            .any(|word| word == "install" || word == "uninstall")
        {
            return Some("安装或删除运行时依赖");
        }
    }

    None
}

fn is_low_risk(command: &str) -> bool {
    let mut split = command.split_whitespace();
    let first = match split.next() {
        Some(token) => token,
        None => return false,
    };
    let second = split.next().unwrap_or("");

    match first {
        "ls" | "ll" | "pwd" | "whoami" | "hostname" | "date" | "uptime" | "uname" | "df"
        | "du" | "free" | "ps" | "top" | "htop" | "cat" | "head" | "tail" | "less"
        | "more" | "grep" | "wc" | "find" | "stat" | "file" | "tree" | "echo" | "env"
        | "printenv" | "ss" | "netstat" | "ping" | "traceroute" | "dig" | "nslookup"
        | "journalctl" => true,
        "systemctl" => matches!(
            second,
            "status"
                | "is-active"
                | "is-enabled"
                | "is-failed"
                | "show"
                | "cat"
                | "list-units"
                | "list-unit-files"
        ),
        "docker" => matches!(
            second,
            "ps" | "logs" | "inspect" | "images" | "stats" | "version" | "info" | "top" | "port"
        ),
        "kubectl" => matches!(
            second,
            "get" | "describe" | "logs" | "top" | "version" | "explain" | "api-resources"
        ),
        "git" => matches!(
            second,
            "status" | "log" | "diff" | "show" | "branch" | "remote" | "rev-parse" | "blame"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::PolicyEngine;
    use crate::models::{
        AuditSource, Environment, McpAccessMode, PolicyAction, RiskLevel,
    };

    fn evaluate(
        command: &str,
        environment: Environment,
        mcp_access: McpAccessMode,
        source: AuditSource,
    ) -> crate::models::PolicyDecision {
        PolicyEngine.evaluate(environment, mcp_access, source, command)
    }

    #[test]
    fn blocks_root_deletion_even_for_manual_user() {
        let decision = evaluate(
            "rm -rf /",
            Environment::Dev,
            McpAccessMode::StandardControlled,
            AuditSource::User,
        );
        assert_eq!(decision.action, PolicyAction::Block);
        assert_eq!(decision.risk, RiskLevel::Critical);
    }

    #[test]
    fn allows_read_only_for_agent_with_read_only_access() {
        let decision = evaluate(
            "systemctl status nginx",
            Environment::Prod,
            McpAccessMode::ReadOnly,
            AuditSource::Agent,
        );
        assert_eq!(decision.action, PolicyAction::Allow);
        assert_eq!(decision.risk, RiskLevel::Low);
    }

    #[test]
    fn blocks_agent_calls_when_mcp_disabled() {
        let decision = evaluate(
            "ls -la",
            Environment::Dev,
            McpAccessMode::Disabled,
            AuditSource::Agent,
        );
        assert_eq!(decision.action, PolicyAction::Block);
    }

    #[test]
    fn prod_service_restart_requires_approval() {
        let decision = evaluate(
            "systemctl restart nginx",
            Environment::Prod,
            McpAccessMode::Disabled,
            AuditSource::User,
        );
        assert_eq!(decision.action, PolicyAction::RequireApproval);
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn dev_rm_requires_approval() {
        let decision = evaluate(
            "rm -rf ./cache",
            Environment::Dev,
            McpAccessMode::Disabled,
            AuditSource::User,
        );
        assert_eq!(decision.action, PolicyAction::RequireApproval);
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn read_only_mcp_blocks_unknown_commands() {
        let decision = evaluate(
            "make deploy",
            Environment::Dev,
            McpAccessMode::ReadOnly,
            AuditSource::Agent,
        );
        assert_eq!(decision.action, PolicyAction::Block);
    }
}
