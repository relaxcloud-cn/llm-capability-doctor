//! Generated source data and a single record comparator. No task-specific answer tables.
use crate::model::{CheckId, ToolEvent};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub id: String,
    pub owner: String,
    #[serde(deserialize_with = "whole_version")]
    pub version: u64,
}
fn whole_version<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<u64, D::Error> {
    let value = Value::deserialize(deserializer)?;
    if let Some(number) = value.as_u64() {
        return Ok(number);
    }
    if let Some(number) = value.as_f64() {
        if number.is_finite()
            && number.fract() == 0.0
            && number >= 0.0
            && number < (u64::MAX as f64)
        {
            return Ok(number as u64);
        }
    }
    Err(serde::de::Error::custom(
        "version must be a nonnegative integer",
    ))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecordFile {
    pub records: Vec<Record>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub seed: String,
    pub files: BTreeMap<String, RecordFile>,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeSet {
    pub added: Vec<Record>,
    pub updated: BTreeMap<String, Record>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordDifference {
    pub id: String,
    pub field: String,
    pub expected: Value,
    pub actual: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Validation {
    pub passed: bool,
    pub differences: Vec<RecordDifference>,
    pub process_errors: Vec<String>,
}

impl SourceSnapshot {
    pub fn generate(seed: &str) -> Self {
        let digest = Sha256::digest(seed.as_bytes());
        let owners = [
            "林言", "周宁", "许舟", "陈晨", "张禾", "沈安", "李川", "陆雨",
        ];
        let mut files = BTreeMap::new();
        for file in 0..3 {
            let records = (0..4)
                .map(|row| {
                    let i = file * 4 + row;
                    Record {
                        id: format!("R-{:02}", i + 1),
                        owner: owners[(digest[i] as usize) % owners.len()].into(),
                        version: (digest[i + 12] % 4 + 1) as u64,
                    }
                })
                .collect();
            files.insert(format!("source-{}.json", file + 1), RecordFile { records });
        }
        let bytes = serde_json::to_vec(&files).expect("serializable generated records");
        Self {
            seed: seed.into(),
            files,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }
    pub fn records(&self) -> Vec<Record> {
        self.files
            .values()
            .flat_map(|file| file.records.clone())
            .collect()
    }
    pub fn changes(&self) -> ChangeSet {
        let records = self.records();
        let mut update = records[6].clone();
        update.version += 1;
        ChangeSet {
            added: vec![Record {
                id: "R-13".into(),
                owner: "陈晨".into(),
                version: 1,
            }],
            updated: BTreeMap::from([(update.id.clone(), update)]),
        }
    }
    pub fn expected(&self, changes: Option<&ChangeSet>) -> Result<RecordFile> {
        let mut records: BTreeMap<_, _> = self
            .records()
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        if let Some(changes) = changes {
            for (id, value) in &changes.updated {
                if id != &value.id || !records.contains_key(id) {
                    bail!("无效的源数据变更");
                }
                records.insert(id.clone(), value.clone());
            }
            for record in &changes.added {
                if records.insert(record.id.clone(), record.clone()).is_some() {
                    bail!("新增记录重复");
                }
            }
        }
        Ok(RecordFile {
            records: records.into_values().collect(),
        })
    }
    pub fn write_inputs(&self, workdir: &Path) -> Result<()> {
        std::fs::create_dir_all(workdir.join("inputs"))?;
        for (name, file) in &self.files {
            std::fs::write(
                workdir.join("inputs").join(name),
                serde_json::to_vec_pretty(file)?,
            )?;
        }
        Ok(())
    }
    pub fn prompt(&self) -> String {
        "请读取 inputs 目录中的全部 3 份 JSON 文件，将所有 records 汇总到当前目录的 result.json。每条记录保留 id、owner、version，输出格式为 {\"records\":[...]}。请用文件工具实际读取和写入文件，保持来源值，不新增字段。".into()
    }
    pub fn continuation_prompt(&self) -> String {
        let changes = self.changes();
        let update = changes.updated.values().next().unwrap();
        let added = &changes.added[0];
        format!(
            "请在刚才的 result.json 中新增记录 {}，负责人为 {}，版本为 {}；将 {} 的版本改为 {}。保留其他全部记录及字段值，继续使用同一个结果文件。",
            added.id, added.owner, added.version, update.id, update.version
        )
    }
}
/// Presentation wrappers are allowed for a conversational tool answer, never for a
/// native JSON Schema result or an on-disk JSON artifact.
pub fn parse_record_payload(text: &str, allow_markdown: bool) -> Result<Value> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    if allow_markdown && text.matches("```").count() == 2 {
        let open = text.find("```").unwrap();
        let remainder = &text[open + 3..];
        let newline = remainder.find('\n').context("代码块没有完整正文")?;
        let language = remainder[..newline].trim().to_ascii_lowercase();
        if language.is_empty() || language == "json" {
            let content = &remainder[newline + 1..];
            let close = content.find("```").unwrap();
            return serde_json::from_str(content[..close].trim()).context("代码块中不是有效 JSON");
        }
    }
    bail!("没有可独立校验的 JSON 记录内容")
}
pub fn compare_records(expected: &RecordFile, actual: &Value) -> Validation {
    let mut differences = vec![];
    let mut process_errors = vec![];
    let actual = match serde_json::from_value::<RecordFile>(actual.clone()) {
        Ok(value) => value,
        Err(_) => {
            return Validation {
                passed: false,
                differences,
                process_errors: vec!["结果格式、字段或类型不符合要求".into()],
            };
        }
    };
    let mut seen = BTreeSet::new();
    let mut actual_map = BTreeMap::new();
    for record in actual.records {
        if !seen.insert(record.id.clone()) {
            process_errors.push(format!("记录 ID 重复：{}", record.id));
        }
        actual_map.insert(record.id.clone(), record);
    }
    let expected_map: BTreeMap<_, _> = expected.records.iter().map(|r| (&r.id, r)).collect();
    for (id, expected) in &expected_map {
        match actual_map.get(*id) {
            None => differences.push(RecordDifference {
                id: (*id).clone(),
                field: "record".into(),
                expected: json!(expected),
                actual: Value::Null,
            }),
            Some(actual) => {
                if actual.owner != expected.owner {
                    differences.push(RecordDifference {
                        id: (*id).clone(),
                        field: "owner".into(),
                        expected: json!(expected.owner),
                        actual: json!(actual.owner),
                    });
                }
                if actual.version != expected.version {
                    differences.push(RecordDifference {
                        id: (*id).clone(),
                        field: "version".into(),
                        expected: json!(expected.version),
                        actual: json!(actual.version),
                    });
                }
            }
        }
    }
    for (id, actual) in &actual_map {
        if !expected_map.contains_key(id) {
            differences.push(RecordDifference {
                id: id.clone(),
                field: "record".into(),
                expected: Value::Null,
                actual: json!(actual),
            });
        }
    }
    Validation {
        passed: differences.is_empty() && process_errors.is_empty(),
        differences,
        process_errors,
    }
}
/// Retain the independently held snapshot; never reconstruct expected values from mutable input files.
pub fn validate_task(
    snapshot: &SourceSnapshot,
    changes: Option<&ChangeSet>,
    output: &Value,
    kind: CheckId,
    events: &[ToolEvent],
    normal_end: bool,
    fault_delivered: bool,
) -> Result<Validation> {
    let mut validation = compare_records(&snapshot.expected(changes)?, output);
    let successful_read = events.iter().any(|e| e.name == "read" && !e.is_error);
    let successful_write = events
        .iter()
        .any(|e| matches!(e.name.as_str(), "write" | "edit") && !e.is_error);
    if !normal_end {
        validation.process_errors.push("任务未正常结束".into());
    }
    if !successful_read {
        validation
            .process_errors
            .push("没有实际读取来源文件的证据".into());
    }
    if !successful_write {
        validation
            .process_errors
            .push("没有实际写入结果文件的证据".into());
    }
    if kind == CheckId::Recovery {
        let failed = events.iter().position(|e| e.name == "read" && e.is_error);
        let recovered = failed.is_some_and(|i| {
            events.iter().skip(i + 1).any(|e| {
                ["read", "glob", "grep", "write", "edit"].contains(&e.name.as_str()) && !e.is_error
            })
        });
        if !fault_delivered {
            validation
                .process_errors
                .push("未确认模拟读取故障已经送达模型".into());
        }
        if !recovered {
            validation
                .process_errors
                .push("没有故障之后继续执行成功的工具证据".into());
        }
    }
    validation.passed = validation.differences.is_empty() && validation.process_errors.is_empty();
    Ok(validation)
}
pub fn read_result(workdir: &Path) -> Result<Value> {
    let path = workdir.join("result.json");
    let meta = std::fs::symlink_metadata(&path).context("任务没有生成 result.json")?;
    if !meta.file_type().is_file() || meta.len() > 1_048_576 {
        bail!("结果必须是当前任务目录中的普通 JSON 文件");
    }
    serde_json::from_slice(&std::fs::read(path)?).context("result.json 不是有效 JSON")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(name: &str, error: bool) -> ToolEvent {
        ToolEvent {
            call_id: format!("{name}-{error}"),
            name: name.into(),
            arguments: json!({}),
            result: json!({}),
            is_error: error,
        }
    }
    #[test]
    fn integer_numeric_values_accept_equivalent_json_serialization_not_strings_or_fractions() {
        let value = serde_json::from_str::<RecordFile>(
            r#"{"records":[{"id":"x","owner":"a","version":2.0}]}"#,
        )
        .unwrap();
        assert_eq!(value.records[0].version, 2);
        for version in [json!("2"), json!(true), json!(2.5), json!(-1)] {
            assert!(
                serde_json::from_value::<RecordFile>(
                    json!({"records":[{"id":"x","owner":"a","version":version}]})
                )
                .is_err()
            );
        }
    }
    #[test]
    fn conversational_fences_are_accepted_but_native_json_and_broken_json_are_strict() {
        let source = SourceSnapshot::generate("agentcheck/1")
            .expected(None)
            .unwrap();
        let json = serde_json::to_string(&source).unwrap();
        let fenced = format!("```json\n{json}\n```");
        assert!(compare_records(&source, &parse_record_payload(&fenced, true).unwrap()).passed);
        assert!(parse_record_payload(&fenced, false).is_err());
        assert!(
            compare_records(
                &source,
                &parse_record_payload(&format!("查询结果如下：\n{fenced}\n以上为原始记录。"), true)
                    .unwrap()
            )
            .passed
        );
        assert!(parse_record_payload("{\"records\":[{\"id{\"records\":[]}", false).is_err());
        assert!(parse_record_payload("```json\n{}\n```\n```json\n{}\n```", true).is_err());
    }
    #[test]
    fn comparator_allows_record_reordering_but_detects_content_and_duplicates() {
        let s = SourceSnapshot::generate("fixed-data");
        let expected = s.expected(None).unwrap();
        let mut reversed = expected.clone();
        reversed.records.reverse();
        assert!(compare_records(&expected, &json!(reversed)).passed);
        reversed.records[0].version += 1;
        let mismatch = compare_records(&expected, &json!(reversed));
        assert!(!mismatch.passed);
        assert_eq!(mismatch.differences[0].field, "version");
        let mut duplicated = expected.clone();
        duplicated.records.push(duplicated.records[0].clone());
        assert!(!compare_records(&expected, &json!(duplicated)).passed);
    }
    #[test]
    fn task_claims_files_and_abnormal_end_cannot_substitute_for_process_evidence() {
        let s = SourceSnapshot::generate("same-source");
        let expected = json!(s.expected(None).unwrap());
        assert!(
            !validate_task(&s, None, &expected, CheckId::Continuous, &[], true, false)
                .unwrap()
                .passed
        );
        let events = [event("read", false), event("write", false)];
        assert!(
            !validate_task(
                &s,
                None,
                &expected,
                CheckId::Continuous,
                &events,
                false,
                false
            )
            .unwrap()
            .passed
        );
        assert!(
            validate_task(
                &s,
                None,
                &expected,
                CheckId::Continuous,
                &events,
                true,
                false
            )
            .unwrap()
            .passed
        );
    }
    #[test]
    fn injected_error_delivery_and_recovery_are_required() {
        let s = SourceSnapshot::generate("same-source");
        let expected = json!(s.expected(None).unwrap());
        let events = [
            event("read", true),
            event("read", false),
            event("write", false),
        ];
        assert!(
            !validate_task(&s, None, &expected, CheckId::Recovery, &events, true, false)
                .unwrap()
                .passed
        );
        assert!(
            validate_task(&s, None, &expected, CheckId::Recovery, &events, true, true)
                .unwrap()
                .passed
        );
        let events = [
            event("read", false),
            event("write", false),
            event("read", true),
        ];
        assert!(
            !validate_task(&s, None, &expected, CheckId::Recovery, &events, true, true)
                .unwrap()
                .passed
        );
    }
    #[test]
    fn continuation_requires_addition_update_and_preservation() {
        let s = SourceSnapshot::generate("continuation");
        let changes = s.changes();
        let expected = s.expected(Some(&changes)).unwrap();
        assert_eq!(expected.records.len(), 13);
        assert!(compare_records(&expected, &json!(expected)).passed);
        let unchanged = s.expected(None).unwrap();
        let result = compare_records(&expected, &json!(unchanged));
        assert_eq!(result.differences.len(), 2);
        let mut bad = expected.clone();
        bad.records[0].owner = "changed unexpectedly".into();
        assert!(!compare_records(&expected, &json!(bad)).passed);
        let mut wrong_type = json!(expected);
        wrong_type["records"][0]["version"] = json!("1");
        assert!(!compare_records(&expected, &wrong_type).passed);
    }
}
