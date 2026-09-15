# 单文件交付验证记录（2026-09-15）

关联 Issue #120；开发基于 `lbl/agent-check`，不使用旧 `main` 代码，没有使用 GitHub Actions。

## 当前状态

四个平台的单文件构建代码已完成。Issue #120 暂不关闭，PR 保持草稿：Linux x86_64 与 Windows x86_64 的原生运行验收尚未完成。

| 平台 | 编译和单文件打包 | 运行验证 | 真实模型报告 |
| --- | --- | --- | --- |
| macOS arm64 | 通过，包内只有 agentcheck | 本机执行；内置 OMP 可启动，GUI 签名和自检通过 | 已完成接入模块检测及内置 OMP 分析，生成 HTML |
| Linux arm64 | 通过，包内只有 agentcheck | Debian 12 arm64 干净容器执行，无预装 OMP、Node.js、Bun | 已完成接入模块检测及内置 OMP 分析，生成 HTML |
| Linux x86_64 | 通过，包内只有 agentcheck | CLI 在 Debian 12 x86_64 模拟环境中可启动；OMP 异常退出；原生待测 | 未通过，不计入已完成 |
| Windows x86_64 | GNU 目标编译通过，ZIP 内只有 agentcheck.exe | PE 架构和 DLL 依赖检查完成；未在 Windows 原生环境运行 | 未执行 |

内置 OMP 固定为 v18.1.20，四个平台均按上游 SHA-256 校验通过；许可证和第三方声明一同内置。Windows CLI 与 OMP 的导入检查没有发现需要另外交付的第三方 DLL，但此检查不能替代 Windows 原生启动测试。

## 已验证行为

- Rust 单元及进程测试共 106 项通过，`cargo fmt --all --check` 和严格 Clippy 检查通过。
- 缓存首次释放、重复使用、损坏修复、并发启动，以及错误平台、非法路径、符号链接和不完整组件拒绝均有测试。
- `--help`、`--version` 不需要模型配置，也不释放组件；空目录中只有单个 CLI 时仍能找到内置规则和 OMP。
- 分析程序不可用时仍保留检测 JSON 和 HTML，分析结果明确记为未能判断。GUI 不可用时继续 CLI；启动 GUI 不会阻塞调用方的输出管道。
- macOS GUI 自检 127 项通过；从包内释放的 App 通过本地签名校验。CLI 启动时须收到窗口已显示的确认，而不是仅凭进程存在判断成功。

真实检测只选择 `ingress`，用于验证“CLI 请求、保存证据、内置 OMP 分析、HTML 生成”这条流程；不是本次重新验收全部模型检测项，也不是生产可靠性声明。报告的分析程序状态为 `completed_normalized`。客户地址、密钥及原始报告不提交到公共仓库。

## x86_64 模拟环境问题

本机是 Apple Silicon，Linux x86_64 通过 QEMU 用户态模拟执行。Rust CLI 可正常运行，但内置 OMP 在启动时返回 SIGABRT。

将校验通过的上游 `omp-linux-x64` 单独复制进同一个干净容器，不经过 AgentCheck 封装，再运行 `--version`，同样触发 JavaScriptCore 的 `MemoryExhaustion`，运行信息为 Bun v1.4.2、glibc v2.36。这说明此次异常不以单文件封装为前提；它也不能证明原生 Linux x86_64 已可用。

不通过删除分析步骤、改用外部 OMP 或只验证文件存在来把本项标记为通过。下一步必须在原生 Linux x86_64 运行验收脚本并保留报告。

## 外发前待办

1. 原生 Linux x86_64 完成内置 OMP 启动和真实模型报告验证。
2. 原生 Windows x86_64 使用 PowerShell 验收脚本完成启动、缓存复用和真实模型报告验证。
3. 根据正式发行渠道完成 macOS 公证、Windows 发布签名以及安全软件检查；目前的本地构建不代表这些项目已通过。
