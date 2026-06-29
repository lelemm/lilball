# Fidget-VK

A Vulkan-rendered desktop "fidget toy": a glowing ball you can drag, throw,
and watch bounce around, with motion trails and particles. Inspired by the
feel of squishy/throwable desktop toys.

The original design targets a Windows always-on-top transparent overlay
(Win32 + `ash`). The current code keeps a Linux preview shell (winit + `ash`)
and uses a native Win32 shell on Windows for topmost transparent overlay
windowing, per-object click-through, tray menu actions, and global hotkeys. The
simulation and renderer are deliberately platform independent so both shells can
reuse them.

## Workspace layout

- `crates/fidget-sim` — platform-independent simulation (ball physics, wall
  collisions, drag/throw interaction, particles, motion trail). Pure Rust, no
  graphics or OS dependencies, fully unit tested.
- `crates/fidget-vk` — the app: Linux `winit` shell, native Win32 shell, and
  shared `ash` Vulkan renderer.
- `shaders/` — GLSL compiled to SPIR-V at build time by `build.rs`.

## Build / test / run

- Build: `cargo build` (debug) or `cargo build --release`.
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`.
- Tests: `cargo test -p fidget-sim` (the simulation is the testable core).
- Headless physics demo (no GPU needed): `cargo run -p fidget-sim --bin sim_demo`.
- Run the GUI app: `cargo run -p fidget-vk` (or `./target/release/fidget-vk`).

In-app controls: left-drag to grab/throw the ball, hold right-click near the
spring to grab/deflect it, sweep very quickly while holding right-click to
briefly entangle it, sweep very quickly across the spring to cut it, `C` to
cut/recall the spring, `N` to fling it in a random direction, `G` to toggle
gravity, `H` to show/hide the egui parameter HUD, `R`/`Space` to reset, `Esc`
to quit. The HUD exposes gravity, string
elasticity/stiffness, damping, and hook Y offset, including explicit Hook higher
/ Hook lower buttons; negative hook offset places the string hook off-screen
above the desktop.

## Cursor Cloud specific instructions

- Rendering uses Mesa **lavapipe** (software Vulkan, device name `llvmpipe`) —
  there is no hardware GPU. It works but is CPU-bound, so prefer
  `--release` when running the GUI and keep the window modest in size.
- The GUI needs an X server. Run it against the desktop display with these
  env vars (lavapipe + a valid runtime dir, and force the lavapipe ICD so the
  loader doesn't waste time on absent hardware ICDs):
  - `export DISPLAY=:1`
  - `export XDG_RUNTIME_DIR=/tmp/xdg-runtime && mkdir -p $XDG_RUNTIME_DIR`
  - `export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`
  Then `./target/release/fidget-vk`. Run it from a tmux session so it keeps
  running while you inspect it.
- Note: `vulkaninfo --summary` crashes on this X server's WSI probe
  (`BadMatch` on `X_CreateWindow`); this is a `vulkaninfo` quirk, **not** a
  problem with the app — actual swapchain rendering (and `vkcube`) work fine.
- Automated mouse tools cannot "flick" fast enough at button-release to impart
  throw velocity, so a mouse-drag release looks like the ball just drops. Use
  the `N` (fling) key to demonstrate momentum / bouncing / trails instead.
- System dependencies (not installed by the update script; present in the VM
  image / snapshot): `glslang-tools` (provides `glslangValidator`, required by
  `build.rs`), `mesa-vulkan-drivers` + `vulkan-tools` + `libvulkan1`, the
  winit X11/Wayland libs (`libxkbcommon-*`, `libx11-dev`, `libxcb1-dev`,
  `libxrandr-dev`, `libxi-dev`, `libwayland-dev`), and a Rust toolchain
  **>= 1.85** (deps such as `wayland-protocols` require edition 2024). The
  `mingw-w64` cross toolchain is installed for Windows cross-builds.
