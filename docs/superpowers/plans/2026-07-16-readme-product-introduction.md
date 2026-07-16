# README Product Introduction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the existing technical-manual README with the approved concise Chinese product introduction.

**Architecture:** Keep the document self-contained and renderer-neutral by using only standard Markdown headings, a blockquote, an ordered list, emphasis, and a table. Preserve every user-provided capability and workflow statement without retaining old technical-manual content or adding unsupported product claims.

**Tech Stack:** CommonMark/GitHub Flavored Markdown

---

### Task 1: Replace and verify the product README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Replace the complete README**

Set `README.md` to the following exact content:

```markdown
# LLM Capability Doctor（大模型能力诊断工具）

> 面向智能体平台模型接入场景的一站式、全维度大模型适配性诊断工具。

## 工具开发背景

在智能体平台版本的实际对接落地过程中，客户侧接入的大模型类型繁杂、能力参差不齐，无法提前确保模型能够被智能体平台正常兼容调用。

为高效、标准化地核验客户大模型的适配性，特开发本诊断工具，实现一站式、全维度检测。

## 核心使用流程

仅需一个检测脚本，配合本地预装的专属 Skill 工具，即可完成全流程操作：

1. **拷贝检测脚本**：将单个检测脚本拷贝至客户现场。
2. **执行自动检测**：运行一条命令，启动全自动检测。
3. **导出执行日志**：检测完成后，导出完整执行日志并传回本地。
4. **生成诊断报告**：通过自研 Skill 工具解析日志，一键生成标准化的大模型体检 HTML 报告。

> **全流程仅需两步：执行脚本、解析日志。**

依托可视化检测报告，可将诊断结果作为客观依据，与客户进行高效沟通。

## 设计核心考量

| 核心考量 | 设计说明 |
| --- | --- |
| **检测维度全面** | 覆盖网络连通性、上下文长度上限、并发性能、工具调用能力、思维链能力、敏感词过滤策略等关键指标。 |
| **现场使用极简** | 仅需执行脚本、解析日志，无需复杂部署与配置。 |
| **结果分级清晰** | 区分核心必过检测项与次要优化检查项，重点突出。 |
| **报告证据完整** | 清晰呈现每项检测的逻辑说明、输入参数与实际输出结果，确保结论客观、可追溯。 |
```

- [ ] **Step 2: Verify the required structure and content**

Run:

```bash
rg -n '^# LLM Capability Doctor|^## 工具开发背景|^## 核心使用流程|^## 设计核心考量|网络连通性|上下文长度上限|并发性能|工具调用能力|思维链能力|敏感词过滤策略|核心必过检测项|次要优化检查项' README.md
```

Expected: all three sections, the project title, all six diagnostic dimensions, and both result tiers are found.

- [ ] **Step 3: Check Markdown whitespace and the final diff**

Run:

```bash
git diff --check
git diff -- README.md
```

Expected: `git diff --check` exits with status 0; the README diff shows complete removal of the old technical manual and only the approved product introduction as the replacement.

- [ ] **Step 4: Commit the README replacement**

```bash
git add README.md docs/superpowers/plans/2026-07-16-readme-product-introduction.md
git commit -m "docs: replace README with product introduction"
```
