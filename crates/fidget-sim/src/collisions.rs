use glam::Vec2;

use crate::ball::Ball;
use crate::bounds::Bounds;

/// Description of a single wall hit produced during collision resolution, so
/// the caller can spawn impact particles at the contact point.
#[derive(Debug, Clone, Copy)]
pub struct Impact {
    pub point: Vec2,
    pub normal: Vec2,
    /// Speed component into the wall before the bounce (>= 0).
    pub speed: f32,
}

/// Resolve the ball against the four walls of `bounds`, reflecting velocity by
/// the ball's restitution and applying tangential friction. Returns any impacts
/// so the world can emit sparks.
pub fn resolve_walls(ball: &mut Ball, bounds: &Bounds) -> Vec<Impact> {
    resolve_walls_with_bottom(ball, bounds, true)
}

/// Variant used when the spring has been cut: the ball still respects the
/// side/top walls, but gravity may carry it through the bottom edge.
pub fn resolve_walls_with_bottom(
    ball: &mut Ball,
    bounds: &Bounds,
    collide_bottom: bool,
) -> Vec<Impact> {
    let mut impacts = Vec::new();
    let r = ball.radius;

    // Left wall.
    if ball.pos.x - r < bounds.left {
        let speed = ball.vel.x.min(0.0).abs();
        ball.pos.x = bounds.left + r;
        ball.vel.x = ball.vel.x.abs() * ball.restitution;
        ball.vel.y *= 1.0 - ball.friction;
        ball.spin *= 1.0 - ball.friction;
        let n = Vec2::new(1.0, 0.0);
        ball.apply_impact(n, impulse_for(speed));
        impacts.push(Impact {
            point: Vec2::new(bounds.left, ball.pos.y),
            normal: n,
            speed,
        });
    }
    // Right wall.
    if ball.pos.x + r > bounds.right {
        let speed = ball.vel.x.max(0.0);
        ball.pos.x = bounds.right - r;
        ball.vel.x = -ball.vel.x.abs() * ball.restitution;
        ball.vel.y *= 1.0 - ball.friction;
        ball.spin *= 1.0 - ball.friction;
        let n = Vec2::new(-1.0, 0.0);
        ball.apply_impact(n, impulse_for(speed));
        impacts.push(Impact {
            point: Vec2::new(bounds.right, ball.pos.y),
            normal: n,
            speed,
        });
    }
    // Top wall.
    if ball.pos.y - r < bounds.top {
        let speed = ball.vel.y.min(0.0).abs();
        ball.pos.y = bounds.top + r;
        ball.vel.y = ball.vel.y.abs() * ball.restitution;
        ball.vel.x *= 1.0 - ball.friction;
        ball.spin *= 1.0 - ball.friction;
        let n = Vec2::new(0.0, 1.0);
        ball.apply_impact(n, impulse_for(speed));
        impacts.push(Impact {
            point: Vec2::new(ball.pos.x, bounds.top),
            normal: n,
            speed,
        });
    }
    // Bottom wall.
    if collide_bottom && ball.pos.y + r > bounds.bottom {
        let speed = ball.vel.y.max(0.0);
        ball.pos.y = bounds.bottom - r;
        ball.vel.y = -ball.vel.y.abs() * ball.restitution;
        ball.vel.x *= 1.0 - ball.friction;
        ball.spin *= 1.0 - ball.friction;
        let n = Vec2::new(0.0, -1.0);
        ball.apply_impact(n, impulse_for(speed));
        impacts.push(Impact {
            point: Vec2::new(ball.pos.x, bounds.bottom),
            normal: n,
            speed,
        });
    }

    impacts
}

fn impulse_for(speed: f32) -> f32 {
    // Map incoming speed to a 0..0.6 squash impulse with diminishing returns.
    (speed / 2500.0).clamp(0.0, 0.6)
}
