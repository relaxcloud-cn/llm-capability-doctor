# 大模型能力诊断工具项目介绍 PPT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 生成一份面向 FDE 交付人员的 8 页政务风项目介绍 PPT，完整覆盖开发背景、46 项检测的 8 个大类和保姆级使用流程。

**Architecture:** 每页主体先由 Codex Imagen 独立生成一张 16:9 中文整页图片，再使用 `@oai/artifact-tool` 将图片全幅嵌入 PPTX 并写入中文演讲备注。所有中间脚本和检查记录放在项目的 `.superpowers/tmp/model-doctor-ppt-20260824/`，最终 PPTX 和 8 张图片放在 `materials/model-capability-doctor-ppt/`。

**Tech Stack:** Codex Imagen、Node.js ES Modules、`@oai/artifact-tool`、PowerPoint 渲染检查工具。

---

### Task 1: 生成 8 张政务风整页图片

**Files:**
- Create: `materials/model-capability-doctor-ppt/images/slide-01.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-02.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-03.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-04.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-05.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-06.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-07.png`
- Create: `materials/model-capability-doctor-ppt/images/slide-08.png`
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/prompt-records.txt`

- [ ] **Step 1: 写出逐页精确文案和提示词**

  为封面、开发背景、工具价值、检测范围、系统与架构准备、连接信息准备、命令执行、结果交付分别写一条 Imagen 提示词。所有提示词固定 16:9、简体中文、浅灰背景、藏蓝模块、深红标题、金色分隔，不生成 Logo、水印或角标。

- [ ] **Step 2: 逐页调用 Codex Imagen**

  每页单独调用一次内置图片生成工具，生成失败时只缩短该页文字并重试 Imagen。禁止使用 HTML、SVG、Pillow、matplotlib 或本地程序补画页面主体。

- [ ] **Step 3: 保存并检查图片尺寸**

  将每张成功图片复制到 `materials/model-capability-doctor-ppt/images/`，运行：

  ```bash
  file materials/model-capability-doctor-ppt/images/slide-*.png
  ```

  预期：共 8 个可读取的宽屏 PNG 文件。

- [ ] **Step 4: 逐页视觉检查**

  使用本地图片查看工具检查每张图，确认标题、主体结构、底部价值句清楚，不存在严重错字、裁切、重叠、Logo、水印或角标。

### Task 2: 合成带中文备注的 PPTX

**Files:**
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/build-deck.mjs`
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/source-notes.txt`
- Create: `materials/model-capability-doctor-ppt/大模型能力诊断工具项目介绍.pptx`

- [ ] **Step 1: 加载工作区运行时依赖**

  调用工作区依赖加载器，取得 `RUNTIME_NODE`、`RUNTIME_NODE_MODULES` 和 `RUNTIME_BIN_DIR` 的绝对路径，并在构建命令中逐项设置。

- [ ] **Step 2: 标记一次创建操作**

  运行：

  ```bash
  "$RUNTIME_NODE" "$SKILL_DIR/container_tools/mark_artifact_operation_started.mjs" --operation-kind create --expected-output-count 1 --output-format pptx
  ```

  预期：命令退出码为 0；整个创建流程只运行一次。

- [ ] **Step 3: 编写 Artifact Tool 构建脚本**

  脚本创建 `1280x720` 演示文稿，为每页插入对应 PNG 并设置为全幅 `cover`，随后写入中文 speaker notes。备注包含本页讲解重点、客户价值、交付注意事项和讲解收束。

- [ ] **Step 4: 导出 PPTX**

  运行：

  ```bash
  "$RUNTIME_NODE" .superpowers/tmp/model-doctor-ppt-20260824/build-deck.mjs
  ```

  预期：生成 `materials/model-capability-doctor-ppt/大模型能力诊断工具项目介绍.pptx`，页数为 8。

### Task 3: 渲染与验收

**Files:**
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/rendered/slide-1.png` through `slide-8.png`
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/montage.png`
- Create: `.superpowers/tmp/model-doctor-ppt-20260824/qa-ledger.txt`

- [ ] **Step 1: 渲染全部幻灯片**

  运行：

  ```bash
  /Users/libolun/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3 \
    "$SKILL_DIR/container_tools/render_slides.py" \
    materials/model-capability-doctor-ppt/大模型能力诊断工具项目介绍.pptx
  ```

  预期：在 PPTX 同级渲染目录生成 8 张逐页 PNG。

- [ ] **Step 2: 创建全局蒙版并逐页检查**

  创建 8 页蒙版用于检查节奏和一致性，再单独查看 8 张渲染图，核对图片未被裁切、没有空白页、顺序正确、文字可理解。

- [ ] **Step 3: 检查画布溢出与 PPTX 契约**

  运行：

  ```bash
  /Users/libolun/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/bin/python3 \
    "$SKILL_DIR/container_tools/slides_test.py" \
    materials/model-capability-doctor-ppt/大模型能力诊断工具项目介绍.pptx
  ```

  预期：无超出画布的对象；PPTX 共 8 页且每页均有备注。

- [ ] **Step 4: 记录最终验收结果**

  在 `qa-ledger.txt` 记录图片数量、PPTX 页数、备注页数、渲染页数和逐页视觉检查结论。只有全部通过后才交付最终路径。
