# 大模型能力诊断工具

## 1. 准备 CLI

Linux x86_64：

```bash
cp ./model-capability-doctor-v0.12.0-linux-x86_64 ./model-capability-doctor
chmod +x ./model-capability-doctor
./model-capability-doctor --version
```

Linux ARM64：

```bash
cp ./model-capability-doctor-v0.12.0-linux-arm64 ./model-capability-doctor
chmod +x ./model-capability-doctor
./model-capability-doctor --version
```

macOS ARM64：

```bash
cp ./model-capability-doctor-v0.12.0-macos-arm64 ./model-capability-doctor
chmod +x ./model-capability-doctor
./model-capability-doctor --version
```

## 2. 执行检测

Linux：

```bash
read -r -s -p 'API Key: ' MODEL_API_KEY
printf '\n'
export MODEL_API_KEY
```

macOS：

```zsh
read -s 'MODEL_API_KEY?API Key: '
export MODEL_API_KEY
```

执行：

```bash
mkdir -p ./model-doctor-output
chmod 700 ./model-doctor-output
./model-capability-doctor \
  --url 'https://model.example/v1/chat/completions' \
  --model 'your-model-name' \
  --log-file './model-doctor-output/model-doctor.log'
unset MODEL_API_KEY
```

## 3. 交付日志

```bash
ls -lh ./model-doctor-output/model-doctor.log
```

将 `./model-doctor-output/model-doctor.log` 发回即可。
