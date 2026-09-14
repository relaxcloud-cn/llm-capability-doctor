#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root_dir/Cargo.toml" | head -n 1)"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
output_dir="${RELEASE_OUTPUT_DIR:-$root_dir/dist}"
package_name="llm-capability-doctor-v${version}-${target}"
omp_version="${OMP_VERSION:-v18.1.20}"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

case "$target" in
  x86_64-pc-windows-msvc|x86_64-pc-windows-gnu) omp_asset="omp-windows-x64"; omp_name="omp.exe" ;;
  aarch64-apple-darwin) omp_asset="omp-darwin-arm64"; omp_name="omp" ;;
  x86_64-unknown-linux-gnu) omp_asset="omp-linux-x64"; omp_name="omp" ;;
  aarch64-unknown-linux-gnu) omp_asset="omp-linux-arm64"; omp_name="omp" ;;
  *)
    echo "不支持的发行目标：$target（支持 Windows x86_64、macOS arm64、Linux x86_64、Linux arm64）" >&2
    exit 1
    ;;
esac

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_architecture() {
  local file_path="$1"
  local target_name="$2"
  local label="$3"
  local file_info
  file_info="$(file -b "$file_path")"
  case "$target_name" in
    x86_64-pc-windows-msvc|x86_64-pc-windows-gnu|x86_64-unknown-linux-gnu)
      [[ "$file_info" =~ (x86-64|x86_64|AMD64) ]] || {
        echo "$label 架构不匹配：目标 $target_name，实际 $file_info" >&2
        exit 1
      }
      ;;
    aarch64-apple-darwin)
      [[ "$file_info" =~ arm64 ]] || {
        echo "$label 架构不匹配：目标 $target_name，实际 $file_info" >&2
        exit 1
      }
      ;;
    aarch64-unknown-linux-gnu)
      [[ "$file_info" =~ (aarch64|ARM64) ]] || {
        echo "$label 架构不匹配：目标 $target_name，实际 $file_info" >&2
        exit 1
      }
      ;;
  esac
}

mkdir -p "$output_dir"
cargo build --manifest-path "$root_dir/Cargo.toml" --release --locked --target "$target"

binary="$root_dir/target/$target/release/llm-capability-doctor"
if [[ ! -x "$binary" ]]; then
  echo "release 二进制不存在：$binary" >&2
  exit 1
fi
verify_architecture "$binary" "$target" "CLI"

omp_url="https://github.com/can1357/oh-my-pi/releases/download/${omp_version}/${omp_asset}"
omp_checksums_url="https://github.com/can1357/oh-my-pi/releases/download/${omp_version}/SHA256SUMS.txt"
omp_download="$stage_dir/$omp_name"
if [[ -n "${OMP_BINARY_PATH:-}" ]]; then
  if [[ ! -f "$OMP_BINARY_PATH" ]]; then
    echo "OMP_BINARY_PATH 文件不存在：$OMP_BINARY_PATH" >&2
    exit 1
  fi
  install -m 0755 "$OMP_BINARY_PATH" "$omp_download"
  expected_omp_sha="$(hash_file "$omp_download")"
else
  curl --fail --location --retry 3 --output "$omp_download" "$omp_url"
  expected_omp_sha="$(curl --fail --location --retry 3 "$omp_checksums_url" | awk -v name="$omp_asset" '$2 == name {print $1}')"
fi
actual_omp_sha="$(hash_file "$omp_download")"
if [[ -z "$expected_omp_sha" || "$actual_omp_sha" != "$expected_omp_sha" ]]; then
  echo "OhMyPi 校验失败：$omp_asset" >&2
  exit 1
fi
chmod 0755 "$omp_download"
verify_architecture "$omp_download" "$target" "OhMyPi omp"

cli_name="llm-capability-doctor"
if [[ "$target" == *-windows-* ]]; then
  cli_name="llm-capability-doctor.exe"
fi
install -m 0755 "$binary" "$stage_dir/$cli_name"
install -m 0644 "$root_dir/release/README.md" "$stage_dir/README.md"
mkdir -p "$stage_dir/rules"
install -m 0644 "$root_dir/rules/agentcheck-evaluation.json" "$stage_dir/rules/agentcheck-evaluation.json"
package_items=("$cli_name" "$omp_name" README.md VERSION BUILD.txt SHA256SUMS rules)
if [[ "$target" == *-apple-darwin && "$(uname -s)" == "Darwin" ]]; then
  "$root_dir/scripts/build-desktop-gui.sh" >/dev/null
  cp -R "$root_dir/prototypes/agent-check-desktop/build/AgentCheck.app" "$stage_dir/AgentCheck.app"
  package_items+=(AgentCheck.app)
fi
printf '%s\n' "$version" > "$stage_dir/VERSION"
printf '%s\n' "目标：$target" "构建：$(date -u '+%Y-%m-%dT%H:%M:%SZ')" > "$stage_dir/BUILD.txt"
(cd "$stage_dir" && printf '%s  %s\n' "$(hash_file "$cli_name")" "$cli_name" > SHA256SUMS && printf '%s  %s\n' "$(hash_file "$omp_name")" "$omp_name" >> SHA256SUMS)

archive="$output_dir/${package_name}.tar.gz"
tar -C "$stage_dir" -czf "$archive" "${package_items[@]}"
(cd "$output_dir" && hash_file "$(basename "$archive")" > "$(basename "$archive").sha256")
printf '发行包：%s\n' "$archive"
printf '版本：%s\n' "$version"
printf '目标：%s\n' "$target"
