#!/usr/bin/env bash
set -euo pipefail

# In-process dock verification (reports/document-tabs-and-splits). The probe
# process drives the real explorer's dock scene entirely inside itself — tab
# buttons through their own performClick action path, hover anatomy through
# the item's own tracking handlers, tab drops through the strip and content
# hosts' own NSDraggingDestination selectors over a uniquely named
# pasteboard, and the dirty-close veto through the real confirmation sheet —
# no global input injection, no focus stealing beyond the app's own
# activation, no AX attachment to any other process, and never the user's
# general pasteboard.
#
#   1. Locate the three tabs with native titles, toggle state, and the
#      accessibility extract (role, label, selected).
#   2. Select, hover anatomy (dirty dot / hover close), explicit split with
#      weight application, protocol-level strip move and edge-drop split.
#   3. Dirty-close veto through the native sheet, close-last collapse, the
#      per-tab context menu, and the save/restore layout round trip.
#
# The probe runs once per appearance so the captures cover light and dark.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit dock verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${RINKA_DOCK_PROBE_OUT:-${CARGO_TARGET_DIR:-$root/target}/dock-probe}"
mkdir -p "$out"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"

for appearance in light dark; do
  echo "== dock probe ($appearance) =="
  mkdir -p "$out/$appearance"
  RINKA_APPKIT_DOCK_PROBE=1 \
  RINKA_APPKIT_APPEARANCE="$appearance" \
  RINKA_APPKIT_DOCK_PROBE_AX_DUMP="$out/ax-tabs-$appearance.txt" \
  RINKA_APPKIT_WINDOW_CAPTURE_DIR="$out/$appearance" \
    "$executable" --scene dock 2>&1 | tee "$out/probe-$appearance.log"
  grep -q "Rinka dock probe result=PASS" "$out/probe-$appearance.log"
  grep -q "Rinka AppKit appearance requested=$appearance" "$out/probe-$appearance.log"
done

if ! diff -u "$out/ax-tabs-light.txt" "$out/ax-tabs-dark.txt" >"$out/ax-diff.txt"; then
  echo "AX extracts differ between appearances:" >&2
  cat "$out/ax-diff.txt" >&2
  exit 1
fi

echo "dock probe PASS (evidence in $out)"
