#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit visual verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$root/target/appkit-visual-matrix}"
executable="$root/target/debug/rinka-explorer"
temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/rinka-appkit-visual.XXXXXX")"
inventory_probe="$temporary_dir/appkit-window-inventory"
app_pid=""

cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -rf "$temporary_dir"
}
trap cleanup EXIT

mkdir -p "$output_dir"
"$root/scripts/bundle-macos.sh" >/dev/null
xcrun swiftc "$root/scripts/appkit-window-inventory.swift" -o "$inventory_probe"

for appearance in light dark; do
  for scene in ready empty busy error; do
    label="$scene-$appearance"
    log="$output_dir/$label.log"
    inventory_path="$output_dir/$label-windows.json"
    expected_count=1
    if [[ "$scene" == "busy" ]]; then
      expected_count=2
    fi

    RINKA_APPKIT_APPEARANCE="$appearance" \
      RINKA_APPKIT_SCENE_PROBE="$scene" \
      RINKA_APPKIT_SCENE_PROBE_HOLD=1 \
      RINKA_APPKIT_WINDOW_LIVE_PROBE=1 \
      "$executable" --scene "$scene" >"$log" 2>&1 &
    app_pid=$!

    previous_inventory=""
    stable_inventory=""
    for _ in $(seq 1 100); do
      if ! kill -0 "$app_pid" 2>/dev/null; then
        sed -n '1,260p' "$log" >&2
        echo "AppKit visual process exited before capture: $label" >&2
        exit 1
      fi
      current_inventory="$($inventory_probe "$app_pid")"
      current_count="$(jq 'length' <<<"$current_inventory")"
      if [[ "$current_count" == "$expected_count" ]] \
        && [[ "$current_inventory" == "$previous_inventory" ]] \
        && grep -q "Rinka scene probe scene=$scene result=PASS" "$log"; then
        stable_inventory="$current_inventory"
        break
      fi
      previous_inventory="$current_inventory"
      sleep 0.1
    done

    if [[ -z "$stable_inventory" ]]; then
      sed -n '1,260p' "$log" >&2
      echo "AppKit windows did not reach stable external geometry: $label" >&2
      exit 1
    fi
    printf '%s\n' "$stable_inventory" >"$inventory_path"

    if ! jq -e '
      [.[] | select(.title == "Rinka Explorer")] | length == 1 and
      (.[0].bounds.Width == 1120) and
      (.[0].bounds.Height == 720)
    ' <<<"$stable_inventory" >/dev/null; then
      jq . <<<"$stable_inventory" >&2
      echo "AppKit main-window bounds do not match 1120 by 720: $label" >&2
      exit 1
    fi
    if [[ "$scene" == "busy" ]] && ! jq -e '
      [.[] | select(.title == "Connection Activity")] | length == 1 and
      (.[0].bounds.Width == 380) and
      (.[0].bounds.Height == 192)
    ' <<<"$stable_inventory" >/dev/null; then
      jq . <<<"$stable_inventory" >&2
      echo "AppKit activity-panel bounds are invalid: $label" >&2
      exit 1
    fi

    main_id="$(jq -r '.[] | select(.title == "Rinka Explorer") | .id' <<<"$stable_inventory")"
    screencapture -x -o -l "$main_id" "$output_dir/$label-main.png"
    if [[ "$scene" == "busy" ]]; then
      panel_id="$(jq -r '.[] | select(.title == "Connection Activity") | .id' <<<"$stable_inventory")"
      screencapture -x -o -l "$panel_id" "$output_dir/$label-panel.png"
    fi

    if ! grep -q "Rinka scene probe scene=$scene result=PASS" "$log"; then
      sed -n '1,260p' "$log" >&2
      echo "AppKit scene did not pass before visual capture: $label" >&2
      exit 1
    fi

    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
    app_pid=""
    echo "AppKit visual PASS appearance=$appearance scene=$scene windows=$expected_count"
  done
done

shasum -a 256 "$output_dir"/*.png >"$output_dir/SHA256SUMS"
for png in "$output_dir"/*.png; do
  pixel_width="$(sips -g pixelWidth "$png" | awk '/pixelWidth:/ {print $2}')"
  pixel_height="$(sips -g pixelHeight "$png" | awk '/pixelHeight:/ {print $2}')"
  printf '%s\t%s\t%s\n' "$(basename "$png")" "$pixel_width" "$pixel_height"
done >"$output_dir/pixel-dimensions.tsv"

printf '{"mainCaptures":8,"panelCaptures":2,"result":"PASS"}\n' \
  >"$output_dir/result.json"
