#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root_dir/Cargo.toml" | head -n 1)"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
output_dir="${RELEASE_OUTPUT_DIR:-$root_dir/dist}"
omp_version="${OMP_VERSION:-v18.1.20}"
cargo_command="${AGENTCHECK_CARGO_COMMAND:-build}"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

case "$target" in
  x86_64-pc-windows-msvc|x86_64-pc-windows-gnu)
    omp_asset="omp-windows-x64.exe"; omp_name="omp.exe"; cli_name="agentcheck.exe"; cargo_binary="llm-capability-doctor.exe" ;;
  aarch64-apple-darwin)
    omp_asset="omp-darwin-arm64"; omp_name="omp"; cli_name="agentcheck"; cargo_binary="llm-capability-doctor" ;;
  x86_64-unknown-linux-gnu)
    omp_asset="omp-linux-x64"; omp_name="omp"; cli_name="agentcheck"; cargo_binary="llm-capability-doctor" ;;
  aarch64-unknown-linux-gnu)
    omp_asset="omp-linux-arm64"; omp_name="omp"; cli_name="agentcheck"; cargo_binary="llm-capability-doctor" ;;
  *) echo "不支持的目标：$target；支持 Windows x86_64、macOS arm64、Linux x86_64 和 Linux arm64" >&2; exit 1 ;;
esac
case "$cargo_command" in build|zigbuild) ;; *) echo "AGENTCHECK_CARGO_COMMAND 只能是 build 或 zigbuild" >&2; exit 1 ;; esac

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

verify_architecture() {
  local info
  info="$(file -b "$1")"
  case "$target" in
    x86_64-pc-windows-*) [[ "$info" == *PE32+* && "$info" =~ (x86-64|x86_64|AMD64) ]] ;;
    aarch64-apple-darwin) [[ "$info" == *Mach-O* && "$info" == *arm64* ]] ;;
    x86_64-unknown-linux-gnu) [[ "$info" == *ELF* && "$info" =~ (x86-64|x86_64) ]] ;;
    aarch64-unknown-linux-gnu) [[ "$info" == *ELF* && "$info" =~ (aarch64|ARM64) ]] ;;
  esac || { echo "目标平台不匹配：$1；目标 $target；实际 $info" >&2; exit 1; }
}

asset_dir="$stage_dir/downloads"
mkdir -p "$asset_dir" "$stage_dir/bundle/runtime" "$stage_dir/delivery" "$output_dir"
output_dir="$(cd "$output_dir" && pwd)"
for asset in "$omp_asset" SHA256SUMS.txt LICENSE THIRD-PARTY-NOTICES.txt; do
  if [[ "$asset" == "$omp_asset" && -n "${OMP_BINARY_PATH:-}" ]]; then
    cp "$OMP_BINARY_PATH" "$asset_dir/$asset"
  elif [[ -n "${OMP_ASSET_DIR:-}" ]]; then
    cp "$OMP_ASSET_DIR/$asset" "$asset_dir/$asset"
  else
    curl --fail --location --retry 3 --output "$asset_dir/$asset" "https://github.com/can1357/oh-my-pi/releases/download/${omp_version}/${asset}"
  fi
done
expected_sha="${OMP_SHA256:-$(awk -v asset="$omp_asset" '$2 == asset { print $1 }' "$asset_dir/SHA256SUMS.txt")}"
if [[ -z "$expected_sha" || "$(hash_file "$asset_dir/$omp_asset")" != "$expected_sha" ]]; then
  echo "OhMyPi 校验失败：$omp_asset；本地自定义程序必须提供 OMP_SHA256" >&2
  exit 1
fi
verify_architecture "$asset_dir/$omp_asset"
install -m 0755 "$asset_dir/$omp_asset" "$stage_dir/bundle/runtime/$omp_name"
install -m 0644 "$asset_dir/LICENSE" "$stage_dir/bundle/runtime/OMP-LICENSE.txt"
install -m 0644 "$asset_dir/THIRD-PARTY-NOTICES.txt" "$stage_dir/bundle/runtime/THIRD-PARTY-NOTICES.txt"
printf '%s\n' "$target" > "$stage_dir/bundle/TARGET"
printf '%s\n' "$omp_version" > "$stage_dir/bundle/runtime/OMP-VERSION.txt"

if [[ "$target" == aarch64-apple-darwin && "${AGENTCHECK_SKIP_GUI:-0}" != 1 ]]; then
  gui_app="${AGENTCHECK_GUI_APP:-}"
  if [[ -z "$gui_app" ]]; then
    if [[ "$(uname -s)" != Darwin ]]; then
      echo "构建 macOS 桌面组件需要 macOS 或 AGENTCHECK_GUI_APP；只打 CLI 可设置 AGENTCHECK_SKIP_GUI=1" >&2
      exit 1
    fi
    bash "$root_dir/scripts/build-desktop-gui.sh" >/dev/null
    gui_app="$root_dir/prototypes/agent-check-desktop/build/AgentCheck.app"
  fi
  verify_architecture "$gui_app/Contents/MacOS/AgentCheck"
  mkdir -p "$stage_dir/bundle/gui"
  cp -R "$gui_app" "$stage_dir/bundle/gui/AgentCheck.app"
fi

AGENTCHECK_BUNDLE_DIR="$stage_dir/bundle" cargo "$cargo_command" --manifest-path "$root_dir/Cargo.toml" \
  --release --locked --features bundled-runtime --target "$target"
target_dir="${CARGO_TARGET_DIR:-$root_dir/target}"
binary="$target_dir/$target/release/$cargo_binary"
[[ -f "$binary" ]] || { echo "没有找到编译产物：$binary" >&2; exit 1; }
verify_architecture "$binary"
install -m 0755 "$binary" "$stage_dir/delivery/$cli_name"

package_name="agentcheck-v${version}-${target}"
if [[ "$target" == *-windows-* ]]; then
  archive="$output_dir/${package_name}.zip"
  (cd "$stage_dir/delivery" && zip -q "$stage_dir/package.zip" "$cli_name")
  mv "$stage_dir/package.zip" "$archive"
else
  archive="$output_dir/${package_name}.tar.gz"
  COPYFILE_DISABLE=1 tar -C "$stage_dir/delivery" -czf "$archive" "$cli_name"
fi
printf '%s  %s\n' "$(hash_file "$archive")" "$(basename "$archive")" > "$archive.sha256"
printf '发行包：%s\n包内唯一文件：%s\n目标：%s\n' "$archive" "$cli_name" "$target"
