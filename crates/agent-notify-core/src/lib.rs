use serde::{Deserialize, Serialize};
use std::{
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_TTL_SECONDS: u64 = 120;
pub const MAX_SUMMARY_CHARS: usize = 160;
pub const UHK_EXEC_MACRO_COMMAND: u8 = 0x14;
pub const UHK_MAX_USB_PAYLOAD_SIZE: usize = 63;
pub const UHK_MAX_MACRO_COMMAND_BYTES: usize = UHK_MAX_USB_PAYLOAD_SIZE - 2;

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    WaitingInput,
    Done,
    Failed,
}

impl AgentState {
    pub fn default_priority(self) -> u8 {
        match self {
            AgentState::WaitingInput => 90,
            AgentState::Failed => 80,
            AgentState::Done => 50,
            AgentState::Running => 20,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentState::Running => "RUN",
            AgentState::WaitingInput => "INPUT",
            AgentState::Done => "DONE",
            AgentState::Failed => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentEventInput {
    pub agent: String,
    pub host: String,
    pub repo: Option<String>,
    pub state: AgentState,
    pub summary: Option<String>,
    pub priority: Option<u8>,
    pub ttl_seconds: Option<u64>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentEvent {
    pub id: Uuid,
    pub received_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
    pub agent: String,
    pub host: String,
    pub repo: Option<String>,
    pub state: AgentState,
    pub summary: Option<String>,
    pub priority: u8,
    pub ttl_seconds: u64,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BridgeStatus {
    pub host: String,
    pub app_version: String,
    pub keyboard_present: bool,
    pub paused: bool,
    pub last_display: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeClientMessage {
    Status { status: BridgeStatus },
    RequestLatest,
    DismissLatest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeServerMessage {
    Event { event: AgentEvent },
    Clear { reason: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DismissResponse {
    pub dismissed: bool,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum EventError {
    #[error("{field} is required")]
    Required { field: &'static str },
    #[error("{field} is too long")]
    TooLong { field: &'static str },
}

impl AgentEventInput {
    pub fn into_event(self) -> Result<AgentEvent, EventError> {
        validate_required("agent", &self.agent)?;
        validate_required("host", &self.host)?;
        validate_optional_len("repo", self.repo.as_deref(), 80)?;
        validate_optional_len("summary", self.summary.as_deref(), MAX_SUMMARY_CHARS)?;
        validate_optional_len("run_id", self.run_id.as_deref(), 80)?;

        let ttl_seconds = self
            .ttl_seconds
            .unwrap_or(DEFAULT_TTL_SECONDS)
            .clamp(1, 3600);
        let now = unix_ms();
        let ttl_ms = Duration::from_secs(ttl_seconds).as_millis() as u64;

        Ok(AgentEvent {
            id: Uuid::new_v4(),
            received_at_unix_ms: now,
            expires_at_unix_ms: now + ttl_ms,
            agent: normalize_field(&self.agent),
            host: normalize_field(&self.host),
            repo: self.repo.map(|value| normalize_field(&value)),
            state: self.state,
            summary: self.summary.map(|value| normalize_field(&value)),
            priority: self
                .priority
                .unwrap_or_else(|| self.state.default_priority()),
            ttl_seconds,
            run_id: self.run_id.map(|value| normalize_field(&value)),
        })
    }
}

impl AgentEvent {
    pub fn is_live(&self) -> bool {
        self.expires_at_unix_ms > unix_ms()
    }
}

pub fn choose_latest(current: Option<AgentEvent>, next: AgentEvent) -> AgentEvent {
    match current {
        Some(current) if same_work_item(&current, &next) => next,
        Some(current)
            if current.is_live()
                && (current.priority, current.received_at_unix_ms)
                    > (next.priority, next.received_at_unix_ms) =>
        {
            current
        }
        _ => next,
    }
}

fn same_work_item(left: &AgentEvent, right: &AgentEvent) -> bool {
    if left.run_id.is_some() && left.run_id == right.run_id {
        return true;
    }

    left.agent == right.agent && left.host == right.host && left.repo == right.repo
}

pub fn macro_command_for_event(event: &AgentEvent) -> Result<String, EventError> {
    let base = concise_display_text(event);
    match event.state {
        AgentState::WaitingInput => quoted_macro_command("setLedTxt 0 notification", &base),
        AgentState::Done | AgentState::Failed | AgentState::Running => {
            quoted_macro_command("notify", &base)
        }
    }
}

pub fn clear_macro_command() -> &'static str {
    "setLedTxt 1 notification \"\""
}

pub fn uhk_exec_macro_report(report_id: u8, command: &str) -> Result<Vec<u8>, EventError> {
    if command.len() > UHK_MAX_MACRO_COMMAND_BYTES {
        return Err(EventError::TooLong {
            field: "macro_command",
        });
    }

    let mut report = Vec::with_capacity(command.len() + 3);
    report.push(report_id);
    report.push(UHK_EXEC_MACRO_COMMAND);
    report.extend_from_slice(command.as_bytes());
    report.push(0);
    Ok(report)
}

pub fn local_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(hostname_from_file)
        .or_else(hostname_from_command)
        .map(|value| normalize_field(&value))
        .filter(|value| !value.is_empty())
}

pub fn concise_display_text(event: &AgentEvent) -> String {
    let mut parts = vec![
        short_token(&event.host, 10),
        short_token(&event.agent, 8),
        event.state.label().to_string(),
    ];

    if let Some(repo) = &event.repo {
        parts.push(short_token(repo, 18));
    } else if let Some(summary) = &event.summary {
        parts.push(short_token(summary, 18));
    }

    sanitize_macro_string(&parts.join(" "))
}

fn hostname_from_file() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn hostname_from_command() -> Option<String> {
    Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
}

fn quoted_macro_command(prefix: &str, value: &str) -> Result<String, EventError> {
    let overhead = prefix.len() + 3;
    let max_value_len = UHK_MAX_MACRO_COMMAND_BYTES.saturating_sub(overhead);
    let escaped = escape_and_truncate_macro_string(value, max_value_len);
    let command = format!("{prefix} \"{escaped}\"");

    if command.len() > UHK_MAX_MACRO_COMMAND_BYTES {
        return Err(EventError::TooLong {
            field: "macro_command",
        });
    }

    Ok(command)
}

fn validate_required(field: &'static str, value: &str) -> Result<(), EventError> {
    if value.trim().is_empty() {
        return Err(EventError::Required { field });
    }
    if value.chars().count() > 80 {
        return Err(EventError::TooLong { field });
    }
    Ok(())
}

fn validate_optional_len(
    field: &'static str,
    value: Option<&str>,
    max: usize,
) -> Result<(), EventError> {
    if value.is_some_and(|value| value.chars().count() > max) {
        return Err(EventError::TooLong { field });
    }
    Ok(())
}

fn normalize_field(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short_token(value: &str, max_chars: usize) -> String {
    let value = value
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .trim()
        .to_string();
    truncate_chars(&value, max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>()
        + "~"
}

fn sanitize_macro_string(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_graphic() || ch == ' ' {
                ch
            } else {
                '?'
            }
        })
        .collect()
}

fn escape_and_truncate_macro_string(value: &str, max_bytes: usize) -> String {
    let mut escaped_value = String::new();
    for ch in value.chars() {
        let escaped = match ch {
            '\\' => "\\\\",
            '"' => "\\\"",
            _ => {
                if escaped_value.len() + ch.len_utf8() <= max_bytes {
                    escaped_value.push(ch);
                    continue;
                }
                break;
            }
        };

        if escaped_value.len() + escaped.len() <= max_bytes {
            escaped_value.push_str(escaped);
        } else {
            break;
        }
    }
    escaped_value
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(state: AgentState, priority: Option<u8>) -> AgentEvent {
        AgentEventInput {
            agent: "codex".into(),
            host: "workstation-01".into(),
            repo: Some("agent-notify".into()),
            state,
            summary: Some("finished work".into()),
            priority,
            ttl_seconds: Some(60),
            run_id: None,
        }
        .into_event()
        .unwrap()
    }

    #[test]
    fn defaults_priority_by_state() {
        assert_eq!(event(AgentState::WaitingInput, None).priority, 90);
        assert_eq!(event(AgentState::Done, None).priority, 50);
    }

    #[test]
    fn builds_uhk_report() {
        let report = uhk_exec_macro_report(4, "notify \"hi\"").unwrap();
        assert_eq!(report[0], 4);
        assert_eq!(report[1], UHK_EXEC_MACRO_COMMAND);
        assert_eq!(report.last(), Some(&0));
    }

    #[test]
    fn macro_command_fits_uhk_payload() {
        let event = event(AgentState::WaitingInput, None);
        let command = macro_command_for_event(&event).unwrap();
        assert!(command.len() <= UHK_MAX_MACRO_COMMAND_BYTES);
    }

    #[test]
    fn clear_macro_command_fits_uhk_payload() {
        assert!(clear_macro_command().len() <= UHK_MAX_MACRO_COMMAND_BYTES);
    }

    #[test]
    fn macro_command_accounts_for_escaped_bytes() {
        let event = AgentEventInput {
            agent: "\"\"\"\"\"\"\"\"".into(),
            host: "\"\"\"\"\"\"\"\"\"\"".into(),
            repo: Some("\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"\"".into()),
            state: AgentState::WaitingInput,
            summary: None,
            priority: None,
            ttl_seconds: Some(60),
            run_id: None,
        }
        .into_event()
        .unwrap();

        let command = macro_command_for_event(&event).unwrap();

        assert!(command.contains("\\\""));
        assert!(command.len() <= UHK_MAX_MACRO_COMMAND_BYTES);
    }

    #[test]
    fn latest_prefers_live_higher_priority() {
        let low = event(AgentState::Done, None);
        let mut high = event(AgentState::WaitingInput, None);
        high.host = "other-host".into();
        assert_eq!(choose_latest(Some(low), high.clone()).id, high.id);
    }

    #[test]
    fn latest_supersedes_same_work_item() {
        let waiting = event(AgentState::WaitingInput, None);
        let done = event(AgentState::Done, None);
        assert_eq!(choose_latest(Some(waiting), done.clone()).id, done.id);
    }
}
