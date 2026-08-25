use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Adapter {
    OpenAiChat,
    Anthropic,
    Google,
}

impl Adapter {
    pub const fn id(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai-chat",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
        }
    }
}

pub struct Contract {
    pub version: &'static str,
    pub commit: &'static str,
    pub adapters: [Adapter; 3],
}

pub struct SourceFile {
    pub path: &'static str,
    pub sha256: &'static str,
}

pub struct Rule {
    pub id: &'static str,
    pub adapter: Adapter,
    pub requirement: &'static str,
    pub source_file: &'static str,
    pub source_test: &'static str,
    pub effect: &'static str,
}

pub static CONTRACT: Contract = Contract {
    version: "v2.7.42",
    commit: "34493b12666a5fd69d69d730b13eabb5ec9d7235",
    adapters: [Adapter::OpenAiChat, Adapter::Anthropic, Adapter::Google],
};

pub static SOURCE_FILES: [SourceFile; 4] = [
    SourceFile {
        path: "src/adapters/openai-chat.ts",
        sha256: "ea32bc0aab76a954ed37e2431c56e8ec31f356dfbfbc60eedc1b4a0c870c4cac",
    },
    SourceFile {
        path: "src/adapters/anthropic.ts",
        sha256: "8799310036aeaf5b2208cd658ea9965da4b85d8e4368d6231834b09842b25ab7",
    },
    SourceFile {
        path: "src/adapters/google.ts",
        sha256: "7a4e07ca5351461316a428c1b12ff9359add957015b4c7d3648cb5a87a29505a",
    },
    SourceFile {
        path: "src/bridge.ts",
        sha256: "ae6be9e720176eb867efc4b3110a82c30959ab853f887387cfa6777a650222de",
    },
];

pub static RULES: &[Rule] = &[
    Rule {
        id: "OCX-CHAT-STREAM-006",
        adapter: Adapter::OpenAiChat,
        requirement: "OpenAI Chat 流必须以 [DONE] 正常结束。",
        source_file: "src/adapters/openai-chat.ts",
        source_test: "tests/openai-chat-eof.test.ts",
        effect: "OpenCodex 无法确认这轮响应已完成。",
    },
    Rule {
        id: "OCX-CHAT-TOOL-004",
        adapter: Adapter::OpenAiChat,
        requirement: "流式工具调用的 function.name 必须是非空字符串。",
        source_file: "src/adapters/openai-chat.ts",
        source_test: "tests/openai-chat-dangling-toolcalls.test.ts",
        effect: "OpenCodex 无法生成 Codex 工具调用事件。",
    },
    Rule {
        id: "OCX-ANTH-STREAM-006",
        adapter: Adapter::Anthropic,
        requirement: "Anthropic 流必须以 message_stop 正常结束。",
        source_file: "src/adapters/anthropic.ts",
        source_test: "tests/anthropic-compatible-stream.test.ts",
        effect: "OpenCodex 无法确认这轮响应已完成。",
    },
    Rule {
        id: "OCX-ANTH-TOOL-005",
        adapter: Adapter::Anthropic,
        requirement: "同一轮 Anthropic tool_use 的 ID 必须唯一且可用于 tool_result 关联。",
        source_file: "src/adapters/anthropic.ts",
        source_test: "tests/anthropic-compatible-stream.test.ts",
        effect: "OpenCodex 无法把工具结果回传给正确的调用。",
    },
    Rule {
        id: "OCX-GOOGLE-STREAM-006",
        adapter: Adapter::Google,
        requirement: "Google 流必须返回有效的候选结束状态。",
        source_file: "src/adapters/google.ts",
        source_test: "tests/google-adapter.test.ts",
        effect: "OpenCodex 无法确认这轮响应已完成。",
    },
    Rule {
        id: "OCX-GOOGLE-TOOL-004",
        adapter: Adapter::Google,
        requirement: "Google functionCall 的 args 必须是对象。",
        source_file: "src/adapters/google.ts",
        source_test: "tests/google-adapter.test.ts",
        effect: "OpenCodex 无法生成有效的函数调用。",
    },
];

pub fn contract_digest() -> String {
    let mut hasher = Sha256::new();
    hasher.update(CONTRACT.version.as_bytes());
    hasher.update([0]);
    hasher.update(CONTRACT.commit.as_bytes());
    for rule in RULES {
        for value in [
            rule.id,
            rule.adapter.id(),
            rule.requirement,
            rule.source_file,
            rule.source_test,
            rule.effect,
        ] {
            hasher.update([0]);
            hasher.update(value.as_bytes());
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}
