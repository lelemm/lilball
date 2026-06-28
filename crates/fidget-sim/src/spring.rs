use glam::Vec2;

use crate::ball::Ball;
use crate::bounds::Bounds;

/// Temporary wrap point created when a fast cursor sweep snags the spring.
#[derive(Debug, Clone, Copy)]
pub struct CursorEntanglement {
    pub center: Vec2,
    pub radius: f32,
    pub angle: f32,
    pub angular_velocity: f32,
    pub age: f32,
    pub max_age: f32,
}

impl CursorEntanglement {
    pub fn target(&self) -> Vec2 {
        self.center + Vec2::new(self.angle.cos(), self.angle.sin()) * self.radius
    }

    pub fn tangent_velocity(&self) -> Vec2 {
        Vec2::new(-self.angle.sin(), self.angle.cos()) * self.angular_velocity * self.radius
    }
}

/// Brief spring deflection from the cursor brushing or crossing the string.
#[derive(Debug, Clone, Copy)]
pub struct CursorIntersection {
    pub point: Vec2,
    pub displacement: Vec2,
    pub cursor_vel: Vec2,
    pub age: f32,
    pub max_age: f32,
}

impl CursorIntersection {
    pub fn strength(&self) -> f32 {
        (1.0 - self.age / self.max_age).clamp(0.0, 1.0)
    }
}

/// Anchored spring that suspends the ball from the top of the play area.
#[derive(Debug, Clone, Copy)]
pub struct SpringState {
    pub anchor: Vec2,
    pub hook_offset_y: f32,
    pub rest_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub max_force: f32,
    pub recall_margin: f32,
    pub attached: bool,
    pub intersection: Option<CursorIntersection>,
    pub entanglement: Option<CursorEntanglement>,
    pub intersection_capture_radius: f32,
    pub entangle_capture_radius: f32,
    pub entangle_min_cursor_speed: f32,
}

impl SpringState {
    pub fn new(bounds: Bounds, ball_pos: Vec2) -> Self {
        let hook_offset_y = -120.0;
        let anchor = anchor_for(bounds) + Vec2::Y * hook_offset_y;
        Self {
            anchor,
            hook_offset_y,
            rest_length: ball_pos.distance(anchor).max(1.0),
            stiffness: 85.0,
            damping: 20.0,
            max_force: 18_000.0,
            recall_margin: 900.0,
            attached: true,
            intersection: None,
            entanglement: None,
            intersection_capture_radius: 110.0,
            entangle_capture_radius: 72.0,
            entangle_min_cursor_speed: 850.0,
        }
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.anchor = anchor_for(bounds) + Vec2::Y * self.hook_offset_y;
        self.rest_length = bounds.center().distance(self.anchor).max(1.0);
    }

    pub fn set_hook_offset_y(&mut self, bounds: Bounds, offset_y: f32) {
        self.hook_offset_y = offset_y.clamp(-600.0, 260.0);
        self.anchor = anchor_for(bounds) + Vec2::Y * self.hook_offset_y;
        self.rest_length = bounds.center().distance(self.anchor).max(1.0);
    }

    pub fn rest_position(&self) -> Vec2 {
        self.anchor + Vec2::Y * self.rest_length
    }

    pub fn cut(&mut self) {
        self.attached = false;
        self.intersection = None;
        self.entanglement = None;
    }

    pub fn attach(&mut self) {
        self.attached = true;
        self.intersection = None;
        self.entanglement = None;
    }

    pub fn force_on(&self, ball: &Ball) -> Vec2 {
        if !self.attached {
            return Vec2::ZERO;
        }

        if let Some(entanglement) = self.entanglement {
            let target = entanglement.target();
            let desired_vel = entanglement.tangent_velocity();
            let mut force = (target - ball.pos) * 210.0 + (desired_vel - ball.vel) * 26.0;
            let mag = force.length();
            if mag > self.max_force {
                force *= self.max_force / mag;
            }
            return force;
        }

        let mut force = self.base_force_on(ball);
        if let Some(intersection) = self.intersection {
            let strength = intersection.strength();
            force += intersection.displacement * (42.0 * strength);
            force += intersection.cursor_vel * (0.16 * strength);
        }

        clamp_force(force, self.max_force)
    }

    fn base_force_on(&self, ball: &Ball) -> Vec2 {
        let delta = ball.pos - self.anchor;
        let len = delta.length();
        if len <= 1e-4 {
            return Vec2::ZERO;
        }

        let dir = delta / len;
        let stretch = len - self.rest_length;
        let radial_speed = ball.vel.dot(dir);
        (-self.stiffness * stretch - self.damping * radial_speed) * dir
    }

    pub fn should_recall(&self, ball: &Ball, bounds: Bounds) -> bool {
        !self.attached && ball.pos.y - ball.radius > bounds.bottom + self.recall_margin
    }

    pub fn cut_impulse(&self) -> Vec2 {
        let mut impulse = Vec2::ZERO;
        if let Some(intersection) = self.intersection {
            let strength = intersection.strength();
            impulse += intersection.displacement * (4.2 * strength);
            impulse += intersection.cursor_vel * (0.32 * strength);
        }
        if let Some(entanglement) = self.entanglement {
            impulse += entanglement.tangent_velocity() * 0.55;
        }
        clamp_len(impulse, 1800.0)
    }

    pub fn update_intersection_sweep(
        &mut self,
        ball: &Ball,
        prev_cursor: Vec2,
        cursor: Vec2,
        cursor_vel: Vec2,
    ) -> bool {
        if !self.attached {
            return false;
        }

        let spring_distance = segment_distance(prev_cursor, cursor, self.anchor, ball.pos);
        let ball_distance = cursor.distance(ball.pos) - ball.radius;
        let distance = spring_distance.min(ball_distance.max(0.0));
        if distance > self.intersection_capture_radius {
            return false;
        }

        let nearest = nearest_point_on_segment(cursor, self.anchor, ball.pos);
        let mut displacement = cursor - nearest;
        if displacement.length_squared() < 1.0 {
            let spring_dir = (ball.pos - self.anchor).normalize_or_zero();
            let cursor_dir = cursor_vel.normalize_or_zero();
            displacement = if cursor_dir.length_squared() > 0.0 {
                cursor_dir
            } else {
                Vec2::new(-spring_dir.y, spring_dir.x)
            };
        }

        let proximity = 1.0 - (distance / self.intersection_capture_radius).clamp(0.0, 1.0);
        let speed_boost = (cursor_vel.length() / 1400.0).clamp(0.25, 1.0);
        let strength = proximity * speed_boost;
        let max_displacement = 130.0 * strength.max(0.2);
        let displacement = displacement.normalize_or_zero() * max_displacement;

        self.intersection = Some(CursorIntersection {
            point: nearest + displacement * 0.55,
            displacement,
            cursor_vel,
            age: 0.0,
            max_age: 0.55 + speed_boost * 0.35,
        });
        true
    }

    pub fn try_entangle(&mut self, ball: &Ball, cursor: Vec2, cursor_vel: Vec2) -> bool {
        self.try_entangle_sweep(ball, cursor, cursor, cursor_vel)
    }

    pub fn try_entangle_sweep(
        &mut self,
        ball: &Ball,
        prev_cursor: Vec2,
        cursor: Vec2,
        cursor_vel: Vec2,
    ) -> bool {
        if !self.attached || self.entanglement.is_some() {
            return false;
        }

        let cursor_speed = cursor_vel.length();
        if cursor_speed < self.entangle_min_cursor_speed {
            return false;
        }

        let spring_hit = segment_distance(prev_cursor, cursor, self.anchor, ball.pos)
            <= self.entangle_capture_radius;
        let ball_hit = cursor.distance(ball.pos) <= ball.radius + self.entangle_capture_radius;
        if !spring_hit && !ball_hit {
            return false;
        }

        let to_ball = ball.pos - cursor;
        let radius = to_ball.length().clamp(ball.radius * 1.25, 180.0);
        let start_dir = if to_ball.length_squared() > 1.0 {
            to_ball.normalize()
        } else {
            Vec2::Y
        };
        let inertia = (cursor_speed + ball.speed() * 0.5).clamp(0.0, 4500.0);
        let cross = cursor_vel.perp_dot(start_dir);
        let spin_sign = if cross.abs() > 1.0 {
            cross.signum()
        } else if cursor_vel.x >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let angular_velocity = spin_sign * (inertia / radius).clamp(5.0, 18.0);
        let max_age = (0.45 + inertia / 2600.0).clamp(0.65, 1.8);

        self.entanglement = Some(CursorEntanglement {
            center: cursor,
            radius,
            angle: start_dir.y.atan2(start_dir.x),
            angular_velocity,
            age: 0.0,
            max_age,
        });
        self.intersection = None;
        true
    }

    pub fn update_transients(&mut self, cursor: Vec2, dt: f32) {
        if let Some(mut intersection) = self.intersection {
            intersection.age += dt;
            intersection.displacement *= (-dt * 4.8).exp();
            if intersection.age >= intersection.max_age || intersection.displacement.length() < 1.0
            {
                self.intersection = None;
            } else {
                self.intersection = Some(intersection);
            }
        }

        if let Some(mut entanglement) = self.entanglement {
            entanglement.center = cursor;
            entanglement.age += dt;
            entanglement.angle += entanglement.angular_velocity * dt;
            entanglement.angular_velocity *= (-dt * 1.15).exp();

            if entanglement.age >= entanglement.max_age || entanglement.angular_velocity.abs() < 1.4
            {
                self.entanglement = None;
            } else {
                self.entanglement = Some(entanglement);
            }
        }
    }
}

fn anchor_for(bounds: Bounds) -> Vec2 {
    let inset = (bounds.height() * 0.08).clamp(36.0, 72.0);
    Vec2::new(bounds.center().x, bounds.top + inset)
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    p.distance(nearest_point_on_segment(p, a, b))
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

fn segment_distance(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f32 {
    if segments_intersect(a0, a1, b0, b1) {
        return 0.0;
    }

    distance_to_segment(a0, b0, b1)
        .min(distance_to_segment(a1, b0, b1))
        .min(distance_to_segment(b0, a0, a1))
        .min(distance_to_segment(b1, a0, a1))
}

fn segments_intersect(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> bool {
    let a = a1 - a0;
    let b = b1 - b0;
    let denom = a.perp_dot(b);
    if denom.abs() <= 1e-5 {
        return false;
    }

    let delta = b0 - a0;
    let t = delta.perp_dot(b) / denom;
    let u = delta.perp_dot(a) / denom;
    (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)
}

fn clamp_force(force: Vec2, max: f32) -> Vec2 {
    clamp_len(force, max)
}

fn clamp_len(v: Vec2, max: f32) -> Vec2 {
    let len = v.length();
    if len > max && len > 0.0 {
        v * (max / len)
    } else {
        v
    }
}
