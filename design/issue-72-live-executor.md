# Issue #72：真实 Chat Completions 传输与执行器

版本：`chat-completions-transport/v1`  
关联：#35 的真实执行前置  
支持协议：OpenAI 风格 Chat Completions HTTP POST  
默认 TLS：`reqwest` Rustls，证书校验开启  
凭据：`--api-key`（隐藏参数）或 `MODEL_API_KEY` 环境变量
超时：`--timeout-seconds`，默认 300 秒，必须为正整数

## 1. 使用方式

从 Release 构建的二进制启动：

```bash
MODEL_API_KEY='替换为授权测试密钥' \
llm-capability-doctor \
  --url 'https://service.example.test/v1/chat/completions' \
  --model 'model-id' \
  --no-gui \
  --format json \
  --output ./agentcheck-run.json
```

专项运行示例：

```bash
MODEL_API_KEY='替换为授权测试密钥' \
llm-capability-doctor \
  --url 'https://service.example.test/v1/chat/completions' \
  --model 'model-id' \
  --modules ingress,baseline \
  --no-gui
```

密钥只进入内存中的 HTTP Authorization 头；统一记录、错误文本、JSON 输出和 GUI 数据均不保存密钥。

## 2. 执行支持矩阵

| 项目 | 当前真实执行 | 结果边界 |
| --- | --- | --- |
| `ingress` | Chat Completions 请求、HTTP 状态、JSON 结构和耗时 | 协议接入冒烟；不替代身份长期管理 |
| `specification` | 真实请求和 Chat Completions 响应解析 | 当前入口为冒烟；未覆盖七类全部固定样本时保持 `inconclusive` |
| `capability` | 真实请求和响应证据 | 当前入口为冒烟；未执行 240 个中文单元时保持 `inconclusive` |
| `performance` | 真实请求耗时和传输结果 | 单次冒烟不形成正式 P50/P95 或并发结论，保持 `inconclusive` |
| `agent` | 真实请求可达性和响应证据 | 未接入 OMP 工作区/工具权限闭环，保持 `inconclusive` |
| `baseline` | 真实响应进入结构检查入口 | 结构差异只展示为结构事实，不直接决定整体可用性 |

## 3. 错误与重试

每个请求最多 3 次尝试。网络异常、429 和 5xx 会重试，并在证据中保存每次重试原因；401/403 标为 `invalid_execution`，不归因模型；其他非 2xx 标为 `inconclusive`；2xx 但不是可识别 Chat Completions 结构标为协议 `fail`。

CLI 的 `--stop-after` 仍由统一编排器控制：停止前事实和证据保留，剩余项目为 `unverified`，整体结论为 `inconclusive`。

## 4. 离线验收

运行：

```bash
cargo test transport
cargo test live_executor
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
```

覆盖成功响应、Bearer 请求、凭据脱敏、认证失败、429 三次重试、超时、无效 JSON、专项选择和真实证据进入统一记录。

## 5. 交接与限制

本 Issue 交付真实 HTTP 传输和执行器入口，不声称六个模块的完整正式测量已经完成。#35 必须从 Release 包继续执行授权服务的完整固定清单；当前复杂模块缺少完整样本或 OMP 闭环时，报告必须保留 `inconclusive` 和限制说明。
