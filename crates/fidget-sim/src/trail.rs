use std::collections::VecDeque;

use glam::Vec2;

/// A single sample along the motion trail.
#[derive(Debug, Clone, Copy)]
pub struct TrailPoint {
    pub pos: Vec2,
    /// Age in seconds since the point was recorded.
    pub age: f32,
    pub radius: f32,
}

/// Fixed-length ring buffer of recent ball positions used to render a fading
/// ribbon/glow trail behind the ball.
#[derive(Debug, Clone)]
pub struct Trail {
    points: VecDeque<TrailPoint>,
    capacity: usize,
    /// How long (seconds) a point lives before being dropped.
    lifetime: f32,
    /// Minimum distance between recorded points so the trail samples evenly.
    min_spacing: f32,
}

impl Trail {
    pub fn new(capacity: usize, lifetime: f32) -> Self {
        Self {
            points: VecDeque::with_capacity(capacity),
            capacity: capacity.max(2),
            lifetime,
            min_spacing: 4.0,
        }
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TrailPoint> {
        self.points.iter()
    }

    /// Record the current ball position (if it has moved far enough).
    pub fn record(&mut self, pos: Vec2, radius: f32) {
        let far_enough = self
            .points
            .back()
            .is_none_or(|p| p.pos.distance(pos) >= self.min_spacing);
        if far_enough {
            if self.points.len() >= self.capacity {
                self.points.pop_front();
            }
            self.points.push_back(TrailPoint { pos, age: 0.0, radius });
        }
    }

    /// Age all points and drop expired ones.
    pub fn update(&mut self, dt: f32) {
        for p in &mut self.points {
            p.age += dt;
        }
        while self.points.front().is_some_and(|p| p.age > self.lifetime) {
            self.points.pop_front();
        }
    }

    /// Alpha 0..1 for a point given its age (linear fade).
    pub fn alpha_for(&self, p: &TrailPoint) -> f32 {
        (1.0 - p.age / self.lifetime).clamp(0.0, 1.0)
    }

    pub fn clear(&mut self) {
        self.points.clear();
    }
}
