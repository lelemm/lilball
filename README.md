# Fidget-VK

Fidget-VK is a Vulkan-rendered desktop fidget toy with two play modes: a textured
ball hanging from a spring that can be dragged, thrown, cut loose, recalled, and
tangled with the cursor, and a **Marbles** mode where procedural glass marbles
bounce around the desktop, collide with each other, crack on hard impacts, and
shatter into glass flecks.

The app has a Linux transparent overlay preview built with Rust, winit,
ash/Vulkan, and egui. On Windows it now uses a native Win32 shell for the
original desktop-overlay behavior: topmost transparent windowing, per-object
click-through, a tray menu, and global hotkeys around the same simulation and
Vulkan renderer.

## Screenshots

### Transparent full-desktop overlay

![Transparent Linux overlay](docs/images/transparent-overlay.png)

### Image-backed soccer ball material

![Soccer ball material in the overlay](docs/images/soccer-ball.png)

### egui parameter HUD

![egui parameter HUD](docs/images/egui-hud.png)

### Marbles mode on the desktop

![Marbles scattered on the desktop](docs/images/marbles-desktop.png)

### Marbles mode HUD

![Marbles mode controls](docs/images/marbles-hud.png)

### Marbles scatter

![Marbles after a scatter fling](docs/images/marbles-scatter.png)

## Features

- Full-desktop transparent Linux overlay preview.
- Vulkan rendering through `ash`.
- Image-backed soccer ball material with shader lighting and seam relief.
- Spring physics with gravity, damping, elasticity, and off-screen hook support.
- Hold right-click near the spring to grab/deflect it and push the ball.
- Very fast right-click sweeps across the spring can create temporary entanglement.
- Very fast cursor sweeps across the spring cut it; the threshold is configurable.
- Cut/recall behavior: release the ball, let it fall, then recall it to the spring.
- Motion trails, particles, impact sparks, squash/stretch, and glow.
- **Marbles mode** (`P` or the HUD mode selector):
  - procedural glass marbles with unique swirl patterns and specular lighting
  - desktop snapshot backdrop so marbles sit on top of your wallpaper
  - marble–marble and marble–wall collisions with spin and rolling
  - left-drag grab/throw and right-click kick/sweep interactions
  - hard impacts crack marbles; shattered marbles emit glass fleck particles
  - spawn, scatter, and clear controls in the HUD and tray menu
  - multi-monitor aware bounds (marbles bounce off visible monitor edges)
- egui HUD for runtime tuning:
  - gravity
  - string elasticity/stiffness
  - string damping
  - hook Y offset
  - reset
  - cut/recall
- Native Win32 overlay shell with per-object click-through.
- Windows tray menu for HUD, mode toggle, marble spawn/scatter/clear, reset,
  cut/recall, gravity, fling, and quit.
- Windows global hotkeys for core actions.
- Linux, native Windows, and Linux-to-Windows cross-build CI.
- Release packaging for Linux and Windows zip assets.

## Controls

### Shared

- `P`: toggle Fidget / Marbles mode.
- `H`: show/hide the HUD.
- `Esc`: quit.
- `1` / `2` / `3`: small / medium / large toy size (marble radius range in
  Marbles mode).

### Fidget mode

- Left drag: grab/throw the ball.
- Hold right-click near string: grab/deflect the spring.
- Very fast right-click sweep across string: temporary entanglement until release.
- Very fast sweep across string: cut the spring.
- `C`: cut/recall the spring.
- `N`: fling the ball.
- `G`: toggle gravity.
- `R` or `Space`: reset the ball.

### Marbles mode

- Left drag: grab and throw a marble.
- Hold right-click and sweep: kick marbles (fast sweeps impart more damage).
- `O`: spawn a marble.
- `F`: scatter all marbles.
- `Delete`: clear all marbles.
- `N`: scatter/fling all marbles.
- `R` or `Space`: spawn a marble.

On Windows, the same actions are also available from the tray menu. Global
hotkeys use `Ctrl+Alt` plus the matching key, for example `Ctrl+Alt+H` for the
HUD, `Ctrl+Alt+P` to toggle mode, and `Ctrl+Alt+Esc` to quit.

## Build and test

### Linux

Install system dependencies:

```bash
sudo apt-get update
sudo apt-get install -y \
  glslang-tools \
  libvulkan1 \
  mesa-vulkan-drivers \
  pkg-config \
  libx11-dev \
  libxcb1-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev \
  libwayland-dev \
  libxrandr-dev \
  libxi-dev \
  libxcursor-dev \
  libxinerama-dev
```

Build and run:

```bash
cargo build --release -p fidget-vk
./target/release/fidget-vk
```

Run checks:

```bash
cargo test -p fidget-sim
cargo clippy --workspace --all-targets -- -D warnings
```

### Windows cross-build from Linux/WSL

Install cross-build dependencies:

```bash
sudo apt-get install -y glslang-tools mingw-w64 gcc-mingw-w64-x86-64
```

Build the Windows executable:

```bash
tools/build-windows.sh
```

Output:

```text
target/x86_64-pc-windows-gnu/release/fidget-vk.exe
```

## GitHub Actions

The CI workflow runs on:

- pushes to `main`
- pushes to `cursor/**`
- pull requests targeting `main`
- published GitHub releases

Release builds upload zip packages:

- `fidget-vk-linux-x86_64.zip`
- `fidget-vk-windows-x86_64.zip`

The Windows cross-build job also uploads:

- `fidget-vk-windows-x86_64-cross.zip`

## Current status

Ready:

- Linux transparent overlay preview.
- Marbles mode with procedural glass marbles and shatter particles.
- Native Win32 transparent overlay shell.
- Per-object click-through on Windows.
- Windows system tray menu.
- Windows global hotkeys.
- Linux-to-Windows GNU cross-compilation.
- CI and release packaging.

Planned:

- Windows startup/install packaging.
