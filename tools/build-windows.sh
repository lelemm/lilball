#!/usr/bin/env bash
set -euo pipefail

TARGET="${TARGET:-x86_64-pc-windows-gnu}"
CRATE="${CRATE:-fidget-vk}"

if ! command -v glslangValidator >/dev/null 2>&1; then
  echo "glslangValidator is required. Install glslang-tools." >&2
  exit 1
fi

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "x86_64-w64-mingw32-gcc is required. Install mingw-w64." >&2
  exit 1
fi

rustup target add "$TARGET"
cargo build --release -p "$CRATE" --target "$TARGET"

echo "Built target/$TARGET/release/fidget-vk.exe"
