#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo 'This build recipe currently verifies macOS arm64 only.' >&2
  exit 2
fi
node scripts/prepare-runtime.mjs aarch64-apple-darwin
npm ci --prefix desktop
npm run package:mac --prefix desktop
python3 scripts/prepare-bundle.py aarch64-apple-darwin
AGENTCHECK_BUNDLE_DIR="$PWD/.cache/bundles/aarch64-apple-darwin" cargo build --release --features bundled-runtime
mkdir -p dist
cp target/release/agent-check dist/agent-check
shasum -a 256 dist/agent-check > dist/SHA256SUMS
printf '%s\n' 'Built dist/agent-check. Target-machine acceptance and distribution signing are separate release gates.'
