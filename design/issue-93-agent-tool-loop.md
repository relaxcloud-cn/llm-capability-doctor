# Issue #93：真实 Chat Completions 工具调用与 Agent 闭环

版本：`agent-chat-completions-tool-loop/v1`  
父任务：[#15](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/15)  
实现交接：[#35](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/35)

## 1. 目标

将真实 Chat Completions 的 Agent 请求从单轮普通文本升级为受控工具闭环：发送固定工具 schema，接收 assistant `tool_calls`，执行允许的工作区工具，将 `tool` 结果回传给同一对话，直到最终消息或安全终止。未提供工具调用能力时保持 `inconclusive`，不把文本自述当作工具事实。

## 2. 固定协议

每回合使用 Chat Completions 请求体中的 `messages`、`tools` 和 `tool_choice: auto`。首版工具名和参数如下：

| 工具 | 参数 | 允许范围 |
| --- | --- | --- |
| `read_file` | `path` | 当前场景声明的 input/workspace 根 |
| `write_file` | `path`、`content` | 当前场景声明的 workspace 根 |
| `list_directory` | `path` | 当前场景声明的 workspace 根 |

工具定义使用 `additionalProperties: false`。每个场景最多执行 6 个模型回合；每个工具调用都必须产生工具事件、工具返回事件和对应的 `tool_call_id` 回传消息。首版不开放移动、复制、任意 shell、网络和目录外路径。

## 3. 工作区和安全边界

执行器使用每个固定场景独立的内存工作区，预置输入文件和隐藏 expected 目录。工具路径必须位于场景根目录、不得包含目录穿越、不得访问隐藏 expected 目录，并且必须匹配该工具的声明权限。拒绝访问记录 `PermissionDecision::Denied` 和 `PermissionEffect::UnauthorizedWrite`，不修改工作区。

最终产物由工作区快照核对路径、存在性、内容和 SHA-256；最终消息与产物事实分开记录。模型说“已完成”但没有匹配产物时，A8 不通过或保持相应的事实状态，不能据此生成使用承诺。

## 4. 状态与证据

每个真实场景保存：请求 messages/tools、响应状态和脱敏响应、回合耗时、tool call/name/arguments、tool return、权限事件、工作区产物快照、最终消息、终态和 evidence refs。Authorization header 和 API key 永不进入证据。

事实映射遵循现有 Agent 规则：工具参数、工具返回使用、多轮状态、权限、工具失败处理和真实交付分别赋值；没有可观察事实的检查保持 `inconclusive`。模块总状态只有在全部场景没有缺口且无失败时才可为 `pass`；任一确认失败为 `fail`，否则为 `inconclusive`。

## 5. 受控验证

| 案例 | 预期 |
| --- | --- |
| 服务返回工具调用 | CLI 发送工具 schema，解析 call 并回传工具结果 |
| 读文件后写结果再最终消息 | 工作区产物匹配，事件顺序和三回合证据可追溯 |
| 访问 expected 或目录外路径 | 拒绝并记录权限事件，工作区保持不变 |
| 缺少工具调用或仅返回普通文本 | 不伪造 Agent 事实，相关检查保持 `inconclusive` |
| 无效 JSON、401/403、429 或传输失败 | 保留传输/环境异常，不归因模型能力 |

本实现不代表所有真实服务都支持工具调用，也不代表 #34 可靠性门禁已通过；生产发布仍需目标服务的授权端到端证据。
