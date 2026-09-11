# Issue #35：首版发行候选与端到端验收记录

记录日期：2026-09-11  
集成基线：`lbl/agent-check`  
验收版本：Cargo package `0.1.0`  
候选状态：`release-candidate`

## 交付入口

从干净 checkout 执行：

```bash
RELEASE_OUTPUT_DIR=./dist ./scripts/package-release.sh
tar -xzf ./dist/llm-capability-doctor-v0.1.0-<target>.tar.gz -C /tmp/agentcheck-release
sha256sum -c SHA256SUMS
./llm-capability-doctor --version
```

脚本只打包 release 二进制、`README.md`、`VERSION`、`BUILD.txt` 和 `SHA256SUMS`，不带源码、凭据、`target/` 或开发工作区文件。

## AC 记录

| AC | 检查方法 | 实际结果 | 状态 |
| --- | --- | --- | --- |
| AC-01 | `cargo build --release --locked`；运行打包脚本；包内 `--version` | 当前构建目标可安装、可识别 `0.1.0`；其他目标不作默认支持声明 | 已验证 |
| AC-02 | `cargo test`；完整执行器 fixed-list 测试；CLI 选择/停止测试 | 默认入口使用六模块完整执行器；规格 27 个固定样本、能力 240 单元、Agent 10 场景、基线 BC01-BC14，性能按固定正式计划；专项和停止范围保留 | 已验证（受服务可用性影响） |
| AC-03 | GUI 单测：桌面、无桌面、CI、启动失败；CLI `--no-gui` | GUI 不可用或启动失败保留 CLI；界面失败不改写模型结论 | 已验证 |
| AC-04 | 检查 `design/issue-33`、`design/issue-34` 等价材料及版本 | 回归测试通过；#34 发布门禁仍是 `insufficient_evidence`，因此候选不宣称生产可靠性 | 候选门禁保留 |
| AC-05 | 包内 `VERSION`、`SHA256SUMS`、README 和本记录 | 版本、包、说明一致；凭据只来自环境变量；限制和结果归属明确 | 已验证 |

## 真实服务执行记录

正式真实服务执行需要用户提供已授权服务 URL、模型和密钥，不能用仓库 fixture 冒充客户测量。本版本会把脱敏请求/响应、模块计划数、实际执行数、报告和运行 ID 写入统一 JSON；网络、认证、限流和协议异常不归因模型。

## 发布评审

- 已纳入：Rust CLI、Chat Completions 真实传输、六模块固定清单入口、统一记录、GUI/CLI 分流、可重复发行打包。
- 明确限制：当前 Chat Completions 适配器不携带供应商专属工具 schema；Agent 没有观察到工具/权限/产物事实时保持 `inconclusive`。
- #34 结论：受控保留集未发现已知误判，但证据不足以证明生产可靠性，发布状态必须保持候选。
- 不纳入：费用预算、断点续跑、复杂数据生命周期、全业务验证和未经确认的平台扩展。

因此本记录支持“可安装发行候选”交付，不把候选包写成生产发布通过。Issue #35 只有在目标服务的真实验收证据归档后才能升级为最终发布结论。
