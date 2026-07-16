#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit transition verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="$root/target/debug/rinka-explorer"

"$root/scripts/bundle-macos.sh" >/dev/null

for appearance in light dark; do
  log="$(mktemp "${TMPDIR:-/tmp}/rinka-${appearance}-transition.XXXXXX")"
  if ! RINKA_APPKIT_APPEARANCE="$appearance" \
    RINKA_APPKIT_TRANSITION_PROBE=1 \
    "$executable" --scene ready >"$log" 2>&1; then
    sed -n '1,320p' "$log" >&2
    echo "AppKit transition process failed: appearance=$appearance" >&2
    exit 1
  fi

  state_count="$(grep -c 'Rinka transition probe phase=.* step=.* state=' "$log")"
  if [[ "$state_count" != "48" ]] \
    || grep -q 'frame_matches=false' "$log" \
    || grep -q 'settlement_timeout' "$log" \
    || ! grep -q 'Rinka transition probe result=PASS' "$log"; then
    sed -n '1,320p' "$log" >&2
    echo "AppKit transition assertion failed: appearance=$appearance states=$state_count" >&2
    exit 1
  fi

  echo "AppKit transition PASS appearance=$appearance states=$state_count"
done
