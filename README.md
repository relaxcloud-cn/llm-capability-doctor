# 大模型能力诊断工具

## 检测范围

工具共执行 46 项检测，覆盖模型接入、能力适配和稳定性验证。

| 检测方向 | 覆盖内容 |
| --- | --- |
| 接口与协议 | URL 可达性、协议识别、鉴权、同步与流式生成、流结束、Token usage、错误可观测性 |
| 结构化结果与上下文 | JSON 输出、字段类型、嵌套数据、证据引用、8K 至 128K 上下文、多轮修正记忆 |
| 指令、推理与工具调用 | 精确输出、格式约束、信息抽取、摘要、Thinking、逻辑推理、单工具/串行/并行工具调用及失败恢复 |
| 性能、稳定性与护栏 | 首字节与完整响应延迟、重复成功率、P50/P95、4 至 32 并发、中文与英文安全业务词可用性 |

## 执行命令

进入解压后的程序目录，将命令中的接口地址、模型名称和 API Key 替换为实际值后执行。

### Windows x86_64（CMD）

```cmd
if not exist "model-doctor-output" mkdir "model-doctor-output"
.\model-capability-doctor-v0.12.0-windows-x86_64.exe --url "https://model.example/v1/chat/completions" --model "your-model-name" --api-key "your-api-key" --log-file ".\model-doctor-output\model-doctor.log"
```

### macOS ARM64

```bash
chmod +x ./model-capability-doctor-v0.12.0-macos-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-macos-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux x86_64

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-x86_64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-linux-x86_64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

### Linux ARM64

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-arm64
mkdir -p ./model-doctor-output
./model-capability-doctor-v0.12.0-linux-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

## 输出结果
执行命令默认完成 46 项模型能力检测、AI模型网关兼容性检测，并输出：

1.以.log为后缀的文件：该文件记录了检测项的所有curl请求的输入和输出，用来给技术人员分析使用的。

2.以.md为后缀的文件：本次检测的自分析报告，用来直接查看检测结论，适合给客户看。

