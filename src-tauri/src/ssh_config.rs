//! Read-only parser for OpenSSH's `~/.ssh/config`.
//!
//! ShellX keeps its own connection store, so importing is a one-shot preview:
//! the frontend shows the parsed hosts, lets the user select them and then
//! creates normal connections. Nothing here connects to a server.

use std::collections::HashSet;
use std::env;
use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigPreview {
    pub path: String,
    pub hosts: Vec<SshConfigHost>,
    pub warnings: Vec<String>,
    pub default_user: String,
}

#[derive(Debug, Default, Clone)]
struct Options {
    host_name: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
}

#[derive(Debug, Clone)]
struct Block {
    patterns: Vec<String>,
    options: Options,
}

pub fn preview_default() -> Result<SshConfigPreview, String> {
    let path = default_config_path()?;
    let content = std::fs::read_to_string(&path)
        .map_err(|error| format!("无法读取 {}：{error}", path.display()))?;
    let default_user = env::var("USERNAME")
        .or_else(|_| env::var("USER"))
        .unwrap_or_default();
    let (hosts, warnings) = parse_config(&content, &default_user);
    Ok(SshConfigPreview {
        path: path.display().to_string(),
        hosts,
        warnings,
        default_user,
    })
}

fn default_config_path() -> Result<PathBuf, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .ok_or_else(|| "无法确定用户主目录".to_string())?;
    Ok(PathBuf::from(home).join(".ssh").join("config"))
}

fn parse_config(content: &str, default_user: &str) -> (Vec<SshConfigHost>, Vec<String>) {
    let mut global = Options::default();
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;
    let mut warnings = Vec::new();
    let mut ignore_options = false;
    let mut saw_include = false;
    let mut saw_proxy_command = false;

    for raw_line in content.lines() {
        let line = raw_line
            .trim()
            .trim_start_matches('\u{feff}')
            .trim()
            .to_string();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (keyword, value) = split_directive(&line);
        match keyword.as_str() {
            "host" => {
                if let Some(block) = current.take() {
                    blocks.push(block);
                }
                ignore_options = false;
                let patterns: Vec<String> = value
                    .split_whitespace()
                    .map(unquote)
                    .filter(|pattern| !pattern.is_empty())
                    .collect();
                if patterns.is_empty() {
                    warnings.push(format!("忽略没有主机名的 Host 行：{line}"));
                    current = None;
                } else {
                    current = Some(Block {
                        patterns,
                        options: Options::default(),
                    });
                }
            }
            "match" => {
                if let Some(block) = current.take() {
                    blocks.push(block);
                }
                ignore_options = true;
                warnings.push("已跳过 Match 规则块".to_string());
            }
            "include" => {
                if !saw_include {
                    saw_include = true;
                    warnings.push(
                        "Include 指令未展开，如需导入请手动处理被包含的文件".to_string(),
                    );
                }
            }
            "proxycommand" => {
                if !saw_proxy_command {
                    saw_proxy_command = true;
                    warnings.push("ProxyCommand 暂不支持导入".to_string());
                }
            }
            "hostname" | "port" | "user" | "identityfile" | "proxyjump" => {
                if ignore_options {
                    continue;
                }
                let target = current
                    .as_mut()
                    .map(|block| &mut block.options)
                    .unwrap_or(&mut global);
                match keyword.as_str() {
                    "hostname" => target.host_name = Some(unquote(&value)),
                    "port" => match value.trim().parse::<u16>() {
                        Ok(port) if port > 0 => target.port = Some(port),
                        _ => warnings.push(format!("忽略无效端口：{value}")),
                    },
                    "user" => target.user = Some(unquote(&value)),
                    "identityfile" => {
                        let first = first_value(&value);
                        if !first.is_empty() {
                            if target.identity_file.is_some() {
                                warnings.push(
                                    "IdentityFile 指定了多个值，仅导入第一个".to_string(),
                                );
                            }
                            target.identity_file = Some(first);
                        }
                    }
                    "proxyjump" => {
                        let jump = unquote(value.trim());
                        if !jump.is_empty() && !jump.eq_ignore_ascii_case("none") {
                            target.proxy_jump = Some(jump);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if let Some(block) = current.take() {
        blocks.push(block);
    }

    // Global directives and the first `Host *` block provide fallback values.
    let mut defaults = global;
    if let Some(star) = blocks
        .iter()
        .find(|block| block.patterns.iter().any(|pattern| pattern == "*"))
    {
        merge_missing(&mut defaults, &star.options);
    }

    let mut hosts = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for block in &blocks {
        for pattern in &block.patterns {
            if pattern == "*" {
                continue;
            }
            if pattern.starts_with('!')
                || pattern.contains('*')
                || pattern.contains('?')
                || pattern.contains('[')
            {
                warnings.push(format!("已跳过通配符 Host 规则：{pattern}"));
                continue;
            }
            if !seen.insert(pattern.clone()) {
                warnings.push(format!("重复的 Host {pattern} 已跳过"));
                continue;
            }

            let alias = pattern.clone();
            let host = block
                .options
                .host_name
                .as_ref()
                .or(defaults.host_name.as_ref())
                .map(|host_name| host_name.replace("%h", &alias))
                .unwrap_or_else(|| alias.clone());
            let port = block.options.port.or(defaults.port).unwrap_or(22);
            let username = block
                .options
                .user
                .clone()
                .or_else(|| defaults.user.clone())
                .filter(|user| !user.is_empty())
                .unwrap_or_else(|| default_user.to_string());
            let identity_file = block
                .options
                .identity_file
                .clone()
                .or_else(|| defaults.identity_file.clone());
            let proxy_jump = block
                .options
                .proxy_jump
                .clone()
                .or_else(|| defaults.proxy_jump.clone());

            hosts.push(SshConfigHost {
                alias,
                host,
                port,
                username,
                identity_file,
                proxy_jump,
            });
        }
    }

    if hosts.is_empty() {
        warnings.push("没有找到可导入的具体 Host 条目".to_string());
    }
    (hosts, warnings)
}

fn split_directive(line: &str) -> (String, String) {
    if let Some((key, value)) = line.split_once('=') {
        if !key.trim().contains(char::is_whitespace) {
            return (key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    (
        parts.next().unwrap_or("").trim().to_ascii_lowercase(),
        parts.next().unwrap_or("").trim().to_string(),
    )
}

fn first_value(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = value.strip_prefix(quote) {
            if let Some(end) = rest.find(quote) {
                return rest[..end].to_string();
            }
        }
    }
    value.split_whitespace().next().unwrap_or("").to_string()
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn merge_missing(base: &mut Options, extra: &Options) {
    if base.host_name.is_none() {
        base.host_name = extra.host_name.clone();
    }
    if base.port.is_none() {
        base.port = extra.port;
    }
    if base.user.is_none() {
        base.user = extra.user.clone();
    }
    if base.identity_file.is_none() {
        base.identity_file = extra.identity_file.clone();
    }
    if base.proxy_jump.is_none() {
        base.proxy_jump = extra.proxy_jump.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# global defaults
User deploy
Include ~/.ssh/config.d/*.conf

Host *
  IdentityFile ~/.ssh/star_key
  ServerAliveInterval 30

Host web
  HostName 10.0.0.10
  Port 2222
  User ops
  IdentityFile "~/.ssh/web key"
  ProxyJump bastion

Host db staging-db
  HostName 10.0.0.20
  Port 2200

Host *.internal
  User ignored

Host web
  User duplicate

Host=db
  Port=9999
"#;

    #[test]
    fn parses_hosts_with_defaults_and_overrides() {
        let (hosts, warnings) = parse_config(SAMPLE, "fallback");
        let web = hosts.iter().find(|host| host.alias == "web").unwrap();
        assert_eq!(web.host, "10.0.0.10");
        assert_eq!(web.port, 2222);
        assert_eq!(web.username, "ops");
        assert_eq!(web.identity_file.as_deref(), Some("~/.ssh/web key"));
        assert_eq!(web.proxy_jump.as_deref(), Some("bastion"));

        let db = hosts.iter().find(|host| host.alias == "db").unwrap();
        assert_eq!(db.host, "10.0.0.20");
        assert_eq!(db.port, 2200);
        assert_eq!(db.username, "deploy");
        // Host * overrides the global IdentityFile for hosts without their own.
        assert_eq!(db.identity_file.as_deref(), Some("~/.ssh/star_key"));

        assert!(hosts.iter().any(|host| host.alias == "staging-db"));
        assert!(warnings.iter().any(|warning| warning.contains("Include")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("通配符 Host 规则")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("重复的 Host web")));
    }

    #[test]
    fn falls_back_to_os_user_and_supports_percent_h() {
        let content = "Host example\n  HostName %h.example.com\n";
        let (hosts, _) = parse_config(content, "localuser");
        assert_eq!(hosts[0].host, "example.example.com");
        assert_eq!(hosts[0].username, "localuser");
        assert_eq!(hosts[0].port, 22);
    }

    #[test]
    fn skips_match_blocks_and_invalid_ports() {
        let content = "\
Host good
  Port 99999

Match host bad
  User ignored

Host other
  Port 2022
";
        let (hosts, warnings) = parse_config(content, "user");
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].port, 22);
        assert_eq!(hosts[1].port, 2022);
        assert!(warnings.iter().any(|warning| warning.contains("无效端口")));
        assert!(warnings.iter().any(|warning| warning.contains("Match")));
    }

    #[test]
    fn returns_a_warning_when_nothing_is_importable() {
        let (hosts, warnings) = parse_config("Host *\n  User deploy\n", "user");
        assert!(hosts.is_empty());
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("没有找到可导入")));
    }
}
