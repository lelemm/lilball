#!/usr/bin/env bash
set -euo pipefail

export DISPLAY="${DISPLAY:-:1}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/xdg-runtime}"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
mkdir -p "$XDG_RUNTIME_DIR"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/fidget-vk"
OUT_DIR="$ROOT/docs/images"
SETTINGS_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/fidget-vk"

mkdir -p "$OUT_DIR" "$SETTINGS_DIR"

cat >"$SETTINGS_DIR/settings.json" <<'EOF'
{
  "mode": "marbles",
  "ball": {
    "radius": 48.0,
    "restitution": 0.82,
    "friction": 0.08,
    "color_inner": [0.75, 0.92, 1.0, 1.0],
    "color_outer": [0.15, 0.5, 1.0, 1.0]
  },
  "visuals": {
    "particles": true,
    "trail": true,
    "max_particles": 2600,
    "spring_visual": "spring",
    "rubber_band_thickness": 0.72
  },
  "sim": {
    "gravity": 600.0,
    "max_speed": 4500.0,
    "cut_spring_cursor_speed": 3600.0,
    "bounce_bottom_edge": false,
    "single_monitor_bounds": false,
    "toy_size": "large"
  }
}
EOF

pkill -f 'target/release/fidget-vk' 2>/dev/null || true
sleep 0.5

"$BIN" &
APP_PID=$!

echo "Waiting for fidget-vk window..."
WID=""
for _ in $(seq 1 80); do
  WID="$(xdotool search --name 'Fidget-VK' 2>/dev/null | head -1 || true)"
  if [[ -n "$WID" ]]; then
    break
  fi
  sleep 0.25
done
if [[ -z "$WID" ]]; then
  echo "failed to find Fidget-VK window" >&2
  kill "$APP_PID" 2>/dev/null || true
  exit 1
fi

sleep 2
xdotool windowfocus "$WID"
sleep 0.3
xdotool mousemove 960 600 click 1
sleep 0.3

echo "Seeding marbles..."
for _ in $(seq 1 28); do
  xdotool key --window "$WID" r
  sleep 0.06
done
sleep 0.8

echo "Capturing marbles on desktop..."
scrot -o "$OUT_DIR/marbles-desktop.png"

echo "Showing HUD..."
xdotool key --window "$WID" h
sleep 0.8
scrot -o "$OUT_DIR/marbles-hud.png"
xdotool key --window "$WID" h
sleep 0.3

echo "Scattering marbles..."
xdotool key --window "$WID" n
sleep 1.2
scrot -o "$OUT_DIR/marbles-scatter.png"

echo "Stopping fidget-vk..."
kill "$APP_PID" 2>/dev/null || true
wait "$APP_PID" 2>/dev/null || true

ls -lh "$OUT_DIR"/marbles-*.png
