#!/bin/bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP="$ROOT/build/AgentCheck.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$ROOT/build/module-cache"
xcrun swiftc -swift-version 5 -O -module-cache-path "$ROOT/build/module-cache" \
    -target arm64-apple-macosx14.0 -framework SwiftUI -framework AppKit \
    "$ROOT"/Sources/*.swift \
    -o "$APP/Contents/MacOS/AgentCheck"
cp "$ROOT/Info.plist" "$APP/Contents/Info.plist"
codesign --force --sign - "$APP"
printf '%s\n' "$APP"
