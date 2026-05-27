use crate::ipc::{HidBrokerRequest, HidBrokerResponse, read_message, write_message};
use crate::uhk;
use agent_notify_core::{clear_macro_command, macro_command_for_event};
use std::io::{BufRead, Write};
use tracing::info;

pub trait HidBackend {
    fn keyboard_present(&mut self) -> bool;
    fn display_macro_command(&mut self, command: &str) -> anyhow::Result<()>;
}

#[derive(Debug, Default)]
pub struct RealHidBackend;

impl HidBackend for RealHidBackend {
    fn keyboard_present(&mut self) -> bool {
        uhk::keyboard_present()
    }

    fn display_macro_command(&mut self, command: &str) -> anyhow::Result<()> {
        uhk::display_macro_command(command)
    }
}

#[derive(Debug, Default)]
pub struct MockHidBackend {
    pub commands: Vec<String>,
}

impl HidBackend for MockHidBackend {
    fn keyboard_present(&mut self) -> bool {
        true
    }

    fn display_macro_command(&mut self, command: &str) -> anyhow::Result<()> {
        info!(%command, "mock UHK display");
        self.commands.push(command.to_string());
        Ok(())
    }
}

pub fn handle_request<B>(backend: &mut B, request: HidBrokerRequest) -> HidBrokerResponse
where
    B: HidBackend,
{
    match request {
        HidBrokerRequest::ProbeKeyboard => HidBrokerResponse::KeyboardPresent {
            present: backend.keyboard_present(),
        },
        HidBrokerRequest::SetDisplay { event } => match macro_command_for_event(&event) {
            Ok(command) => match backend.display_macro_command(&command) {
                Ok(()) => HidBrokerResponse::Ok {
                    display: Some(command),
                },
                Err(err) => error_response("hid_write_failed", err),
            },
            Err(err) => error_response("invalid_event", err),
        },
        HidBrokerRequest::Clear { reason } => {
            let command = clear_macro_command();
            match backend.display_macro_command(command) {
                Ok(()) => {
                    info!(%reason, "cleared UHK display");
                    HidBrokerResponse::Ok { display: None }
                }
                Err(err) => error_response("hid_clear_failed", err),
            }
        }
        HidBrokerRequest::Shutdown => HidBrokerResponse::Ok { display: None },
    }
}

pub fn run_stdio<B, R, W>(backend: &mut B, reader: &mut R, writer: &mut W) -> anyhow::Result<()>
where
    B: HidBackend,
    R: BufRead,
    W: Write,
{
    while let Some(request) = read_message::<_, HidBrokerRequest>(reader)? {
        let shutdown = matches!(request, HidBrokerRequest::Shutdown);
        let response = handle_request(backend, request);
        write_message(writer, &response)?;
        if shutdown {
            break;
        }
    }
    Ok(())
}

fn error_response(code: &'static str, err: impl std::fmt::Display) -> HidBrokerResponse {
    HidBrokerResponse::Error {
        code: code.to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_notify_core::{AgentEventInput, AgentState};
    use std::io::{BufReader, Cursor};

    fn sample_event() -> agent_notify_core::AgentEvent {
        AgentEventInput {
            agent: "codex".to_string(),
            host: "workstation".to_string(),
            repo: Some("agent-notify".to_string()),
            state: AgentState::WaitingInput,
            summary: Some("waiting for input".to_string()),
            priority: None,
            ttl_seconds: Some(60),
            run_id: None,
        }
        .into_event()
        .unwrap()
    }

    #[test]
    fn set_display_generates_macro_inside_broker() {
        let mut backend = MockHidBackend::default();
        let response = handle_request(
            &mut backend,
            HidBrokerRequest::SetDisplay {
                event: sample_event(),
            },
        );

        assert_eq!(
            response,
            HidBrokerResponse::Ok {
                display: backend.commands.first().cloned()
            }
        );
        assert_eq!(backend.commands.len(), 1);
        assert!(backend.commands[0].starts_with("setLedTxt 0 notification"));
    }

    #[test]
    fn clear_generates_known_clear_macro_inside_broker() {
        let mut backend = MockHidBackend::default();
        let response = handle_request(
            &mut backend,
            HidBrokerRequest::Clear {
                reason: "test".to_string(),
            },
        );

        assert_eq!(response, HidBrokerResponse::Ok { display: None });
        assert_eq!(backend.commands, vec![clear_macro_command().to_string()]);
    }

    #[test]
    fn stdio_loop_processes_requests_until_shutdown() {
        let requests = [HidBrokerRequest::ProbeKeyboard, HidBrokerRequest::Shutdown];
        let mut input = Vec::new();
        for request in requests {
            write_message(&mut input, &request).unwrap();
        }

        let mut backend = MockHidBackend::default();
        let mut reader = BufReader::new(Cursor::new(input));
        let mut output = Vec::new();
        run_stdio(&mut backend, &mut reader, &mut output).unwrap();

        let mut reader = BufReader::new(Cursor::new(output));
        let first: HidBrokerResponse = read_message(&mut reader).unwrap().unwrap();
        let second: HidBrokerResponse = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(first, HidBrokerResponse::KeyboardPresent { present: true });
        assert_eq!(second, HidBrokerResponse::Ok { display: None });
    }
}
