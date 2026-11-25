use crate::models::{
    AuditSource, Environment, McpAccessMode, PolicyAction, PolicyDecision, RiskLevel,
};
use crate::shell::{self, Segment};

/// Deterministic command policy. This runs in the Rust core, never in the
/// frontend, so an AI agent cannot bypass it by prompt manipulation.
///
/// The classifier works on the *segments* produced by [`crate::shell`], not on
/// the raw prefix of the command line: every simple command in a chain, every
/// command substitution and every inline interpreter script is scored, and the
/// worst score wins. A harmless-looking prefix (`ls && ...`) therefore cannot
/// downgrade a destructive tail.
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
            push_unique(&mut reasons, "生产环境：中高风险命令强制人工审批".into());
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

fn push_unique(reasons: &mut Vec<String>, reason: String) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn classify(command: &str) -> (RiskLevel, Vec<String>) {
    if command.trim().is_empty() {
        return (RiskLevel::Medium, vec!["空命令".into()]);
    }
    if shell::is_catastrophic_raw(command) {
        return (
            RiskLevel::Critical,
            vec!["命中极高危破坏性命令规则".into()],
        );
    }

    let analysis = shell::analyze(command);
    if analysis.segments.is_empty() {
        return (
            RiskLevel::Medium,
            vec!["无法解析出可执行程序".into()],
        );
    }

    let mut worst = RiskLevel::Low;
    let mut reasons: Vec<String> = Vec::new();
    for segment in &analysis.segments {
        let (risk, why) = classify_segment(segment);
        if risk > worst {
            worst = risk;
            reasons = why;
        } else if risk == worst && risk != RiskLevel::Low {
            for reason in why {
                push_unique(&mut reasons, reason);
            }
        }
    }

    // Constructs we cannot resolve statically (`curl … | sh`, `$((…))` nesting
    // beyond the limit, unterminated quotes) are never treated as read-only.
    if analysis.dynamic && worst < RiskLevel::High {
        worst = RiskLevel::High;
        push_unique(&mut reasons, "包含无法静态解析的动态命令".into());
    }

    if analysis.is_chained() && worst != RiskLevel::Low {
        push_unique(
            &mut reasons,
            format!("命令链包含 {} 段子命令，按最高风险判定", analysis.segments.len()),
        );
    }

    if reasons.is_empty() {
        reasons.push("只读检查类命令".into());
    }

    (worst, reasons)
}

fn classify_segment(segment: &Segment) -> (RiskLevel, Vec<String>) {
    if let Some(reason) = catastrophic_segment(segment) {
        return (RiskLevel::Critical, vec![reason]);
    }
    if let Some(reason) = high_risk_segment(&segment.program, &segment.args) {
        return (RiskLevel::High, vec![reason.to_string()]);
    }
    if is_low_risk_segment(segment) {
        return (RiskLevel::Low, vec!["只读检查类命令".into()]);
    }
    if segment.redirect {
        return (
            RiskLevel::Medium,
            vec!["包含输出重定向，可能写入文件".into()],
        );
    }
    (
        RiskLevel::Medium,
        vec!["未命中只读白名单，按中风险处理".into()],
    )
}

/// Hard blocks: destructive actions that should never run, even with approval.
fn catastrophic_segment(segment: &Segment) -> Option<String> {
    let program = segment.program.as_str();
    let args = &segment.args;

    if program.starts_with("mkfs") {
        return Some("格式化文件系统".into());
    }
    if matches!(program, "wipefs" | "blkdiscard" | "hdparm") {
        return Some("直接操作块设备".into());
    }
    if program == "dd" && args.iter().any(|arg| arg.starts_with("of=/dev/")) {
        return Some("裸写块设备".into());
    }

    // `rm -rf /` with any option spelling or argument order.
    if program == "rm" {
        let recursive = args.iter().any(|arg| {
            arg == "--recursive"
                || (arg.starts_with('-')
                    && !arg.starts_with("--")
                    && (arg.contains('r') || arg.contains('R')))
        });
        let targets_root = args.iter().any(|arg| {
            let cleaned = arg.trim_matches(|character| character == '"' || character == '\'');
            cleaned == "/" || cleaned == "/*"
        });
        if recursive && targets_root {
            return Some("递归删除根目录".into());
        }
    }

    // Redirection straight onto a block device.
    if segment.redirect {
        let writes_device = args
            .iter()
            .any(|arg| arg.starts_with("/dev/sd") || arg.starts_with("/dev/nvme") || arg.starts_with("/dev/vd"));
        if writes_device {
            return Some("重定向写入块设备".into());
        }
    }

    None
}

fn high_risk_reason_for(program: &str) -> Option<&'static str> {
    match program {
        "rm" | "rmdir" | "shred" => Some("包含删除操作"),
        "chmod" | "chown" | "chgrp" | "setfacl" | "chattr" => Some("变更文件权限或所有者"),
        "kill" | "pkill" | "killall" | "xkill" => Some("终止进程"),
        "useradd" | "userdel" | "usermod" | "groupadd" | "groupdel" | "adduser" | "deluser" => {
            Some("变更系统用户")
        }
        "passwd" | "visudo" | "chpasswd" => Some("变更账户或提权配置"),
        "crontab" | "at" | "batch" | "systemd-run" => Some("变更计划任务"),
        "mount" | "umount" | "parted" | "mkswap" | "truncate" => {
            Some("变更磁盘或文件系统")
        }
        "iptables" | "ip6tables" | "nft" | "ufw" | "firewall-cmd" => Some("变更防火墙规则"),
        "shutdown" | "reboot" | "poweroff" | "halt" | "init" | "kexec" | "systemctl" => None,
        "alias" | "unalias" => Some("定义或修改 shell 别名"),
        "insmod" | "rmmod" | "modprobe" | "depmod" => Some("加载或卸载内核模块"),
        "chroot" | "unshare" | "nsenter" => Some("切换执行环境或命名空间"),
        "nc" | "ncat" | "netcat" | "socat" => Some("建立网络连接或隧道"),
        "dd" => Some("直接操作块设备"),
        "fdisk" => Some("变更磁盘分区"),
        _ => None,
    }
}

fn high_risk_segment(program: &str, args: &[String]) -> Option<&'static str> {
    let first = args.first().map(String::as_str).unwrap_or("");

    match program {
        "shutdown" | "reboot" | "poweroff" | "halt" | "init" | "kexec" => {
            return Some("变更主机运行状态")
        }
        "systemctl" => {
            if matches!(
                first,
                "restart"
                    | "stop"
                    | "start"
                    | "disable"
                    | "enable"
                    | "mask"
                    | "daemon-reload"
                    | "kill"
                    | "isolate"
                    | "set-property"
            ) {
                return Some("变更系统服务状态");
            }
        }
        "service" => return Some("变更系统服务状态"),
        "docker" | "podman" | "nerdctl" => {
            if matches!(
                first,
                "rm" | "rmi" | "stop" | "kill" | "prune" | "run" | "exec" | "restart" | "system"
            ) {
                return Some("变更容器或执行容器内命令");
            }
        }
        "kubectl" | "helm" => {
            if matches!(
                first,
                "delete" | "apply" | "scale" | "rollout" | "edit" | "patch" | "exec" | "replace"
            ) {
                return Some("变更集群资源");
            }
        }
        "git" => {
            if first == "push" {
                return Some("推送远端仓库");
            }
            if first == "reset" && args.iter().any(|arg| arg == "--hard") {
                return Some("重置工作区历史");
            }
            if first == "clean" {
                return Some("清理未跟踪文件");
            }
        }
        "sed" => {
            if args
                .iter()
                .any(|argument| argument == "--in-place" || argument.starts_with("-i"))
            {
                return Some("原地修改文件");
            }
        }
        "find" => {
            if args
                .iter()
                .any(|argument| matches!(argument.as_str(), "-delete" | "-exec" | "-execdir" | "-ok"))
            {
                return Some("查找并删除或执行命令");
            }
        }
        "sysctl" => {
            if args.iter().any(|argument| argument == "-w" || argument == "--write") {
                return Some("修改内核参数");
            }
        }
        _ => {}
    }

    if matches!(
        program,
        "apt" | "apt-get" | "yum" | "dnf" | "pacman" | "zypper" | "apk"
    ) && args
        .iter()
        .any(|word| matches!(word.as_str(), "install" | "remove" | "purge" | "upgrade" | "dist-upgrade" | "erase" | "add" | "del"))
    {
        return Some("安装或删除系统软件包");
    }
    if matches!(program, "pip" | "pip3" | "npm" | "pnpm" | "yarn" | "gem" | "cargo")
        && args
            .iter()
            .any(|word| matches!(word.as_str(), "install" | "uninstall" | "add" | "remove"))
    {
        return Some("安装或删除运行时依赖");
    }

    high_risk_reason_for(program)
}

fn is_low_risk_segment(segment: &Segment) -> bool {
    if segment.redirect {
        return false;
    }
    let program = segment.program.as_str();
    let first = segment.args.first().map(String::as_str).unwrap_or("");

    match program {
        "ls" | "ll" | "pwd" | "whoami" | "hostname" | "date" | "uptime" | "uname" | "df"
        | "du" | "free" | "ps" | "top" | "htop" | "cat" | "head" | "tail" | "less" | "more"
        | "grep" | "wc" | "stat" | "file" | "tree" | "echo" | "env" | "printenv" | "ss"
        | "netstat" | "ping" | "traceroute" | "dig" | "nslookup" | "journalctl" | "id"
        | "groups" | "w" | "who" | "last" | "which" | "type" | "locale" | "lscpu" | "lsblk" => true,
        "find" => !segment
            .args
            .iter()
            .any(|argument| matches!(argument.as_str(), "-delete" | "-exec" | "-execdir" | "-ok")),
        "systemctl" => matches!(
            first,
            "status"
                | "is-active"
                | "is-enabled"
                | "is-failed"
                | "show"
                | "cat"
                | "list-units"
                | "list-unit-files"
        ),
        "docker" | "podman" => matches!(
            first,
            "ps" | "logs" | "inspect" | "images" | "stats" | "version" | "info" | "top" | "port"
        ),
        "kubectl" => matches!(
            first,
            "get" | "describe" | "logs" | "top" | "version" | "explain" | "api-resources"
        ),
        "git" => matches!(
            first,
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

    fn manual(command: &str) -> crate::models::PolicyDecision {
        evaluate(
            command,
            Environment::Dev,
            McpAccessMode::Disabled,
            AuditSource::User,
        )
    }

    fn agent_prod_readonly(command: &str) -> crate::models::PolicyDecision {
        evaluate(
            command,
            Environment::Prod,
            McpAccessMode::ReadOnly,
            AuditSource::Agent,
        )
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

    /* ---- chained-command bypasses (previously allowed as read-only) ---- */

    #[test]
    fn chained_delete_is_not_read_only() {
        let decision = manual("ls -la && rm -rf ~");
        assert_eq!(decision.risk, RiskLevel::High);
        assert_eq!(decision.action, PolicyAction::RequireApproval);
    }

    #[test]
    fn chained_service_stop_is_not_read_only() {
        let decision = manual("cat /tmp/log; systemctl stop nginx");
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn readonly_agent_cannot_smuggle_a_tail_command() {
        let decision = agent_prod_readonly("ls -la; rm -rf /var/www");
        assert_eq!(decision.action, PolicyAction::Block);
    }

    #[test]
    fn inline_interpreter_script_is_scored() {
        let decision = manual("bash -c 'rm -rf /'");
        assert_eq!(decision.risk, RiskLevel::Critical);
        assert_eq!(decision.action, PolicyAction::Block);
    }

    #[test]
    fn command_substitution_is_scored() {
        let decision = manual("ls $(rm -rf /)");
        assert_eq!(decision.risk, RiskLevel::Critical);
        assert_eq!(decision.action, PolicyAction::Block);
    }

    #[test]
    fn piped_script_is_never_read_only() {
        let decision = manual("echo 'reboot' | sh");
        assert!(decision.risk >= RiskLevel::High);
        assert_eq!(decision.action, PolicyAction::RequireApproval);
    }

    #[test]
    fn absolute_path_and_sudo_do_not_downgrade() {
        assert_eq!(manual("/bin/rm -rf /var/www").risk, RiskLevel::High);
        assert_eq!(manual("sudo rm -rf /var/www").risk, RiskLevel::High);
        assert_eq!(manual("sudo rm -rf /").risk, RiskLevel::Critical);
    }

    #[test]
    fn find_with_exec_is_high_risk() {
        let decision = manual("find /var/log -name '*.log' -delete");
        assert_eq!(decision.risk, RiskLevel::High);
    }

    #[test]
    fn plain_read_only_stays_low_and_allowed() {
        for command in [
            "ls -la",
            "df -h",
            "systemctl status nginx",
            "docker ps",
            "git status",
        ] {
            let decision = manual(command);
            assert_eq!(decision.risk, RiskLevel::Low, "{command}");
            assert_eq!(decision.action, PolicyAction::Allow, "{command}");
        }
    }
}
