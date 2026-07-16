#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:-/tmp/rinka-gtk-wayland-evidence}

for command in dbus-run-session identify jq rg weston weston-screenshooter; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command is missing: $command" >&2
    exit 1
  fi
done

mkdir -p "$output"
rm -f "$output"/*.log "$output"/*.png "$output"/*.json "$output"/*.txt

cd "$root"
cargo build -p rinka-explorer

export RINKA_GTK_ROOT="$root"
export RINKA_GTK_OUTPUT="$output"

dbus-run-session -- bash -euo pipefail <<'SESSION'
runtime_directory="$RINKA_GTK_OUTPUT/runtime"
mkdir -p "$runtime_directory"
chmod 700 "$runtime_directory"
export XDG_RUNTIME_DIR="$runtime_directory"
export WAYLAND_DISPLAY=wayland-rinka

weston \
  --backend=headless \
  --renderer=pixman \
  --width=1440 \
  --height=900 \
  --socket="$WAYLAND_DISPLAY" \
  --idle-time=0 \
  --debug \
  --no-config \
  --log="$RINKA_GTK_OUTPUT/weston.log" &
weston_pid=$!
application_pid=

stop_processes() {
  if [[ -n "$application_pid" ]]; then
    kill "$application_pid" 2>/dev/null || true
    wait "$application_pid" 2>/dev/null || true
  fi
  kill "$weston_pid" 2>/dev/null || true
  wait "$weston_pid" 2>/dev/null || true
}
trap stop_processes EXIT

for attempt in $(seq 1 100); do
  if [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
    break
  fi
  if ! kill -0 "$weston_pid" 2>/dev/null; then
    cat "$RINKA_GTK_OUTPUT/weston.log" >&2
    exit 1
  fi
  sleep 0.1
done
test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"

GDK_BACKEND=wayland \
ADW_DEBUG_COLOR_SCHEME=prefer-dark \
RINKA_GTK_LAYOUT_PROBE=1 \
WAYLAND_DEBUG=client \
  "$RINKA_GTK_ROOT/target/debug/rinka-explorer" \
  --scene ready \
  >"$RINKA_GTK_OUTPUT/application.stdout.log" \
  2>"$RINKA_GTK_OUTPUT/application.wayland.log" &
application_pid=$!

for attempt in $(seq 1 150); do
  if rg -q 'xdg_toplevel.*set_title\("Rinka Explorer"\)' \
    "$RINKA_GTK_OUTPUT/application.wayland.log"; then
    break
  fi
  if ! kill -0 "$application_pid" 2>/dev/null; then
    cat "$RINKA_GTK_OUTPUT/application.wayland.log" >&2
    exit 1
  fi
  sleep 0.1
done

kill -0 "$application_pid"
rg -q 'xdg_wm_base.*get_xdg_surface' "$RINKA_GTK_OUTPUT/application.wayland.log"
rg -q 'xdg_surface.*get_toplevel' "$RINKA_GTK_OUTPUT/application.wayland.log"
rg -q 'xdg_toplevel.*set_title\("Rinka Explorer"\)' \
  "$RINKA_GTK_OUTPUT/application.wayland.log"

for attempt in $(seq 1 150); do
  if rg -q 'wl_surface.*attach\(wl_buffer' \
    "$RINKA_GTK_OUTPUT/application.wayland.log"; then
    break
  fi
  if ! kill -0 "$application_pid" 2>/dev/null; then
    cat "$RINKA_GTK_OUTPUT/application.wayland.log" >&2
    exit 1
  fi
  sleep 0.1
done
rg -q 'wl_surface.*attach\(wl_buffer' \
  "$RINKA_GTK_OUTPUT/application.wayland.log"
sleep 1
rg -q \
  'RINKA_GTK_WINDOW_CONTRACT title="Rinka Explorer" expected-content=1120x720 content=1120x720 .*result=PASS' \
  "$RINKA_GTK_OUTPUT/application.wayland.log"

(
  cd "$RINKA_GTK_OUTPUT"
  weston-screenshooter
)

screenshot=$(find "$RINKA_GTK_OUTPUT" -maxdepth 1 -name '*.png' -print -quit)
test -n "$screenshot"
identify "$screenshot"
identify -format '%w %h\n' "$screenshot" \
  | rg -q '^1440 900$'

if rg -n 'Adwaita-WARNING|Gtk-CRITICAL|GLib-GObject-CRITICAL|GTK host error|panicked at' \
  "$RINKA_GTK_OUTPUT"/*.log; then
  echo "GTK runtime diagnostic detected" >&2
  exit 1
fi
SESSION

(
  uname -a
  weston --version
  pkg-config --modversion gtk4
  pkg-config --modversion libadwaita-1
) >"$output/system.txt"

jq -n \
  --arg compositor weston-headless \
  --arg protocol xdg_toplevel \
  '{compositor: $compositor, protocol: $protocol, result: "PASS"}' \
  >"$output/result.json"
cat "$output/result.json"
