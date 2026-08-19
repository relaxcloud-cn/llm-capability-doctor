use serde_json::Value;

use super::Protocol;

pub(crate) const MAX_SSE_RECORD_BYTES: usize = 1024 * 1024;
const MAX_SSE_DELIMITER_BYTES: usize = 4;
const SSE_RECORD_DELIMITERS: [&[u8]; 4] = [b"\r\n\r\n", b"\r\n\n", b"\n\r\n", b"\n\n"];

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
    scan_cursor: usize,
    state: StreamState,
    error_message: Option<&'static str>,
    anthropic_stop_reason: Option<String>,
    anthropic_message_stopped: bool,
}

impl StreamInspector {
    pub(crate) fn new(protocol: Protocol) -> Self {
        debug_assert!(Self::supports(protocol));
        Self {
            protocol,
            buffer: Vec::new(),
            scan_cursor: 0,
            state: StreamState::Pending,
            error_message: None,
            anthropic_stop_reason: None,
            anthropic_message_stopped: false,
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

    pub(crate) fn push(&mut self, mut chunk: &[u8]) -> StreamControl {
        while !chunk.is_empty() {
            let available =
                (MAX_SSE_RECORD_BYTES + MAX_SSE_DELIMITER_BYTES).saturating_sub(self.buffer.len());
            if available == 0 {
                return self.stop_oversized_record();
            }
            let copied = available.min(chunk.len());
            self.buffer.extend_from_slice(&chunk[..copied]);
            chunk = &chunk[copied..];

            if self.inspect_complete_records() == StreamControl::Stop {
                return StreamControl::Stop;
            }
            if self.residual_exceeds_limit() {
                return self.stop_oversized_record();
            }
        }
        StreamControl::Continue
    }

    pub(crate) fn finish(&mut self) -> StreamControl {
        let control = if self.buffer.is_empty() {
            StreamControl::Continue
        } else {
            let record = std::mem::take(&mut self.buffer);
            self.scan_cursor = 0;
            if record.len() > MAX_SSE_RECORD_BYTES {
                return self.stop_oversized_record();
            }
            if record.iter().all(|byte| matches!(byte, b'\r' | b'\n')) {
                StreamControl::Continue
            } else {
                self.inspect_record(&record)
            }
        };
        if control == StreamControl::Stop {
            return control;
        }
        self.finish_anthropic_fallback();
        StreamControl::Continue
    }

    pub(crate) fn state(&self) -> StreamState {
        self.state
    }

    pub(crate) fn error_message(&self) -> Option<&'static str> {
        self.error_message
    }

    fn inspect_complete_records(&mut self) -> StreamControl {
        let mut buffer = std::mem::take(&mut self.buffer);
        let mut record_start = 0;
        let mut scan_cursor = self.scan_cursor.min(buffer.len());

        while let Some((record_end, delimiter_len)) = find_record_delimiter(&buffer, scan_cursor) {
            if record_end - record_start > MAX_SSE_RECORD_BYTES {
                self.scan_cursor = 0;
                return self.stop_oversized_record();
            }
            if self.inspect_record(&buffer[record_start..record_end]) == StreamControl::Stop {
                self.scan_cursor = 0;
                return StreamControl::Stop;
            }
            record_start = record_end + delimiter_len;
            scan_cursor = record_start;
        }

        if record_start > 0 {
            let remaining = buffer.len() - record_start;
            buffer.copy_within(record_start.., 0);
            buffer.truncate(remaining);
        }
        self.scan_cursor = buffer.len().saturating_sub(MAX_SSE_DELIMITER_BYTES - 1);
        self.buffer = buffer;
        StreamControl::Continue
    }

    fn residual_exceeds_limit(&self) -> bool {
        if self.buffer.len() <= MAX_SSE_RECORD_BYTES {
            return false;
        }
        let possible_delimiter = &self.buffer[MAX_SSE_RECORD_BYTES..];
        !SSE_RECORD_DELIMITERS
            .iter()
            .any(|delimiter| delimiter.starts_with(possible_delimiter))
    }

    fn stop_oversized_record(&mut self) -> StreamControl {
        self.buffer = Vec::new();
        self.scan_cursor = 0;
        self.mark_error("SSE record exceeded the 1 MiB safety limit");
        StreamControl::Stop
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
        let finish_reason = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty());
        let has_usage = value.get("usage").is_some_and(Value::is_object);
        if finish_reason.is_some_and(|reason| matches!(reason, "length" | "content_filter")) {
            self.mark_error("OpenAI Chat stream ended with an incomplete finish reason");
        } else if finish_reason.is_some() || has_usage {
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
        if event_type == Some("message_delta")
            && !self.anthropic_message_stopped
            && let Some(reason) = value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
        {
            self.anthropic_stop_reason = Some(reason.to_owned());
        }
        if event_type == Some("message_stop") {
            self.anthropic_message_stopped = true;
            if self
                .anthropic_stop_reason
                .as_deref()
                .is_some_and(|reason| matches!(reason, "max_tokens" | "content_filter"))
            {
                self.mark_error("Anthropic stream ended with an incomplete stop reason");
            } else {
                self.mark_success();
            }
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
        let finish_reason = value
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|candidates| candidates.first())
            .and_then(|candidate| candidate.get("finishReason"))
            .and_then(Value::as_str)
            .filter(|reason| !reason.is_empty());
        let has_usage = value.get("usageMetadata").is_some_and(Value::is_object);
        if finish_reason.is_some_and(|reason| {
            matches!(
                reason,
                "MAX_TOKENS"
                    | "SAFETY"
                    | "RECITATION"
                    | "BLOCKLIST"
                    | "PROHIBITED_CONTENT"
                    | "SPII"
            )
        }) {
            self.mark_error("Google stream ended with an incomplete finish reason");
        } else if finish_reason.is_some() || has_usage {
            self.mark_success();
        }
        StreamControl::Continue
    }

    fn finish_anthropic_fallback(&mut self) {
        if self.protocol != Protocol::AnthropicMessages
            || self.anthropic_message_stopped
            || self.state != StreamState::Pending
        {
            return;
        }
        let Some(reason) = self.anthropic_stop_reason.as_deref() else {
            return;
        };
        if matches!(reason, "max_tokens" | "refusal" | "content_filter") {
            self.mark_error("Anthropic stream ended with an incomplete fallback stop reason");
        } else {
            self.mark_success();
        }
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

fn find_record_delimiter(buffer: &[u8], start: usize) -> Option<(usize, usize)> {
    for index in start..buffer.len() {
        for delimiter in SSE_RECORD_DELIMITERS {
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
