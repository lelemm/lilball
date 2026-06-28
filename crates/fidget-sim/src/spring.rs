use glam::Vec2;

use crate::ball::Ball;
use crate::bounds::Bounds;

/// Anchored spring that suspends the ball from the top of the play area.
#[derive(Debug, Clone, Copy)]
pub struct SpringState {
    pub anchor: Vec2,
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub max_force: f32,
    pub recall_margin: f32,
    pub attached: bool,
}

impl SpringState {
    pub fn new(bounds: Bounds, ball_pos: Vec2) -> Self {
        let anchor = anchor_for(bounds);
        Self {
            anchor,
            rest_length: ball_pos.distance(anchor).max(1.0),
            stiffness: 85.0,
            damping: 20.0,
            max_force: 18_000.0,
            recall_margin: 180.0,
            attached: true,
        }
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.anchor = anchor_for(bounds);
        self.rest_length = bounds.center().distance(self.anchor).max(1.0);
    }

    pub fn rest_position(&self) -> Vec2 {
        self.anchor + Vec2::Y * self.rest_length
    }

    pub fn cut(&mut self) {
        self.attached = false;
    }

    pub fn attach(&mut self) {
        self.attached = true;
    }

    pub fn force_on(&self, ball: &Ball) -> Vec2 {
        if !self.attached {
            return Vec2::ZERO;
        }

        let delta = ball.pos - self.anchor;
        let len = delta.length();
        if len <= 1e-4 {
            return Vec2::ZERO;
        }

        let dir = delta / len;
        let stretch = len - self.rest_length;
        let radial_speed = ball.vel.dot(dir);
        let mut force = (self.stiffness * stretch - self.damping * radial_speed) * dir;
        let mag = force.length();
        if mag > self.max_force {
            force *= self.max_force / mag;
        }
        force
    }

    pub fn should_recall(&self, ball: &Ball, bounds: Bounds) -> bool {
        !self.attached && ball.pos.y - ball.radius > bounds.bottom + self.recall_margin
    }
}

fn anchor_for(bounds: Bounds) -> Vec2 {
    let inset = (bounds.height() * 0.08).clamp(36.0, 72.0);
    Vec2::new(bounds.center().x, bounds.top + inset)
}
