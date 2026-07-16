#!/usr/bin/env bash
set -euo pipefail

# In-process drag-and-drop verification (reports/drag-and-drop). The probe
# process drives the real explorer entirely inside itself — a constructed
# NSDraggingInfo double over uniquely named pasteboards, the retained table
# delegates' own validateDrop/acceptDrop selectors, and the real
# NSFilePromiseProvider delegate — no global input injection, no focus
# stealing beyond the app's own activation, no AX attachment to any other
# process, and never the user's general pasteboard.
#
#   1. OS file drop-in onto the file table yields the dropped paths and
#      position in the status note.
#   2. The README.md row's pasteboard writer promises README.md.txt and
#      materializes it lazily, exactly once, on the main queue.
#   3. The same writer's typed payload drops onto the sidebar's Documents
#      row and the move intent reconciles the note.
#
# A second run with RINKA_EXPLORER_DISABLE_DRAG=1 produces the drag-free
# accessibility dump; the diff proves the AX tree is equivalent with and
# without drag declarations.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit drag-and-drop verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${RINKA_DRAG_DROP_PROBE_OUT:-${CARGO_TARGET_DIR:-$root/target}/drag-drop-probe}"
mkdir -p "$out"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"

echo "== drag-drop probe (drag declarations attached) =="
RINKA_APPKIT_DRAG_DROP_PROBE=1 \
RINKA_APPKIT_DRAG_DROP_PROBE_AX_DUMP="$out/ax-with-drag.txt" \
RINKA_APPKIT_WINDOW_CAPTURE_DIR="$out" \
  "$executable" --scene ready 2>&1 | tee "$out/probe-with-drag.log"

echo "== drag-drop probe (Empty scene keeps the drop-files-here promise) =="
RINKA_APPKIT_DRAG_DROP_PROBE=1 \
RINKA_APPKIT_WINDOW_CAPTURE_DIR="$out" \
  "$executable" --scene empty 2>&1 | tee "$out/probe-empty-scene.log"

echo "== accessibility dump (drag declarations stripped) =="
RINKA_APPKIT_DRAG_DROP_PROBE=1 \
RINKA_EXPLORER_DISABLE_DRAG=1 \
RINKA_APPKIT_DRAG_DROP_PROBE_AX_DUMP="$out/ax-without-drag.txt" \
  "$executable" --scene ready 2>&1 | tee "$out/probe-without-drag.log"

grep -q "Rinka drag-drop probe result=PASS" "$out/probe-with-drag.log"
grep -q "Rinka drag-drop probe result=PASS" "$out/probe-empty-scene.log"
grep -q "Rinka drag-drop probe result=PASS" "$out/probe-without-drag.log"

if diff -u "$out/ax-without-drag.txt" "$out/ax-with-drag.txt" >"$out/ax-diff.txt"; then
  echo "AX trees are equivalent with and without drag declarations"
else
  echo "AX trees differ between drag-enabled and drag-free runs:" >&2
  cat "$out/ax-diff.txt" >&2
  exit 1
fi

echo "drag-drop probe PASS (evidence in $out)"
