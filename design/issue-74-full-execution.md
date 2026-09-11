# Issue #74：六模块完整固定样本执行

版本：`live-module-execution/v1`  
前置：`chat-completions-transport/v1`（Issue #72）

## 执行入口

Release CLI 默认使用完整执行器；库测试继续使用 `LiveExecutor::new` 的单请求冒烟模式，避免测试意外发起大量外部请求。完整执行器由 `LiveExecutor::new_full` 创建。

## 固定清单

| 模块 | 正式清单 | 结果对象 | 结论边界 |
| --- | ---: | --- | --- |
| specification | 7 类、30 个样本 | `SpecificationReport` | 通过只表示样本响应被接受；不自动宣称最大上下文或长期稳定性 |
| capability | 240 个中文单元 | `CapabilityScorecard` | 使用既有接受规则评分，不把未知输出包装成正确 |
| performance | 固定计划的正式请求数 | `PerformanceReport` | 保存首响、完整耗时、结束状态和错误；错误不计入正常速度指标 |
| agent | 10 个 OMP 场景 | `AgentReport` | 未观察到工具/权限/交付事实时保持 `inconclusive` |
| baseline | BC01-BC14 | `BaselineReport` | 只报告结构比较，结构差异不推导整体可用性 |

每个模块的 CLI 证据包含计划数、实际执行数、版本、模块报告和脱敏的请求/响应记录。`DetectionRecord` 仍只接收一个模块级证据引用，模块报告内部保留样本级标识，避免把大量独立证据误写成多个生命周期尝试。

## 状态语义

- 网络、认证和权限问题属于 `invalid_execution` 或 `inconclusive`，不归因模型。
- 2xx 但响应结构不可识别属于协议失败。
- Agent 适配器会保留真实模型回复，但没有工具调用、权限事件或产物观察时不会判定通过。
- 受保护的完整性能计划在服务错误后保留已完成样本并停止受影响的正式负载；报告会标记不可测量限制。

## 已知限制

当前 Chat Completions 传输层只提供统一文本消息入口，不携带供应商专属工具 schema。因此 Agent 报告可以真实记录模型回复和场景覆盖，但只有服务返回并被适配器观察到的工具、权限和产物事实才能形成通过/失败结论。Issue #35 发布验收必须把该限制写入发布说明。
