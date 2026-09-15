# AgentCheck 单文件 CLI

每个平台单独交付一个压缩包。解压后只有 `agentcheck`，Windows 对应 `agentcheck.exe`。客户不需要另装 OhMyPi、Node.js、Bun 或准备规则文件。检测时需要访问客户的模型接口，组件释放过程不访问网络。

## 选择平台

| 运行环境 | 编译目标 | 包内唯一文件 | 桌面端 |
| --- | --- | --- | --- |
| macOS arm64 | aarch64-apple-darwin | agentcheck | 内置 macOS App，当前 GUI 要求 macOS 14 或以上 |
| Linux x86_64 | x86_64-unknown-linux-gnu | agentcheck | 当前无 Linux GUI，自动使用命令行 |
| Linux arm64 | aarch64-unknown-linux-gnu | agentcheck | 当前无 Linux GUI，自动使用命令行 |
| Windows x86_64 | x86_64-pc-windows-msvc 或 x86_64-pc-windows-gnu | agentcheck.exe | 当前无 Windows GUI，自动使用命令行 |

Linux 包面向使用 glibc 的系统，不声明兼容 Alpine/musl。交叉编译成功不代表已经在相应平台运行验证；各平台的验收结果以单独的验证记录为准。

## 使用

```bash
./agentcheck --help
./agentcheck --version
./agentcheck --runtime-check
export MODEL_API_KEY='客户授权的密钥'
./agentcheck \
  --url 'https://service.example.test/v1/chat/completions' \
  --model 'model-id' \
  --no-gui \
  --report-dir ./agentcheck-report \
  --html ./agentcheck-report/report.html
```

Windows PowerShell：

```powershell
$env:MODEL_API_KEY = '客户授权的密钥'
.\agentcheck.exe --runtime-check
.\agentcheck.exe --url 'https://service.example.test/v1/chat/completions' --model 'model-id' --no-gui --report-dir .\agentcheck-report --html .\agentcheck-report\report.html
```

也可以通过 `--api-key` 传密钥，但命令可能被终端历史记录保存。报告目录包含检测记录、每个模块的输入输出、OhMyPi 的分析结果和最终 HTML。OhMyPi 使用客户要检测的同一个模型。分析失败或证据不足会明确保留为未能判断，不会伪装成检测通过。

`--no-gui` 强制命令行；`--gui` 请求打开桌面端，两者不能同时使用。默认根据桌面环境自动选择。SSH/CI 默认不会打开桌面端；没有对应 GUI、GUI 释放失败或启动后立即退出时，继续命令行检测。CLI 等待桌面端确认窗口已显示，超过 10 秒仍未确认就结束本次桌面端启动并回到命令行。GUI 不继承 CLI 的输入输出管道，因此脚本不会等待窗口关闭。GUI 调用同一个 CLI 的 `--no-gui` 流程，模型信息、检测范围和报告路径均继续传递，不会重复弹窗。

## 内部文件

CLI 本体和判定规则直接编译在程序中。首次需要 OMP 或 GUI 时才释放对应组件；纯 CLI 不释放 GUI。规则和 HTML 生成不依赖旁边的目录或源码仓库。

| 平台 | 默认缓存位置 |
| --- | --- |
| macOS | `~/Library/Caches/AgentCheck` |
| Linux | `$XDG_CACHE_HOME/agentcheck`，未设置时为 `~/.cache/agentcheck` |
| Windows | `%LOCALAPPDATA%\AgentCheck\Cache` |

可以设置 `AGENTCHECK_CACHE_DIR` 指定可写、允许执行程序的绝对路径。没有可用用户目录时必须设置此项；挂载了 `noexec` 的目录不能启动 OMP。Unix 的应用缓存目录使用当前用户私有权限；Windows 默认目录继承用户配置目录权限，指定公共共享目录前应检查访问权限。

每个组件按目标平台和文件校验值隔离。程序在启动时校验缓存，用进程锁避免重复释放，在临时目录写完并校验后才切换；损坏的旧目录会保留在 `.stale-*` 下，不覆盖正在运行的程序。确认没有检测进程后，可以手动清理旧缓存以回收空间。

`--help` 和 `--version` 不创建缓存。`--runtime-check` 不访问模型，会执行内置 OMP 的版本命令并检查可用 GUI 的释放结果；它不是实际模型检测，也不代表 GUI 窗口已经验证。使用 `--licenses` 查看内置第三方许可声明。`OMP_BIN` 仍可显式指定外部分析程序用于排查问题。

## 本地构建

```bash
bash scripts/package-release.sh aarch64-apple-darwin
bash scripts/package-release.sh aarch64-unknown-linux-gnu
bash scripts/package-release.sh x86_64-unknown-linux-gnu
bash scripts/package-release.sh x86_64-pc-windows-msvc
```

需要 Rust 目标工具链及对应链接器。macOS GUI 在 macOS 上使用 Xcode Command Line Tools 构建。Windows 可从 Git Bash/MSYS2 执行打包脚本；脚本需要 `curl`、`file`、`zip` 和 SHA-256 工具。解压后的 CLI 本身不依赖这些构建工具，不调用系统 `tar`。

脚本固定使用 OMP `v18.1.20`，按上游校验文件校验可执行程序，并内置许可证及第三方声明。缓存好的上游文件可通过 `OMP_ASSET_DIR` 提供；自定义 `OMP_BINARY_PATH` 需要提供匹配的 `OMP_SHA256`。它们都是构建配置，不是客户运行要求。

Linux 交叉编译可设置 `AGENTCHECK_CARGO_COMMAND=zigbuild`，Windows GNU 可配置 `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`。`AGENTCHECK_SKIP_GUI=1` 可以构建无 GUI 的 macOS 包；`AGENTCHECK_GUI_APP` 可以指定已构建并签名的 App。不得把其他平台的可执行程序打入当前包，脚本同时检查操作系统格式和 CPU 架构。

压缩包旁边生成 `.sha256` 校验文件，校验文件不放进客户解压目录。默认构建只做本地签名，不代表已经通过 Apple 公证、Windows 发布签名或客户安全软件检查。正式外发前仍需在干净目标系统验证启动和网络访问。

## 验收入口

在目标系统执行 `bash scripts/verify-single-file.sh /path/to/agentcheck /new/output/directory`。Windows 使用 `powershell -File scripts/verify-single-file.ps1 -Binary .\agentcheck.exe`。设置 `MODEL_URL`、`MODEL_ID`、`MODEL_API_KEY` 后，脚本还会执行一次接入模块的真实检测并保存报告。

Linux 的本地隔离环境可通过 `scripts/verification/Dockerfile` 构建；镜像只有系统组件和 HTTPS 证书，不预装 OMP、Node.js 或 Bun。不要把在模拟器内的结果直接当成原生系统通过。2026-09-15 的 x86_64 模拟验证中，上游 OMP 本身出现 JavaScriptCore 内存分配失败，原生 x86_64 验收仍需补充。
