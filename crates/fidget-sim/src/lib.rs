//! Platform-independent simulation for the Fidget-VK overlay toy.
//!
//! This crate contains everything that does not depend on Win32 or Vulkan:
//! the ball physics, wall collisions, drag/throw interaction, the particle
//! pool, and the motion trail. Keeping it platform independent means the core
//! "feel" of the toy can be exercised and unit-tested on any host (including
//! Linux CI), while the `fidget-vk` binary owns the Windows/Vulkan shell.

pub mod ball;
pub mod bounds;
pub mod collisions;
pub mod interaction;
pub mod particles;
pub mod spring;
pub mod trail;
pub mod world;

pub use ball::Ball;
pub use bounds::{BottomEdge, Bounds};
pub use interaction::{InteractionState, PointerSample};
pub use particles::{Particle, ParticleKind, ParticleSystem};
pub use spring::{CursorEntanglement, CursorIntersection, SpringState};
pub use trail::{Trail, TrailPoint};
pub use world::{World, WorldConfig};

pub use glam::Vec2;

/// Fixed simulation timestep (seconds). 120 Hz gives stable, low-tunnelling
/// physics independent of the display refresh rate.
pub const FIXED_DT: f32 = 1.0 / 120.0;
