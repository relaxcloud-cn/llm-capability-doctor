pub mod analysis;
pub mod audit;
pub mod catalog;
pub mod checks;
pub mod cli;
pub mod context_capacity;
pub mod evidence;
pub mod http;
pub mod opencodex;
pub mod protocol;
pub mod redaction;
pub mod runner;
pub mod terminal;

pub(crate) mod private_file;

#[cfg(test)]
mod opencodex_contract_tests {
    use crate::opencodex::contract::{Adapter, CONTRACT, contract_digest};

    #[test]
    fn contract_is_pinned_to_opencodex_v2742_and_has_three_adapters() {
        assert_eq!(CONTRACT.version, "v2.7.42");
        assert_eq!(CONTRACT.commit, "34493b12666a5fd69d69d730b13eabb5ec9d7235");
        assert_eq!(
            CONTRACT.adapters,
            [Adapter::OpenAiChat, Adapter::Anthropic, Adapter::Google]
        );
        assert_eq!(contract_digest().len(), 64);
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod terminal_progress_tests {
    use std::path::PathBuf;

    use crate::terminal::{collection_line, display_category, display_path, display_title};

    #[test]
    fn terminal_labels_are_customer_readable_and_plain_text() {
        assert_eq!(display_category("上下文"), "上下文能力");
        assert_eq!(display_title("018", None), "实测模型可用上下文 Token 上限");
        assert_eq!(
            display_title("057", Some(32)),
            "测试模型在 32 并发下能否正常响应"
        );
        let line = collection_line(14, 42, "上下文能力", "实测模型可用上下文 Token 上限", None);
        assert_eq!(
            line,
            "[采集 14/42] 上下文能力 | 实测模型可用上下文 Token 上限"
        );
        assert!(!line.contains('\u{1b}'));
        for test in crate::catalog::CATALOG {
            assert_ne!(display_title(test.id, None), "未命名检测项");
        }
    }

    #[test]
    fn terminal_paths_prefer_relative_paths_without_hiding_external_paths() {
        let current = std::env::current_dir().unwrap();
        let child = current.join("model-doctor-output").join("model-doctor.log");
        let external = PathBuf::from("/private/tmp/model-doctor.log");

        assert_eq!(
            display_path(&child),
            "./model-doctor-output/model-doctor.log"
        );
        assert_eq!(display_path(&external), external.display().to_string());
    }
}
