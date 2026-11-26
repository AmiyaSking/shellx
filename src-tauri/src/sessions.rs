use std::collections::{HashMap, VecDeque};

use chrono::Utc;

use crate::models::{SessionInfo, SessionStatus};

const MAX_OUTPUT_LINES: usize = 2000;
/// Upper bound for one `read_output` page.
const MAX_READ_LINES: usize = 500;

pub struct Session {
    pub id: String,
    connection_id: String,
    connection_alias: String,
    created_at: String,
    last_active_at: String,
    status: SessionStatus,
    output: VecDeque<String>,
    output_bytes: usize,
    running: bool,
    exit_code: Option<i32>,
}

/// One page of buffered session output plus the command's current state.
#[derive(Debug, Clone)]
pub struct SessionOutputPage {
    pub text: String,
    pub next_offset: usize,
    pub eof: bool,
    pub running: bool,
    pub exit_code: Option<i32>,
}

/// In-memory session registry with a bounded output ring buffer. Real SSH
/// channels will attach to these sessions in phase 1 (see `ssh` TODO).
#[derive(Default)]
pub struct SessionManager {
    sessions: HashMap<String, Session>,
}

impl SessionManager {
    pub fn get_or_create(&mut self, connection_id: &str, connection_alias: &str) -> &mut Session {
        let existing = self
            .sessions
            .iter()
            .find(|(_, session)| {
                session.connection_id == connection_id && session.status == SessionStatus::Active
            })
            .map(|(id, _)| id.clone());

        let id = match existing {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let now = Utc::now().to_rfc3339();
                self.sessions.insert(
                    id.clone(),
                    Session {
                        id: id.clone(),
                        connection_id: connection_id.to_string(),
                        connection_alias: connection_alias.to_string(),
                        created_at: now.clone(),
                        last_active_at: now,
                        status: SessionStatus::Active,
                        output: VecDeque::new(),
                        output_bytes: 0,
                        running: false,
                        exit_code: None,
                    },
                );
                id
            }
        };

        let now = Utc::now().to_rfc3339();
        let session = self.sessions.get_mut(&id).expect("session was just resolved");
        session.last_active_at = now;
        session
    }

    pub fn append_output(&mut self, session_id: &str, text: &str) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.output_bytes += text.len();
            for line in text.replace("\r\n", "\n").split('\n') {
                session.output.push_back(line.to_string());
            }
            while session.output.len() > MAX_OUTPUT_LINES {
                session.output.pop_front();
            }
            session.last_active_at = Utc::now().to_rfc3339();
        }
    }

    /// Mark a session as having a command in flight.
    pub fn mark_running(&mut self, id: &str) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.running = true;
            session.exit_code = None;
            session.last_active_at = Utc::now().to_rfc3339();
        }
    }

    /// Record a finished command: append its output and store the exit code.
    pub fn finish_command(&mut self, id: &str, text: &str, exit_code: Option<i32>) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.running = false;
            session.exit_code = exit_code;
        }
        self.append_output(id, text);
    }

    /// Read up to `limit` buffered lines starting at `offset`.
    pub fn read_output(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<SessionOutputPage, String> {
        let session = self
            .sessions
            .get(id)
            .ok_or_else(|| format!("会话 {id} 不存在"))?;
        let limit = limit.clamp(1, MAX_READ_LINES);
        let lines: Vec<&str> = session
            .output
            .iter()
            .skip(offset)
            .take(limit)
            .map(String::as_str)
            .collect();
        let next_offset = offset + lines.len();
        Ok(SessionOutputPage {
            text: lines.join("\n"),
            next_offset,
            // While a command is running more output may still arrive, so the
            // page is never final.
            eof: !session.running && next_offset >= session.output.len(),
            running: session.running,
            exit_code: session.exit_code,
        })
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .values()
            .map(|session| SessionInfo {
                id: session.id.clone(),
                connection_id: session.connection_id.clone(),
                connection_alias: session.connection_alias.clone(),
                created_at: session.created_at.clone(),
                last_active_at: session.last_active_at.clone(),
                status: session.status,
                output_lines: session.output.len(),
                output_bytes: session.output_bytes,
                running: session.running,
                exit_code: session.exit_code,
            })
            .collect();
        sessions.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        sessions
    }

    pub fn active_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|session| session.status == SessionStatus::Active)
            .count()
    }

    pub fn close(&mut self, id: &str) -> bool {
        if let Some(session) = self.sessions.get_mut(id) {
            session.status = SessionStatus::Closed;
            session.last_active_at = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    pub fn close_for_connection(&mut self, connection_id: &str) {
        for session in self.sessions.values_mut() {
            if session.connection_id == connection_id && session.status == SessionStatus::Active {
                session.status = SessionStatus::Closed;
                session.last_active_at = Utc::now().to_rfc3339();
            }
        }
    }
}
