use glam::{Vec2, Vec4};

use crate::ball::Ball;
use crate::bounds::Bounds;
use crate::collisions;
use crate::interaction::InteractionState;
use crate::particles::ParticleSystem;
use crate::spring::SpringState;
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
    pub spring: SpringState,

    accumulator: f32,
    cursor: Vec2,
    cursor_vel: Vec2,
    cursor_time: Option<f32>,
    mote_accum: f32,
    nudge_seed: u32,
}

impl World {
    pub fn new(config: WorldConfig, bounds: Bounds) -> Self {
        let ball = Ball::new(bounds.center(), 42.0);
        let spring = SpringState::new(bounds, ball.pos);
        let particles = ParticleSystem::new(config.max_particles);
        let trail = Trail::new(64, 0.45);
        Self {
            cursor: ball.pos,
            cursor_vel: Vec2::ZERO,
            cursor_time: None,
            config,
            bounds,
            ball,
            trail,
            particles,
            interaction: InteractionState::default(),
            spring,
            accumulator: 0.0,
            mote_accum: 0.0,
            nudge_seed: 0x9E37_79B9,
        }
    }

    /// Launch the ball in a pseudo-random direction at `speed` px/s. Handy as
    /// a "fling" hotkey and as the optional random-nudge wake behaviour.
    pub fn nudge(&mut self, speed: f32) {
        self.nudge_seed = self
            .nudge_seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        let angle = (self.nudge_seed >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU;
        self.ball.vel = Vec2::new(angle.cos(), angle.sin()) * speed;
        self.ball.wake();
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = bounds;
        self.spring.set_bounds(bounds);
    }

    /// Reset the ball onto the intact spring, at rest.
    pub fn reset(&mut self) {
        self.recall_to_spring();
    }

    /// Cut the anchor spring so gravity can pull the ball out of the bottom of
    /// the play area.
    pub fn cut_spring(&mut self) {
        let cut_impulse = self.spring.cut_impulse();
        self.spring.cut();
        self.ball.vel += cut_impulse;
        self.ball.wake();
    }

    /// Reattach the spring and recall the ball to its hanging rest position.
    pub fn attach_spring(&mut self) {
        self.recall_to_spring();
    }

    pub fn toggle_spring(&mut self) {
        if self.spring.attached {
            self.cut_spring();
        } else {
            self.attach_spring();
        }
    }

    pub fn spring_attached(&self) -> bool {
        self.spring.attached
    }

    fn recall_to_spring(&mut self) {
        let r = self.ball.radius;
        self.spring.attach();
        self.ball = Ball::new(self.spring.rest_position(), r);
        self.cursor_vel = Vec2::ZERO;
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

    pub fn gravity_strength(&self) -> f32 {
        self.config.gravity.y
    }

    pub fn set_gravity_strength(&mut self, gravity: f32) {
        self.config.gravity = Vec2::new(0.0, gravity.clamp(0.0, 2400.0));
        self.ball.wake();
    }

    pub fn spring_stiffness(&self) -> f32 {
        self.spring.stiffness
    }

    pub fn set_spring_stiffness(&mut self, stiffness: f32) {
        self.spring.stiffness = stiffness.clamp(15.0, 420.0);
        self.ball.wake();
    }

    pub fn spring_damping(&self) -> f32 {
        self.spring.damping
    }

    pub fn set_spring_damping(&mut self, damping: f32) {
        self.spring.damping = damping.clamp(2.0, 90.0);
        self.ball.wake();
    }

    pub fn hook_offset_y(&self) -> f32 {
        self.spring.hook_offset_y
    }

    pub fn set_hook_offset_y(&mut self, offset_y: f32) {
        self.spring.set_hook_offset_y(self.bounds, offset_y);
        self.ball.wake();
    }

    // --- Interaction -------------------------------------------------------

    /// Returns true if a grab started (cursor was over the ball).
    pub fn grab(&mut self, cursor: Vec2, now: f32) -> bool {
        self.cursor = cursor;
        if InteractionState::hit_test(&self.ball, cursor) {
            self.spring.entanglement = None;
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
        let prev_cursor = self.cursor;
        if let Some(prev_time) = self.cursor_time {
            let dt = now - prev_time;
            if dt > 1e-4 {
                self.cursor_vel = (cursor - self.cursor) / dt;
            }
        }
        self.cursor_time = Some(now);
        self.cursor = cursor;
        if self.ball.grabbed {
            self.interaction.update_cursor(cursor, now);
        } else {
            let displaced = self.spring.update_intersection_sweep(
                &self.ball,
                prev_cursor,
                cursor,
                self.cursor_vel,
            );
            let entangled =
                self.spring
                    .try_entangle_sweep(&self.ball, prev_cursor, cursor, self.cursor_vel);
            if displaced || entangled {
                self.ball.wake();
            }
            if self.config.particles_enabled {
                let count = if entangled { 10 } else { 3 };
                if displaced || entangled {
                    self.particles.emit_motes(
                        cursor,
                        self.cursor_vel,
                        count,
                        self.config.color_outer,
                    );
                }
            }
        }
    }

    pub fn release(&mut self, now: f32) {
        if self.ball.grabbed {
            self.interaction.release(&mut self.ball, now);
            if self.config.particles_enabled {
                self.particles.emit_burst(
                    self.ball.pos,
                    self.ball.speed(),
                    self.config.color_outer,
                );
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
            self.interaction
                .apply_spring(&mut self.ball, self.cursor, dt);
        } else if !self.ball.asleep {
            self.spring.update_transients(self.cursor, dt);
            // Integrate gravity, the anchored spring, and drag.
            let spring_force = self.spring.force_on(&self.ball);
            self.ball.vel += (self.config.gravity + spring_force / self.ball.mass) * dt;
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
        let impacts = collisions::resolve_walls_with_bottom(
            &mut self.ball,
            &self.bounds,
            self.spring.attached,
        );
        if self.config.particles_enabled {
            for im in &impacts {
                if im.speed > 60.0 {
                    self.particles.emit_impact(
                        im.point,
                        im.normal,
                        im.speed,
                        self.config.color_outer,
                    );
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
                    self.particles.emit_motes(
                        self.ball.pos,
                        self.ball.vel,
                        1,
                        self.config.color_inner,
                    );
                    self.mote_accum -= 400.0;
                }
            }
        }

        if self.spring.should_recall(&self.ball, self.bounds) {
            self.recall_to_spring();
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
            || self.spring.intersection.is_some()
            || self.spring.entanglement.is_some()
            || !self.ball.asleep
            || !self.particles.is_empty()
            || !self.trail.is_empty()
    }
}
