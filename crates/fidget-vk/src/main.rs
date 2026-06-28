//! Fidget-VK: a Vulkan-rendered desktop fidget toy.
//!
//! Currently a Linux build (winit + ash). The Windows always-on-top,
//! click-through, tray + global-hotkey overlay described in the design doc is
//! a planned follow-up; the platform-specific shell is intentionally isolated
//! so the simulation and renderer can be reused there.

mod app;
mod config;
mod renderer;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    app::run()
}
