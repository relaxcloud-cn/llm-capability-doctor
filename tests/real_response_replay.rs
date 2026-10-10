use agent_check::{
    config::Protocol,
    dataset::{RecordFile, SourceSnapshot, compare_records, parse_record_payload},
    transport::parse_json,
};
use serde_json::Value;

#[test]
fn real_tool_answer_preserves_correct_values_inside_markdown() {
    let evidence: Value =
        serde_json::from_str(include_str!("fixtures/tool-answer-fenced.json")).unwrap();
    let body = evidence["attempts"].as_array().unwrap().last().unwrap()["response_body"]
        .as_str()
        .unwrap();
    let response = parse_json(Protocol::ChatCompletions, body.as_bytes()).unwrap();
    let source = SourceSnapshot::generate("agentcheck/1").records();
    let expected = RecordFile {
        records: vec![source[0].clone(), source[6].clone(), source[11].clone()],
    };
    assert!(response.normal_end);
    assert!(
        compare_records(
            &expected,
            &parse_record_payload(&response.text, true).unwrap()
        )
        .passed
    );
}
#[test]
fn real_native_schema_response_is_invalid_despite_a_normal_stop() {
    let evidence: Value =
        serde_json::from_str(include_str!("fixtures/native-schema-invalid.json")).unwrap();
    assert_eq!(
        evidence["request"]["response_format"]["type"],
        "json_schema"
    );
    let body = evidence["attempts"].as_array().unwrap().last().unwrap()["response_body"]
        .as_str()
        .unwrap();
    let response = parse_json(Protocol::ChatCompletions, body.as_bytes()).unwrap();
    assert!(response.normal_end);
    assert!(parse_record_payload(&response.text, false).is_err());
}
