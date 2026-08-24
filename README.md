# 大模型能力诊断工具

## 执行检测

请先将命令中的接口地址、模型名称和 API Key 替换为实际值。

Linux x86_64：

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-x86_64 && mkdir -p ./model-doctor-output && ./model-capability-doctor-v0.12.0-linux-x86_64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

Linux ARM64：

```bash
chmod +x ./model-capability-doctor-v0.12.0-linux-arm64 && mkdir -p ./model-doctor-output && ./model-capability-doctor-v0.12.0-linux-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

macOS ARM64：

```bash
chmod +x ./model-capability-doctor-v0.12.0-macos-arm64 && mkdir -p ./model-doctor-output && ./model-capability-doctor-v0.12.0-macos-arm64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log'
```

Windows x86_64（PowerShell）：

```powershell
New-Item -ItemType Directory -Force .\model-doctor-output | Out-Null; .\model-capability-doctor-v0.12.0-windows-x86_64.exe --url "https://model.example/v1/chat/completions" --model "your-model-name" --api-key "your-api-key" --log-file ".\model-doctor-output\model-doctor.log"
```

## 客户侧自分析（可选）

在上面的检测命令末尾追加 `--self-analyze`。CLI 会先完成全部 46 项检测并关闭日志，然后通过同一个接口、模型、API Key 和调用方式，让被测模型分批分析自己的本地证据。

```bash
./model-capability-doctor-v0.12.0-linux-x86_64 --url 'https://model.example/v1/chat/completions' --model 'your-model-name' --api-key 'your-api-key' --log-file './model-doctor-output/model-doctor.log' --self-analyze
```

执行时，终端使用纯文本输出采集进度（类别和检测目的）、4/8/16/32 并发波次、自分析批次进度和自动重试；不依赖 ANSI 颜色或光标控制，适用于 Windows、macOS、Linux 和日志重定向。分析结果写入同目录的 `model-doctor-self-analysis.json` 和 `model-doctor-self-analysis.md`。Markdown 顶部给出 AI模型网关层数据结构兼容性总体结论：通过时说明已验证的接口与工具调用数据结构能力，不通过时逐条写明具体字段、事件或关联差异；随后按能力大分类给出“满足/不满足”结论，并单独列出 4、8、16、32 并发档位的成功数、失败数、平均响应时间和失败请求错误。并发平均响应时间按该档位全部请求（含失败请求）的 `time_total` 计算。`057` 的 4、8、16、32 并发证据会分别发送给被测模型分析，单段证据上限为 16KiB，单个分析包上限为 256KiB，避免高并发请求共用一个过小的证据片段。所有检测和自分析请求默认超时为 300 秒，可用 `--timeout` 覆盖；自分析遇到超时、网络错误、HTTP 429 或 HTTP 5xx 会自动重试，最多 3 次。日志和分析文件均保留在客户环境；CLI 不提供日志上传或独立分析地址。每项 PASS/FAIL 由被测模型根据本地证据和通过标准决定，CLI 只校验返回结构与证据引用，不覆盖模型结论。单批分析失败会在 JSON 和 Markdown 中标记为 `ANALYSIS_UNAVAILABLE`，不会改变检测日志已成功生成的状态。

## 查看本地结果

Linux/macOS：

```bash
ls -lh ./model-doctor-output/model-doctor.log ./model-doctor-output/model-doctor-self-analysis.json ./model-doctor-output/model-doctor-self-analysis.md
```

Windows PowerShell：

```powershell
Get-Item .\model-doctor-output\model-doctor.log, .\model-doctor-output\model-doctor-self-analysis.json, .\model-doctor-output\model-doctor-self-analysis.md
```
