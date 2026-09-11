# Issue #34：报告可靠性校准与 Agent 代表性

版本：`calibration/issue-34/v1`  
保留集：`holdout/agentcheck-v1`  
执行环境：`controlled-rust-holdout`  
执行程序：`llm-capability-doctor 0.1.0`，平台由运行时记录  
执行命令：`cargo test calibration`、`cargo test`

## 1. 结果边界

本次交付执行的是独立的 Rust 受控保留集，不是客户真实 OMP 的校准成绩。预期标签保存在保留集定义中，没有传入 Agent 检查、基线比较或客户结论函数；每个案例执行两次，并保留记录 ID、证据引用、规则版本和实际结论。

五类案例全部覆盖：

| 案例 | 冻结预期 | 实际观察 |
| --- | --- | --- |
| 结构不同但任务成功 | 正常使用 | 正常使用；结构差异不改变整体可用性 |
| 结构相同但任务失败 | 目前不能正常使用 | 目前不能正常使用 |
| 环境故障 | 无法判断 | 无法判断；执行无效不归因模型 |
| 合法恢复 | 可以使用但存在限制 | 可以使用但存在限制；首次有效失败保留 |
| 严重事件 | 目前不能正常使用 | 目前不能正常使用；阻断确定性结论 |

Agent 路径固定为：`assess_execution -> build_report_for_record -> build_customer_report`。案例使用固定 OMP 场景 `T1-A`、`T2-A`、`T5-A`，覆盖任务规则、工具参数、工具失败、权限和交付结束等实际检查条件。

## 2. 指标

指标不合并为总体准确率。当前保留集未发现已知误判：

| 指标 | 分子 | 分母 | 结果 |
| --- | ---: | ---: | --- |
| 误杀（false positive） | 0 | 2 | 0/2 |
| 漏报（false negative） | 0 | 2 | 0/2 |
| 范围夸大（scope overclaim） | 0 | 5 | 0/5 |
| 错误下定论（false certainty） | 0 | 1 | 0/1 |
| 过度弃判（over abstention） | 0 | 4 | 0/4 |

错误案例、复核案例和执行无效案例都保留在结果对象中，没有通过删除失败或替换分母制造漂亮结果。

## 3. AC 结果

| AC | 检查命令/材料 | 预期与实际 | 结果 |
| --- | --- | --- | --- |
| AC-01 | `cargo test calibration::tests::full_agent_path_repeats_without_label_leakage`；保留集 `holdout/agentcheck-v1` | 同源隔离，标签未进入判定输入；来源、版本和证据引用均在 `CalibrationCase`/`CalibrationEvidence` | 已验证 |
| AC-02 | `cargo test calibration::tests::independent_holdout_covers_all_five_known_outcome_classes` | 五类已知结果均执行两次，保留实际结论和环境故障状态 | 已验证 |
| AC-03 | `cargo test calibration::tests::metrics_are_separate_and_release_gate_does_not_claim_reliability` | 五类指标分别保存分子、分母和受影响案例，过度弃判分母为 4 | 已验证 |
| AC-04 | `cargo test calibration::tests::full_agent_path_repeats_without_label_leakage` | 所有案例经过完整 Agent 报告链路；合法恢复仍保留首次有效失败 | 已验证 |
| AC-05 | `cargo test calibration::tests::metrics_are_separate_and_release_gate_does_not_claim_reliability` | 门禁为 `insufficient_evidence`，不放行统计可靠性或生产泛化 | 已验证门禁阻断 |

## 4. 发布门禁

当前发布决定是：`insufficient_evidence`。

原因：保留集没有发现已知误判，但只有 5 个受控案例、每案 2 次，且没有真实客户 OMP 执行、人工争议复核和生产分布证据。因此本结果只能说明已覆盖案例的规则链路一致，不能证明生产误判率、统计置信区间或全部业务工作流可靠。

出现以下任一情况时，必须阻止受影响的确定结论发布：

1. 已知严重漏判或可复现误判；
2. 重复执行结论不一致；
3. Agent 路径、权限、工作区或证据链缺失；
4. 真实执行、人工复核或必要环境信息未提供。

## 5. 交接给 #35

后续端到端交付必须接入真实服务执行器，保存脱敏请求/响应和运行环境，复用本校准对象的版本、记录、证据和门禁字段。#35 不得把本受控保留集结果写成首版产品已经达到统计可靠性标准。
