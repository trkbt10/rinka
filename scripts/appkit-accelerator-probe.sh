#!/usr/bin/env bash
set -euo pipefail

# Live verification of declared keyboard accelerators on the AppKit host.
# The probe process posts real key-down events through its own event queue
# (no global input injection) and asserts the routed scene changes plus the
# text-field precedence policy. Because window-scoped chords require key
# status, another application activating mid-run fails the probe honestly
# (never vacuously); the run is retried a bounded number of times.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit accelerator verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"
attempts="${RINKA_ACCELERATOR_PROBE_ATTEMPTS:-5}"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

log="$(mktemp "${TMPDIR:-/tmp}/rinka-accelerator-probe.XXXXXX")"

probe_once() {
  if ! RINKA_APPKIT_ACCELERATOR_PROBE=1 "$executable" --scene ready >"$log" 2>&1; then
    return 1
  fi
  # Scene assertions come from the probe; the routing-soundness lines come
  # from the key monitor itself, proving each outcome was routed under the
  # focus fact the policy claims (never vacuously).
  for line in \
    "probe step=initial_scene expected_scene=ready observed_scene=ready pass=true" \
    "event chord=Primary+2 text_focus=false outcome=dispatched window=explorer-main id=scene-empty" \
    "probe step=chord_dispatch expected_scene=empty observed_scene=empty pass=true" \
    "probe step=focus_search_field pass=true" \
    "event chord=Primary+1 text_focus=true outcome=dispatched window=explorer-main id=scene-ready" \
    "probe step=global_over_text_input expected_scene=ready observed_scene=ready pass=true" \
    "probe step=refocus_search_field pass=true" \
    "event chord=Primary+3 text_focus=true outcome=withheld window=explorer-main id=scene-error" \
    "probe step=text_field_precedence expected_scene=ready observed_scene=ready pass=true" \
    "event chord=Primary+3 text_focus=false outcome=dispatched window=explorer-main id=scene-error" \
    "probe step=chord_after_unfocus expected_scene=error observed_scene=error pass=true" \
    "probe step=menu_key_equivalent"; do
    if ! grep -q "Rinka accelerator $line" "$log"; then
      return 1
    fi
  done
  grep -q "Rinka accelerator probe result=PASS" "$log"
}

for attempt in $(seq 1 "$attempts"); do
  if probe_once; then
    grep "Rinka accelerator probe" "$log"
    echo "AppKit accelerator probe PASS attempt=$attempt (log: $log)"
    exit 0
  fi
  echo "AppKit accelerator probe attempt $attempt/$attempts did not pass (desktop contention or regression); log tail:" >&2
  tail -n 12 "$log" >&2
done

sed -n '1,240p' "$log" >&2
echo "AppKit accelerator probe failed after $attempts attempts" >&2
exit 1
