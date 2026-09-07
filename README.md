# 大模型能力诊断工具

## 检测范围

工具共执行 42 项检测，覆盖模型接入、能力适配和稳定性验证。

检测分为 **11 项刚性**和 **31 项柔性**。刚性项用于核心智能体接入及部署门槛；柔性项影响质量、效率和体验，不单独阻断使用。证据不足记为无法判断，不视为通过。

刚性项编号：`001`、`002`、`003`、`004`、`005`、`006`、`018`、`040`、`041`、`046`、`057`；其余为柔性项。检测目录、JSON 结果的 `requirementLevels` 和 Markdown 明细均标明分类。

- `018`（上下文容量实测）：沿用原逻辑，**124000 Token 即可通过**，为 128K 要求保留回复余量。优先使用服务端 usage；缺失时按构造值估计并标注来源。低于阈值的上限判为不通过；超时、疑似截断等无法确定容量的情况记为无法判断。
- `057`（并发响应时间与成功率）：检测 **4、8、16 并发**，分别展示失败请求数量与全部请求的平均响应时间。通过要求为 **4 并发零失败，且平均响应时间不超过 30 秒**，等于 30 秒通过；8、16 并发仅作参考。请求需传输完成、HTTP 2xx、协议有效且模型回复非空；计时缺失不能判为通过。

018 和 057 的最终状态由本地证据规则确定，模型自分析候选保留供审计，避免模型忽略刚性门槛。

| 检测方向 | 覆盖内容 |
| --- | --- |
| 接口与协议 | URL 可达性、协议识别、鉴权、同步与流式生成、流结束、Token usage、错误可观测性 |
| 结构化结果与上下文 | JSON 输出、字段类型、嵌套数据、证据引用、上下文容量实测（按实测 Token 判定 128K 要求）、多轮修正记忆 |
| 指令、推理与工具调用 | 精确输出、格式约束、信息抽取、摘要、Thinking、逻辑推理、单工具/串行/并行工具调用及失败恢复 |
| 性能、稳定性与护栏 | 首字节与完整响应延迟、重复成功率、P50/P95、4/8/16 并发、中文与英文安全业务词可用性 |

## 执行命令

进入解压后的程序目录，将命令中的接口地址、模型名称和 API Key 替换为实际值后执行。

### Windows x86_64（CMD）

```cmd
if not exist "model-doctor-output" mkdir "model-doctor-output"
.\model-capability-doctor-v0.13.0-windows-x86_64.exe --url "https://model.example/v1/chat/completions" --model "your-model-name" --api-key "your-api-key" --log-file ".\model-doctor-output\model-doctor.log"
```

### macOS ARM64

```bash
chmod +x ./model-capability-doctor-v0.13.0-macos-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.13.0-macos-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux x86_64

```bash
chmod +x ./model-capability-doctor-v0.13.0-linux-x86_64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.13.0-linux-x86_64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux ARM64

```bash
chmod +x ./model-capability-doctor-v0.13.0-linux-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.13.0-linux-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

## 输出结果
执行命令默认完成 42 项模型能力检测、LLM模型网关兼容性检测和被测模型自分析，并输出：

1.以.log为后缀的文件：该文件记录了检测项的所有curl请求的输入和输出，用来给技术人员分析使用的。

2.以.md为后缀的文件：本次检测的自分析报告，用来直接查看检测结论，适合给客户看。
