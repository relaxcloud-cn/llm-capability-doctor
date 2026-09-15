#!/usr/bin/env bash
set -euo pipefail

# Run on the target OS. A successful cross-build alone is not runtime acceptance.
binary="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
report_root="${2:-$(mktemp -d)}"
mkdir -p "$report_root"
report_root="$(cd "$report_root" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
name="$(basename "$binary")"
install -m 0755 "$binary" "$work/$name"
cd "$work"
unset OMP_BIN AGENTCHECK_GUI_PATH
export AGENTCHECK_CACHE_DIR="$report_root/cache"
export HOME="$report_root/home"
export XDG_CONFIG_HOME="$HOME/.config"
export PI_CODING_AGENT_DIR="$HOME/.omp/agent"
mkdir -p "$HOME"
"./$name" --version > "$report_root/version.txt"
[[ ! -e "$AGENTCHECK_CACHE_DIR" ]] || { echo '版本查询不应释放组件' >&2; exit 1; }
"./$name" --runtime-check > "$report_root/runtime-first.json"
"./$name" --runtime-check > "$report_root/runtime-reused.json"
"./$name" --no-gui --help > "$report_root/help.txt"
"./$name" --licenses > "$report_root/licenses.txt"

if [[ -n "${MODEL_URL:-}" && -n "${MODEL_ID:-}" && -n "${MODEL_API_KEY:-}" ]]; then
  "./$name" --url "$MODEL_URL" --model "$MODEL_ID" --no-gui \
    --modules ingress --timeout-seconds 90 \
    --report-dir "$report_root/model" --html "$report_root/model/report.html" \
    > "$report_root/model-output.txt" 2> "$report_root/model-progress.txt"
fi
printf '检查结果：%s\n' "$report_root"
