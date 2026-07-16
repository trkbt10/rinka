#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
output=${1:-/tmp/rinka-gtk-evidence}

for command in Xvfb awk dbus-run-session gsettings import identify jq python3 rg sha256sum wmctrl xdotool; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "required command is missing: $command" >&2
    exit 1
  fi
done

mkdir -p "$output/logs"
rm -f "$output"/*.png "$output"/*.json "$output"/*.txt "$output"/SHA256SUMS

cd "$root"
cargo build -p rinka-explorer

export RINKA_GTK_ROOT="$root"
export RINKA_GTK_OUTPUT="$output"

dbus-run-session -- bash -euo pipefail <<'SESSION'
export DISPLAY=:97
export GDK_BACKEND=x11
export NO_AT_BRIDGE=0

Xvfb :97 -screen 0 1600x1000x24 -nolisten tcp >"$RINKA_GTK_OUTPUT/logs/xvfb.log" 2>&1 &
xvfb_pid=$!
openbox >"$RINKA_GTK_OUTPUT/logs/openbox.log" 2>&1 &
window_manager_pid=$!
application_pid=

stop_processes() {
  if [[ -n "$application_pid" ]]; then
    kill "$application_pid" 2>/dev/null || true
    wait "$application_pid" 2>/dev/null || true
  fi
  kill "$window_manager_pid" "$xvfb_pid" 2>/dev/null || true
  wait "$window_manager_pid" 2>/dev/null || true
  wait "$xvfb_pid" 2>/dev/null || true
}
trap stop_processes EXIT

for attempt in $(seq 1 100); do
  if xdpyinfo -display :97 >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
xdpyinfo -display :97 >/dev/null
gsettings set org.gnome.desktop.interface toolkit-accessibility true

window_id_for() {
  local title=$1
  local window=
  for attempt in $(seq 1 150); do
    window=$(xdotool search --onlyvisible --name "^${title}$" 2>/dev/null | head -n 1 || true)
    if [[ -n "$window" ]]; then
      printf '%s\n' "$window"
      return 0
    fi
    sleep 0.1
  done
  echo "window did not appear: $title" >&2
  return 1
}

assert_geometry() {
  local window=$1
  local expected_width=$2
  local expected_height=$3
  local label=$4
  local geometry_file="$RINKA_GTK_OUTPUT/${label}-geometry.txt"
  xdotool getwindowgeometry --shell "$window" >"$geometry_file"
  if ! grep -qx "WIDTH=${expected_width}" "$geometry_file"; then
    cat "$geometry_file" >&2
    return 1
  fi
  if ! grep -qx "HEIGHT=${expected_height}" "$geometry_file"; then
    cat "$geometry_file" >&2
    return 1
  fi
}

capture_window() {
  local window=$1
  local destination=$2
  import -window "$window" "$destination"
  identify "$destination"
}

geometry_value() {
  local geometry_file=$1
  local key=$2
  awk -F= -v key="$key" '$1 == key { print $2 }' "$geometry_file"
}

assert_content_allocation() {
  local log=$1
  local title=$2
  local expected_width=$3
  local expected_height=$4
  local latest=
  latest=$(rg "RINKA_GTK_CONTENT_ALLOCATION title=\"${title}\"" "$log" | tail -n 1)
  case "$latest" in
    *"content=${expected_width}x${expected_height} "*) ;;
    *)
      printf '%s\n' "$latest" >&2
      return 1
      ;;
  esac
}

for appearance in light dark; do
  if [[ "$appearance" == light ]]; then
    gsettings set org.gnome.desktop.interface color-scheme default
    appearance_scheme=prefer-light
  else
    gsettings set org.gnome.desktop.interface color-scheme prefer-dark
    appearance_scheme=prefer-dark
  fi

  for scene in ready empty busy error; do
    for size in wide narrow; do
      if [[ "$size" == wide ]]; then
        width=1120
        height=720
      else
        width=760
        height=520
      fi
      label="${scene}-${appearance}-${width}x${height}"
      log="$RINKA_GTK_OUTPUT/logs/${label}.log"
      RINKA_GTK_LAYOUT_PROBE=1 \
        ADW_DEBUG_COLOR_SCHEME="$appearance_scheme" \
        "$RINKA_GTK_ROOT/target/debug/rinka-explorer" \
        --scene "$scene" >"$log" 2>&1 &
      application_pid=$!
      main_window=$(window_id_for "Rinka Explorer")
      sleep 0.9
      if ! rg -q \
        'RINKA_GTK_WINDOW_CONTRACT title="Rinka Explorer" expected-content=1120x720 content=1120x720 .*result=PASS' \
        "$log"; then
        rg 'RINKA_GTK_WINDOW_CONTRACT' "$log" >&2 || cat "$log" >&2
        exit 1
      fi
      initial_geometry="$RINKA_GTK_OUTPUT/${label}-initial-geometry.txt"
      xdotool getwindowgeometry --shell "$main_window" >"$initial_geometry"
      initial_outer_width=$(geometry_value "$initial_geometry" WIDTH)
      initial_outer_height=$(geometry_value "$initial_geometry" HEIGHT)
      chrome_width=$((initial_outer_width - 1120))
      chrome_height=$((initial_outer_height - 720))
      if (( chrome_width < 0 || chrome_height < 0 )); then
        cat "$initial_geometry" >&2
        exit 1
      fi
      outer_width=$((width + chrome_width))
      outer_height=$((height + chrome_height))
      wmctrl -ir "$main_window" -e "0,20,20,${outer_width},${outer_height}"
      sleep 0.4
      assert_geometry "$main_window" "$outer_width" "$outer_height" "${label}-settled"
      sleep 1.1
      assert_geometry "$main_window" "$outer_width" "$outer_height" "${label}-stable"
      assert_content_allocation "$log" "Rinka Explorer" "$width" "$height"

      if [[ "$scene" == ready && "$appearance" == light && "$size" == wide ]]; then
        python3 "$RINKA_GTK_ROOT/scripts/gtk-a11y-probe.py" \
          --cycles 3 \
          --output "$RINKA_GTK_OUTPUT/ready-light-a11y.json"
      fi

      capture_window "$main_window" "$RINKA_GTK_OUTPUT/${label}.png"
      if [[ "$scene" == busy ]]; then
        panel_window=$(window_id_for "Connection Activity")
        xdotool getwindowgeometry --shell "$panel_window" \
          >"$RINKA_GTK_OUTPUT/${label}-panel-geometry.txt"
        if ! rg -q \
          'RINKA_GTK_WINDOW_CONTRACT .*expected-content=380x160 content=380x160 .*result=PASS' \
          "$log"; then
          rg 'RINKA_GTK_WINDOW_CONTRACT' "$log" >&2 || cat "$log" >&2
          exit 1
        fi
        xdotool windowactivate --sync "$panel_window"
        sleep 0.2
        capture_window \
          "$panel_window" \
          "$RINKA_GTK_OUTPUT/${label}-panel.png"
        python3 "$RINKA_GTK_ROOT/scripts/gtk-panel-a11y-probe.py" \
          --output "$RINKA_GTK_OUTPUT/${label}-panel-a11y.json"
      fi

      kill "$application_pid"
      wait "$application_pid" 2>/dev/null || true
      application_pid=
      sleep 0.2
    done
  done
done
SESSION

main_count=$(find "$output" -maxdepth 1 -name '*x*.png' ! -name '*-panel.png' | wc -l | tr -d ' ')
panel_count=$(find "$output" -maxdepth 1 -name '*-panel.png' | wc -l | tr -d ' ')
if [[ "$main_count" != 16 || "$panel_count" != 4 ]]; then
  echo "unexpected capture count: main=$main_count panel=$panel_count" >&2
  exit 1
fi

if rg -n "Adwaita-WARNING|Gtk-CRITICAL|GLib-GObject-CRITICAL|GTK host error|panicked at" "$output/logs"; then
  echo "GTK runtime diagnostic detected" >&2
  exit 1
fi

for size in 1120x720 760x520; do
  light_sum=$(sha256sum "$output/ready-light-${size}.png" | cut -d ' ' -f 1)
  dark_sum=$(sha256sum "$output/ready-dark-${size}.png" | cut -d ' ' -f 1)
  if [[ "$light_sum" == "$dark_sum" ]]; then
    echo "light and dark captures are identical at $size" >&2
    exit 1
  fi
done

(
  uname -a
  rustc --version
  cargo --version
  pkg-config --modversion gtk4
  pkg-config --modversion libadwaita-1
) >"$output/system.txt"

find "$output" -maxdepth 1 -name '*.png' -print0 \
  | sort -z \
  | xargs -0 sha256sum >"$output/SHA256SUMS"

jq -n \
  --argjson mainCaptures "$main_count" \
  --argjson panelCaptures "$panel_count" \
  '{mainCaptures: $mainCaptures, panelCaptures: $panelCaptures, result: "PASS"}' \
  >"$output/result.json"
cat "$output/result.json"
