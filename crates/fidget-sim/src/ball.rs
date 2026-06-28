use glam::Vec2;

/// The fidget ball: position/velocity plus the visual squash-and-stretch state
/// that the renderer reads each frame.
#[derive(Debug, Clone, Copy)]
pub struct Ball {
    pub pos: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    pub mass: f32,
    pub restitution: f32,
    pub friction: f32,
    /// Whether the ball is currently held by the cursor.
    pub grabbed: bool,
    /// Decaying squash impulse magnitude (0 = relaxed). Driven by wall impacts.
    pub squash_impulse: f32,
    /// Direction along which the squash is applied (unit vector, collision normal).
    pub squash_dir: Vec2,
    /// Accumulated visual roll angle in radians, derived from travelled distance.
    pub roll_angle: f32,
    /// Last non-zero movement direction used to orient the rolling texture.
    pub roll_dir: Vec2,
    /// Seconds the ball has been (nearly) still; used to drive sleep.
    pub still_time: f32,
    pub asleep: bool,
}

impl Ball {
    pub fn new(pos: Vec2, radius: f32) -> Self {
        Self {
            pos,
            vel: Vec2::ZERO,
            radius,
            mass: 1.0,
            restitution: 0.82,
            friction: 0.08,
            grabbed: false,
            squash_impulse: 0.0,
            squash_dir: Vec2::Y,
            roll_angle: 0.0,
            roll_dir: Vec2::X,
            still_time: 0.0,
            asleep: false,
        }
    }

    /// Current speed in pixels/second.
    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    /// Non-uniform scale used for squash-and-stretch, derived from velocity and
    /// the decaying impact impulse. Returns `(scale_along_vel, scale_perp)`.
    ///
    /// While moving fast the ball stretches along its velocity; on impact it
    /// briefly squashes perpendicular to the collision normal.
    pub fn squash_scale(&self, max_speed: f32) -> Vec2 {
        let stretch = (self.speed() / max_speed).clamp(0.0, 0.35);
        let mut x = 1.0 + stretch;
        let mut y = 1.0 - stretch * 0.45;

        // Impact squash: compress along the collision normal direction.
        let impact = self.squash_impulse.clamp(0.0, 0.6);
        x += impact * 0.5;
        y -= impact * 0.5;
        Vec2::new(x.max(0.2), y.max(0.2))
    }

    /// Register a wall impact so the renderer can show a squash.
    pub fn apply_impact(&mut self, normal: Vec2, strength: f32) {
        self.squash_dir = normal;
        self.squash_impulse = (self.squash_impulse + strength).min(0.6);
        self.wake();
    }

    /// Advance the visual roll from world-space movement. One full radius of
    /// travel rotates the material by one radian, matching a rolling sphere.
    pub fn roll_by(&mut self, delta: Vec2) {
        let distance = delta.length();
        if distance <= 0.001 || self.radius <= 0.001 {
            return;
        }
        self.roll_dir = delta / distance;
        self.roll_angle =
            (self.roll_angle + distance / self.radius).rem_euclid(std::f32::consts::TAU);
    }

    pub fn wake(&mut self) {
        self.asleep = false;
        self.still_time = 0.0;
    }
}
