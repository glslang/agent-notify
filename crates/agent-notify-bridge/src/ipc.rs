use agent_notify_core::AgentEvent;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{BufRead, Write};

pub const MAX_IPC_MESSAGE_BYTES: usize = 8192;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HidBrokerRequest {
    ProbeKeyboard,
    SetDisplay { event: AgentEvent },
    Clear { reason: String },
    Shutdown,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HidBrokerResponse {
    Ok { display: Option<String> },
    KeyboardPresent { present: bool },
    Error { code: String, message: String },
}

pub fn write_message<W, T>(writer: &mut W, message: &T) -> anyhow::Result<()>
where
    W: Write,
    T: Serialize,
{
    let mut encoded = serde_json::to_vec(message).context("failed to encode IPC message")?;
    if encoded.len() > MAX_IPC_MESSAGE_BYTES {
        bail!("IPC message exceeds {MAX_IPC_MESSAGE_BYTES} bytes");
    }
    encoded.push(b'\n');
    writer
        .write_all(&encoded)
        .context("failed to write IPC message")?;
    writer.flush().context("failed to flush IPC message")?;
    Ok(())
}

pub fn read_message<R, T>(reader: &mut R) -> anyhow::Result<Option<T>>
where
    R: BufRead,
    T: DeserializeOwned,
{
    let mut encoded = Vec::new();
    let size = reader
        .read_until(b'\n', &mut encoded)
        .context("failed to read IPC message")?;
    if size == 0 {
        return Ok(None);
    }
    if encoded.last() == Some(&b'\n') {
        encoded.pop();
    }
    if encoded.len() > MAX_IPC_MESSAGE_BYTES {
        bail!("IPC message exceeds {MAX_IPC_MESSAGE_BYTES} bytes");
    }
    let message = serde_json::from_slice(&encoded).context("failed to decode IPC message")?;
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_notify_core::{AgentEventInput, AgentState};
    use proptest::prelude::*;
    use serde::{Serialize, de::DeserializeOwned};
    use std::io::{BufReader, Cursor};

    fn sample_event() -> AgentEvent {
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

    fn arb_string(max_chars: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(any::<char>(), 0..=max_chars)
            .prop_map(|chars| chars.into_iter().collect())
    }

    fn arb_nonblank_string(max_chars: usize) -> impl Strategy<Value = String> {
        arb_string(max_chars.saturating_sub(1)).prop_map(|tail| format!("x{tail}"))
    }

    fn arb_agent_state() -> impl Strategy<Value = AgentState> {
        prop_oneof![
            Just(AgentState::Running),
            Just(AgentState::WaitingInput),
            Just(AgentState::Done),
            Just(AgentState::Failed),
        ]
    }

    fn arb_event() -> impl Strategy<Value = AgentEvent> {
        (
            arb_nonblank_string(40),
            arb_nonblank_string(40),
            prop::option::of(arb_string(40)),
            arb_agent_state(),
            prop::option::of(arb_string(80)),
            prop::option::of(any::<u8>()),
            prop::option::of(1_u64..=3_600),
            prop::option::of(arb_string(40)),
        )
            .prop_map(
                |(agent, host, repo, state, summary, priority, ttl_seconds, run_id)| {
                    AgentEventInput {
                        agent,
                        host,
                        repo,
                        state,
                        summary,
                        priority,
                        ttl_seconds,
                        run_id,
                    }
                    .into_event()
                    .unwrap()
                },
            )
    }

    fn arb_request() -> impl Strategy<Value = HidBrokerRequest> {
        prop_oneof![
            Just(HidBrokerRequest::ProbeKeyboard),
            arb_event().prop_map(|event| HidBrokerRequest::SetDisplay { event }),
            arb_string(80).prop_map(|reason| HidBrokerRequest::Clear { reason }),
            Just(HidBrokerRequest::Shutdown),
        ]
    }

    fn arb_response() -> impl Strategy<Value = HidBrokerResponse> {
        prop_oneof![
            prop::option::of(arb_string(80)).prop_map(|display| HidBrokerResponse::Ok { display }),
            any::<bool>().prop_map(|present| HidBrokerResponse::KeyboardPresent { present }),
            (arb_string(40), arb_string(120))
                .prop_map(|(code, message)| HidBrokerResponse::Error { code, message }),
        ]
    }

    fn assert_json_line_round_trip<T>(message: T) -> proptest::test_runner::TestCaseResult
    where
        T: DeserializeOwned + Serialize,
    {
        let expected = serde_json::to_value(&message).unwrap();
        let mut encoded = Vec::new();
        write_message(&mut encoded, &message).unwrap();

        prop_assert_eq!(encoded.last(), Some(&b'\n'));
        prop_assert_eq!(encoded.iter().filter(|byte| **byte == b'\n').count(), 1);
        prop_assert!(encoded.len() <= MAX_IPC_MESSAGE_BYTES + 1);

        let mut reader = BufReader::new(Cursor::new(encoded));
        let decoded: T = read_message(&mut reader).unwrap().unwrap();
        let actual = serde_json::to_value(decoded).unwrap();
        prop_assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn request_round_trips_as_json_line() {
        let request = HidBrokerRequest::SetDisplay {
            event: sample_event(),
        };
        let mut encoded = Vec::new();
        write_message(&mut encoded, &request).unwrap();

        let mut reader = BufReader::new(Cursor::new(encoded));
        let decoded: HidBrokerRequest = read_message(&mut reader).unwrap().unwrap();
        assert!(matches!(decoded, HidBrokerRequest::SetDisplay { .. }));
    }

    #[test]
    fn unknown_raw_macro_request_is_rejected() {
        let raw = br#"{"type":"display_macro","command":"notify \"hello\""}
"#;
        let mut reader = BufReader::new(Cursor::new(raw));
        let err = read_message::<_, HidBrokerRequest>(&mut reader).unwrap_err();
        assert!(err.to_string().contains("failed to decode IPC message"));
    }

    #[test]
    fn oversized_messages_are_rejected() {
        let raw = vec![b'a'; MAX_IPC_MESSAGE_BYTES + 1];
        let mut reader = BufReader::new(Cursor::new(raw));
        let err = read_message::<_, HidBrokerRequest>(&mut reader).unwrap_err();
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn max_size_message_with_newline_is_accepted_by_reader() {
        let fixed_len = r#"{"type":"error","code":"x","message":""#.len() + r#""}"#.len();
        let message = "x".repeat(MAX_IPC_MESSAGE_BYTES - fixed_len);
        let response = HidBrokerResponse::Error {
            code: "x".to_string(),
            message,
        };
        let mut encoded = Vec::new();
        write_message(&mut encoded, &response).unwrap();
        assert_eq!(encoded.len(), MAX_IPC_MESSAGE_BYTES + 1);

        let mut reader = BufReader::new(Cursor::new(encoded));
        let decoded: HidBrokerResponse = read_message(&mut reader).unwrap().unwrap();
        assert!(matches!(decoded, HidBrokerResponse::Error { .. }));
    }

    proptest! {
        #[test]
        fn generated_requests_round_trip_as_json_lines(request in arb_request()) {
            assert_json_line_round_trip(request)?;
        }

        #[test]
        fn generated_responses_round_trip_as_json_lines(response in arb_response()) {
            assert_json_line_round_trip(response)?;
        }

        #[test]
        fn arbitrary_raw_frames_decode_or_fail_cleanly(mut raw in prop::collection::vec(any::<u8>(), 0..=MAX_IPC_MESSAGE_BYTES + 32)) {
            raw.push(b'\n');
            let mut reader = BufReader::new(Cursor::new(raw));
            let result = read_message::<_, HidBrokerRequest>(&mut reader);

            if let Err(err) = result {
                let message = err.to_string();
                prop_assert!(
                    message.contains("failed to decode IPC message")
                        || message.contains("exceeds"),
                    "unexpected raw-frame error: {message}"
                );
            }
        }
    }
}
