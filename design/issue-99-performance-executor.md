# 性能执行器实现：矩阵、事件时间线与统一证据

版本：1.0
整理日期：2026-09-11
关联任务：[#99](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/99)
设计来源：[#18](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/18)

## 实现范围

本次实现把 #18 的性能口径接入 Rust CLI，不改变五类性能的设计边界，也不把短时实测解释为生产容量或长期可靠性。

## 执行规则

- 每个 `PerformancePlan` 的响应模式、输入档位、输出档位和并发目标都展开为独立维度；样本 ID 包含 workload、模式、长度和并发档位，避免不同 P05 子场景碰撞。
- 每个维度先执行 `warmup_count` 个预热请求，再执行 `formal_request_limit` 个正式请求。预热样本保留在统一记录中，但报告聚合只使用 `Formal` 样本。
- 并发批次使用 barrier 同步起跑，每个请求在线程中使用独立 transport 调用；样本同时保存目标并发和本批实际派发并发。保护窗口按当前维度计时，停止后不自动升档。
- 非流式请求只记录完整响应时间；流式请求保存原始 SSE 字节、事件顺序、事件时间、正文增量、推理增量、结束事件和组装正文。跨网络块拆分的 SSE 行会在完整行后解析。
- 服务没有可靠 token usage 时，`TokenCountSource` 保持 `Unavailable`，只展示字符数和内容块间隔，不生成 token 速率。
- 每个样本先写入 `DetectionRecord.evidence`，再把真实 evidence ID 写入 `PerformanceSample.evidence_refs`，报告通过 `build_report_for_record` 校验引用归属。

## 验证记录

| AC | 检查方法 | 结果 |
| --- | --- | --- |
| AC-01 | `performance_matrix_expands_every_declared_dimension`；固定计划维度为 `2/1/4/1/3/2/3` | 通过 |
| AC-02 | 执行器按维度执行预热、正式批次、并发目标和独立计时窗口；现有 warmup 排除测试保持通过 | 通过 |
| AC-03 | `parses_stream_events_when_sse_lines_are_split_across_network_chunks`；验证正文、事件和 `[DONE]` | 通过 |
| AC-04 | 现有无 token 计数、错误、超时、长度短缺测试保持通过；执行器不伪造 token 计数 | 通过 |
| AC-05 | 每个性能样本写入 `cli-performance-*` 证据并由统一报告校验引用 | 通过 |
| AC-06 | `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`、`cargo test --locked`、release build | 待提交前执行 |

## 受控真实端点边界

本次实现不执行完整性能矩阵。完整矩阵包含长时间窗口和大量正式请求，真实端点只应在明确授权的受控 smoke 场景中验证请求协议和报告结构；完整高负载验收不作为本 Issue 的默认命令。
