//! User-facing settings, persisted as JSON.
//!
//! On Linux this lives under the XDG config dir (e.g.
//! `~/.config/fidget-vk/settings.json`). The Windows port will instead use
//! `%APPDATA%/FidgetVK/settings.json`.

use serde::{Deserialize, Serialize};

use fidget_sim::WorldConfig;
use glam::{Vec2, Vec4};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BallSettings {
    pub radius: f32,
    pub restitution: f32,
    pub friction: f32,
    /// Inner/core colour, linear RGBA 0..1.
    pub color_inner: [f32; 4],
    /// Outer/glow colour, linear RGBA 0..1.
    pub color_outer: [f32; 4],
}

impl Default for BallSettings {
    fn default() -> Self {
        Self {
            radius: 48.0,
            restitution: 0.82,
            friction: 0.08,
            color_inner: [0.75, 0.92, 1.0, 1.0],
            color_outer: [0.15, 0.5, 1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VisualSettings {
    pub particles: bool,
    pub trail: bool,
    pub max_particles: usize,
}

impl Default for VisualSettings {
    fn default() -> Self {
        Self {
            particles: true,
            trail: true,
            max_particles: 2000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SimSettings {
    /// Downward gravity in px/s^2 (0 disables gravity).
    pub gravity: f32,
    pub max_speed: f32,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            gravity: 600.0,
            max_speed: 4500.0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ball: BallSettings,
    pub visuals: VisualSettings,
    pub sim: SimSettings,
}

impl Settings {
    /// Load settings from the platform config dir, falling back to defaults.
    pub fn load() -> Self {
        match Self::path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("invalid settings file, using defaults: {e}");
                Settings::default()
            }),
            None => Settings::default(),
        }
    }

    /// Persist settings to disk (best effort).
    pub fn save(&self) {
        if let Some(path) = Self::path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(text) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(path, text);
            }
        }
    }

    fn path() -> Option<std::path::PathBuf> {
        directories::ProjectDirs::from("dev", "FidgetVK", "fidget-vk")
            .map(|d| d.config_dir().join("settings.json"))
    }

    /// Build the simulation config from these settings.
    pub fn world_config(&self) -> WorldConfig {
        WorldConfig {
            gravity: Vec2::new(0.0, self.sim.gravity),
            max_speed: self.sim.max_speed,
            max_particles: self.visuals.max_particles,
            trail_enabled: self.visuals.trail,
            particles_enabled: self.visuals.particles,
            color_inner: Vec4::from_array(self.ball.color_inner),
            color_outer: Vec4::from_array(self.ball.color_outer),
            ..WorldConfig::default()
        }
    }
}
