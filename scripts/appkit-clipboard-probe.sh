#!/usr/bin/env bash
set -euo pipefail

# Live pbcopy/pbpaste interop verification for the clipboard service
# (reports/clipboard-access). The probe process presses the explorer's real
# "Paste Path" and "Copy Path" buttons in-process and drives the focused
# search field's own standard Copy action — no global input injection, no
# focus stealing, no AX attachment, nothing that can land in another
# window — while this script owns the cross-process assertions through
# pbcopy and pbpaste:
#
#   1. pbcopy seeds text  → the app's Paste Path must read it back.
#   2. the app's focused native search field performs its own Copy and
#      Paste (the actions Cmd+C / Cmd+V key equivalents resolve to) → the
#      selection must round-trip through the pasteboard with no rinka code
#      in that path.
#   3. the app's Copy Path writes the current directory path → pbpaste
#      must reproduce it.
#
# The user's clipboard text is saved before seeding and restored on every
# exit path. Only the text flavor can be restored this way; `clipboard
# info` is logged before and after so the evidence records exactly what
# was present.

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit clipboard verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
executable="${CARGO_TARGET_DIR:-$root/target}/debug/rinka-explorer"

cargo build --manifest-path "$root/Cargo.toml" -p rinka-explorer

saved_text="$(pbpaste 2>/dev/null || true)"
restore_clipboard() { printf '%s' "$saved_text" | pbcopy; }
trap restore_clipboard EXIT

echo "clipboard flavors before: $(osascript -e 'clipboard info' 2>/dev/null || echo unavailable)"

seed="rinka clipboard interop 日本語"
printf '%s' "$seed" | pbcopy

log="$(mktemp "${TMPDIR:-/tmp}/rinka-clipboard-probe.XXXXXX")"
if ! RINKA_APPKIT_CLIPBOARD_PROBE=1 "$executable" --scene ready >"$log" 2>&1; then
  sed -n '1,80p' "$log" >&2
  echo "clipboard probe process failed" >&2
  exit 1
fi

status=0
if ! grep -qF "Rinka clipboard probe step=paste observed=\"Clipboard: $seed\" pass=true" "$log"; then
  echo "FAIL: the app did not read back the pbcopy-seeded text" >&2
  status=1
fi

if ! grep -qF "Rinka clipboard probe step=native_field_copy observed=\"rinka native field copy 検証\" pass=true" "$log"; then
  echo "FAIL: the native search field's own Copy did not reach the pasteboard" >&2
  status=1
fi
if ! grep -qF "Rinka clipboard probe step=native_field_paste observed=\"rinka native field copy 検証\" pass=true" "$log"; then
  echo "FAIL: the native search field's own Paste did not read the pasteboard" >&2
  status=1
fi

# Location::RemoteProject's path on the macOS consumer.
expected_path="/home/trkbt10/project"
copied="$(pbpaste)"
if [[ "$copied" != "$expected_path" ]]; then
  echo "FAIL: pbpaste read '$copied', expected '$expected_path'" >&2
  status=1
fi
if ! grep -qF "Rinka clipboard probe step=copy observed=\"Copied $expected_path\" pass=true" "$log"; then
  echo "FAIL: the app did not confirm the copy" >&2
  status=1
fi
if ! grep -q "Rinka clipboard probe result=PASS" "$log"; then
  echo "FAIL: probe did not finish PASS" >&2
  status=1
fi

grep "Rinka clipboard probe" "$log" || true
if [[ "$status" == 0 ]]; then
  echo "pbpaste after app copy: $copied"
  echo "AppKit clipboard probe PASS (log: $log)"
else
  sed -n '1,80p' "$log" >&2
fi
restore_clipboard
echo "clipboard flavors after restore: $(osascript -e 'clipboard info' 2>/dev/null || echo unavailable)"
exit "$status"
