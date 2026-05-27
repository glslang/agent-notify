#[cfg(windows)]
use crate::ipc::{HidBrokerRequest, HidBrokerResponse, read_message, write_message};
use crate::uhk;
use agent_notify_core::{AgentEvent, clear_macro_command, macro_command_for_event};
#[cfg(windows)]
use anyhow::{Context, bail};
#[cfg(windows)]
use std::io::{BufReader, BufWriter};
#[cfg(any(test, windows))]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct DisplayAdapter {
    inner: DisplayAdapterInner,
}

enum DisplayAdapterInner {
    Mock(MockDisplayAdapter),
    Platform(PlatformDisplayAdapter),
}

impl DisplayAdapter {
    pub fn new(mock: bool) -> Self {
        let inner = if mock {
            DisplayAdapterInner::Mock(MockDisplayAdapter)
        } else {
            DisplayAdapterInner::Platform(PlatformDisplayAdapter::new())
        };
        Self { inner }
    }

    pub fn keyboard_present(&mut self) -> bool {
        match &mut self.inner {
            DisplayAdapterInner::Mock(display) => display.keyboard_present(),
            DisplayAdapterInner::Platform(display) => display.keyboard_present(),
        }
    }

    pub fn display_event(&mut self, event: &AgentEvent) -> anyhow::Result<String> {
        match &mut self.inner {
            DisplayAdapterInner::Mock(display) => display.display_event(event),
            DisplayAdapterInner::Platform(display) => display.display_event(event),
        }
    }

    pub fn clear(&mut self, reason: &str) -> anyhow::Result<()> {
        match &mut self.inner {
            DisplayAdapterInner::Mock(display) => display.clear(reason),
            DisplayAdapterInner::Platform(display) => display.clear(reason),
        }
    }
}

struct MockDisplayAdapter;

impl MockDisplayAdapter {
    fn keyboard_present(&mut self) -> bool {
        true
    }

    fn display_event(&mut self, event: &AgentEvent) -> anyhow::Result<String> {
        let command = macro_command_for_event(event)?;
        tracing::info!(%command, "mock UHK display");
        Ok(command)
    }

    fn clear(&mut self, reason: &str) -> anyhow::Result<()> {
        let command = clear_macro_command();
        tracing::info!(%command, %reason, "mock UHK display clear");
        Ok(())
    }
}

#[cfg(windows)]
struct PlatformDisplayAdapter {
    broker: Option<HidBrokerClient>,
}

#[cfg(windows)]
impl PlatformDisplayAdapter {
    fn new() -> Self {
        Self { broker: None }
    }

    fn keyboard_present(&mut self) -> bool {
        match self.request(HidBrokerRequest::ProbeKeyboard) {
            Ok(HidBrokerResponse::KeyboardPresent { present }) => present,
            Ok(response) => {
                tracing::warn!(?response, "unexpected HID broker keyboard probe response");
                false
            }
            Err(err) => {
                tracing::warn!(?err, "failed to probe keyboard through HID broker");
                false
            }
        }
    }

    fn display_event(&mut self, event: &AgentEvent) -> anyhow::Result<String> {
        match self.request(HidBrokerRequest::SetDisplay {
            event: event.clone(),
        })? {
            HidBrokerResponse::Ok {
                display: Some(display),
            } => Ok(display),
            HidBrokerResponse::Ok { display: None } => {
                bail!("HID broker did not return displayed command")
            }
            HidBrokerResponse::Error { code, message } => {
                bail!("HID broker rejected display request ({code}): {message}")
            }
            response => bail!("unexpected HID broker display response: {response:?}"),
        }
    }

    fn clear(&mut self, reason: &str) -> anyhow::Result<()> {
        match self.request(HidBrokerRequest::Clear {
            reason: reason.to_string(),
        })? {
            HidBrokerResponse::Ok { .. } => Ok(()),
            HidBrokerResponse::Error { code, message } => {
                bail!("HID broker rejected clear request ({code}): {message}")
            }
            response => bail!("unexpected HID broker clear response: {response:?}"),
        }
    }

    fn request(&mut self, request: HidBrokerRequest) -> anyhow::Result<HidBrokerResponse> {
        match self.request_once(request.clone()) {
            Ok(response) => Ok(response),
            Err(first_err) => {
                self.broker = None;
                tracing::warn!(?first_err, "restarting HID broker after IPC failure");
                self.request_once(request)
            }
        }
    }

    fn request_once(&mut self, request: HidBrokerRequest) -> anyhow::Result<HidBrokerResponse> {
        if self.broker.is_none() {
            self.broker = Some(HidBrokerClient::spawn().context("failed to start HID broker")?);
        }
        self.broker
            .as_mut()
            .expect("broker is initialized")
            .request(request)
    }
}

#[cfg(not(windows))]
struct PlatformDisplayAdapter;

#[cfg(not(windows))]
impl PlatformDisplayAdapter {
    fn new() -> Self {
        Self
    }

    fn keyboard_present(&mut self) -> bool {
        uhk::keyboard_present()
    }

    fn display_event(&mut self, event: &AgentEvent) -> anyhow::Result<String> {
        let command = macro_command_for_event(event)?;
        uhk::display_macro_command(&command)?;
        Ok(command)
    }

    fn clear(&mut self, _reason: &str) -> anyhow::Result<()> {
        uhk::display_macro_command(clear_macro_command())
    }
}

#[cfg(windows)]
struct HidBrokerClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

#[cfg(windows)]
impl HidBrokerClient {
    fn spawn() -> anyhow::Result<Self> {
        let current_exe = std::env::current_exe().context("failed to locate bridge executable")?;
        let broker_path = broker_path_for_current_exe(&current_exe);
        let mut child = Command::new(&broker_path)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("failed to spawn {}", broker_path.display()))?;
        let stdin = child.stdin.take().context("HID broker stdin missing")?;
        let stdout = child.stdout.take().context("HID broker stdout missing")?;
        Ok(Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
        })
    }

    fn request(&mut self, request: HidBrokerRequest) -> anyhow::Result<HidBrokerResponse> {
        write_message(&mut self.stdin, &request)?;
        let response = read_message(&mut self.stdout)?.context("HID broker closed its stdout")?;
        Ok(response)
    }
}

#[cfg(windows)]
impl Drop for HidBrokerClient {
    fn drop(&mut self) {
        let _ = write_message(&mut self.stdin, &HidBrokerRequest::Shutdown);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(any(test, windows))]
fn broker_path_for_current_exe(current_exe: &Path) -> PathBuf {
    current_exe.with_file_name(format!(
        "agent-notify-hid-broker{}",
        std::env::consts::EXE_SUFFIX
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_notify_core::{AgentEventInput, AgentState};

    fn sample_event() -> AgentEvent {
        AgentEventInput {
            agent: "codex".to_string(),
            host: "workstation".to_string(),
            repo: Some("agent-notify".to_string()),
            state: AgentState::Done,
            summary: Some("complete".to_string()),
            priority: None,
            ttl_seconds: Some(60),
            run_id: None,
        }
        .into_event()
        .unwrap()
    }

    #[test]
    fn mock_display_generates_macro_without_broker() {
        let mut display = DisplayAdapter::new(true);
        assert!(display.keyboard_present());
        let command = display.display_event(&sample_event()).unwrap();
        assert!(command.starts_with("notify "));
        display.clear("test").unwrap();
    }

    #[test]
    fn broker_path_is_sibling_binary() {
        let current = Path::new(r"C:\tools\agent-notify-bridge.exe");
        let broker = broker_path_for_current_exe(current);
        assert!(broker.ends_with(format!(
            "agent-notify-hid-broker{}",
            std::env::consts::EXE_SUFFIX
        )));
    }
}
