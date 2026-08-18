use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Local};
use thiserror::Error;
use url::Url;

use crate::evidence::{
    StreamEndSignal, StreamTermination, ToolContractStatus, ToolLoopOutcome, TransportOutcome,
};
use crate::protocol::{AuthMode, Protocol};
use crate::redaction::Redactor;

pub struct RunMetadata {
    pub run_id: String,
    pub started_at: DateTime<Local>,
    pub url: Url,
    pub model: String,
    pub masked_api_key: String,
    pub selected_test_count: usize,
    pub insecure: bool,
}

pub struct ResponseMetrics {
    pub transport_exit_code: i32,
    pub http_status: Option<u16>,
    pub time_total: Duration,
    pub time_starttransfer: Duration,
    pub size_download: usize,
}

pub struct RequestEvidence {
    pub request_id: String,
    pub started_at: DateTime<Local>,
    pub completed_at: DateTime<Local>,
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
    pub stream: bool,
    pub url: Url,
    pub timeout: Duration,
    pub insecure: bool,
    pub body: String,
    pub metrics: ResponseMetrics,
    pub headers: Vec<(String, String)>,
    pub error: String,
    pub response_body: Vec<u8>,
    pub transport_outcome: TransportOutcome,
    pub stream_termination: StreamTermination,
    pub stream_end_signal: StreamEndSignal,
    pub model_stop_reason: Option<String>,
    pub stream_event_count: usize,
    pub tool_contract_status: ToolContractStatus,
    pub tool_contract_errors: Vec<String>,
    pub tool_loop_turn: usize,
    pub tool_loop_outcome: ToolLoopOutcome,
}

pub struct TestManifest {
    pub id: String,
    pub name: String,
    pub category: String,
    pub completed_at: DateTime<Local>,
    pub request_refs: Vec<String>,
}

pub struct AuditWriter {
    writer: BufWriter<File>,
    redactor: Redactor,
    request_ids: HashSet<String>,
    manifest_ids: HashSet<String>,
    finished: bool,
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("duplicate request block: {0}")]
    DuplicateRequest(String),
    #[error("duplicate test manifest: {0}")]
    DuplicateManifest(String),
    #[error("test manifest references missing request: {0}")]
    MissingRequestReference(String),
    #[error("audit log has already been finished")]
    AlreadyFinished,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AuditWriter {
    pub fn create(
        path: &Path,
        metadata: RunMetadata,
        redactor: Redactor,
    ) -> Result<Self, AuditError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }

        let file = open_private_file(path)?;
        let mut audit = Self {
            writer: BufWriter::new(file),
            redactor,
            request_ids: HashSet::new(),
            manifest_ids: HashSet::new(),
            finished: false,
        };
        audit.write_header(&metadata)?;
        Ok(audit)
    }

    pub fn append_request(&mut self, request: &RequestEvidence) -> Result<(), AuditError> {
        self.ensure_open()?;
        if !self.request_ids.insert(request.request_id.clone()) {
            return Err(AuditError::DuplicateRequest(request.request_id.clone()));
        }

        let rendered = self.render_request(request);
        self.writer.write_all(rendered.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn append_manifest(&mut self, manifest: &TestManifest) -> Result<(), AuditError> {
        self.ensure_open()?;
        if !self.manifest_ids.insert(manifest.id.clone()) {
            return Err(AuditError::DuplicateManifest(manifest.id.clone()));
        }
        if let Some(missing) = manifest
            .request_refs
            .iter()
            .find(|request_id| !self.request_ids.contains(*request_id))
        {
            self.manifest_ids.remove(&manifest.id);
            return Err(AuditError::MissingRequestReference(missing.clone()));
        }

        writeln!(
            self.writer,
            "========== TEST-{} BEGIN ==========",
            manifest.id
        )?;
        writeln!(self.writer, "name: {}", manifest.name)?;
        writeln!(self.writer, "category: {}", manifest.category)?;
        writeln!(
            self.writer,
            "completed_at: {}",
            format_timestamp(manifest.completed_at)
        )?;
        writeln!(
            self.writer,
            "request_refs: {}",
            manifest.request_refs.join(",")
        )?;
        writeln!(
            self.writer,
            "========== TEST-{} END ==========\n",
            manifest.id
        )?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn finish(
        &mut self,
        duration: Duration,
        completed_at: DateTime<Local>,
    ) -> Result<(), AuditError> {
        self.ensure_open()?;
        writeln!(self.writer, "========== RUN SUMMARY ==========")?;
        writeln!(
            self.writer,
            "completed_at: {}",
            format_timestamp(completed_at)
        )?;
        writeln!(self.writer, "duration_seconds: {}", duration.as_secs())?;
        writeln!(self.writer, "request_count: {}", self.request_ids.len())?;
        writeln!(
            self.writer,
            "test_manifest_count: {}",
            self.manifest_ids.len()
        )?;
        writeln!(self.writer, "========== END ==========")?;
        self.writer.flush()?;
        self.finished = true;
        Ok(())
    }

    pub fn request_count(&self) -> usize {
        self.request_ids.len()
    }

    pub fn manifest_count(&self) -> usize {
        self.manifest_ids.len()
    }

    fn write_header(&mut self, metadata: &RunMetadata) -> Result<(), AuditError> {
        writeln!(self.writer, "========== MODEL DOCTOR RUN ==========")?;
        writeln!(self.writer, "run_id: {}", metadata.run_id)?;
        writeln!(self.writer, "script_version: {}", env!("CARGO_PKG_VERSION"))?;
        writeln!(self.writer, "collector_runtime: rust")?;
        writeln!(self.writer, "section_encoding: base64")?;
        writeln!(self.writer, "log_schema: llm-capability-doctor.evidence.v3")?;
        writeln!(
            self.writer,
            "started_at: {}",
            format_timestamp(metadata.started_at)
        )?;
        writeln!(
            self.writer,
            "url: {}",
            self.redactor.redact_url(&metadata.url)
        )?;
        writeln!(
            self.writer,
            "model: {}",
            single_line(&self.redactor.redact_text(&metadata.model))
        )?;
        writeln!(
            self.writer,
            "api_key: {}",
            single_line(&metadata.masked_api_key)
        )?;
        writeln!(self.writer, "curl_version: not_used (native rust reqwest)")?;
        writeln!(
            self.writer,
            "tls_verification: {}",
            if metadata.insecure {
                "disabled"
            } else {
                "enabled"
            }
        )?;
        writeln!(
            self.writer,
            "selected_test_count: {}\n",
            metadata.selected_test_count
        )?;
        self.writer.flush()?;
        Ok(())
    }

    fn render_request(&self, request: &RequestEvidence) -> String {
        let safe_url = self.redactor.redact_url(&request.url);
        let safe_body = self.redactor.redact_text(&request.body);
        let safe_error = self.redactor.redact_text(&request.error);
        let safe_response = self
            .redactor
            .redact_text(&String::from_utf8_lossy(&request.response_body));
        let safe_headers = self.redactor.redact_headers(&request.headers);
        let safe_stream_end_signal = single_line(
            &self
                .redactor
                .redact_text(&request.stream_end_signal.to_string()),
        );
        let safe_model_stop_reason = request.model_stop_reason.as_deref().map_or_else(
            || "none".to_owned(),
            |reason| single_line(&self.redactor.redact_text(reason)),
        );
        let safe_tool_contract_errors: Vec<String> = request
            .tool_contract_errors
            .iter()
            .map(|error| self.redactor.redact_text(error))
            .collect();
        let safe_tool_contract_errors_json = serde_json::to_string(&safe_tool_contract_errors)
            .expect("serializing a string array cannot fail");
        let mut output = String::new();

        push_line(
            &mut output,
            &format!("========== REQUEST {} BEGIN ==========", request.request_id),
        );
        push_line(&mut output, &format!("request_id: {}", request.request_id));
        push_line(
            &mut output,
            &format!("started_at: {}", format_timestamp(request.started_at)),
        );
        push_line(
            &mut output,
            &format!("completed_at: {}", format_timestamp(request.completed_at)),
        );
        push_line(&mut output, &format!("protocol: {}", request.protocol));
        push_line(&mut output, &format!("auth_mode: {}", request.auth_mode));
        push_line(
            &mut output,
            &format!("stream: {}", u8::from(request.stream)),
        );
        push_line(
            &mut output,
            &format!("transport_outcome: {}", request.transport_outcome),
        );
        push_line(
            &mut output,
            &format!("stream_termination: {}", request.stream_termination),
        );
        push_line(
            &mut output,
            &format!("stream_end_signal: {safe_stream_end_signal}"),
        );
        push_line(
            &mut output,
            &format!("model_stop_reason: {safe_model_stop_reason}"),
        );
        push_line(
            &mut output,
            &format!("stream_event_count: {}", request.stream_event_count),
        );
        push_line(
            &mut output,
            &format!("tool_contract_status: {}", request.tool_contract_status),
        );
        push_line(
            &mut output,
            &format!("tool_contract_errors_json: {safe_tool_contract_errors_json}"),
        );
        push_line(
            &mut output,
            &format!("tool_loop_turn: {}", request.tool_loop_turn),
        );
        push_line(
            &mut output,
            &format!("tool_loop_outcome: {}", request.tool_loop_outcome),
        );
        output.push('\n');
        push_line(&mut output, "----- CURL COMMAND BEGIN -----");
        push_line(&mut output, "curl \\");
        push_line(&mut output, "  --silent \\");
        push_line(&mut output, "  --show-error \\");
        push_line(
            &mut output,
            &format!("  --max-time {} \\", request.timeout.as_secs()),
        );
        push_line(&mut output, "  --request POST \\");
        push_line(
            &mut output,
            "  --header 'Accept: application/json, text/event-stream' \\",
        );
        push_line(
            &mut output,
            "  --header 'Content-Type: application/json' \\",
        );
        render_auth_header(&mut output, request.auth_mode);
        if request.insecure {
            push_line(&mut output, "  --insecure \\");
        }
        if request.stream {
            push_line(&mut output, "  --no-buffer \\");
        }
        push_line(&mut output, "  --data-binary @- \\");
        push_line(&mut output, &format!("  {}", shell_quote(&safe_url)));
        push_line(&mut output, "----- CURL COMMAND END -----");
        output.push('\n');
        push_encoded_section(&mut output, "REQUEST BODY", &safe_body);
        push_line(&mut output, "----- RESPONSE METRICS BEGIN -----");
        push_line(
            &mut output,
            &format!("curl_exit_code: {}", request.metrics.transport_exit_code),
        );
        push_line(
            &mut output,
            &format!(
                "http_status: {}",
                request
                    .metrics
                    .http_status
                    .map_or_else(|| "not_available".to_owned(), |status| status.to_string())
            ),
        );
        push_line(
            &mut output,
            &format!(
                "time_total: {:.6}",
                request.metrics.time_total.as_secs_f64()
            ),
        );
        push_line(
            &mut output,
            &format!(
                "time_starttransfer: {:.6}",
                request.metrics.time_starttransfer.as_secs_f64()
            ),
        );
        push_line(
            &mut output,
            &format!("size_download: {}", request.metrics.size_download),
        );
        push_line(&mut output, "----- RESPONSE METRICS END -----");
        output.push('\n');
        push_line(&mut output, "----- RESPONSE HEADERS BEGIN -----");
        for (name, value) in safe_headers {
            push_line(&mut output, &format!("{name}: {value}"));
        }
        push_line(&mut output, "----- RESPONSE HEADERS END -----");
        output.push('\n');
        push_encoded_section(&mut output, "CURL STDERR", &safe_error);
        push_encoded_section(&mut output, "RESPONSE BODY", &safe_response);
        push_line(
            &mut output,
            &format!("========== REQUEST {} END ==========", request.request_id),
        );
        output.push('\n');
        output
    }

    fn ensure_open(&self) -> Result<(), AuditError> {
        if self.finished {
            Err(AuditError::AlreadyFinished)
        } else {
            Ok(())
        }
    }
}

fn push_encoded_section(output: &mut String, name: &str, value: &str) {
    push_line(output, &format!("----- {name} BEGIN -----"));
    push_line(output, &BASE64.encode(value.as_bytes()));
    push_line(output, &format!("----- {name} END -----"));
    output.push('\n');
}

fn push_line(output: &mut String, value: &str) {
    output.push_str(value);
    output.push('\n');
}

fn render_auth_header(output: &mut String, auth_mode: AuthMode) {
    match auth_mode {
        AuthMode::Bearer => push_line(
            output,
            "  --header \"Authorization: Bearer ${MODEL_API_KEY}\" \\",
        ),
        AuthMode::ApiKey => push_line(output, "  --header \"api-key: ${MODEL_API_KEY}\" \\"),
        AuthMode::XApiKey => {
            push_line(output, "  --header \"x-api-key: ${MODEL_API_KEY}\" \\");
            push_line(output, "  --header 'anthropic-version: 2023-06-01' \\");
        }
        AuthMode::XGoogApiKey => {
            push_line(output, "  --header \"x-goog-api-key: ${MODEL_API_KEY}\" \\")
        }
        AuthMode::None => {}
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn format_timestamp(value: DateTime<Local>) -> String {
    value.format("%Y-%m-%dT%H:%M:%S%z").to_string()
}

fn single_line(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}

#[cfg(unix)]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use chrono::Local;
    use tempfile::tempdir;

    use super::{AuditWriter, RequestEvidence, ResponseMetrics, RunMetadata};
    use crate::evidence::{
        StreamEndSignal, StreamTermination, ToolContractStatus, ToolLoopOutcome, TransportOutcome,
    };
    use crate::protocol::{AuthMode, Protocol};
    use crate::redaction::Redactor;

    #[test]
    fn run_header_declares_011_and_evidence_v3() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("audit.log");
        let url: url::Url = "https://example.com/v1/chat/completions"
            .parse()
            .expect("valid URL");
        let metadata = RunMetadata {
            run_id: "run-1".to_owned(),
            started_at: Local::now(),
            url: url.clone(),
            model: "test-model".to_owned(),
            masked_api_key: "[MASKED]".to_owned(),
            selected_test_count: 1,
            insecure: false,
        };
        let audit =
            AuditWriter::create(&path, metadata, Redactor::new("", &url)).expect("create audit");
        drop(audit);

        let output = fs::read_to_string(path).expect("read audit");
        assert!(output.contains("script_version: 0.11.0\n"));
        assert!(output.contains("log_schema: llm-capability-doctor.evidence.v3\n"));
    }

    #[test]
    fn request_block_emits_v3_metadata_before_encoded_sections() {
        let (audit, _) = test_audit("");
        let request = sample_request();

        let output = audit.render_request(&request);
        let start = output.find("transport_outcome:").expect("v3 metadata");
        let end = output
            .find("----- CURL COMMAND BEGIN -----")
            .expect("curl section");

        assert_eq!(
            &output[start..end],
            concat!(
                "transport_outcome: completed_eof\n",
                "stream_termination: completed\n",
                "stream_end_signal: [DONE]\n",
                "model_stop_reason: stop\n",
                "stream_event_count: 3\n",
                "tool_contract_status: conformant\n",
                "tool_contract_errors_json: []\n",
                "tool_loop_turn: 1\n",
                "tool_loop_outcome: completed\n",
                "\n",
            )
        );
    }

    #[test]
    fn tool_contract_errors_are_redacted_json_strings() {
        let (audit, _) = test_audit("secret-token");
        let mut request = sample_request();
        request.model_stop_reason = None;
        request.tool_contract_status = ToolContractStatus::NonConformant;
        request.tool_contract_errors =
            vec!["bad secret-token".to_owned(), "line\n\"quoted\"".to_owned()];

        let output = audit.render_request(&request);

        assert!(output.contains("model_stop_reason: none\n"));
        assert!(output.contains(
            "tool_contract_errors_json: [\"bad [REDACTED]\",\"line\\n\\\"quoted\\\"\"]\n"
        ));
        assert!(!output.contains("secret-token"));
    }

    #[test]
    fn dynamic_stream_metadata_is_redacted_and_single_line() {
        let (audit, _) = test_audit("secret-token");
        let mut request = sample_request();
        request.stream_end_signal = StreamEndSignal::GeminiFinishReason(
            "STOP-secret-token\r\nforged_signal: true".to_owned(),
        );
        request.model_stop_reason = Some("stop-secret-token\r\nforged_reason: true".to_owned());

        let output = audit.render_request(&request);

        assert!(output.contains(
            "stream_end_signal: finishReason:STOP-[REDACTED]\\r\\nforged_signal: true\n"
        ));
        assert!(output.contains("model_stop_reason: stop-[REDACTED]\\r\\nforged_reason: true\n"));
        assert!(!output.contains("secret-token"));
        assert!(!output.contains("\nforged_signal:"));
        assert!(!output.contains("\nforged_reason:"));
    }

    fn test_audit(api_key: &str) -> (AuditWriter, tempfile::TempDir) {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("audit.log");
        let url: url::Url = "https://example.com/v1/chat/completions"
            .parse()
            .expect("valid URL");
        let metadata = RunMetadata {
            run_id: "run-1".to_owned(),
            started_at: Local::now(),
            url: url.clone(),
            model: "test-model".to_owned(),
            masked_api_key: "[MASKED]".to_owned(),
            selected_test_count: 1,
            insecure: false,
        };
        let audit = AuditWriter::create(&path, metadata, Redactor::new(api_key, &url))
            .expect("create audit");
        (audit, directory)
    }

    fn sample_request() -> RequestEvidence {
        let now = Local::now();
        RequestEvidence {
            request_id: "request-1".to_owned(),
            started_at: now,
            completed_at: now,
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::Bearer,
            stream: true,
            url: "https://example.com/v1/chat/completions"
                .parse()
                .expect("valid URL"),
            timeout: Duration::from_secs(30),
            insecure: false,
            body: "{}".to_owned(),
            metrics: ResponseMetrics {
                transport_exit_code: 0,
                http_status: Some(200),
                time_total: Duration::from_millis(20),
                time_starttransfer: Duration::from_millis(10),
                size_download: 2,
            },
            headers: Vec::new(),
            error: String::new(),
            response_body: b"{}".to_vec(),
            transport_outcome: TransportOutcome::CompletedEof,
            stream_termination: StreamTermination::Completed,
            stream_end_signal: StreamEndSignal::OpenAiDone,
            model_stop_reason: Some("stop".to_owned()),
            stream_event_count: 3,
            tool_contract_status: ToolContractStatus::Conformant,
            tool_contract_errors: Vec::new(),
            tool_loop_turn: 1,
            tool_loop_outcome: ToolLoopOutcome::Completed,
        }
    }
}
