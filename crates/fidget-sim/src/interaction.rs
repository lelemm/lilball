use std::collections::VecDeque;

use glam::Vec2;

use crate::ball::Ball;

/// A timestamped pointer position used to estimate throw velocity.
#[derive(Debug, Clone, Copy)]
pub struct PointerSample {
    pub pos: Vec2,
    pub time: f32,
}

/// Tracks the cursor while dragging so the ball can be thrown with the cursor's
/// recent velocity on release. Movement is driven by a critically-ish damped
/// spring rather than teleporting, which feels much nicer.
#[derive(Debug, Clone)]
pub struct InteractionState {
    pub spring_k: f32,
    pub spring_damping: f32,
    pub max_force: f32,
    pub throw_multiplier: f32,
    pub max_speed: f32,
    /// How far back (seconds) to integrate cursor motion for throw velocity.
    pub velocity_window: f32,

    grab_offset: Vec2,
    samples: VecDeque<PointerSample>,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self {
            spring_k: 1800.0,
            spring_damping: 70.0,
            max_force: 50_000.0,
            throw_multiplier: 1.0,
            max_speed: 4500.0,
            velocity_window: 0.08,
            grab_offset: Vec2::ZERO,
            samples: VecDeque::new(),
        }
    }
}

impl InteractionState {
    /// Returns true if `cursor` is within the ball's radius.
    pub fn hit_test(ball: &Ball, cursor: Vec2) -> bool {
        ball.pos.distance(cursor) <= ball.radius
    }

    /// Begin a grab. `time` is the current monotonic time in seconds.
    pub fn begin_grab(&mut self, ball: &mut Ball, cursor: Vec2, time: f32) {
        ball.grabbed = true;
        ball.wake();
        self.grab_offset = ball.pos - cursor;
        self.samples.clear();
        self.samples.push_back(PointerSample { pos: cursor, time });
    }

    /// Record cursor movement while grabbed.
    pub fn update_cursor(&mut self, cursor: Vec2, time: f32) {
        self.samples.push_back(PointerSample { pos: cursor, time });
        let cutoff = time - self.velocity_window * 2.0;
        while self.samples.len() > 2 && self.samples.front().is_some_and(|s| s.time < cutoff) {
            self.samples.pop_front();
        }
    }

    /// Apply the cursor spring for one fixed step while grabbed.
    pub fn apply_spring(&self, ball: &mut Ball, cursor: Vec2, dt: f32) {
        if !ball.grabbed {
            return;
        }
        let target = cursor + self.grab_offset;
        let mut force = self.spring_k * (target - ball.pos) - self.spring_damping * ball.vel;
        let mag = force.length();
        if mag > self.max_force {
            force *= self.max_force / mag;
        }
        let accel = force / ball.mass;
        ball.vel += accel * dt;
        ball.pos += ball.vel * dt;
    }

    /// Release the ball, imparting the recent weighted cursor velocity.
    pub fn release(&mut self, ball: &mut Ball, time: f32) {
        ball.grabbed = false;
        let v = self.weighted_velocity(time) * self.throw_multiplier;
        ball.vel = clamp_len(v, self.max_speed);
        ball.wake();
        self.samples.clear();
    }

    /// Weighted average velocity over the recent sample window. More recent
    /// segments are weighted more heavily so flicks feel responsive.
    pub fn weighted_velocity(&self, now: f32) -> Vec2 {
        if self.samples.len() < 2 {
            return Vec2::ZERO;
        }
        let mut sum = Vec2::ZERO;
        let mut wsum = 0.0;
        let mut it = self.samples.iter();
        let mut prev = *it.next().unwrap();
        for s in it {
            let dt = s.time - prev.time;
            if dt > 1e-5 {
                let seg_v = (s.pos - prev.pos) / dt;
                // Weight by recency relative to `now`.
                let age = (now - s.time).max(0.0);
                let w = (-age / self.velocity_window).exp();
                sum += seg_v * w;
                wsum += w;
            }
            prev = *s;
        }
        if wsum > 0.0 {
            sum / wsum
        } else {
            Vec2::ZERO
        }
    }
}

fn clamp_len(v: Vec2, max: f32) -> Vec2 {
    let len = v.length();
    if len > max && len > 0.0 {
        v * (max / len)
    } else {
        v
    }
}
