use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::models::{Connection, ConnectionInput};

/// Persistent connection metadata. Secrets never live here; see `vault.rs`.
pub struct ConnectionStore {
    path: PathBuf,
    connections: Vec<Connection>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ConnectionFile {
    #[serde(default)]
    connections: Vec<Connection>,
}

impl ConnectionStore {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("connections.json");
        let connections = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<ConnectionFile>(&text).ok())
            .map(|file| file.connections)
            .unwrap_or_default();
        Self { path, connections }
    }

    pub fn list(&self) -> Vec<Connection> {
        self.connections.clone()
    }

    pub fn get(&self, id: &str) -> Option<Connection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id)
            .cloned()
    }

    pub fn authorized_count(&self) -> usize {
        self.connections
            .iter()
            .filter(|connection| connection.mcp_access.is_authorized())
            .count()
    }

    pub fn upsert(&mut self, input: ConnectionInput) -> Result<Connection, String> {
        if input.alias.trim().is_empty() {
            return Err("连接别名不能为空".into());
        }
        if input.host.trim().is_empty() {
            return Err("主机地址不能为空".into());
        }
        if input.username.trim().is_empty() {
            return Err("用户名不能为空".into());
        }
        if input.port == 0 {
            return Err("端口必须在 1-65535 之间".into());
        }

        let existing_index = input
            .id
            .as_deref()
            .and_then(|id| self.connections.iter().position(|item| item.id == id));
        let now = Utc::now().to_rfc3339();

        let (id, created_at) = match existing_index {
            Some(index) => (
                self.connections[index].id.clone(),
                self.connections[index].created_at.clone(),
            ),
            None => (uuid::Uuid::new_v4().to_string(), now.clone()),
        };

        let connection = Connection {
            id,
            alias: input.alias.trim().to_string(),
            host: input.host.trim().to_string(),
            port: input.port,
            username: input.username.trim().to_string(),
            group: input.group.trim().to_string(),
            tags: input.tags,
            environment: input.environment,
            auth: input.auth,
            mcp_access: input.mcp_access,
            jump_host: input.jump_host,
            remark: input.remark,
            term: if input.term.trim().is_empty() {
                "xterm-256color".to_string()
            } else {
                input.term.trim().to_string()
            },
            encoding: if input.encoding.trim().is_empty() {
                "utf-8".to_string()
            } else {
                input.encoding.trim().to_lowercase()
            },
            startup_command: input.startup_command.trim().to_string(),
            legacy_algorithms: input.legacy_algorithms,
            created_at,
            updated_at: now,
        };

        match existing_index {
            Some(index) => self.connections[index] = connection.clone(),
            None => self.connections.push(connection.clone()),
        }
        self.persist()?;
        Ok(connection)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let before = self.connections.len();
        self.connections.retain(|connection| connection.id != id);
        if self.connections.len() == before {
            return Err("连接不存在".into());
        }
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("无法创建数据目录: {error}"))?;
        }
        let file = ConnectionFile {
            connections: self.connections.clone(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|error| format!("序列化连接配置失败: {error}"))?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, json).map_err(|error| format!("写入连接配置失败: {error}"))?;
        fs::rename(&temp, &self.path).map_err(|error| format!("替换连接配置失败: {error}"))?;
        Ok(())
    }
}
