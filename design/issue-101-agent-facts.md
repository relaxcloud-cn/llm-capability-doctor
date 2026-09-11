# Agent 事实映射与隐藏产物验收

版本：1.0
整理日期：2026-09-11
关联任务：[#101](https://github.com/relaxcloud-cn/llm-capability-doctor/issues/101)

## 实现结论

- 被拒绝的读取、写入、隐藏目录和路径穿越只记录 `PermissionDecision::Denied` 与 `PermissionEffect::None`；只有真实发生的越权写入证据才允许使用 `UnauthorizedWrite`。
- 隐藏 expected 原文继续只存在于执行器内部，报告序列化只保留路径和 digest。实际内容与隐藏值不一致时，DeliveryAndEnd 为 `inconclusive`，不在模型未获知验收原文的情况下直接判失败。
- `task_rules_followed` 仅在存在工具执行事实时填充；工具返回使用、多轮状态和工具失败恢复分别由回合与工具事件支持，不能用“出现过工具调用”代替完整闭环。
- 每个 Agent 场景的运行证据仍通过统一 `DetectionRecord.evidence` 绑定到 sample、attempt、event、permission 和 artifact。

## 验证记录

| AC | 检查方法 | 结果 |
| --- | --- | --- |
| AC-01 | 读取和写入隐藏路径回归测试；真实越权写入保持 A7 失败测试 | 通过 |
| AC-02 | 隐藏 expected 内容不一致回归测试；报告 JSON 不再包含 expected 原文 | 通过 |
| AC-03 | Agent facts 仅从工具/回合事实填充；缺少闭环事实保留未知 | 通过 |
| AC-04 | 既有统一证据绑定测试保持通过 | 通过 |
| AC-05 | `cargo fmt --all`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`（80 tests）、`cargo build --release --locked`；真实豆包端点 Agent 回归 10 个固定场景 | 通过 |
