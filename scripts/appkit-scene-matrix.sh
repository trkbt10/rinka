#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit scene verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
bundle="$root/target/Rinka Explorer.app"
executable="$root/target/debug/rinka-explorer"

"$root/scripts/bundle-macos.sh" >/dev/null
codesign --verify --deep --strict "$bundle"

for appearance in light dark; do
  for scene in ready empty busy error; do
    log="$(mktemp "${TMPDIR:-/tmp}/rinka-${appearance}-${scene}.XXXXXX")"
    if ! RINKA_APPKIT_APPEARANCE="$appearance" \
      RINKA_APPKIT_SCENE_PROBE="$scene" \
      "$executable" --scene "$scene" >"$log" 2>&1; then
      sed -n '1,240p' "$log" >&2
      echo "AppKit scene process failed: appearance=$appearance scene=$scene" >&2
      exit 1
    fi
    if ! grep -q "Rinka AppKit appearance requested=$appearance .* pass=true" "$log"; then
      sed -n '1,240p' "$log" >&2
      echo "AppKit appearance assertion failed: appearance=$appearance scene=$scene" >&2
      exit 1
    fi
    if ! grep -q "Rinka scene probe scene=$scene result=PASS" "$log"; then
      sed -n '1,240p' "$log" >&2
      echo "AppKit scene assertion failed: appearance=$appearance scene=$scene" >&2
      exit 1
    fi
    if [[ "$scene" == "busy" ]] && ! grep -q "action=cancel-refresh" "$log"; then
      sed -n '1,240p' "$log" >&2
      echo "AppKit panel action assertion failed: appearance=$appearance" >&2
      exit 1
    fi
    echo "AppKit scene PASS appearance=$appearance scene=$scene"
  done
done
