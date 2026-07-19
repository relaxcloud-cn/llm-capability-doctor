# LLM Capability Doctor（大模型能力诊断工具）

面向智能体平台模型接入场景的一站式大模型适配性诊断工具。Rust CLI
原生执行网络请求，不依赖客户服务器上的 Bash、curl、Python 或 OpenSSL
动态库；检测完成后输出 `llm-capability-doctor.evidence.v1` 日志，现有
Model Doctor Report Skill 可直接解析该日志并生成 HTML 报告。

## 检测范围

- 62 个固定检测项，覆盖接口协议、结构化输出、上下文、指令与文本、
  Thinking、工具调用、性能稳定性及护栏词汇。
- 自动识别 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages、
  Gemini GenerateContent 和 Ollama Chat。
- 只采集完整请求与响应证据，不在现场 CLI 中给出通过或失败判断。
- 支持同步、流式、多轮、工具回传、重复采样和 4/8/16/32 并发检测。

## 构建

本地需要 Rust 1.85 或更高版本：

```bash
cargo build --release
```

生成的可执行文件为：

```text
target/release/model-capability-doctor
```

CI 发布以下 Linux 架构的单文件二进制，macOS 用于本地开发和测试：

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`

## 执行检测

优先通过环境变量提供 API Key，避免密钥进入 Shell 历史：

```bash
MODEL_API_KEY='secret' ./target/release/model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --log-file './contract-test.log'
```

未指定 `--log-file` 时，日志默认写入当前目录下的
`model-doctor-YYYYMMDD-HHMMSS.log`。日志文件权限在 Unix 平台设置为 `0600`。

常用参数：

| 参数 | 说明 |
| --- | --- |
| `--url URL` | 完整模型接口 URL，不自动改写路径。 |
| `--model MODEL` | 发送给模型接口的模型名。 |
| `--api-key KEY` | API Key；显式值优先于 `MODEL_API_KEY`。 |
| `--log-file PATH` | 指定 evidence-v1 日志路径。 |
| `--timeout SECONDS` | 单次请求超时，默认 120 秒。 |
| `--only IDS` | 只执行逗号分隔的检测项，例如 `001,040,062`。 |
| `--list-tests` | 输出完整 62 项目录并退出。 |
| `--insecure` | 跳过 HTTPS 证书校验。 |

例如只检测同步生成和英文安全词：

```bash
MODEL_API_KEY='secret' ./target/release/model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '004,062'
```

查看目录：

```bash
./target/release/model-capability-doctor --list-tests
```

### `--insecure` 安全提示

危险：`--insecure` 会关闭 TLS 证书和主机身份校验，只应在受控网络中临时
连接自签名证书接口。生产现场应优先将私有 CA 安装到服务器系统信任库，
保持默认的严格校验。启用该参数时，运行头会记录
`tls_verification: disabled`，每个请求审计命令也会记录 `--insecure`。

## 生成诊断报告

检测结束后，将 `.log` 文件传回分析环境，安装仓库中的
`skills/creating-model-doctor-reports` Skill，然后让 Codex 使用 Model Doctor
Report Skill 分析日志。报告生成仍由 Skill 负责，Rust CLI 不包含评判逻辑。

## 开发验证

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

测试使用本地模型 fixture 和自签名 HTTPS fixture，不访问真实模型。

## Shell 参考实现

`model-capability-doctor.sh` 保留为 v0.7.0 行为对照和历史兼容实现。Rust CLI
不调用该脚本，也不调用 curl；新增能力以 Rust CLI 为主。
