# Issue #95：真实 Agent 报告证据引用修复

版本：`agent-report-evidence/v1`  
父任务：[#15](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/15)  
上游：[#93](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/93)

## 问题

Agent 执行器在 CLI 编排层写入模块 evidence 之前生成 AgentReport。若执行内部事件、权限事件和产物快照引用尚未创建的统一记录 evidence ID，报告构建会失败；旧实现使用 `.ok()` 静默丢弃错误，最终客户看到 `report: null`。

## 修复规则

AgentReport 构建阶段只接受当前报告已知的统一 evidence 引用；执行器内部事实保留在模块 evidence 的回合树中，不提前伪造统一记录 ID。模块证据仍包含请求 messages/tools、响应、工具调用、工具返回、权限事件和工作区事实，因此可从模块 evidence 追溯完整执行。

报告构建使用显式 `Result`：

- 成功时序列化完整 AgentReport，包含 10 个场景、attempts、samples 和 check summaries；
- 失败时模块保持非成功状态，reason 保留具体错误，原始回合证据仍保留；
- 不把报告构建失败包装成通过，也不静默写入 `null`。

## 验收

| 案例 | 预期 |
| --- | --- |
| 10 个真实/mock Agent 请求返回普通最终消息 | `evidence_payload.report` 为完整对象，场景和样本数量均为 10 |
| 多回合工具调用 | 回合证据保留，报告可解析，整体状态仍按事实缺口判定 |
| 构建阶段出现错误 | reason 暴露错误，模块不报告成功 |
| 运行中包含 API key | 报告和证据均不包含 API key |

本修复不改变 Agent 样本、工具权限、总体可靠性门禁或生产发布范围。
