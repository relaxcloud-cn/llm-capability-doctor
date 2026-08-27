use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::catalog::CATALOG;
use crate::redaction::Redactor;

const RUN_MARKER: &str = "========== MODEL DOCTOR RUN ==========";
const SUMMARY_MARKER: &str = "========== RUN SUMMARY ==========";
const END_MARKER: &str = "========== END ==========";
const REQUIRED_REQUEST_FIELDS: [&str; 9] = [
    "transport_outcome",
    "stream_termination",
    "stream_end_signal",
    "model_stop_reason",
    "stream_event_count",
    "tool_contract_status",
    "tool_contract_errors_json",
    "tool_loop_turn",
    "tool_loop_outcome",
];

#[derive(Clone, Debug)]
pub struct ParsedEvidence {
    pub source: EvidenceSource,
    pub run: BTreeMap<String, String>,
    pub requests: BTreeMap<String, ParsedRequest>,
    pub tests: BTreeMap<String, ParsedTest>,
    /// 解析失败但被跳过（而非整体报错）的请求块，含各自原因。
    pub degraded_requests: Vec<DegradedEntry>,
    /// 解析失败或引用了降级请求的检测块，含各自原因。
    pub degraded_tests: Vec<DegradedEntry>,
}

#[derive(Clone, Debug)]
pub struct DegradedEntry {
    pub id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSource {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRequest {
    pub request_id: String,
    pub metadata: BTreeMap<String, String>,
    pub metrics: BTreeMap<String, String>,
    pub curl_command: String,
    pub request_body: String,
    pub response_headers: String,
    pub stderr: String,
    pub response_body: String,
}

#[derive(Clone, Debug)]
pub struct ParsedTest {
    pub id: String,
    pub name: String,
    pub category: String,
    pub request_refs: Vec<String>,
}

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("evidence log is missing a complete run summary")]
    MissingRunSummary,
    #[error("invalid evidence contract: {0}")]
    InvalidContract(String),
    #[error("malformed evidence log: {0}")]
    Malformed(String),
    #[error("invalid base64 section {section}: {source}")]
    InvalidBase64 {
        section: String,
        source: base64::DecodeError,
    },
    #[error("evidence log is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn read(path: &Path, redactor: &Redactor) -> Result<ParsedEvidence, EvidenceError> {
    let raw = std::fs::read(path)?;
    let text = std::str::from_utf8(&raw)?;
    if !text.contains(SUMMARY_MARKER) || !text.ends_with(&format!("{END_MARKER}\n")) {
        return Err(EvidenceError::MissingRunSummary);
    }

    let mut run = parse_run_header(text)?;
    require(&run, "script_version", env!("CARGO_PKG_VERSION"))?;
    require(&run, "collector_runtime", "rust")?;
    require(&run, "section_encoding", "base64")?;
    require(&run, "log_schema", "llm-capability-doctor.evidence.v4")?;
    require(&run, "compatibility_profile", "llm-gateway-data-format")?;
    if run.contains_key("collection_profile") {
        return Err(EvidenceError::InvalidContract(
            "collection_profile is forbidden for evidence.v4".into(),
        ));
    }
    require_count(&run, "selected_test_count", CATALOG.len())?;
    for field in ["url", "model"] {
        if let Some(value) = run.get_mut(field) {
            *value = redactor.redact_text(value);
        }
    }

    let (requests, degraded_requests) = parse_requests(text, redactor)?;
    let (tests, degraded_tests) = parse_tests(text, &requests, &degraded_requests)?;
    let summary = parse_summary(text)?;
    require_count(
        &summary,
        "request_count",
        requests.len() + degraded_requests.len(),
    )?;
    require_count(
        &summary,
        "test_manifest_count",
        tests.len() + degraded_tests.len(),
    )?;

    let expected_ids: HashSet<_> = CATALOG.iter().map(|test| test.id).collect();
    let mut actual_ids: HashSet<_> = tests.keys().map(String::as_str).collect();
    actual_ids.extend(degraded_tests.iter().map(|entry| entry.id.as_str()));
    if actual_ids != expected_ids {
        return Err(EvidenceError::InvalidContract(format!(
            "evidence.v4 must contain the exact {}-check catalog",
            CATALOG.len()
        )));
    }

    let digest = Sha256::digest(&raw);
    let sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(ParsedEvidence {
        source: EvidenceSource {
            path: path.to_path_buf(),
            file_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("evidence.log")
                .to_owned(),
            size_bytes: raw.len(),
            sha256,
        },
        run,
        requests,
        tests,
        degraded_requests,
        degraded_tests,
    })
}

fn parse_run_header(text: &str) -> Result<BTreeMap<String, String>, EvidenceError> {
    let rest = text
        .strip_prefix(&format!("{RUN_MARKER}\n"))
        .ok_or_else(|| EvidenceError::InvalidContract("missing run header".into()))?;
    let end = rest
        .find("==========")
        .ok_or_else(|| EvidenceError::Malformed("run header has no following block".into()))?;
    Ok(key_values(&rest[..end]))
}

fn parse_summary(text: &str) -> Result<BTreeMap<String, String>, EvidenceError> {
    let start = text
        .find(SUMMARY_MARKER)
        .ok_or(EvidenceError::MissingRunSummary)?
        + SUMMARY_MARKER.len();
    let body = &text[start..];
    let end = body
        .find(END_MARKER)
        .ok_or(EvidenceError::MissingRunSummary)?;
    Ok(key_values(&body[..end]))
}

fn parse_requests(
    text: &str,
    redactor: &Redactor,
) -> Result<(BTreeMap<String, ParsedRequest>, Vec<DegradedEntry>), EvidenceError> {
    let mut requests = BTreeMap::new();
    let mut degraded = Vec::new();
    for block in blocks(text, BlockKind::Request)? {
        let (request_id, body) = match block {
            ParsedBlock::Intact(id, body) => (id, Some(body)),
            ParsedBlock::Degraded(id) => (id, None),
        };
        if requests.contains_key(&request_id)
            || degraded.iter().any(|e: &DegradedEntry| e.id == request_id)
        {
            return Err(EvidenceError::Malformed(format!(
                "duplicate request block {request_id}"
            )));
        }
        match body {
            Some(body) => match parse_request_block(&request_id, &body, redactor) {
                Ok(parsed) => {
                    requests.insert(request_id.clone(), parsed);
                }
                Err(reason) => degraded.push(DegradedEntry {
                    id: request_id,
                    reason,
                }),
            },
            None => degraded.push(DegradedEntry {
                id: request_id,
                reason: "请求块损坏：缺少 END 标记".to_owned(),
            }),
        }
    }
    if requests.is_empty() {
        if degraded.is_empty() {
            return Err(EvidenceError::InvalidContract(
                "evidence contains no request blocks".into(),
            ));
        }
        return Err(EvidenceError::InvalidContract(
            "evidence contains no intact request blocks".into(),
        ));
    }
    Ok((requests, degraded))
}

fn parse_request_block(
    request_id: &str,
    block: &str,
    redactor: &Redactor,
) -> Result<ParsedRequest, String> {
    let metadata_end = block
        .find("----- CURL COMMAND BEGIN -----")
        .ok_or_else(|| format!("请求块损坏：缺少 CURL COMMAND 段（request {request_id}）"))?;
    let metadata = key_values(&block[..metadata_end]);
    if metadata.get("request_id") != Some(&request_id.to_owned()) {
        return Err(format!(
            "请求块损坏：request_id 与块标识不一致（request {request_id}）"
        ));
    }
    for field in REQUIRED_REQUEST_FIELDS {
        if !metadata.contains_key(field) {
            return Err(format!(
                "请求块损坏：缺少字段 {field}（request {request_id}）"
            ));
        }
    }
    Ok(ParsedRequest {
        request_id: request_id.to_owned(),
        metadata: redact_map(metadata, redactor),
        metrics: key_values(section(block, "RESPONSE METRICS").map_err(degradation_reason)?),
        curl_command: redactor
            .redact_text(section(block, "CURL COMMAND").map_err(degradation_reason)?),
        request_body: decode_section(block, "REQUEST BODY", redactor)
            .map_err(|error| degradation_reason(error.to_string()))?,
        response_headers: redactor
            .redact_text(section(block, "RESPONSE HEADERS").map_err(degradation_reason)?),
        stderr: decode_section(block, "CURL STDERR", redactor)
            .map_err(|error| degradation_reason(error.to_string()))?,
        response_body: decode_section(block, "RESPONSE BODY", redactor)
            .map_err(|error| degradation_reason(error.to_string()))?,
    })
}

fn parse_tests(
    text: &str,
    requests: &BTreeMap<String, ParsedRequest>,
    degraded_requests: &[DegradedEntry],
) -> Result<(BTreeMap<String, ParsedTest>, Vec<DegradedEntry>), EvidenceError> {
    let degraded_request_ids: HashSet<&str> = degraded_requests
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    let mut tests = BTreeMap::new();
    let mut degraded = Vec::new();
    for block in blocks(text, BlockKind::Test)? {
        let (test_id, body) = match block {
            ParsedBlock::Intact(id, body) => (id, Some(body)),
            ParsedBlock::Degraded(id) => (id, None),
        };
        if tests.contains_key(&test_id) || degraded.iter().any(|e: &DegradedEntry| e.id == test_id)
        {
            return Err(EvidenceError::Malformed(format!(
                "duplicate test block {test_id}"
            )));
        }
        let Some(body) = body else {
            degraded.push(DegradedEntry {
                id: test_id,
                reason: "检测块损坏：缺少 END 标记".to_owned(),
            });
            continue;
        };
        let values = key_values(&body);
        if ["result", "expected", "detected", "conclusion", "status"]
            .iter()
            .any(|field| values.contains_key(*field))
        {
            return Err(EvidenceError::InvalidContract(format!(
                "test {test_id} contains a forbidden judgment field"
            )));
        }
        match parse_test_block(&test_id, &values, requests, &degraded_request_ids) {
            Ok(parsed) => {
                tests.insert(test_id.clone(), parsed);
            }
            Err(reason) => degraded.push(DegradedEntry {
                id: test_id,
                reason,
            }),
        }
    }
    Ok((tests, degraded))
}

fn parse_test_block(
    test_id: &str,
    values: &BTreeMap<String, String>,
    requests: &BTreeMap<String, ParsedRequest>,
    degraded_request_ids: &HashSet<&str>,
) -> Result<ParsedTest, String> {
    let name = non_empty(values, "name", test_id).map_err(degradation_reason)?;
    let category = non_empty(values, "category", test_id).map_err(degradation_reason)?;
    let raw_refs = values
        .get("request_refs")
        .ok_or_else(|| format!("检测块损坏：缺少 request_refs（test {test_id}）"))?;
    let request_refs: Vec<String> = if raw_refs.is_empty() {
        Vec::new()
    } else {
        raw_refs
            .split(',')
            .map(str::trim)
            .map(str::to_owned)
            .collect()
    };
    let unique: HashSet<_> = request_refs.iter().collect();
    if request_refs.iter().any(String::is_empty) || unique.len() != request_refs.len() {
        return Err(format!(
            "检测块损坏：request_refs 格式无效（test {test_id}）"
        ));
    }
    if let Some(reference) = request_refs
        .iter()
        .find(|reference| !requests.contains_key(*reference))
    {
        return Err(if degraded_request_ids.contains(reference.as_str()) {
            format!("引用的请求 {reference} 证据损坏（test {test_id}）")
        } else {
            format!("检测块损坏：引用的请求 {reference} 不存在（test {test_id}）")
        });
    }
    Ok(ParsedTest {
        id: test_id.to_owned(),
        name,
        category,
        request_refs,
    })
}

/// 将读取端错误转为面向报告的降级原因文案。
fn degradation_reason(error: impl std::fmt::Display) -> String {
    format!("证据损坏：{error}")
}

#[derive(Clone, Copy)]
enum BlockKind {
    Request,
    Test,
}

enum ParsedBlock {
    Intact(String, String),
    /// 块标识可读但缺少 END 标记（例如文件中段损坏），跳过并降级。
    Degraded(String),
}

fn blocks(text: &str, kind: BlockKind) -> Result<Vec<ParsedBlock>, EvidenceError> {
    let (prefix, suffix, end_prefix) = match kind {
        BlockKind::Request => (
            "========== REQUEST ",
            " BEGIN ==========",
            "========== REQUEST ",
        ),
        BlockKind::Test => ("========== TEST-", " BEGIN ==========", "========== TEST-"),
    };
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find(prefix) {
        let start = cursor + relative_start;
        if start != 0 && text.as_bytes().get(start.wrapping_sub(1)) != Some(&b'\n') {
            cursor = start + prefix.len();
            continue;
        }
        let identifier_start = start + prefix.len();
        let identifier_end = text[identifier_start..]
            .find(suffix)
            .map(|offset| identifier_start + offset)
            .ok_or_else(|| EvidenceError::Malformed("unterminated block header".into()))?;
        let identifier = text[identifier_start..identifier_end].to_owned();
        let body_start = identifier_end + suffix.len();
        let body_start = body_start + usize::from(text.as_bytes().get(body_start) == Some(&b'\n'));
        let end_marker = format!("{end_prefix}{identifier} END ==========");
        let Some(relative_end) = text[body_start..].find(&end_marker) else {
            // 单块缺 END 不再让整个读取失败：记为降级，从块体起点继续扫描后续块。
            output.push(ParsedBlock::Degraded(identifier));
            cursor = body_start;
            continue;
        };
        let body_end = body_start + relative_end;
        output.push(ParsedBlock::Intact(
            identifier,
            text[body_start..body_end].trim_end_matches('\n').to_owned(),
        ));
        cursor = body_end + end_marker.len();
    }
    Ok(output)
}

fn section<'a>(block: &'a str, name: &str) -> Result<&'a str, EvidenceError> {
    let start_marker = format!("----- {name} BEGIN -----\n");
    let end_marker = format!("\n----- {name} END -----");
    let bare_end_marker = format!("----- {name} END -----");
    let start = block
        .find(&start_marker)
        .map(|index| index + start_marker.len())
        .ok_or_else(|| EvidenceError::Malformed(format!("missing section {name}")))?;
    let remainder = &block[start..];
    // 空内容区块（例如请求在收到响应头之前失败）紧跟 END 标记，没有前导换行。
    let end = remainder
        .find(&end_marker)
        .map(|offset| start + offset)
        .or_else(|| remainder.starts_with(&bare_end_marker).then_some(start))
        .ok_or_else(|| EvidenceError::Malformed(format!("unterminated section {name}")))?;
    Ok(&block[start..end])
}

fn decode_section(block: &str, name: &str, redactor: &Redactor) -> Result<String, EvidenceError> {
    let encoded = section(block, name)?;
    let bytes = BASE64
        .decode(encoded)
        .map_err(|source| EvidenceError::InvalidBase64 {
            section: name.to_owned(),
            source,
        })?;
    let decoded = std::str::from_utf8(&bytes)?;
    Ok(redactor.redact_text(decoded))
}

fn key_values(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(": ")?;
            key.chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
                .then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

fn redact_map(values: BTreeMap<String, String>, redactor: &Redactor) -> BTreeMap<String, String> {
    values
        .into_iter()
        .map(|(key, value)| (key, redactor.redact_text(&value)))
        .collect()
}

fn require(
    values: &BTreeMap<String, String>,
    field: &str,
    expected: &str,
) -> Result<(), EvidenceError> {
    if values.get(field).map(String::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(EvidenceError::InvalidContract(format!(
            "{field} must equal {expected}"
        )))
    }
}

fn require_count(
    values: &BTreeMap<String, String>,
    field: &str,
    expected: usize,
) -> Result<(), EvidenceError> {
    let actual = values
        .get(field)
        .and_then(|value| value.parse::<usize>().ok());
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(EvidenceError::InvalidContract(format!(
            "{field} must equal {expected}"
        )))
    }
}

fn non_empty(
    values: &BTreeMap<String, String>,
    field: &str,
    test_id: &str,
) -> Result<String, EvidenceError> {
    values
        .get(field)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| EvidenceError::Malformed(format!("test {test_id} has no {field}")))
}

#[cfg(test)]
pub(crate) use tests::{complete_test_log, two_request_log};

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use url::Url;

    use super::*;
    use crate::catalog::CATALOG;
    use crate::redaction::Redactor;

    #[test]
    fn reads_complete_v4_and_preserves_ordered_refs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, complete_test_log()).unwrap();
        let redactor = Redactor::new(
            "secret-token",
            &Url::parse("https://example.test/v1/chat/completions").unwrap(),
        );

        let parsed = read(&path, &redactor).unwrap();

        assert_eq!(parsed.tests.len(), CATALOG.len());
        assert_eq!(parsed.requests.len(), 1);
        assert_eq!(parsed.tests["001"].request_refs, vec!["test-shared"]);
        assert_eq!(
            parsed.requests["test-shared"].curl_command,
            "curl 'https://example.test/v1/chat/completions'"
        );
        assert_eq!(parsed.source.sha256.len(), 64);
    }

    #[test]
    fn empty_sections_from_pre_response_failures_still_parse() {
        // 请求在收到响应头之前失败时，RESPONSE HEADERS 区块为空（BEGIN 后直接 END）。
        let log = complete_test_log().replace(
            "----- RESPONSE HEADERS BEGIN -----\ncontent-type: application/json\n----- RESPONSE HEADERS END -----",
            "----- RESPONSE HEADERS BEGIN -----\n----- RESPONSE HEADERS END -----",
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, log).unwrap();
        let redactor = Redactor::new("secret-token", &Url::parse("https://example.test").unwrap());

        let parsed = read(&path, &redactor).unwrap();

        assert_eq!(parsed.requests["test-shared"].response_headers, "");
    }

    #[test]
    fn rejects_incomplete_run_before_analysis() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(
            &path,
            complete_test_log().replace("========== END ==========\n", ""),
        )
        .unwrap();
        let redactor = Redactor::new("secret-token", &Url::parse("https://example.test").unwrap());

        assert!(matches!(
            read(&path, &redactor),
            Err(EvidenceError::MissingRunSummary)
        ));
    }

    #[test]
    fn redacts_known_credentials_from_decoded_sections() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(
            &path,
            complete_test_log().replace("cmVzcG9uc2U=", "c2VjcmV0LXRva2Vu"),
        )
        .unwrap();
        let redactor = Redactor::new("secret-token", &Url::parse("https://example.test").unwrap());

        let parsed = read(&path, &redactor).unwrap();

        assert_eq!(parsed.requests["test-shared"].response_body, "[REDACTED]");
    }

    #[test]
    fn redacts_known_credentials_from_curl_command() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(
            &path,
            complete_test_log().replace(
                "curl 'https://example.test/v1/chat/completions'",
                "curl -H 'Authorization: Bearer secret-token' 'https://example.test/v1/chat/completions?api_key=query-secret'",
            ),
        )
        .unwrap();
        let redactor = Redactor::new(
            "secret-token",
            &Url::parse("https://example.test/v1/chat/completions?api_key=query-secret").unwrap(),
        );

        let curl_command = &read(&path, &redactor).unwrap().requests["test-shared"].curl_command;

        assert!(curl_command.contains("[REDACTED]"));
        assert!(!curl_command.contains("secret-token"));
        assert!(!curl_command.contains("query-secret"));
    }

    /// 在 complete_test_log 基础上加入第二个完好请求 test-shared-2，
    /// 并把 001 的引用改指向它；其余检测仍引用 test-shared。
    pub(crate) fn two_request_log() -> String {
        let mut log = complete_test_log();
        let block_start = log
            .find("========== REQUEST test-shared BEGIN ==========")
            .unwrap();
        let end_marker = "========== REQUEST test-shared END ==========";
        let block_end = log.find(end_marker).unwrap() + end_marker.len() + 1;
        let second_block = log[block_start..block_end].replace("test-shared", "test-shared-2");
        log.insert_str(block_end, &second_block);
        log = log.replace("request_count: 1", "request_count: 2");
        let test_start = log.find("========== TEST-001 BEGIN ==========").unwrap();
        let test_end = log.find("========== TEST-001 END ==========").unwrap();
        let repointed = log[test_start..test_end].replace(
            "request_refs: test-shared\n",
            "request_refs: test-shared-2\n",
        );
        log.replace_range(test_start..test_end, &repointed);
        log
    }

    fn redactor_for_test() -> Redactor {
        Redactor::new("secret-token", &Url::parse("https://example.test").unwrap())
    }

    #[test]
    fn corrupt_request_degrades_instead_of_failing_whole_log() {
        let mut log = two_request_log();
        let target = "----- RESPONSE BODY BEGIN -----\ncmVzcG9uc2U=";
        let index = log.find(target).unwrap();
        log.replace_range(
            index..index + target.len(),
            "----- RESPONSE BODY BEGIN -----\n!!!not-base64!!!",
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, log).unwrap();

        let parsed = read(&path, &redactor_for_test()).unwrap();

        assert_eq!(parsed.requests.len(), 1);
        assert!(parsed.requests.contains_key("test-shared-2"));
        assert_eq!(parsed.degraded_requests.len(), 1);
        assert_eq!(parsed.degraded_requests[0].id, "test-shared");
        assert!(parsed.degraded_requests[0].reason.contains("证据损坏"));
        assert_eq!(parsed.tests.len(), 1);
        assert_eq!(parsed.degraded_tests.len(), CATALOG.len() - 1);
        assert!(
            parsed
                .degraded_tests
                .iter()
                .all(|entry| entry.reason.contains("引用的请求 test-shared 证据损坏"))
        );
    }

    #[test]
    fn request_block_without_end_marker_degrades_and_scanning_continues() {
        let log =
            two_request_log().replace("\n========== REQUEST test-shared END ==========\n", "\n");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, log).unwrap();

        let parsed = read(&path, &redactor_for_test()).unwrap();

        assert_eq!(parsed.degraded_requests.len(), 1);
        assert_eq!(
            parsed.degraded_requests[0].reason,
            "请求块损坏：缺少 END 标记"
        );
        assert!(parsed.requests.contains_key("test-shared-2"));
        assert_eq!(parsed.tests.len(), 1);
        assert_eq!(parsed.degraded_tests.len(), CATALOG.len() - 1);
    }

    #[test]
    fn corrupt_test_block_degrades_instead_of_failing_whole_log() {
        let mut log = complete_test_log();
        let marker = "========== TEST-042 BEGIN ==========\n";
        let start = log.find(marker).unwrap() + marker.len();
        let name_line_end = start + log[start..].find('\n').unwrap() + 1;
        log.replace_range(start..name_line_end, "");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, log).unwrap();

        let parsed = read(&path, &redactor_for_test()).unwrap();

        assert_eq!(parsed.tests.len(), CATALOG.len() - 1);
        assert_eq!(parsed.degraded_tests.len(), 1);
        assert_eq!(parsed.degraded_tests[0].id, "042");
        assert!(parsed.degraded_tests[0].reason.contains("name"));
        assert!(parsed.degraded_requests.is_empty());
    }

    #[test]
    fn all_requests_corrupt_still_fails_loudly() {
        let log = complete_test_log().replace("cmVzcG9uc2U=", "!!!not-base64!!!");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("doctor.log");
        std::fs::write(&path, log).unwrap();

        match read(&path, &redactor_for_test()) {
            Err(EvidenceError::InvalidContract(message)) => {
                assert!(message.contains("no intact request blocks"));
            }
            other => panic!("expected fatal contract error, got {other:?}"),
        }
    }

    pub(crate) fn complete_test_log() -> String {
        let mut log = String::from(
            "========== MODEL DOCTOR RUN ==========\n\
             run_id: run-1\n\
             script_version: 0.13.0\n\
             collector_runtime: rust\n\
             section_encoding: base64\n\
             log_schema: llm-capability-doctor.evidence.v4\n\
             compatibility_profile: llm-gateway-data-format\n\
             started_at: 2026-08-21T10:00:00+0800\n\
             url: https://example.test/v1/chat/completions\n\
             model: test-model\n\
             api_key: [MASKED]\n\
             curl_version: not_used (native rust reqwest)\n\
             tls_verification: enabled\n\
             selected_test_count: 42\n\
             \n\
             ========== REQUEST test-shared BEGIN ==========\n\
             request_id: test-shared\n\
             started_at: 2026-08-21T10:00:01+0800\n\
             completed_at: 2026-08-21T10:00:02+0800\n\
             protocol: openai_chat\n\
             auth_mode: bearer\n\
             stream: 0\n\
             transport_outcome: completed_eof\n\
             stream_termination: not_applicable\n\
             stream_end_signal: none\n\
             model_stop_reason: stop\n\
             stream_event_count: 0\n\
             tool_contract_status: not_applicable\n\
             tool_contract_errors_json: []\n\
             tool_loop_turn: 0\n\
             tool_loop_outcome: not_applicable\n\
             \n\
             ----- CURL COMMAND BEGIN -----\n\
             curl 'https://example.test/v1/chat/completions'\n\
             ----- CURL COMMAND END -----\n\
             \n\
             ----- REQUEST BODY BEGIN -----\n\
             e30=\n\
             ----- REQUEST BODY END -----\n\
             \n\
             ----- RESPONSE METRICS BEGIN -----\n\
             curl_exit_code: 0\n\
             http_status: 200\n\
             time_total: 1.000000\n\
             time_starttransfer: 0.500000\n\
             size_download: 8\n\
             ----- RESPONSE METRICS END -----\n\
             \n\
             ----- RESPONSE HEADERS BEGIN -----\n\
             content-type: application/json\n\
             ----- RESPONSE HEADERS END -----\n\
             \n\
             ----- CURL STDERR BEGIN -----\n\
             \n\
             ----- CURL STDERR END -----\n\
             \n\
             ----- RESPONSE BODY BEGIN -----\n\
             cmVzcG9uc2U=\n\
             ----- RESPONSE BODY END -----\n\
             \n\
             ========== REQUEST test-shared END ==========\n\
             \n",
        );
        for test in CATALOG {
            writeln!(log, "========== TEST-{} BEGIN ==========", test.id).unwrap();
            writeln!(log, "name: {}", test.name).unwrap();
            writeln!(log, "category: {}", test.category).unwrap();
            writeln!(log, "completed_at: 2026-08-21T10:00:02+0800").unwrap();
            writeln!(log, "request_refs: test-shared").unwrap();
            writeln!(log, "========== TEST-{} END ==========\n", test.id).unwrap();
        }
        log.push_str(
            "========== RUN SUMMARY ==========\n\
             completed_at: 2026-08-21T10:01:00+0800\n\
             duration_seconds: 60\n\
             request_count: 1\n\
             test_manifest_count: 42\n\
             ========== END ==========\n",
        );
        log
    }
}
