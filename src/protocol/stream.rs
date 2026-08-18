use serde_json::Value;

use super::Protocol;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamControl {
    Continue,
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StreamState {
    Pending,
    Success,
    Error,
}

pub(crate) struct StreamInspector {
    protocol: Protocol,
    buffer: Vec<u8>,
    state: StreamState,
    error_message: Option<&'static str>,
}

impl StreamInspector {
    pub(crate) fn new(protocol: Protocol) -> Self {
        debug_assert!(Self::supports(protocol));
        Self {
            protocol,
            buffer: Vec::new(),
            state: StreamState::Pending,
            error_message: None,
        }
    }

    pub(crate) fn supports(protocol: Protocol) -> bool {
        matches!(
            protocol,
            Protocol::OpenAiChat
                | Protocol::OpenAiResponses
                | Protocol::AnthropicMessages
                | Protocol::GeminiGenerateContent
        )
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> StreamControl {
        self.buffer.extend_from_slice(chunk);
        while let Some((record_end, delimiter_len)) = find_record_delimiter(&self.buffer) {
            let record = self.buffer[..record_end].to_vec();
            self.buffer.drain(..record_end + delimiter_len);
            if self.inspect_record(&record) == StreamControl::Stop {
                return StreamControl::Stop;
            }
        }
        StreamControl::Continue
    }

    pub(crate) fn finish(&mut self) -> StreamControl {
        if self.buffer.is_empty() {
            return StreamControl::Continue;
        }
        let record = std::mem::take(&mut self.buffer);
        if record.iter().all(|byte| matches!(byte, b'\r' | b'\n')) {
            return StreamControl::Continue;
        }
        self.inspect_record(&record)
    }

    pub(crate) fn state(&self) -> StreamState {
        self.state
    }

    pub(crate) fn error_message(&self) -> Option<&'static str> {
        self.error_message
    }

    fn inspect_record(&mut self, record: &[u8]) -> StreamControl {
        let record = String::from_utf8_lossy(record);
        let mut event = None;
        let mut data_lines = Vec::new();

        for line in record.split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if let Some(value) = line.strip_prefix("event:") {
                event = Some(value.strip_prefix(' ').unwrap_or(value));
                continue;
            }
            match self.protocol {
                Protocol::OpenAiChat => {
                    if let Some(value) = line.strip_prefix("data: ") {
                        data_lines.push(value);
                    }
                }
                Protocol::OpenAiResponses
                | Protocol::AnthropicMessages
                | Protocol::GeminiGenerateContent => {
                    if let Some(value) = line.strip_prefix("data:") {
                        data_lines.push(value.strip_prefix(' ').unwrap_or(value));
                    }
                }
                Protocol::OllamaChat | Protocol::Unknown => {}
            }
        }

        if event.is_some_and(is_explicit_error_event) {
            self.mark_error("stream emitted an explicit error event");
            return StreamControl::Stop;
        }
        if self.protocol == Protocol::OpenAiResponses
            && matches!(event, Some("response.failed" | "response.incomplete"))
        {
            self.mark_error("OpenAI Responses stream ended with a failure event");
            return StreamControl::Stop;
        }
        if data_lines.is_empty() {
            return StreamControl::Continue;
        }

        let data = data_lines.join("\n");
        if self.protocol == Protocol::OpenAiChat && data == "[DONE]" {
            self.mark_success();
            return StreamControl::Stop;
        }

        let value = match serde_json::from_str::<Value>(&data) {
            Ok(value) => value,
            Err(_) if self.protocol == Protocol::AnthropicMessages => {
                return StreamControl::Continue;
            }
            Err(_) => {
                self.mark_error("stream contained malformed JSON data");
                return StreamControl::Continue;
            }
        };
        let event_type = value.get("type").and_then(Value::as_str).or(event);

        if event_type.is_some_and(is_explicit_error_event) {
            self.mark_error("stream emitted an explicit error event");
            return StreamControl::Stop;
        }

        match self.protocol {
            Protocol::OpenAiChat => self.inspect_chat(&value),
            Protocol::OpenAiResponses => self.inspect_responses(event_type),
            Protocol::AnthropicMessages => self.inspect_anthropic(event_type, &value),
            Protocol::GeminiGenerateContent => self.inspect_google(&value),
            Protocol::OllamaChat | Protocol::Unknown => StreamControl::Continue,
        }
    }

    fn inspect_chat(&mut self, value: &Value) -> StreamControl {
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("error"))
        {
            self.mark_error("OpenAI Chat stream returned an error object");
            return StreamControl::Stop;
        }
        let has_finish_reason =
            value
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(|choices| {
                    choices
                        .iter()
                        .any(|choice| non_empty_string(choice.get("finish_reason")))
                });
        let has_usage = value.get("usage").is_some_and(Value::is_object);
        if has_finish_reason || has_usage {
            self.mark_success();
        }
        StreamControl::Continue
    }

    fn inspect_responses(&mut self, event_type: Option<&str>) -> StreamControl {
        match event_type {
            Some("response.failed" | "response.incomplete") => {
                self.mark_error("OpenAI Responses stream ended with a failure event");
                StreamControl::Stop
            }
            Some("response.completed") => {
                self.mark_success();
                StreamControl::Continue
            }
            _ => StreamControl::Continue,
        }
    }

    fn inspect_anthropic(&mut self, event_type: Option<&str>, value: &Value) -> StreamControl {
        let has_stop_reason = event_type == Some("message_delta")
            && value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .is_some_and(|reason| non_empty_string(Some(reason)));
        if event_type == Some("message_stop") || has_stop_reason {
            self.mark_success();
        }
        StreamControl::Continue
    }

    fn inspect_google(&mut self, value: &Value) -> StreamControl {
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("error"))
        {
            self.mark_error("Google GenerateContent stream returned an error object");
            return StreamControl::Stop;
        }
        let has_finish_reason = value
            .get("candidates")
            .and_then(Value::as_array)
            .is_some_and(|candidates| {
                candidates
                    .iter()
                    .any(|candidate| non_empty_string(candidate.get("finishReason")))
            });
        let has_usage = value.get("usageMetadata").is_some_and(Value::is_object);
        if has_finish_reason || has_usage {
            self.mark_success();
        }
        StreamControl::Continue
    }

    fn mark_success(&mut self) {
        if self.state == StreamState::Pending {
            self.state = StreamState::Success;
        }
    }

    fn mark_error(&mut self, message: &'static str) {
        self.state = StreamState::Error;
        if self.error_message.is_none() {
            self.error_message = Some(message);
        }
    }
}

fn find_record_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    for index in 0..buffer.len() {
        for delimiter in [b"\r\n\r\n".as_slice(), b"\r\n\n", b"\n\r\n", b"\n\n"] {
            if buffer[index..].starts_with(delimiter) {
                return Some((index, delimiter.len()));
            }
        }
    }
    None
}

fn is_explicit_error_event(event: &str) -> bool {
    event == "error" || event.ends_with(".error")
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
}
