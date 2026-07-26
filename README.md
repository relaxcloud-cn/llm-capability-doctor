# LLM Capability Doctor（大模型能力诊断工具）

用于客户现场采集大模型接口能力证据，为判断模型是否满足项目要求提供依据。
Rust CLI 在现场一次性采集完整请求与响应，生成
`llm-capability-doctor.evidence.v1` 日志；日志带回分析环境后，由 Model Doctor
Report Skill 逐项判定并生成 HTML 报告。

CLI 原生发送网络请求，不调用 Bash、curl、Python 或 OpenSSL 动态库。客户服务器
可以不连接公网，只需能够访问待测模型接口。

## 现场使用流程

### 1. 准备 CLI

从 [Rust CLI 工作流](https://github.com/relaxcloud-cn/llm-capability-doctor/actions/workflows/rust-cli.yml)
中选择来源为已批准 `main` 提交的成功运行记录，核对该运行记录的提交 SHA，再下载
与客户服务器架构对应的 GitHub Actions 产物：

| 客户服务器 | Actions 产物 |
| --- | --- |
| Linux x86_64 | `model-capability-doctor-x86_64-unknown-linux-gnu` |
| Linux ARM64 | `model-capability-doctor-aarch64-unknown-linux-gnu` |

解压后将其中的 `model-capability-doctor` 复制到客户服务器，并赋予执行权限：

```bash
chmod +x ./model-capability-doctor
./model-capability-doctor --version
sha256sum ./model-capability-doctor
```

复制前记录二进制的 SHA-256，传入客户环境后重新计算并比对。报告分析应使用同一
提交中的 `skills/creating-model-doctor-reports`，避免 CLI 与判定规则版本不匹配。
macOS 上可以用 `shasum -a 256` 计算 SHA-256。

这些二进制是 glibc 目标；较旧或非 glibc Linux 环境应先验证兼容性。仓库当前
没有自动发布 GitHub Release，工作流产物从 Actions 页面下载。

也可以在装有 Rust 1.85 或更高版本的同平台机器上构建：

```bash
cargo build --release --locked
```

生成文件位于 `target/release/model-capability-doctor`。

### 2. 在客户环境执行一次检测

先在源码仓库之外准备仅当前用户可访问的输出目录。日志、评估 JSON 和 HTML
报告都包含客户请求或模型响应，应统一存放在该目录：

```bash
MODEL_DOCTOR_OUTPUT='/path/outside/repository/model-doctor-output'
mkdir -p "$MODEL_DOCTOR_OUTPUT"
chmod 700 "$MODEL_DOCTOR_OUTPUT"
```

推荐由现场密钥管理方式注入 `MODEL_API_KEY`，避免密钥出现在进程参数中。使用
Bash 时，也可以无回显读取；输入内容不会写入 Shell 历史：

```bash
read -rsp 'API Key: ' MODEL_API_KEY && printf '\n'
export MODEL_API_KEY
```

执行默认现场检测：

```bash
./model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --log-file "$MODEL_DOCTOR_OUTPUT/your-model-model-doctor.log"
```

也可以直接使用 `--api-key`，但密钥会出现在 Shell 历史和进程参数中：

```bash
./model-capability-doctor --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file "$MODEL_DOCTOR_OUTPUT/your-model-model-doctor.log"
```

`--url` 必须是完整模型接口地址，CLI 不会自动补充或改写路径。未指定
`--log-file` 时，日志写入当前目录下的
`model-doctor-YYYYMMDD-HHMMSS.log`；Unix 平台会将日志权限设置为 `0600`。
除 `--list-tests` 外，URL、模型名和 API Key 都是必填项。CLI 会在协议探测时
自动使用 Bearer、`api-key`、`x-api-key` 或 `x-goog-api-key` 等对应认证头。

默认执行精简后的现场检测模式，共 29 个高价值检测项。通常无需增加参数。

需要完整执行全部 46 个检测项时：

```bash
./model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --full \
  --log-file "$MODEL_DOCTOR_OUTPUT/your-model-full-model-doctor.log"
```

只执行指定检测项时：

```bash
./model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --only '001,040,060' \
  --log-file "$MODEL_DOCTOR_OUTPUT/your-model-custom-model-doctor.log"
```

检测结束后清除当前 Shell 中的密钥：

```bash
unset MODEL_API_KEY
```

### 3. 带回日志

检测结束后，CLI 会打印总耗时、总请求数、测试清单数和日志绝对路径。将生成的
`.log` 文件带回分析环境，并放入源码仓库之外的受保护目录。日志以及后续生成的
评估 JSON、HTML 报告都包含原始证据，必须按客户敏感数据管理，不应提交到 Git。

### 4. 使用 Skill 生成报告

在分析环境签出与 CLI 相同的已批准提交，然后从仓库根目录安装 Skill：

```bash
mkdir -p "$HOME/.codex/skills/creating-model-doctor-reports"
cp -R ./skills/creating-model-doctor-reports/. "$HOME/.codex/skills/creating-model-doctor-reports/"
```

在 Codex 中执行：

```text
使用 $creating-model-doctor-reports 分析 `/绝对路径/your-model-model-doctor.log`，逐项判定 PASS 或 FAIL，并生成能力报告。
```

Skill 会在日志旁生成评估 JSON 和自包含 HTML 报告，并对每个已采集检测项给出
PASS 或 FAIL。CLI 只负责采集证据，不在客户现场给出结论；Skill 也不会自动给出
整个项目是否可用的总判定，实施人员应将项目必需项与逐项结果进行对照。

一份结构完整的日志可以直接完成一次报告分析。如果日志版本不匹配、结构校验
失败或采集过程被中断，需要重新执行 CLI 采集，不应让 Skill 猜测缺失证据。

## 检测模式与范围

| 模式 | 检测项数 | 用途 |
| --- | ---: | --- |
| 默认现场模式 | 29 | 一次覆盖项目接入最关键的高价值能力。 |
| `--full` | 46 | 执行完整核心目录，用于更全面的兼容性调查。 |
| `--only IDS` | 自定义 | 执行逗号分隔的检测项，例如 `001,040,060`。 |

46 个核心检测项覆盖以下能力：

| 领域 | 主要检查内容 |
| --- | --- |
| 接口与协议 | 可达性、协议识别、鉴权、同步与流式响应、Token usage、错误可观测性。 |
| 结构化结果 | 裸 JSON、字段类型、嵌套结构、项目核心结果和证据引用。 |
| 上下文 | 用约 3.2 万至 51.2 万字符近似测试 8K 至 128K Token 档位，并检查多轮修正记忆。 |
| 指令与文本 | 精确输出、组合格式、多字段抽取和限长摘要。 |
| Thinking 与推理 | Thinking 档位、推理 Token、思考与答案分离、流式事件和逻辑推理。 |
| 工具调用 | 工具选择、参数约束、并行与串行调用、结果忠实性及失败恢复。 |
| 性能与稳定性 | 首字节、完整响应、重复成功率、P50/P95 延迟和并发响应时间。 |
| 护栏与词汇 | 告警、分诊、漏洞等中英文安全业务词汇是否可正常用于项目任务。 |

### 请求负载提示

默认 29 项现场模式包含约 51.2 万字符的上下文请求和一轮 8 并发请求。`--full`
还会执行约 3.2 万、6.4 万、12.8 万、25.6 万和 51.2 万字符的上下文请求，以及
4、8、16、32 并发波次。执行前应确认客户授权、模型上下文上限、配额、费用和
限流策略；不要在未经批准的生产接口上直接运行。

查看实际目录：

```bash
./model-capability-doctor --list-tests
```

CLI 会自动识别 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages、
Gemini GenerateContent 和 Ollama Chat 协议。

## 参数

| 参数 | 说明 |
| --- | --- |
| `--url URL` | 完整模型接口 URL，不自动改写路径。 |
| `--model MODEL` | 发送给模型接口的模型名。 |
| `--api-key KEY` | API Key；显式值优先于 `MODEL_API_KEY`。 |
| `--log-file PATH` | 指定 evidence-v1 日志路径。 |
| `--timeout SECONDS` | 单次请求超时，默认 120 秒。 |
| `--only IDS` | 只执行逗号分隔的检测项；不能与 `--full` 同时使用。 |
| `--full` | 执行完整 46 项，而不是默认现场模式。 |
| `--list-tests` | 输出完整 46 项目录并退出。 |
| `--insecure` | 跳过 HTTPS 证书和主机身份校验。 |

### `--insecure` 安全提示

危险：`--insecure` 会关闭 TLS 证书和主机身份校验，只应用于受控网络中的临时
自签名证书接口。生产现场应优先将私有
CA 安装到服务器系统信任库并保持默认严格校验。启用后，日志会明确记录 TLS
校验已关闭。

## 开发验证

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --locked
cargo build --release --locked
```

测试使用本地模型 fixture 和自签名 HTTPS fixture，不访问真实模型。

## Shell 参考实现

`model-capability-doctor.sh` 仅保留为 v0.7.0 行为对照和历史兼容实现。Rust CLI
不调用该脚本，也不调用 curl；后续能力以 Rust CLI 为准。
