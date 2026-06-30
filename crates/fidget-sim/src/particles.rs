use glam::{Vec2, Vec4};

/// What spawned a particle, which controls its look and behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    /// Soft motes left behind while the ball moves.
    Mote,
    /// Bright sparks emitted on a wall impact.
    Spark,
    /// Burst emitted when the ball is thrown.
    Burst,
    /// Glass flecks emitted when a marble cracks or shatters.
    Shard,
}

#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub pos: Vec2,
    pub vel: Vec2,
    pub life: f32,
    pub max_life: f32,
    pub size: f32,
    pub color: Vec4,
    pub kind: ParticleKind,
}

impl Particle {
    /// Remaining life in 0..1.
    pub fn life_frac(&self) -> f32 {
        (self.life / self.max_life).clamp(0.0, 1.0)
    }
}

/// A fixed-capacity pool of CPU-simulated particles. New particles overwrite
/// the oldest once capacity is reached, so emission never allocates after
/// construction and frame cost stays bounded.
#[derive(Debug, Clone)]
pub struct ParticleSystem {
    particles: Vec<Particle>,
    capacity: usize,
    /// Simple deterministic PRNG state so emission is testable.
    rng: u32,
}

impl ParticleSystem {
    pub fn new(capacity: usize) -> Self {
        Self {
            particles: Vec::with_capacity(capacity),
            capacity: capacity.max(1),
            rng: 0x1234_5678,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Particle> {
        self.particles.iter()
    }

    pub fn clear(&mut self) {
        self.particles.clear();
    }

    /// xorshift32 -> 0..1
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32).clamp(0.0, 1.0)
    }

    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }

    fn push(&mut self, p: Particle) {
        if self.particles.len() < self.capacity {
            self.particles.push(p);
        } else {
            // Overwrite the particle with the least remaining life.
            let mut idx = 0;
            let mut min_life = f32::INFINITY;
            for (i, q) in self.particles.iter().enumerate() {
                if q.life < min_life {
                    min_life = q.life;
                    idx = i;
                }
            }
            self.particles[idx] = p;
        }
    }

    /// Advance all particles and remove the dead ones.
    pub fn update(&mut self, dt: f32, drag: f32) {
        for p in &mut self.particles {
            p.life -= dt;
            p.vel *= 1.0 - drag * dt;
            p.pos += p.vel * dt;
        }
        self.particles.retain(|p| p.life > 0.0);
    }

    /// Emit faint motes trailing the moving ball.
    pub fn emit_motes(&mut self, pos: Vec2, vel: Vec2, count: usize, color: Vec4) {
        for _ in 0..count {
            let jitter = Vec2::new(self.rand_range(-6.0, 6.0), self.rand_range(-6.0, 6.0));
            let spread = Vec2::new(self.rand_range(-20.0, 20.0), self.rand_range(-20.0, 20.0));
            let max_life = self.rand_range(0.25, 0.5);
            let size = self.rand_range(2.0, 5.0);
            self.push(Particle {
                pos: pos + jitter,
                vel: vel * 0.05 + spread,
                life: max_life,
                max_life,
                size,
                color,
                kind: ParticleKind::Mote,
            });
        }
    }

    /// Emit sparks along the collision tangent on a wall impact.
    pub fn emit_impact(&mut self, point: Vec2, normal: Vec2, speed: f32, color: Vec4) {
        let tangent = Vec2::new(-normal.y, normal.x);
        let count = ((speed / 120.0) as usize).clamp(4, 40);
        for _ in 0..count {
            let along = self.rand_range(-1.0, 1.0);
            let out = self.rand_range(0.2, 1.0);
            let sp = self.rand_range(120.0, 120.0 + speed * 0.6);
            let max_life = self.rand_range(0.18, 0.4);
            let size = self.rand_range(2.0, 4.0);
            let dir = (tangent * along + normal * out).normalize_or_zero();
            self.push(Particle {
                pos: point + normal * 2.0,
                vel: dir * sp,
                life: max_life,
                max_life,
                size,
                color,
                kind: ParticleKind::Spark,
            });
        }
    }

    /// Emit a radial burst when the ball is thrown.
    pub fn emit_burst(&mut self, pos: Vec2, speed: f32, color: Vec4) {
        let count = ((speed / 60.0) as usize).clamp(8, 64);
        for i in 0..count {
            let jitter = self.rand_range(-0.3, 0.3);
            let sp = self.rand_range(80.0, 80.0 + speed * 0.4);
            let max_life = self.rand_range(0.25, 0.55);
            let size = self.rand_range(2.5, 5.0);
            let ang = (i as f32 / count as f32) * std::f32::consts::TAU + jitter;
            let dir = Vec2::new(ang.cos(), ang.sin());
            self.push(Particle {
                pos,
                vel: dir * sp,
                life: max_life,
                max_life,
                size,
                color,
                kind: ParticleKind::Burst,
            });
        }
    }

    /// Emit sharper glass-like flecks for marble impacts and shatters.
    pub fn emit_shards(&mut self, pos: Vec2, speed: f32, count: usize, color: Vec4) {
        let count = count.clamp(4, 96);
        for i in 0..count {
            let jitter = self.rand_range(-0.22, 0.22);
            let sp = self.rand_range(70.0, 160.0 + speed * 0.45);
            let max_life = self.rand_range(0.18, 0.72);
            let size = self.rand_range(1.5, 5.8);
            let ang = (i as f32 / count as f32) * std::f32::consts::TAU + jitter;
            let dir = Vec2::new(ang.cos(), ang.sin());
            let pos_jitter = self.rand_range(0.0, 8.0);
            let vel_jitter = Vec2::new(self.rand_range(-30.0, 30.0), self.rand_range(-30.0, 30.0));
            self.push(Particle {
                pos: pos + dir * pos_jitter,
                vel: dir * sp + vel_jitter,
                life: max_life,
                max_life,
                size,
                color,
                kind: ParticleKind::Shard,
            });
        }
    }
}
