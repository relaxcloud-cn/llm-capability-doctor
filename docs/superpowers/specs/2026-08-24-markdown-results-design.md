# Markdown 检测结果设计

## 目标

在客户侧自分析完成后，除现有 JSON 外生成一个同目录的 Markdown 文件，供客户快速阅读和内部追溯。Markdown 不重新判定能力，所有 PASS/FAIL 仍来自被测模型；渲染层只投影已验证的自分析结果。

## 输出文件

- 输入：现有 `model-doctor-self-analysis.json` 对应的内存 artifact。
- 输出：与日志同目录的 `model-doctor-self-analysis.md`。
- 输出路径沿用 JSON 的碰撞安全策略，不覆盖已有文件。
- JSON 写入失败或 Markdown 写入失败都作为自分析输出错误处理；检测日志本身不被回滚。

## 文档结构

### 1. 标题和运行信息

包含报告标题、生成时间、请求模型、检测协议、分析来源和 46 项总计。Endpoint 只显示已脱敏的值，不显示 API Key。

### 2. 大分类结论

按现有 catalog 的 8 个分类输出一个表格：

| 能力分类 | 结论 | 具体原因 |
|---|---|---|
| 上下文 | 满足 | 6/6 项检测通过 |
| 工具调用 | 不满足 | 046 官方工具协议结构合规：模型判断工具调用结构不完整；049 工具失败恢复：分析批次不可用（具体错误） |

分类状态只有两种：

- `满足`：分类下的每个检测项都是已验证的模型 `PASS`。
- `不满足`：分类下存在模型 `FAIL` 或 `ANALYSIS_UNAVAILABLE`。

“具体原因”必须列出每个非 PASS 项的编号、名称和原因：

- `FAIL` 使用模型返回的 `failureCause`。
- `ANALYSIS_UNAVAILABLE` 使用批次错误或结果中的 limitation，并明确标注为分析不可用。
- 分类全部通过时显示通过数量，不编造额外原因。

### 3. 46 项明细表

主表保持适合扫描的宽度：

| 编号 | 分类 | 检测项 | 结果 | 原因 |
|---|---|---|---|---|

结果显示为 `PASS`、`FAIL` 或 `ANALYSIS_UNAVAILABLE`。PASS 行原因显示 `-`；FAIL 和不可用行显示简短原因。

### 4. 非通过项详情

仅为 `FAIL` 和 `ANALYSIS_UNAVAILABLE` 输出详情小节。每项包含：

- 检测编号、分类和名称；
- 结果和 `decisionSource`（可用结果应为 `TARGET_MODEL`）；
- 模型失败原因；
- 模型观察；
- 证据引用；
- 限制说明和分析不可用错误（如有）。

Markdown 表格中的 `|`、换行和空值必须转义或归一化，避免模型文本破坏文档结构。

## 数据和判定边界

- 分类归属和检测名称来自 `src/catalog.rs`，不从模型文本推断。
- 分类结论只读取 `validatedStatus` 和 `analysisState`。
- Markdown 不引入规则引擎，不恢复已删除的硬判定。
- 对不可用批次，分类只显示“不满足”，详情必须写清楚“分析不可用”，不能伪造模型 FAIL 原因。
- 已脱敏的 endpoint、日志路径和模型输出沿用现有 artifact 数据，不额外写入 API Key。

## 兼容性和失败处理

- 不带 `--self-analyze` 时不生成 Markdown。
- 带 `--self-analyze` 时 JSON 和 Markdown 使用同一次分析结果，避免两份文件结论不一致。
- 任一分析批次失败仍继续生成完整文件；对应检测项为 `ANALYSIS_UNAVAILABLE`。
- Markdown 文件写入采用临时文件和碰撞安全路径，避免留下半份报告。

## 测试

- 验证全通过时，8 个分类均显示“满足”，主表包含 46 项。
- 验证含 FAIL 时，分类显示“不满足”，原因包含编号、名称和 `failureCause`。
- 验证含不可用批次时，分类显示“不满足”，原因明确包含“分析不可用”和批次错误。
- 验证 Markdown 特殊字符转义、输出碰撞和脱敏字段。
- 保持现有 JSON、Rust 全量测试、Clippy、release 构建和 Python 测试通过。
