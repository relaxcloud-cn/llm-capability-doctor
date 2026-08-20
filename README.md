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

## 交付日志

Linux/macOS：

```bash
ls -lh ./model-doctor-output/model-doctor.log
```

Windows PowerShell：

```powershell
Get-Item .\model-doctor-output\model-doctor.log
```

将生成的 `model-doctor.log` 发回即可。
