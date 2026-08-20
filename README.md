# LLM Capability Doctor（大模型能力诊断工具）

用于客户现场采集大模型接口能力证据，为判断模型是否满足项目要求提供依据。
Rust CLI 在现场一次性采集完整请求与响应，生成
`llm-capability-doctor.evidence.v4` 日志；v4 日志记录
`compatibility_profile: opencodex-2.7.42-data-format`。日志带回分析环境后，由 Model Doctor
Report Skill 逐项判定并生成 `llm-capability-doctor.assessment.v9` 和 HTML 报告。

CLI 原生发送网络请求，不调用 Bash、curl、Python 或 OpenSSL 动态库。客户服务器
可以不连接公网，只需能够访问待测模型接口。

## 现场使用流程

### 1. 准备 CLI

从 [GitHub Releases](https://github.com/relaxcloud-cn/llm-capability-doctor/releases)
下载与客户机器匹配的裸二进制：

| 客户机器 | Release 文件 |
| --- | --- |
| Linux x86_64 | `model-capability-doctor-v0.12.0-linux-x86_64` |
| Linux ARM64 | `model-capability-doctor-v0.12.0-linux-arm64` |

Release 文件不是压缩包。下载后将对应文件重命名为 `model-capability-doctor` 并赋予
执行权限，不需要创建软链接。

Linux x86_64：

```bash
mv ./model-capability-doctor-v0.12.0-linux-x86_64 ./model-capability-doctor
chmod +x ./model-capability-doctor
./model-capability-doctor --version
sha256sum ./model-capability-doctor
```

Linux ARM64：

```bash
mv ./model-capability-doctor-v0.12.0-linux-arm64 ./model-capability-doctor
chmod +x ./model-capability-doctor
./model-capability-doctor --version
sha256sum ./model-capability-doctor
```

Linux Release 文件采用 GNU libc 动态链接，适用于对应 CPU 架构的主流 glibc Linux
发行版，不支持在纯 MUSL 环境中直接运行。
复制前记录二进制的 SHA-256，传入客户环境后重新计算并比对。报告分析应使用同一
Release 对应提交中的 `skills/creating-model-doctor-reports`，避免 CLI 与判定规则
版本不匹配。

也可以在装有 Rust 1.85 或更高版本的同平台机器上构建：

```bash
cargo build --release --locked
```

生成文件位于 `target/release/model-capability-doctor`。

GitHub Actions 仍会构建 `x86_64-unknown-linux-gnu` 和
`aarch64-unknown-linux-gnu` 流水线产物；现场手动发布和使用的是上表中的裸二进制。

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

执行完整检测：

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

`--url` 必须是完整模型接口地址，CLI 通常不会自动补充或改写路径。唯一例外是已识别的
Google 流式请求：当路径以官方方法 `:generateContent` 或
`:streamGenerateContent` 结尾时，CLI 会派生 `:streamGenerateContent` 并设置
`?alt=sse`，且不保留原查询参数。自定义路径（包括方法后的尾随斜杠）保持不变。模型接口 URL 不接受
fragment，包括空的 `#`。未指定
`--log-file` 时，日志写入当前目录下的
`model-doctor-YYYYMMDD-HHMMSS.log`；Unix 平台会将日志权限设置为 `0600`。
除 `--list-tests` 外，URL、模型名和 API Key 都是必填项。CLI 会在协议探测时
自动使用 Bearer、`api-key`、`x-api-key` 或 `x-goog-api-key` 等对应认证头。

默认执行全部 46 个检测项，不需要也不接受检测模式或检测项 ID 参数。

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

Skill 会在日志旁生成 `llm-capability-doctor.assessment.v9` 评估 JSON 和自包含 HTML
报告，并对全部 46 个检测项给出 PASS 或 FAIL。工具检测验证官方协议的调用与结果
关联以及完整工具闭环。报告程序同时给出 OpenCodex 数据格式兼容性和通用能力结论。OpenCodex 结论的八个硬门槛是
002、004、005、006、040、041、043、047；045 仍是增强能力项，不影响该结论。

兼容性只支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages 和
Gemini GenerateContent 四类协议。Ollama Chat 仍可被协议探测识别，但会得到
OpenCodex 数据格式不兼容。报告只接受 evidence.v4，旧日志必须重新采集。流式合同只读取首个 choice/candidate，并将
OpenCodex 会转成 `response.incomplete` 的截断或过滤终止判为失败；Google
官方流式方法的查询串固定为 `?alt=sse`。该结论的范围边界是：
`仅判断本轮模型端数据格式，不覆盖鉴权、网络、部署或 ClawOps 运行环境。`
因此它不是整个项目或完整 ClawOps 运行链路的可用性判定。

一份结构完整的日志可以直接完成一次报告分析。如果日志版本不匹配、结构校验
失败或采集过程被中断，需要重新执行 CLI 采集，不应让 Skill 猜测缺失证据。

## 检测范围

46 个固定检测项覆盖以下能力：

| 领域 | 主要检查内容 |
| --- | --- |
| 接口与协议 | 可达性、协议识别、鉴权、同步与流式响应、Token usage、错误可观测性。 |
| 结构化结果 | 裸 JSON、字段类型、嵌套结构、项目核心结果和证据引用。 |
| 上下文 | 用约 3.2 万至 51.2 万字符近似测试 8K 至 128K Token 档位，并检查多轮修正记忆。 |
| 指令与文本 | 精确输出、组合格式、多字段抽取和限长摘要。 |
| Thinking 与推理 | Thinking 档位、思考与答案分离、流式事件和逻辑推理。 |
| 工具调用 | 官方协议结构、完整工具闭环、工具选择、参数约束、并行与串行调用、结果忠实性及失败恢复。 |
| 性能与稳定性 | 首字节、完整响应、重复成功率、P50/P95 延迟和并发响应时间。 |
| 护栏与词汇 | 告警、分诊、漏洞等中英文安全业务词汇是否可正常用于项目任务。 |

### 请求负载提示

每次检测都会执行约 3.2 万、6.4 万、12.8 万、25.6 万和 51.2 万字符的上下文请求，
以及 4、8、16、32 四个并发波次。执行前应确认客户授权、模型上下文上限、配额、费用和
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
| `--url URL` | 不含 fragment 的完整模型接口 URL；仅已识别的 Google 官方流式方法会派生 `:streamGenerateContent` 和 `alt=sse`，自定义路径不变。 |
| `--model MODEL` | 发送给模型接口的模型名。 |
| `--api-key KEY` | API Key；显式值优先于 `MODEL_API_KEY`。 |
| `--log-file PATH` | 指定 evidence-v4 日志路径。 |
| `--timeout SECONDS` | 单次请求超时，默认 120 秒。 |
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
python3 -m unittest discover -s skills/creating-model-doctor-reports/tests -p 'test_*.py' -v
```

测试使用本地模型 fixture 和自签名 HTTPS fixture，不访问真实模型。

## Shell 参考实现

`model-capability-doctor.sh` 仅保留为 Shell 参考实现，固定执行其 61 项历史目录，
不接受检测项选择参数。Rust CLI 不调用该脚本，也不调用 curl；后续能力以 Rust CLI 为准。
