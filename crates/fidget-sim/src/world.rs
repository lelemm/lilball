use glam::{Vec2, Vec4};

use crate::FIXED_DT;
use crate::ball::Ball;
use crate::bounds::{BottomEdge, Bounds};
use crate::collisions;
use crate::interaction::InteractionState;
use crate::particles::ParticleSystem;
use crate::spring::SpringState;
use crate::trail::Trail;

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
    pub ball_radius: f32,
    pub spring_interaction_scale: f32,
    pub spring_length_scale: f32,
    pub max_particles: usize,
    pub trail_enabled: bool,
    pub particles_enabled: bool,
    /// If true, detached balls bounce from the bottom edge instead of falling
    /// through the pit below the virtual desktop.
    pub bounce_bottom_edge: bool,
    /// Inner (core) colour of the ball.
    pub color_inner: Vec4,
    /// Outer (rim/glow) colour of the ball.
    pub color_outer: Vec4,
    /// Speed below which the ball is considered still (for sleeping).
    pub sleep_speed: f32,
    /// Seconds of stillness before the ball sleeps.
    pub sleep_delay: f32,
    /// Cursor sweep speed above which crossing the spring cuts it.
    pub cut_spring_cursor_speed: f32,
    /// Effective cursor radius while right-click batting a detached ball.
    pub cursor_ball_radius: f32,
    /// Fraction of cursor momentum transferred through a loose-ball hit.
    pub cursor_ball_momentum_transfer: f32,
    /// Restitution-like bounce from right-click cursor hits.
    pub cursor_ball_bounce: f32,
    /// Per-second damping applied to ball spin.
    pub spin_drag: f32,
    /// Sideways acceleration scale from spin while the ball is moving.
    pub spin_curve: f32,
    pub max_spin_curve_accel: f32,
    pub sleep_spin: f32,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            gravity: Vec2::new(0.0, 600.0),
            air_drag: 0.12,
            particle_drag: 1.2,
            max_speed: 4500.0,
            ball_radius: 42.0,
            spring_interaction_scale: 1.0,
            spring_length_scale: 1.0,
            max_particles: 2000,
            trail_enabled: true,
            particles_enabled: true,
            bounce_bottom_edge: false,
            color_inner: Vec4::new(0.65, 0.85, 1.0, 1.0),
            color_outer: Vec4::new(0.1, 0.45, 1.0, 1.0),
            sleep_speed: 5.0,
            sleep_delay: 2.0,
            cut_spring_cursor_speed: 3600.0,
            cursor_ball_radius: 18.0,
            cursor_ball_momentum_transfer: 0.14,
            cursor_ball_bounce: 0.75,
            spin_drag: 0.8,
            spin_curve: 0.024,
            max_spin_curve_accel: 1800.0,
            sleep_spin: 0.08,
        }
    }
}

/// The full simulation: ball + trail + particles, advanced with a fixed
/// timestep accumulator so behaviour is framerate independent.
pub struct World {
    pub config: WorldConfig,
    pub bounds: Bounds,
    pub bottom_edges: Vec<BottomEdge>,
    pub ball: Ball,
    pub trail: Trail,
    pub particles: ParticleSystem,
    pub interaction: InteractionState,
    pub spring: SpringState,
    pub ball_visible: bool,

    accumulator: f32,
    cursor: Vec2,
    cursor_vel: Vec2,
    cursor_time: Option<f32>,
    detached_cursor_interaction_active: bool,
    mote_accum: f32,
    nudge_seed: u32,
}

impl World {
    pub fn new(config: WorldConfig, bounds: Bounds) -> Self {
        let mut spring = SpringState::new(bounds, bounds.center());
        spring.set_interaction_scale(config.spring_interaction_scale);
        spring.set_length_scale(bounds, config.spring_length_scale);
        let ball = Ball::new(spring.rest_position(), config.ball_radius.clamp(12.0, 96.0));
        let particles = ParticleSystem::new(config.max_particles);
        let trail = Trail::new(64, 0.45);
        Self {
            cursor: ball.pos,
            cursor_vel: Vec2::ZERO,
            cursor_time: None,
            config,
            bounds,
            bottom_edges: vec![BottomEdge::from_bounds(bounds)],
            ball,
            trail,
            particles,
            interaction: InteractionState::default(),
            spring,
            ball_visible: true,
            accumulator: 0.0,
            mote_accum: 0.0,
            detached_cursor_interaction_active: false,
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
        self.ball.spin = 0.0;
        self.ball.wake();
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = bounds;
        self.bottom_edges = vec![BottomEdge::from_bounds(bounds)];
        self.spring.set_bounds(bounds);
    }

    pub fn set_bottom_edges<I>(&mut self, edges: I)
    where
        I: IntoIterator<Item = BottomEdge>,
    {
        self.bottom_edges = edges
            .into_iter()
            .filter(|edge| edge.width() > 0.0)
            .collect();
        if self.bottom_edges.is_empty() {
            self.bottom_edges.push(BottomEdge::from_bounds(self.bounds));
        }
        self.ball.wake();
    }

    /// Reset the ball onto the intact spring, at rest.
    pub fn reset(&mut self) {
        self.recall_to_spring();
    }

    /// Cut the anchor spring so gravity can pull the ball out of the bottom of
    /// the play area.
    pub fn cut_spring(&mut self) {
        if !self.ball_visible {
            return;
        }
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
        if !self.ball_visible {
            self.recall_to_spring();
        } else if self.spring.attached {
            self.cut_spring();
        } else {
            self.attach_spring();
        }
    }

    pub fn spring_attached(&self) -> bool {
        self.ball_visible && self.spring.attached
    }

    pub fn ball_visible(&self) -> bool {
        self.ball_visible
    }

    fn recall_to_spring(&mut self) {
        let r = self.ball.radius;
        self.spring.attach();
        self.ball = Ball::new(self.spring.rest_position(), r);
        self.ball_visible = true;
        self.cursor_vel = Vec2::ZERO;
        self.detached_cursor_interaction_active = false;
        self.trail.clear();
    }

    pub fn spawn_attached_at(&mut self, pos: Vec2) {
        let r = self.ball.radius;
        let pos = self.safe_ball_pos(pos, r);
        self.spring.attach();
        self.spring
            .set_length_scale(self.bounds, self.config.spring_length_scale);
        self.ball = Ball::new(pos, r);
        self.ball_visible = true;
        self.cursor = pos;
        self.cursor_vel = Vec2::ZERO;
        self.cursor_time = None;
        self.detached_cursor_interaction_active = false;
        self.trail.clear();
        self.particles.clear();
    }

    pub fn set_size(
        &mut self,
        ball_radius: f32,
        spring_interaction_scale: f32,
        spring_length_scale: f32,
    ) {
        let radius = ball_radius.clamp(12.0, 96.0);
        self.config.ball_radius = radius;
        self.config.spring_interaction_scale = spring_interaction_scale.clamp(0.45, 1.25);
        self.config.spring_length_scale = spring_length_scale.clamp(0.45, 1.25);
        self.spring
            .set_interaction_scale(self.config.spring_interaction_scale);
        self.spring
            .set_length_scale(self.bounds, self.config.spring_length_scale);
        self.ball.radius = radius;
        if self.ball_visible {
            self.ball.pos = self.safe_ball_pos(self.ball.pos, radius);
        }
        self.ball.wake();
    }

    fn despawn_to_pit(&mut self) {
        self.spring.cut();
        self.spring.clear_cursor_interaction();
        self.ball.grabbed = false;
        self.ball.vel = Vec2::ZERO;
        self.ball.spin = 0.0;
        self.ball.asleep = true;
        self.ball_visible = false;
        self.detached_cursor_interaction_active = false;
        self.mote_accum = 0.0;
        self.trail.clear();
    }

    fn safe_ball_pos(&self, pos: Vec2, radius: f32) -> Vec2 {
        let x_min = self.bounds.left + radius;
        let x_max = self.bounds.right - radius;
        let y_min = self.bounds.top + radius;
        let y_max = self.bounds.bottom - radius;
        Vec2::new(
            if x_min <= x_max {
                pos.x.clamp(x_min, x_max)
            } else {
                self.bounds.center().x
            },
            if y_min <= y_max {
                pos.y.clamp(y_min, y_max)
            } else {
                self.bounds.center().y
            },
        )
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

    pub fn bottom_bounce_enabled(&self) -> bool {
        self.config.bounce_bottom_edge
    }

    pub fn set_bottom_bounce_enabled(&mut self, enabled: bool) {
        self.config.bounce_bottom_edge = enabled;
        self.ball.wake();
    }

    pub fn set_recall_margin(&mut self, margin: f32) {
        self.spring.recall_margin = margin.clamp(0.0, 5000.0);
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
        self.update_cursor_sample(cursor, now);
        self.cursor = cursor;
        if !self.ball_visible {
            return false;
        }
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
        self.update_cursor_sample(cursor, now);
        if !self.ball_visible {
            return;
        }
        if !self.ball.grabbed && self.should_cut_spring_from_cursor(prev_cursor, cursor) {
            self.cut_spring();
            return;
        }
        if self.ball.grabbed {
            self.interaction.update_cursor(cursor, now);
        }
    }

    pub fn interact_spring(&mut self, cursor: Vec2, now: f32) {
        let prev_cursor = self.cursor;
        self.update_cursor_sample(cursor, now);
        if !self.ball_visible {
            return;
        }
        if self.ball.grabbed {
            self.interaction.update_cursor(cursor, now);
            return;
        }
        if !self.spring.attached {
            self.detached_cursor_interaction_active = true;
            let hit = self.bat_detached_ball_with_cursor(prev_cursor, cursor);
            if hit && self.config.particles_enabled {
                self.particles
                    .emit_motes(cursor, self.cursor_vel, 5, self.config.color_outer);
            }
            return;
        }

        let had_support = self.spring.intersection.is_some();
        let displaced =
            self.spring
                .update_intersection_sweep(&self.ball, prev_cursor, cursor, self.cursor_vel);
        let entangled = !had_support
            && self
                .spring
                .try_entangle_sweep(&self.ball, prev_cursor, cursor, self.cursor_vel);
        if !displaced {
            self.spring.intersection = None;
        }
        if displaced || entangled {
            self.ball.wake();
        }
        if self.config.particles_enabled {
            let count = if entangled { 10 } else { 3 };
            if displaced || entangled {
                self.particles
                    .emit_motes(cursor, self.cursor_vel, count, self.config.color_outer);
            }
        }
    }

    pub fn stop_spring_interaction(&mut self) {
        self.detached_cursor_interaction_active = false;
        if !self.ball_visible {
            self.spring.clear_cursor_interaction();
            return;
        }
        if !self.spring.release_cursor_support(&self.ball) {
            self.spring.clear_cursor_interaction();
        }
        self.ball.wake();
    }

    pub fn release(&mut self, now: f32) {
        if self.ball_visible && self.ball.grabbed {
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
        self.ball_visible && self.ball.grabbed
    }

    fn update_cursor_sample(&mut self, cursor: Vec2, now: f32) {
        if let Some(prev_time) = self.cursor_time {
            let dt = now - prev_time;
            if dt > 1e-4 {
                self.cursor_vel = (cursor - self.cursor) / dt;
            }
        }
        self.cursor_time = Some(now);
        self.cursor = cursor;
    }

    fn should_cut_spring_from_cursor(&self, prev_cursor: Vec2, cursor: Vec2) -> bool {
        self.spring
            .cut_normal_speed_for_cursor_sweep(&self.ball, prev_cursor, cursor, self.cursor_vel)
            .is_some_and(|speed| speed >= self.config.cut_spring_cursor_speed)
    }

    fn bat_detached_ball_with_cursor(&mut self, prev_cursor: Vec2, cursor: Vec2) -> bool {
        let cursor_speed = self.cursor_vel.length();
        if cursor_speed < 40.0 && self.ball.speed() < 40.0 {
            return false;
        }

        let hit_radius = self.ball.radius + self.config.cursor_ball_radius;
        let closest = nearest_point_on_segment(self.ball.pos, prev_cursor, cursor);
        let delta = self.ball.pos - closest;
        let distance = delta.length();
        if distance > hit_radius {
            return false;
        }

        let normal = if distance > 1.0 {
            delta / distance
        } else {
            self.cursor_vel.normalize_or_zero()
        };
        if normal.length_squared() <= 0.0 {
            return false;
        }

        let relative_vel = self.cursor_vel - self.ball.vel;
        let approach = relative_vel.dot(normal);
        let penetration = (hit_radius - distance).max(0.0);
        let normal_impulse = normal
            * (approach.max(0.0) * (1.0 + self.config.cursor_ball_bounce) + penetration * 18.0);
        let carried_momentum = self.cursor_vel * self.config.cursor_ball_momentum_transfer;
        self.ball.vel += normal_impulse + carried_momentum;
        self.ball.vel = clamp_len(self.ball.vel, self.config.max_speed);

        let contact = closest - self.ball.pos;
        if contact.length_squared() > 4.0 && self.ball.radius > 1.0 {
            let spin_impulse =
                contact.perp_dot(relative_vel) / (self.ball.radius * self.ball.radius);
            self.ball.add_spin(spin_impulse * 0.7, 80.0);
        }

        self.ball
            .apply_impact(normal, (normal_impulse.length() / 2500.0).clamp(0.05, 0.6));
        true
    }

    fn bat_detached_ball_with_stationary_cursor(&mut self, prev_ball_pos: Vec2) -> bool {
        if !self.detached_cursor_interaction_active || self.spring.attached {
            return false;
        }
        if self.ball.speed() < 40.0 {
            return false;
        }

        let cursor = self.cursor;
        let hit_radius = self.ball.radius + self.config.cursor_ball_radius;
        let closest = nearest_point_on_segment(cursor, prev_ball_pos, self.ball.pos);
        let delta = closest - cursor;
        let distance = delta.length();
        if distance > hit_radius {
            return false;
        }

        let normal = if distance > 1.0 {
            delta / distance
        } else if (prev_ball_pos - cursor).length_squared() > 1.0 {
            (prev_ball_pos - cursor).normalize()
        } else {
            -self.ball.vel.normalize_or_zero()
        };
        if normal.length_squared() <= 0.0 {
            return false;
        }

        let relative_vel = self.cursor_vel - self.ball.vel;
        let approach = relative_vel.dot(normal);
        let penetration = (hit_radius - distance).max(0.0);
        if approach <= 0.0 && penetration < 1.0 {
            return false;
        }

        let normal_impulse = normal
            * (approach.max(0.0) * (1.0 + self.config.cursor_ball_bounce) + penetration * 18.0);
        self.ball.vel += normal_impulse;
        self.ball.vel = clamp_len(self.ball.vel, self.config.max_speed);

        let contact = closest - self.ball.pos;
        if contact.length_squared() > 4.0 && self.ball.radius > 1.0 {
            let spin_impulse =
                contact.perp_dot(relative_vel) / (self.ball.radius * self.ball.radius);
            self.ball.add_spin(spin_impulse * 0.7, 80.0);
        }

        self.ball
            .apply_impact(normal, (normal_impulse.length() / 2500.0).clamp(0.05, 0.6));
        true
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
        if !self.ball_visible {
            return;
        }
        let prev_ball_pos = self.ball.pos;

        if self.ball.grabbed {
            self.interaction
                .apply_spring(&mut self.ball, self.cursor, dt);
        } else if !self.ball.asleep {
            self.spring
                .update_transients(self.cursor, &self.ball, self.config.gravity, dt);
            // Integrate gravity, the anchored spring, and drag.
            let spring_force = self.spring.force_on(&self.ball);
            let spin_curve = self.spin_curve_accel();
            self.ball.vel +=
                (self.config.gravity + spring_force / self.ball.mass + spin_curve) * dt;
            self.ball.vel *= 1.0 - self.config.air_drag * dt;
            // Clamp speed.
            let sp = self.ball.vel.length();
            if sp > self.config.max_speed {
                self.ball.vel *= self.config.max_speed / sp;
            }
            self.ball.pos += self.ball.vel * dt;
            if self.should_cut_spring_from_ball_motion(prev_ball_pos) {
                self.cut_spring();
            }
            if self.bat_detached_ball_with_stationary_cursor(prev_ball_pos)
                && self.config.particles_enabled
            {
                self.particles
                    .emit_motes(self.cursor, self.ball.vel, 4, self.config.color_outer);
            }
        }

        self.ball.roll_by(self.ball.pos - prev_ball_pos);
        self.ball.spin_by(dt);
        self.ball
            .damp_spin(self.config.spin_drag + self.ball.friction * 2.0, dt);

        // Decay the squash impulse.
        self.ball.squash_impulse *= (-dt * 14.0).exp();

        // Resolve walls and emit impact sparks.
        let impacts = if self.config.bounce_bottom_edge {
            collisions::resolve_walls_with_bottom_edges(
                &mut self.ball,
                &self.bounds,
                &self.bottom_edges,
            )
        } else {
            collisions::resolve_walls_with_bottom(
                &mut self.ball,
                &self.bounds,
                self.spring.attached,
            )
        };
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
            self.despawn_to_pit();
        }

        // Sleep handling: stop integrating when at rest with no interaction.
        if !self.ball.grabbed {
            if self.ball.speed() < self.config.sleep_speed
                && self.ball.spin.abs() < self.config.sleep_spin
                && self.config.gravity == Vec2::ZERO
            {
                self.ball.still_time += dt;
                if self.ball.still_time > self.config.sleep_delay {
                    self.ball.asleep = true;
                    self.ball.vel = Vec2::ZERO;
                    self.ball.spin = 0.0;
                }
            } else {
                self.ball.still_time = 0.0;
            }
        }
    }

    fn spin_curve_accel(&self) -> Vec2 {
        let speed = self.ball.speed();
        if speed < 1.0 || self.ball.spin.abs() < self.config.sleep_spin {
            return Vec2::ZERO;
        }

        let dir = self.ball.vel / speed;
        let side = Vec2::new(-dir.y, dir.x);
        let accel = self.ball.spin * speed * self.config.spin_curve;
        side * accel.clamp(
            -self.config.max_spin_curve_accel,
            self.config.max_spin_curve_accel,
        )
    }

    fn should_cut_spring_from_ball_motion(&self, prev_ball_pos: Vec2) -> bool {
        let relative_vel = self.ball.vel - self.cursor_vel;
        self.spring.attached
            && self.cursor_time.is_some()
            && self.spring.intersection.is_none()
            && self.spring.entanglement.is_none()
            && self
                .spring
                .cut_normal_speed_for_moving_spring(
                    &self.ball,
                    prev_ball_pos,
                    self.cursor,
                    relative_vel,
                )
                .is_some_and(|speed| speed >= self.config.cut_spring_cursor_speed)
    }

    /// Whether anything is visibly animating (used for idle frame pacing).
    pub fn is_active(&self) -> bool {
        self.ball_visible
            && (self.ball.grabbed
                || self.spring.intersection.is_some()
                || self.spring.entanglement.is_some()
                || self.ball.spin.abs() >= self.config.sleep_spin
                || !self.ball.asleep
                || !self.particles.is_empty()
                || !self.trail.is_empty())
    }
}

fn nearest_point_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let denom = ab.length_squared();
    if denom <= 1e-4 {
        return a;
    }
    let t = (p - a).dot(ab) / denom;
    a + ab * t.clamp(0.0, 1.0)
}

fn clamp_len(v: Vec2, max: f32) -> Vec2 {
    let len = v.length();
    if len > max && len > 0.0 {
        v * (max / len)
    } else {
        v
    }
}
