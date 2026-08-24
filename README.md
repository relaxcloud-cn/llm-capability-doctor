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

分析结果写入同目录的 `model-doctor-self-analysis.json` 和 `model-doctor-self-analysis.md`。Markdown 顶部按能力大分类给出“满足/不满足”结论，中间列出 46 项检测结果，底部展开失败和分析不可用项的具体原因、观察与证据引用。日志和分析文件均保留在客户环境；CLI 不提供日志上传或独立分析地址。每项 PASS/FAIL 由被测模型根据本地证据和通过标准决定，CLI 只校验返回结构与证据引用，不覆盖模型结论。单批分析失败会在 JSON 和 Markdown 中标记为 `ANALYSIS_UNAVAILABLE`，不会改变检测日志已成功生成的状态。

## 查看本地结果

Linux/macOS：

```bash
ls -lh ./model-doctor-output/model-doctor.log ./model-doctor-output/model-doctor-self-analysis.json ./model-doctor-output/model-doctor-self-analysis.md
```

Windows PowerShell：

```powershell
Get-Item .\model-doctor-output\model-doctor.log, .\model-doctor-output\model-doctor-self-analysis.json, .\model-doctor-output\model-doctor-self-analysis.md
```
