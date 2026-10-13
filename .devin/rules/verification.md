---
trigger: always_on
---

# 本机验证与交付

- macOS 并发回归测试包含同一进程内的 HTTP 客户端和服务端，需足够的文件描述符。使用 `ulimit -n 4096 && cargo test --locked`；随后执行 `cargo clippy --locked --all-targets -- -D warnings` 和 `cargo fmt --check`。shell 上限为 256 时，128 并发用例会触发 `local_resource_limit`，不要为通过测试移除生产资源保护。
- 新构建与客户 ZIP 内的可执行文件统一命名为 `agent-check`；macOS 构建命令为 `./scripts/build-macos.sh`，产物为 `dist/agent-check`。
- `python3 scripts/package-customer.py --output <新 ZIP 路径>` 会核验版本、校验和、解压权限和全新缓存离线自检；不覆盖已有 ZIP。
- `python3 scripts/verify-runtime.py <可执行文件> --output <JSON 路径>` 使用独立缓存验证释放、缓存重用、内容及执行权限修复，不请求模型。
