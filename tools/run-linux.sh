#!/usr/bin/env bash
# Build (release) and run the Fidget-VK app on Linux.
#
# In headless/cloud environments without a real GPU, this falls back to Mesa
# lavapipe (software Vulkan) and an existing X server (defaults to :1).
set -euo pipefail
cd "$(dirname "$0")/.."

export DISPLAY="${DISPLAY:-:1}"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/xdg-runtime}"
mkdir -p "$XDG_RUNTIME_DIR"

# Force the lavapipe ICD only if no real driver is configured.
if [ -z "${VK_ICD_FILENAMES:-}" ] && [ -f /usr/share/vulkan/icd.d/lvp_icd.json ]; then
  export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
fi

export RUST_LOG="${RUST_LOG:-info}"
cargo build --release
exec ./target/release/fidget-vk "$@"
