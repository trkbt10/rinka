#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS bundle generation requires Darwin" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
bundle="$root/target/Rinka Explorer.app"
contents="$bundle/Contents"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer
mkdir -p "$contents/MacOS" "$contents/Resources"
cp "$root/target/debug/rinka-explorer" "$contents/MacOS/rinka-explorer"
cp "$root/packaging/macos/Info.plist" "$contents/Info.plist"
codesign --force --sign - "$bundle"

echo "$bundle"
