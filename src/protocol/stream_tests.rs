use super::stream::{StreamControl, StreamInspector, StreamState};
use crate::protocol::Protocol;

fn fallback_record(protocol: Protocol) -> &'static [u8] {
    match protocol {
        Protocol::OpenAiChat => b"data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
        Protocol::OpenAiResponses => b"data: {\"type\":\"response.completed\"}\n\n",
        Protocol::AnthropicMessages => b"data: {\"type\":\"message_stop\"}\n\n",
        Protocol::GeminiGenerateContent => {
            b"data: {\"candidates\":[{\"finishReason\":\"STOP\"}]}\n\n"
        }
        Protocol::OllamaChat | Protocol::Unknown => unreachable!(),
    }
}

#[test]
fn buffers_lf_records_split_across_chunks() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiChat);

    assert_eq!(
        inspector.push(b"data: {\"choices\":[{\"finish_"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Pending);
    assert_eq!(
        inspector.push(b"reason\":\"stop\"}]}\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Pending);
    assert_eq!(inspector.push(b"\n"), StreamControl::Continue);
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn accepts_crlf_record_delimiters() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiResponses);

    assert_eq!(
        inspector.push(b"data: {\"type\":\"response.completed\"}\r\n\r\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn joins_multiple_data_lines_before_parsing_json() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiResponses);

    assert_eq!(
        inspector.push(
            b"event: response.completed\r\ndata: {\"type\":\r\ndata: \"response.completed\"}\r\n\r\n"
        ),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn finish_dispatches_a_complete_residual_record_without_newline() {
    let mut inspector = StreamInspector::new(Protocol::GeminiGenerateContent);

    assert_eq!(
        inspector.push(b"data: {\"usageMetadata\":{}}"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Pending);
    assert_eq!(inspector.finish(), StreamControl::Continue);
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn chat_done_is_an_immediate_success() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiChat);

    assert_eq!(inspector.push(b"data: [DONE]\n\n"), StreamControl::Stop);
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn chat_requires_the_exact_data_space_prefix() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiChat);

    assert_eq!(inspector.push(b"data:[DONE]\n\n"), StreamControl::Continue);
    assert_eq!(inspector.finish(), StreamControl::Continue);
    assert_eq!(inspector.state(), StreamState::Pending);
}

#[test]
fn protocol_fallback_terminals_succeed_without_stopping() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
    ] {
        let mut inspector = StreamInspector::new(protocol);

        assert_eq!(
            inspector.push(fallback_record(protocol)),
            StreamControl::Continue,
            "{protocol}"
        );
        assert_eq!(inspector.state(), StreamState::Success, "{protocol}");
    }
}

#[test]
fn chat_usage_object_is_a_fallback_success() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiChat);

    assert_eq!(
        inspector.push(b"data: {\"choices\":[],\"usage\":{}}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn anthropic_stop_reason_is_a_fallback_success() {
    let mut inspector = StreamInspector::new(Protocol::AnthropicMessages);

    assert_eq!(
        inspector.push(
            b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n"
        ),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn google_usage_object_is_a_fallback_success() {
    let mut inspector = StreamInspector::new(Protocol::GeminiGenerateContent);

    assert_eq!(
        inspector.push(b"data:{\"usageMetadata\":{}}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn explicit_sse_error_is_immediate_for_every_supported_protocol() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
    ] {
        let mut inspector = StreamInspector::new(protocol);

        assert_eq!(
            inspector.push(b"event: error\ndata: {}\n\n"),
            StreamControl::Stop,
            "{protocol}"
        );
        assert_eq!(inspector.state(), StreamState::Error, "{protocol}");
        assert!(inspector.error_message().is_some(), "{protocol}");
    }
}

#[test]
fn protocol_native_error_terminals_stop_immediately() {
    let cases: [(Protocol, &[u8]); 5] = [
        (
            Protocol::OpenAiChat,
            b"data: {\"error\":{\"message\":\"failed\"}}\n\n",
        ),
        (
            Protocol::OpenAiResponses,
            b"data: {\"type\":\"response.failed\"}\n\n",
        ),
        (
            Protocol::OpenAiResponses,
            b"event: response.incomplete\ndata: {}\n\n",
        ),
        (
            Protocol::AnthropicMessages,
            b"data: {\"type\":\"error\",\"error\":{}}\n\n",
        ),
        (
            Protocol::GeminiGenerateContent,
            b"data: {\"error\":{\"message\":\"failed\"}}\n\n",
        ),
    ];

    for (protocol, record) in cases {
        let mut inspector = StreamInspector::new(protocol);
        assert_eq!(inspector.push(record), StreamControl::Stop, "{protocol}");
        assert_eq!(inspector.state(), StreamState::Error, "{protocol}");
    }
}

#[test]
fn responses_failure_event_stops_even_with_malformed_data() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiResponses);

    assert_eq!(
        inspector.push(b"event: response.failed\ndata: {not-json}\n\n"),
        StreamControl::Stop
    );
    assert_eq!(inspector.state(), StreamState::Error);
}

#[test]
fn malformed_json_marks_chat_responses_and_google_error_but_continues() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::GeminiGenerateContent,
    ] {
        let mut inspector = StreamInspector::new(protocol);

        assert_eq!(
            inspector.push(b"data: {not-json}\n\n"),
            StreamControl::Continue,
            "{protocol}"
        );
        assert_eq!(inspector.state(), StreamState::Error, "{protocol}");
        assert!(inspector.error_message().is_some(), "{protocol}");
    }
}

#[test]
fn anthropic_drops_malformed_json_frames() {
    let mut inspector = StreamInspector::new(Protocol::AnthropicMessages);

    assert_eq!(
        inspector.push(b"data: {not-json}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Pending);
    assert_eq!(
        inspector.push(b"event: message_stop\ndata: {}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
}

#[test]
fn later_error_overrides_fallback_success() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiResponses);

    assert_eq!(
        inspector.push(b"data: {\"type\":\"response.completed\"}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Success);
    assert_eq!(
        inspector.push(b"data: {\"type\":\"response.failed\"}\n\n"),
        StreamControl::Stop
    );
    assert_eq!(inspector.state(), StreamState::Error);
}

#[test]
fn later_success_does_not_overwrite_an_error() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiChat);

    assert_eq!(
        inspector.push(b"data: {not-json}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.state(), StreamState::Error);
    assert_eq!(inspector.push(b"data: [DONE]\n\n"), StreamControl::Stop);
    assert_eq!(inspector.state(), StreamState::Error);
}

#[test]
fn eof_without_a_native_terminal_stays_pending() {
    let mut inspector = StreamInspector::new(Protocol::OpenAiResponses);

    assert_eq!(
        inspector.push(b"data: {\"type\":\"response.output_text.delta\"}\n\n"),
        StreamControl::Continue
    );
    assert_eq!(inspector.finish(), StreamControl::Continue);
    assert_eq!(inspector.state(), StreamState::Pending);
}
