#!/usr/bin/env bash
set -euo pipefail

# Live verification of the runtime window lifecycle on the AppKit host. The
# probe process opens a second explorer window through a real Primary+N key
# equivalent posted to its own queue, proves the reconciled window title
# follows the secondary's scene state, closes it programmatically through
# the window-scoped Primary+Alt+W accelerator (bypassing interception),
# reopens it under a fresh identity, and drives the close-interception
# protocol through performClose:: the deferred close presents the real
# confirmation sheet, Cancel vetoes and the window stays open, Close
# confirms and only then does the native window close. No global input is
# injected and only this probe's own windows are touched.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit window lifecycle verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"
attempts="${RINKA_WINDOW_LIFECYCLE_PROBE_ATTEMPTS:-5}"
capture_dir="${RINKA_APPKIT_WINDOW_LIFECYCLE_CAPTURE_DIR:-}"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

probe_once() {
  local appearance="$1"
  local log="$2"
  local run_capture_dir=""
  if [ -n "$capture_dir" ]; then
    run_capture_dir="$capture_dir/$appearance"
    mkdir -p "$run_capture_dir"
  fi
  if ! env RINKA_APPKIT_WINDOW_LIFECYCLE_PROBE=1 \
    RINKA_APPKIT_APPEARANCE="$appearance" \
    ${run_capture_dir:+RINKA_APPKIT_WINDOW_CAPTURE_DIR="$run_capture_dir"} \
    "$executable" --scene ready >"$log" 2>&1; then
    return 1
  fi
  for line in \
    "probe step=initial_scene observed_scene=ready windows=1 pass=true" \
    "probe step=open_second_window id=explorer-secondary-[0-9]+ key_is_secondary=true title=\"Rinka Explorer — Ready\" main_note=\"window resigned: explorer-main\" pass=true" \
    "probe step=live_title title=\"Rinka Explorer — Empty\" windows=2 pass=true" \
    "probe step=editor_scene title=\"Rinka Explorer — Editor\" pass=true" \
    "probe step=programmatic_close windows=1 key_is_main=true main_note=\"window focused: explorer-main\" pass=true" \
    "probe step=reopen_second_window id=explorer-secondary-[0-9]+ first=Some\\(\"explorer-secondary-[0-9]+\"\\) fresh_identity=true pass=true" \
    "probe step=editor_scene_again title=\"Rinka Explorer — Editor\" pass=true" \
    "probe step=close_deferred_sheet sheet=true windows=2 still_open=true pass=true" \
    "probe step=veto_holds windows=2 pending_tokens=0 pass=true" \
    "probe step=confirm_sheet sheet=true pass=true" \
    "probe step=confirmed_close windows=1 key_is_main=true pending_tokens=0 pass=true" \
    "probe result=PASS"; do
    if ! grep -Eq "Rinka window-lifecycle $line" "$log"; then
      echo "missing: Rinka window-lifecycle $line" >&2
      return 1
    fi
  done
}

run_configuration() {
  local appearance="$1"
  local log
  log="$(mktemp "${TMPDIR:-/tmp}/rinka-window-lifecycle-probe-$appearance.XXXXXX")"
  for attempt in $(seq 1 "$attempts"); do
    if probe_once "$appearance" "$log"; then
      grep -E "Rinka window-lifecycle probe" "$log"
      echo "AppKit window lifecycle probe PASS appearance=$appearance attempt=$attempt (log: $log)"
      return 0
    fi
    echo "AppKit window lifecycle probe appearance=$appearance attempt $attempt/$attempts did not pass; log tail:" >&2
    tail -n 12 "$log" >&2
  done
  sed -n '1,240p' "$log" >&2
  echo "AppKit window lifecycle probe appearance=$appearance failed after $attempts attempts" >&2
  return 1
}

run_configuration light
run_configuration dark
