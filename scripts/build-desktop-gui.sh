#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
gui_dir="$root_dir/prototypes/agent-check-desktop"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "桌面 GUI 发行包仅支持在 macOS 上构建" >&2
  exit 1
fi

if ! command -v xcrun >/dev/null 2>&1; then
  echo "构建桌面 GUI 需要 Xcode Command Line Tools" >&2
  exit 1
fi

bash "$gui_dir/build.sh"
printf '桌面包：%s\n' "$gui_dir/build/AgentCheck.app"
