#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:-/tmp/rinka-gtk-gnome-shell-evidence}
display=${RINKA_GNOME_DISPLAY:-:119}

for command in \
  Xvfb awk convert dbus-run-session gdbus gnome-shell gsettings identify import jq \
  rg xdpyinfo xdotool xprop xwininfo; do
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
export RINKA_GNOME_DISPLAY="$display"

dbus-run-session -- bash -euo pipefail <<'SESSION'
output="$RINKA_GTK_OUTPUT"
display="$RINKA_GNOME_DISPLAY"
xvfb_pid=
shell_pid=
application_pid=
previous_color_scheme=$(gsettings get org.gnome.desktop.interface color-scheme)
previous_idle_delay=$(gsettings get org.gnome.desktop.session idle-delay)

stop_pid() {
  local pid=$1
  [[ -n "$pid" ]] || return 0
  kill "$pid" 2>/dev/null || true
  for _ in $(seq 1 20); do
    kill -0 "$pid" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -KILL "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

stop_processes() {
  stop_pid "$application_pid"
  stop_pid "$shell_pid"
  stop_pid "$xvfb_pid"
  gsettings set org.gnome.desktop.interface color-scheme \
    "$previous_color_scheme" 2>/dev/null || true
  gsettings set org.gnome.desktop.session idle-delay \
    "$previous_idle_delay" 2>/dev/null || true
}
trap stop_processes EXIT

Xvfb "$display" -screen 0 1440x900x24 -nolisten tcp \
  >"$output/xvfb.log" 2>&1 &
xvfb_pid=$!
export DISPLAY="$display"
export GDK_BACKEND=x11
export LIBGL_ALWAYS_SOFTWARE=1
export GSK_RENDERER=cairo
export XDG_SESSION_TYPE=x11

for _ in $(seq 1 100); do
  if xdpyinfo >/dev/null 2>&1; then
    break
  fi
  kill -0 "$xvfb_pid"
  sleep 0.1
done
xdpyinfo >/dev/null

gsettings set org.gnome.desktop.interface color-scheme prefer-dark
gsettings set org.gnome.desktop.session idle-delay 0

gnome-shell \
  --x11 \
  --replace \
  --unsafe-mode \
  --sm-disable \
  --mode=user \
  >"$output/gnome-shell.log" 2>&1 &
shell_pid=$!

for _ in $(seq 1 200); do
  if xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null \
    | rg -q 'window id'; then
    break
  fi
  kill -0 "$shell_pid"
  sleep 0.1
done

xprop -root _NET_SUPPORTING_WM_CHECK \
  >"$output/window-manager.txt"
wm_id=$(awk '{print $NF}' "$output/window-manager.txt")
xprop -id "$wm_id" _NET_WM_NAME WM_CLASS \
  >>"$output/window-manager.txt"
rg -q '_NET_WM_NAME.*"GNOME Shell"' "$output/window-manager.txt"

RINKA_GTK_LAYOUT_PROBE=1 \
  "$RINKA_GTK_ROOT/target/debug/rinka-explorer" \
  --scene ready \
  >"$output/application.stdout.log" \
  2>"$output/application.stderr.log" &
application_pid=$!

window_id=
for _ in $(seq 1 200); do
  window_id=$(xdotool search --onlyvisible --pid "$application_pid" \
    --name '^Rinka Explorer$' 2>/dev/null | head -n 1 || true)
  [[ -n "$window_id" ]] && break
  kill -0 "$application_pid"
  sleep 0.1
done
[[ -n "$window_id" ]]

sleep 5
gdbus call \
  --session \
  --dest org.gnome.Shell \
  --object-path /org/gnome/Shell \
  --method org.gnome.Shell.Eval \
  'Main.overview.hide();' \
  >"$output/hide-overview.txt"
rg -q '^\(true,' "$output/hide-overview.txt"
sleep 1
gdbus call \
  --session \
  --dest org.gnome.Shell \
  --object-path /org/gnome/Shell \
  --method org.gnome.Shell.Eval \
  'Main.overview.visible;' \
  >"$output/overview-visible.txt"
rg -q "^\(true, 'false'\)" "$output/overview-visible.txt"

xdotool windowactivate --sync "$window_id"
xdotool windowmove --sync "$window_id" 100 40
sleep 2
xwininfo -id "$window_id" >"$output/application-window.txt"
rg -q '^  Map State: IsViewable$' "$output/application-window.txt"

import -window root "$output/gnome-shell-x11-1440x900.png"
import -window "$window_id" "$output/gnome-shell-app-default.png"
identify -format '%w %h\n' "$output/gnome-shell-x11-1440x900.png" \
  | rg -q '^1440 900$'
rg -q \
  'RINKA_GTK_WINDOW_CONTRACT title="Rinka Explorer" expected-content=1120x720 content=1120x720 .*result=PASS' \
  "$output/application.stderr.log"
rg 'RINKA_GTK_CONTENT_ALLOCATION title="Rinka Explorer"' \
  "$output/application.stderr.log" | tail -n 1 \
  >"$output/application-content-allocation.txt"
identify -format '%wx%h\n' "$output/gnome-shell-app-default.png" \
  >"$output/application-visible-outer.txt"

if rg -n 'Adwaita-WARNING|Gtk-CRITICAL|GLib-GObject-CRITICAL|GTK host error|panicked at' \
  "$output"/*.log; then
  echo "GTK runtime diagnostic detected" >&2
  exit 1
fi
SESSION

(
  uname -a
  gnome-shell --version
  pkg-config --modversion gtk4
  pkg-config --modversion libadwaita-1
  printf '%s\n' 'session=x11' 'screen=1440x900' \
    'declared-content=1120x720'
) >"$output/system.txt"

shasum -a 256 "$output"/*.png >"$output/SHA256SUMS"

jq -n \
  --arg compositor gnome-shell-mutter \
  --arg session x11 \
  --arg overview hidden \
  '{
    compositor: $compositor,
    session: $session,
    overview: $overview,
    desktopCapture: "1440x900",
    applicationCapture: "declared 1120x720 content",
    result: "PASS"
  }' \
  >"$output/result.json"
cat "$output/result.json"
