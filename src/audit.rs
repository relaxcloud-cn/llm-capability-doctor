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

use crate::cli::CollectionProfile;
use crate::protocol::{AuthMode, Protocol};
use crate::redaction::Redactor;

pub struct RunMetadata {
    pub run_id: String,
    pub started_at: DateTime<Local>,
    pub url: Url,
    pub model: String,
    pub masked_api_key: String,
    pub selected_test_count: usize,
    pub collection_profile: CollectionProfile,
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
        writeln!(self.writer, "script_version: 0.9.0")?;
        writeln!(self.writer, "collector_runtime: rust")?;
        writeln!(
            self.writer,
            "collection_profile: {}",
            metadata.collection_profile
        )?;
        writeln!(self.writer, "section_encoding: base64")?;
        writeln!(self.writer, "log_schema: llm-capability-doctor.evidence.v1")?;
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
