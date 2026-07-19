use std::collections::BTreeSet;

use model_capability_doctor::catalog::{CATALOG, render, select};

#[test]
fn catalog_is_contiguous_and_complete() {
    assert_eq!(CATALOG.len(), 62);
    let expected_ids: Vec<String> = (1..=62).map(|id| format!("{id:03}")).collect();
    let actual_ids: Vec<&str> = CATALOG.iter().map(|test| test.id).collect();
    assert_eq!(actual_ids, expected_ids);
    assert_eq!(CATALOG[0].name, "URL 可达性");
    assert_eq!(CATALOG[61].name, "英文安全词可用性");

    let categories: BTreeSet<&str> = CATALOG.iter().map(|test| test.category).collect();
    for category in [
        "接口与协议",
        "结构化结果",
        "上下文",
        "指令与文本",
        "Thinking 与推理",
        "工具调用",
        "性能与稳定性",
        "护栏与词汇",
    ] {
        assert!(categories.contains(category), "missing {category}");
    }
}

#[test]
fn selection_preserves_requested_order_and_duplicates() {
    let selected = select(Some("062,001,062")).unwrap();
    let ids: Vec<&str> = selected.iter().map(|test| test.id).collect();
    assert_eq!(ids, ["062", "001", "062"]);
}

#[test]
fn selection_rejects_unknown_or_malformed_ids() {
    for value in ["", "1", "000", "063", "001,,002", " 001"] {
        assert!(select(Some(value)).is_err(), "accepted {value:?}");
    }
}

#[test]
fn rendered_catalog_is_tab_delimited_and_newline_terminated() {
    let rendered = render();
    assert_eq!(rendered.lines().count(), 62);
    assert!(rendered.starts_with("001\t接口与协议\tURL 可达性\n"));
    assert!(rendered.ends_with("062\t护栏与词汇\t英文安全词可用性\n"));
}
