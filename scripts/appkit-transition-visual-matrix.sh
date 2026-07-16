#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "AppKit external transition verification requires macOS" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
output_dir="${1:-$root/target/appkit-transition-visual-matrix}"
executable="$root/target/debug/rinka-explorer"
temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/rinka-appkit-transition.XXXXXX")"
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
  log="$output_dir/$appearance.log"
  samples="$output_dir/$appearance-frames.jsonl"
  unique="$output_dir/$appearance-unique-frames.json"
  : >"$samples"

  RINKA_APPKIT_APPEARANCE="$appearance" \
    RINKA_APPKIT_TRANSITION_PROBE=1 \
    RINKA_APPKIT_TRANSITION_PROBE_HOLD=1 \
    "$executable" --scene ready >"$log" 2>&1 &
  app_pid=$!

  baseline_ready=false
  for _ in $(seq 1 200); do
    if ! kill -0 "$app_pid" 2>/dev/null; then
      sed -n '1,320p' "$log" >&2
      echo "AppKit transition process exited before its wide baseline: $appearance" >&2
      exit 1
    fi
    if grep -q 'Rinka transition probe phase=wide baseline=' "$log"; then
      baseline_ready=true
      break
    fi
    sleep 0.01
  done
  if [[ "$baseline_ready" != true ]]; then
    sed -n '1,320p' "$log" >&2
    echo "AppKit transition wide baseline timed out: $appearance" >&2
    exit 1
  fi

  external_baseline_turns=0
  for _ in $(seq 1 200); do
    inventory="$($inventory_probe "$app_pid")"
    frame="$(jq -c '[.[] | select(.title == "Rinka Explorer")][0].bounds // empty' <<<"$inventory")"
    if [[ -n "$frame" ]] \
      && jq -e '.Width == 1120 and .Height == 720' <<<"$frame" >/dev/null; then
      external_baseline_turns=$((external_baseline_turns + 1))
      if [[ "$external_baseline_turns" == 2 ]]; then
        break
      fi
    else
      external_baseline_turns=0
    fi
    sleep 0.005
  done
  if [[ "$external_baseline_turns" != 2 ]]; then
    jq . <<<"${frame:-null}" >&2
    echo "AppKit external wide baseline timed out: $appearance" >&2
    exit 1
  fi

  completed=false
  for sample in $(seq 1 1200); do
    inventory="$($inventory_probe "$app_pid")"
    frame="$(jq -c '[.[] | select(.title == "Rinka Explorer")][0].bounds // empty' <<<"$inventory")"
    if [[ -n "$frame" ]]; then
      jq -cn --argjson sample "$sample" --argjson bounds "$frame" \
        '{sample:$sample,bounds:$bounds}' >>"$samples"
    fi
    if grep -q 'Rinka transition probe result=PASS' "$log"; then
      completed=true
      break
    fi
    if ! kill -0 "$app_pid" 2>/dev/null; then
      sed -n '1,320p' "$log" >&2
      echo "AppKit transition process exited before completion: $appearance" >&2
      exit 1
    fi
    sleep 0.005
  done

  if [[ "$completed" != true ]]; then
    sed -n '1,320p' "$log" >&2
    echo "AppKit external transition sampling timed out: $appearance" >&2
    exit 1
  fi

  jq -s '[.[].bounds] | unique_by([.X,.Y,.Width,.Height])' "$samples" >"$unique"
  if ! jq -e '
    length == 2 and
    ([.[] | select(.Width == 1120 and .Height == 720)] | length == 1) and
    ([.[] | select(.Width == 760 and .Height == 520)] | length == 1)
  ' "$unique" >/dev/null; then
    jq . "$unique" >&2
    echo "AppKit external transition geometry changed outside its two declared extents: $appearance" >&2
    exit 1
  fi

  sample_count="$(wc -l <"$samples" | tr -d ' ')"
  kill "$app_pid" 2>/dev/null || true
  wait "$app_pid" 2>/dev/null || true
  app_pid=""
  echo "AppKit external transition PASS appearance=$appearance samples=$sample_count unique_frames=2"
done

printf '{"appearances":2,"declaredFrames":2,"result":"PASS"}\n' >"$output_dir/result.json"
