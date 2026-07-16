#!/usr/bin/env bash
set -euo pipefail

# Live verification of the declarative application menu bar on the AppKit
# host. The probe process asserts the installed NSApplication.mainMenu
# structure, activates items through their native target/action pairs and
# through real key-equivalent events posted to its own queue, proves the
# standard edit roles against the native search field with no consumer role
# handling, extracts the menu tree's accessibility state, and exits through
# a native menu path: the light run quits through the application menu's
# Quit item and the dark run closes the last window through File > Close
# Window. No global input is injected and only this probe's own windows are
# touched. The general pasteboard is saved and restored around the run.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit menu bar verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"
attempts="${RINKA_MENU_BAR_PROBE_ATTEMPTS:-5}"
capture_dir="${RINKA_APPKIT_MENU_BAR_PROBE_CAPTURE_DIR:-}"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

saved_pasteboard="$(pbpaste 2>/dev/null || true)"
restore_pasteboard() {
  printf '%s' "$saved_pasteboard" | pbcopy 2>/dev/null || true
}
trap restore_pasteboard EXIT

probe_once() {
  local appearance="$1"
  local finish="$2"
  local log="$3"
  local run_capture_dir=""
  if [ -n "$capture_dir" ]; then
    run_capture_dir="$capture_dir/$appearance"
    mkdir -p "$run_capture_dir"
  fi
  if ! env RINKA_APPKIT_MENU_BAR_PROBE=1 \
    RINKA_APPKIT_MENU_BAR_PROBE_FINISH="$finish" \
    RINKA_APPKIT_APPEARANCE="$appearance" \
    ${run_capture_dir:+RINKA_APPKIT_MENU_BAR_PROBE_CAPTURE_DIR="$run_capture_dir"} \
    "$executable" --scene ready >"$log" 2>&1; then
    return 1
  fi
  # The probe is activation-free by design (the clipboard probe's recorded
  # precedent): key_window resolves to the focused window when the desktop
  # grants activation and to the registered fallback (the main window)
  # otherwise, and responder-chain dispatch is mechanism-labeled.
  for line in \
    "probe step=initial_scene observed_scene=ready active=(true|false) pass=true" \
    "probe step=structure .* titles_pass=true windows_menu=true help_menu=true copy_role=true new_folder_chord=true checkmarks=true pass=true" \
    "probe step=checkmark_reconcile empty_state=Some\\(1\\) ready_state=Some\\(0\\) pass=true" \
    "activation item=view-scene-empty key_window=(explorer-main|none) outcome=dispatched window=explorer-main" \
    "probe step=menu_key_equivalent error_state=Some\\(1\\) new_folder_enabled=Some\\(false\\) pass=true" \
    "probe step=disabled_chord_refused note=\"\" pass=true" \
    "probe step=menu_only_chord note=\"New Folder created in Remote Project\" pass=true" \
    "activation item=file-new-folder key_window=(explorer-main|none) outcome=dispatched window=explorer-main" \
    "probe step=edit_roles mechanism=(native|anchored)-chain select_all=true copy=true observed=Some\\(\"rinka menu bar edit roles 検証\"\\) pass=true" \
    "probe step=edit_roles_cut_paste_undo cut=true after_cut=Some\\(\"\"\\) paste=true .* undo_inert=true" \
    "probe step=about_panel pass=true" \
    "probe step=open_view_menu performed=(true|false) opened=true pass=true" \
    "ax label=initial begin" \
    "ax label=final end" \
    "probe result=PASS" \
    "probe finish=$finish .*dispatching=true"; do
    if ! grep -Eq "Rinka menu-bar $line" "$log"; then
      echo "missing: Rinka menu-bar $line" >&2
      return 1
    fi
  done
  # Exactly-once dispatch for the menu-owned Primary+3 posted through the
  # real event queue: the monitor defers it and native menu dispatch fires
  # the item once; the shadowed accelerator entry never dispatches.
  if [ "$(grep -c 'Rinka menu-bar activation item=view-scene-error' "$log")" -ne 1 ]; then
    echo "view-scene-error did not fire exactly once" >&2
    return 1
  fi
  if grep -Eq "outcome=dispatched window=explorer-main id=scene-(ready|empty|error)" "$log"; then
    echo "a shadowed accelerator entry dispatched" >&2
    return 1
  fi
  # The refused Primary+N in the error scene must not have activated.
  if [ "$(grep -c 'Rinka menu-bar activation item=file-new-folder' "$log")" -ne 1 ]; then
    echo "file-new-folder fired other than exactly once" >&2
    return 1
  fi
  # The AX extract must expose labels and enabled state for the edit roles.
  grep -q 'Rinka menu-bar ax .*item title="Copy" enabled=' "$log"
}

run_configuration() {
  local appearance="$1"
  local finish="$2"
  local log
  log="$(mktemp "${TMPDIR:-/tmp}/rinka-menu-bar-probe-$appearance.XXXXXX")"
  for attempt in $(seq 1 "$attempts"); do
    if probe_once "$appearance" "$finish" "$log"; then
      grep -E "Rinka menu-bar (probe|activation)" "$log"
      echo "AppKit menu bar probe PASS appearance=$appearance finish=$finish attempt=$attempt (log: $log)"
      return 0
    fi
    echo "AppKit menu bar probe appearance=$appearance attempt $attempt/$attempts did not pass; log tail:" >&2
    tail -n 12 "$log" >&2
  done
  sed -n '1,240p' "$log" >&2
  echo "AppKit menu bar probe appearance=$appearance failed after $attempts attempts" >&2
  return 1
}

run_configuration light quit
run_configuration dark close
