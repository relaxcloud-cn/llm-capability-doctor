#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root_dir/Cargo.toml" | head -n 1)"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
output_dir="${RELEASE_OUTPUT_DIR:-$root_dir/dist}"
package_name="llm-capability-doctor-v${version}-${target}"
stage_dir="$(mktemp -d)"
trap 'rm -rf "$stage_dir"' EXIT

mkdir -p "$output_dir"
cargo build --manifest-path "$root_dir/Cargo.toml" --release --locked --target "$target"

binary="$root_dir/target/$target/release/llm-capability-doctor"
if [[ ! -x "$binary" ]]; then
  echo "release 二进制不存在：$binary" >&2
  exit 1
fi

install -m 0755 "$binary" "$stage_dir/llm-capability-doctor"
install -m 0644 "$root_dir/release/README.md" "$stage_dir/README.md"
printf '%s\n' "$version" > "$stage_dir/VERSION"
printf '%s\n' "目标：$target" "构建：$(date -u '+%Y-%m-%dT%H:%M:%SZ')" > "$stage_dir/BUILD.txt"
(cd "$stage_dir" && sha256sum llm-capability-doctor > SHA256SUMS)

archive="$output_dir/${package_name}.tar.gz"
tar -C "$stage_dir" -czf "$archive" llm-capability-doctor README.md VERSION BUILD.txt SHA256SUMS
(cd "$output_dir" && sha256sum "$(basename "$archive")" > "$(basename "$archive").sha256")
printf '发行包：%s\n' "$archive"
printf '版本：%s\n' "$version"
printf '目标：%s\n' "$target"
