#!/usr/bin/env bash
set -euo pipefail

# Live verification of window-modal dialogs on the AppKit host.
# The probe process presses the explorer's mounted Upload / Download / Delete
# buttons through `performClick:` inside its own process, confirms the open
# and save panels programmatically, and asserts the confirm sheet's
# destructive idiom (destructive never on the return key, sheet attached to
# the owning window, no app-modal session). No global pointer or keyboard
# input is injected, so the user's desktop is never touched.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit dialog verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"
attempts="${RINKA_DIALOG_PROBE_ATTEMPTS:-5}"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

log="$(mktemp "${TMPDIR:-/tmp}/rinka-dialog-probe.XXXXXX")"

# Deterministic panel fixture: a throwaway directory the panels start in.
fixture="$(mktemp -d "${TMPDIR:-/tmp}/rinka-dialog-fixture.XXXXXX")"
fixture="$(cd "$fixture" && pwd -P)"
printf 'alpha\n' >"$fixture/alpha.txt"
printf 'beta\n' >"$fixture/beta.txt"
trap 'rm -rf "$fixture"' EXIT

capture_dir="${RINKA_DIALOG_PROBE_CAPTURE_DIR:-}"

probe_once() {
  local -a env_args=(
    RINKA_APPKIT_DIALOG_PROBE=1
    RINKA_EXPLORER_PANEL_DIR="$fixture"
  )
  if [[ -n "$capture_dir" ]]; then
    mkdir -p "$capture_dir"
    env_args+=(RINKA_APPKIT_WINDOW_CAPTURE_DIR="$capture_dir")
  fi
  if ! env "${env_args[@]}" "$executable" --scene ready >"$log" 2>&1; then
    return 1
  fi
  for line in \
    "probe step=click_upload pass=true" \
    "probe step=open_panel_sheet is_open_panel=true can_choose_files=true can_choose_directories=true allows_multiple=true" \
    "probe step=open_panel_round_trip" \
    "probe step=save_panel_sheet is_save_panel=true" \
    "probe step=save_panel_round_trip" \
    "probe step=confirm_sheet app_modal=false is_sheet=true parent_is_window=true delete_destructive=true" \
    "probe step=delete_round_trip file_removed=true selection_cleared=true pass=true"; do
    if ! grep -q "Rinka dialog $line" "$log"; then
      return 1
    fi
  done
  grep -q "Rinka dialog probe result=PASS" "$log"
}

for attempt in $(seq 1 "$attempts"); do
  if probe_once; then
    grep "Rinka dialog probe" "$log"
    echo "AppKit dialog probe PASS attempt=$attempt (log: $log)"
    exit 0
  fi
  echo "AppKit dialog probe attempt $attempt/$attempts did not pass (desktop contention or regression); log tail:" >&2
  tail -n 12 "$log" >&2
done

sed -n '1,240p' "$log" >&2
echo "AppKit dialog probe failed after $attempts attempts" >&2
exit 1
