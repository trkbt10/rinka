#!/usr/bin/env bash
set -euo pipefail

# Live verification of the canvas text-input host on the AppKit host.
# The probe process clicks the echo canvas through NSWindow sendEvent:
# (real hit testing → click-to-focus), posts real key-down events through
# its own event queue (no global input injection), drives deterministic
# IME composition sequences directly through the NSTextInputClient
# protocol, and asserts the component-observed state plus the accelerator
# precedence over the focused canvas. One end-to-end typing step routes a
# real key through the desktop's active input source; the probe logs which
# path (raw insertion or live composition) that source produced and asserts
# the delivery either way. Because window-scoped chords require key status,
# another application activating mid-run fails the probe honestly (never
# vacuously); the run is retried a bounded number of times.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit text-input verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"
attempts="${RINKA_TEXT_INPUT_PROBE_ATTEMPTS:-5}"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

log="$(mktemp "${TMPDIR:-/tmp}/rinka-text-input-probe.XXXXXX")"

# Seconds since the user last touched keyboard or mouse.
user_idle_seconds() {
  ioreg -c IOHIDSystem | awk '/HIDIdleTime/ {print int($NF/1000000000); exit}'
}

probe_once() {
  RINKA_APPKIT_TEXT_INPUT_PROBE=1 "$executable" --scene canvas >"$log" 2>&1 &
  local app_pid=$!
  # Cooperative activation (the polite [NSApp activate] the host performs)
  # is refused while another app such as the launching terminal stays
  # frontmost. Window-scoped chord routing requires key status, so — only
  # while the user is verifiably idle, and only for the process this script
  # just spawned — ask System Events to bring the probe app frontmost. If
  # the user is active, no focus is taken and the run fails honestly.
  sleep 1.5
  if [[ "$(user_idle_seconds)" -ge 10 ]]; then
    osascript -e "tell application \"System Events\" to set frontmost of (first application process whose unix id is $app_pid) to true" >/dev/null 2>&1 || true
  fi
  if ! wait "$app_pid"; then
    return 1
  fi
  for line in \
    "probe step=initial_scene observed_scene=canvas pass=true" \
    "probe step=unfocus_canvas expected=\"focused=false\" pass=true" \
    "probe step=click_focus expected=\"focused=true\" pass=true" \
    "probe step=end_to_end_key path=" \
    "probe step=protocol_insert expected=\"echo:\" pass=true" \
    "probe step=caret_rect_text expected=\"wide\" pass=true" \
    "probe step=raw_arrow_key expected=\"key=Left\" pass=true" \
    "probe step=raw_control_chord expected=\"key=Control+C\" pass=true" \
    "probe step=raw_key_repeat expected=\"key=Right repeat\" pass=true" \
    "probe step=ime_preedit expected=\"preedit=\\\"にほんご\\\"\" pass=true" \
    "probe step=ime_preedit_update expected=\"preedit=\\\"にほん語\\\"\" pass=true" \
    "probe step=ime_commit expected=\"日本語\\\" preedit=\\\"\\\"\" pass=true" \
    "probe step=ime_cancel_preedit expected=\"preedit=\\\"かな\\\"\" pass=true" \
    "probe step=ime_cancel expected=\"preedit=\\\"\\\"\" pass=true" \
    "probe step=ime_cancel_left_no_trace pass=true" \
    "probe step=global_over_canvas observed_scene=ready pass=true"; do
    if ! grep -qF "Rinka text-input $line" "$log"; then
      return 1
    fi
  done
  # The routing-soundness lines come from the key monitor itself, proving
  # the withheld and dispatched outcomes were routed under the focus fact
  # the policy claims: an input-accepting canvas counts as text input.
  grep -qF "Rinka text-input probe step=first_responder is_first_responder=true" "$log" || return 1
  grep -qF "step=caret_rect" "$log" || return 1
  grep -qF "advanced=true contained=true pass=true" "$log" || return 1
  grep -q "Rinka accelerator event chord=Primary+2 text_focus=true outcome=withheld window=explorer-main id=scene-empty" "$log" || return 1
  grep -qF "step=withheld_over_canvas scene_unchanged=true chord_reached_canvas=true pass=true" "$log" || return 1
  grep -q "Rinka accelerator event chord=Primary+1 text_focus=true outcome=dispatched window=explorer-main id=scene-ready" "$log" || return 1
  grep -qF "Rinka text-input probe result=PASS" "$log"
}

for attempt in $(seq 1 "$attempts"); do
  if probe_once; then
    grep -E "Rinka (text-input probe|accelerator event)" "$log"
    echo "AppKit text-input probe PASS attempt=$attempt (log: $log)"
    exit 0
  fi
  echo "AppKit text-input probe attempt $attempt/$attempts did not pass (desktop contention or regression); log tail:" >&2
  tail -n 16 "$log" >&2
done

sed -n '1,240p' "$log" >&2
echo "AppKit text-input probe failed after $attempts attempts" >&2
exit 1
