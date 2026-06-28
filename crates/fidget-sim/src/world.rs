use glam::{Vec2, Vec4};

use crate::ball::Ball;
use crate::bounds::Bounds;
use crate::collisions;
use crate::interaction::InteractionState;
use crate::particles::ParticleSystem;
use crate::trail::Trail;
use crate::FIXED_DT;

/// Tunable simulation parameters. The renderer reads colours from here too so
/// the "material" of the ball is data-driven.
#[derive(Debug, Clone)]
pub struct WorldConfig {
    /// Downward acceleration in px/s^2. Defaults to a gentle pull.
    pub gravity: Vec2,
    /// Per-second velocity damping applied to the free-flying ball.
    pub air_drag: f32,
    /// Per-second damping applied to particles.
    pub particle_drag: f32,
    pub max_speed: f32,
    pub max_particles: usize,
    pub trail_enabled: bool,
    pub particles_enabled: bool,
    /// Inner (core) colour of the ball.
    pub color_inner: Vec4,
    /// Outer (rim/glow) colour of the ball.
    pub color_outer: Vec4,
    /// Speed below which the ball is considered still (for sleeping).
    pub sleep_speed: f32,
    /// Seconds of stillness before the ball sleeps.
    pub sleep_delay: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            gravity: Vec2::new(0.0, 600.0),
            air_drag: 0.12,
            particle_drag: 1.2,
            max_speed: 4500.0,
            max_particles: 2000,
            trail_enabled: true,
            particles_enabled: true,
            color_inner: Vec4::new(0.65, 0.85, 1.0, 1.0),
            color_outer: Vec4::new(0.1, 0.45, 1.0, 1.0),
            sleep_speed: 5.0,
            sleep_delay: 2.0,
        }
    }
}

/// The full simulation: ball + trail + particles, advanced with a fixed
/// timestep accumulator so behaviour is framerate independent.
pub struct World {
    pub config: WorldConfig,
    pub bounds: Bounds,
    pub ball: Ball,
    pub trail: Trail,
    pub particles: ParticleSystem,
    pub interaction: InteractionState,

    accumulator: f32,
    cursor: Vec2,
    mote_accum: f32,
}

impl World {
    pub fn new(config: WorldConfig, bounds: Bounds) -> Self {
        let ball = Ball::new(bounds.center(), 42.0);
        let particles = ParticleSystem::new(config.max_particles);
        let trail = Trail::new(64, 0.45);
        Self {
            cursor: ball.pos,
            config,
            bounds,
            ball,
            trail,
            particles,
            interaction: InteractionState::default(),
            accumulator: 0.0,
            mote_accum: 0.0,
        }
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = bounds;
    }

    /// Reset the ball to the centre, at rest.
    pub fn reset(&mut self) {
        let r = self.ball.radius;
        self.ball = Ball::new(self.bounds.center(), r);
        self.trail.clear();
    }

    pub fn toggle_gravity(&mut self) {
        if self.config.gravity == Vec2::ZERO {
            self.config.gravity = Vec2::new(0.0, 600.0);
        } else {
            self.config.gravity = Vec2::ZERO;
        }
        self.ball.wake();
    }

    // --- Interaction -------------------------------------------------------

    /// Returns true if a grab started (cursor was over the ball).
    pub fn grab(&mut self, cursor: Vec2, now: f32) -> bool {
        self.cursor = cursor;
        if InteractionState::hit_test(&self.ball, cursor) {
            self.interaction.begin_grab(&mut self.ball, cursor, now);
            // Small attraction motes toward the cursor when grabbed.
            if self.config.particles_enabled {
                self.particles
                    .emit_motes(cursor, Vec2::ZERO, 6, self.config.color_inner);
            }
            true
        } else {
            false
        }
    }

    pub fn move_cursor(&mut self, cursor: Vec2, now: f32) {
        self.cursor = cursor;
        if self.ball.grabbed {
            self.interaction.update_cursor(cursor, now);
        }
    }

    pub fn release(&mut self, now: f32) {
        if self.ball.grabbed {
            self.interaction.release(&mut self.ball, now);
            if self.config.particles_enabled {
                self.particles
                    .emit_burst(self.ball.pos, self.ball.speed(), self.config.color_outer);
            }
        }
    }

    pub fn is_grabbed(&self) -> bool {
        self.ball.grabbed
    }

    // --- Stepping ----------------------------------------------------------

    /// Advance the world by an arbitrary frame delta using fixed sub-steps.
    pub fn advance(&mut self, frame_dt: f32) {
        // Avoid the "spiral of death" if the app stalls.
        self.accumulator = (self.accumulator + frame_dt).min(0.25);
        while self.accumulator >= FIXED_DT {
            self.step(FIXED_DT);
            self.accumulator -= FIXED_DT;
        }
        // Trail and particles can use the variable delta directly.
        self.trail.update(frame_dt);
        self.particles.update(frame_dt, self.config.particle_drag);
    }

    fn step(&mut self, dt: f32) {
        if self.ball.grabbed {
            self.interaction.apply_spring(&mut self.ball, self.cursor, dt);
        } else if !self.ball.asleep {
            // Integrate gravity + drag.
            self.ball.vel += self.config.gravity * dt;
            self.ball.vel *= 1.0 - self.config.air_drag * dt;
            // Clamp speed.
            let sp = self.ball.vel.length();
            if sp > self.config.max_speed {
                self.ball.vel *= self.config.max_speed / sp;
            }
            self.ball.pos += self.ball.vel * dt;
        }

        // Decay the squash impulse.
        self.ball.squash_impulse *= (-dt * 14.0).exp();

        // Resolve walls and emit impact sparks.
        let impacts = collisions::resolve_walls(&mut self.ball, &self.bounds);
        if self.config.particles_enabled {
            for im in &impacts {
                if im.speed > 60.0 {
                    self.particles
                        .emit_impact(im.point, im.normal, im.speed, self.config.color_outer);
                }
            }
        }

        // Record the trail and emit motes while moving. A sleeping ball must
        // not keep re-seeding the trail, otherwise it never goes idle.
        if self.config.trail_enabled && !self.ball.asleep {
            self.trail.record(self.ball.pos, self.ball.radius);
        }
        if self.config.particles_enabled && !self.ball.grabbed {
            let speed = self.ball.speed();
            if speed > 200.0 {
                self.mote_accum += speed * dt;
                while self.mote_accum > 400.0 {
                    self.particles
                        .emit_motes(self.ball.pos, self.ball.vel, 1, self.config.color_inner);
                    self.mote_accum -= 400.0;
                }
            }
        }

        // Sleep handling: stop integrating when at rest with no interaction.
        if !self.ball.grabbed {
            if self.ball.speed() < self.config.sleep_speed && self.config.gravity == Vec2::ZERO {
                self.ball.still_time += dt;
                if self.ball.still_time > self.config.sleep_delay {
                    self.ball.asleep = true;
                    self.ball.vel = Vec2::ZERO;
                }
            } else {
                self.ball.still_time = 0.0;
            }
        }
    }

    /// Whether anything is visibly animating (used for idle frame pacing).
    pub fn is_active(&self) -> bool {
        self.ball.grabbed
            || !self.ball.asleep
            || !self.particles.is_empty()
            || !self.trail.is_empty()
    }
}
