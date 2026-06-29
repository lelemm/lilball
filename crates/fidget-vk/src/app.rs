//! Platform shell selector for Fidget-VK.

use anyhow::Result;

mod core;

#[cfg(target_os = "windows")]
mod win32_shell;

#[cfg(not(target_os = "windows"))]
mod winit_shell;

pub fn run() -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        win32_shell::run()
    }

    #[cfg(not(target_os = "windows"))]
    {
        winit_shell::run()
    }
}
