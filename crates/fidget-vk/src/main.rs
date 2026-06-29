#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Fidget-VK: a Vulkan-rendered desktop fidget toy.
//!
//! The Linux build uses a winit preview shell; Windows uses a native Win32
//! always-on-top overlay shell with click-through, tray, and global hotkeys.
//! Platform-specific shells stay isolated so the simulation and renderer can be
//! reused across both targets.

mod app;
mod config;
mod renderer;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    app::run()
}
