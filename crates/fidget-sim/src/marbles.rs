use std::collections::{HashMap, HashSet, VecDeque};

use glam::{Vec2, Vec4};

use crate::bounds::Bounds;
use crate::interaction::PointerSample;
use crate::particles::ParticleSystem;
use crate::FIXED_DT;

const DEFAULT_MIN_RADIUS: f32 = 24.0;
const DEFAULT_MAX_RADIUS: f32 = 42.0;
const GRAB_STIFFNESS: f32 = 2200.0;
const GRAB_DAMPING: f32 = 78.0;
const GRAB_MAX_FORCE: f32 = 65_000.0;
const THROW_WINDOW: f32 = 0.08;
const MAX_SPIN: f32 = 85.0;

#[derive(Debug, Clone)]
pub struct MarbleConfig {
    pub min_radius: f32,
    pub max_radius: f32,
    pub max_speed: f32,
    pub drag: f32,
    pub spin_drag: f32,
    pub wall_restitution: f32,
    pub marble_restitution: f32,
    pub friction: f32,
    pub cursor_radius: f32,
    pub cursor_sweep_min_speed: f32,
    pub cursor_bounce: f32,
    pub cursor_momentum_transfer: f32,
    pub cursor_throw_multiplier: f32,
    pub cursor_sweep_impulse_scale: f32,
    pub damage_threshold: f32,
    pub damage_scale: f32,
    pub shatter_health: f32,
    pub max_particles: usize,
    pub particle_drag: f32,
}

impl Default for MarbleConfig {
    fn default() -> Self {
        Self {
            min_radius: DEFAULT_MIN_RADIUS,
            max_radius: DEFAULT_MAX_RADIUS,
            max_speed: 4500.0,
            drag: 0.42,
            spin_drag: 1.1,
            wall_restitution: 0.68,
            marble_restitution: 0.92,
            friction: 0.32,
            cursor_radius: 20.0,
            cursor_sweep_min_speed: 780.0,
            cursor_bounce: 0.42,
            cursor_momentum_transfer: 0.055,
            cursor_throw_multiplier: 0.58,
            cursor_sweep_impulse_scale: 0.38,
            damage_threshold: 1150.0,
            damage_scale: 0.032,
            shatter_health: 0.0,
            max_particles: 2600,
            particle_drag: 1.05,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MarblePattern {
    pub seed: u32,
    pub primary: Vec4,
    pub secondary: Vec4,
    pub accent: Vec4,
    /// x = curve twist, y = brush width, z = curve amount, w = bubble density.
    pub ribbons: Vec4,
    /// x = bubble scale, y = glass tint, z = specular strength, w = refraction strength.
    pub glass: Vec4,
}

#[derive(Debug, Clone)]
pub struct Marble {
    pub id: u64,
    pub pos: Vec2,
    pub vel: Vec2,
    pub radius: f32,
    pub mass: f32,
    pub health: f32,
    pub max_health: f32,
    pub crack: f32,
    pub grabbed: bool,
    pub roll_angle: f32,
    pub roll_dir: Vec2,
    pub spin: f32,
    pub pattern: MarblePattern,
}

impl Marble {
    fn new(id: u64, pos: Vec2, radius: f32, pattern: MarblePattern) -> Self {
        let mass = (radius / DEFAULT_MAX_RADIUS).powi(2).max(0.18);
        Self {
            id,
            pos,
            vel: Vec2::ZERO,
            radius,
            mass,
            health: 100.0,
            max_health: 100.0,
            crack: 0.0,
            grabbed: false,
            roll_angle: 0.0,
            roll_dir: Vec2::X,
            spin: 0.0,
            pattern,
        }
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    fn health_frac(&self) -> f32 {
        (self.health / self.max_health).clamp(0.0, 1.0)
    }

    fn apply_damage(&mut self, speed: f32, cfg: &MarbleConfig) -> bool {
        if speed <= cfg.damage_threshold {
            return false;
        }
        let over = speed - cfg.damage_threshold;
        let damage = (over * cfg.damage_scale).min(85.0);
        self.health -= damage;
        self.crack = self.crack.max(1.0 - self.health_frac()).clamp(0.0, 1.0);
        true
    }

    fn roll_by(&mut self, delta: Vec2) {
        let distance = delta.length();
        if distance <= 0.001 || self.radius <= 0.001 {
            return;
        }
        self.roll_dir = delta / distance;
        self.roll_angle =
            (self.roll_angle + distance / self.radius).rem_euclid(std::f32::consts::TAU);
    }

    fn spin_by(&mut self, dt: f32) {
        if self.spin.abs() > 0.001 {
            self.roll_angle = (self.roll_angle + self.spin * dt).rem_euclid(std::f32::consts::TAU);
        }
    }

    fn add_spin(&mut self, impulse: f32) {
        self.spin = (self.spin + impulse).clamp(-MAX_SPIN, MAX_SPIN);
    }
}

#[derive(Debug, Clone)]
pub struct MarbleWorld {
    pub config: MarbleConfig,
    pub bounds: Bounds,
    pub visible_bounds: Vec<Bounds>,
    pub marbles: Vec<Marble>,
    pub particles: ParticleSystem,

    boundary_edges: Vec<BoundaryEdge>,
    accumulator: f32,
    cursor: Vec2,
    cursor_vel: Vec2,
    cursor_time: Option<f32>,
    grabbed_id: Option<u64>,
    grab_offset: Vec2,
    samples: VecDeque<PointerSample>,
    rng: u32,
    next_id: u64,
}

impl MarbleWorld {
    pub fn new(config: MarbleConfig, bounds: Bounds) -> Self {
        let visible_bounds = vec![bounds];
        let boundary_edges = build_boundary_edges(&visible_bounds, bounds);
        Self {
            particles: ParticleSystem::new(config.max_particles),
            config,
            bounds,
            visible_bounds,
            marbles: Vec::new(),
            boundary_edges,
            accumulator: 0.0,
            cursor: bounds.center(),
            cursor_vel: Vec2::ZERO,
            cursor_time: None,
            grabbed_id: None,
            grab_offset: Vec2::ZERO,
            samples: VecDeque::new(),
            rng: 0x6D61_7262,
            next_id: 1,
        }
    }

    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.set_visible_bounds(bounds, [bounds]);
    }

    pub fn set_visible_bounds<I>(&mut self, bounds: Bounds, visible_bounds: I)
    where
        I: IntoIterator<Item = Bounds>,
    {
        self.bounds = bounds;
        self.visible_bounds = visible_bounds
            .into_iter()
            .map(normalized_bounds)
            .filter(|bounds| bounds.width() > 1.0 && bounds.height() > 1.0)
            .collect();
        if self.visible_bounds.is_empty() {
            self.visible_bounds.push(bounds);
        }
        self.boundary_edges = build_boundary_edges(&self.visible_bounds, bounds);
        for marble in &mut self.marbles {
            marble.pos =
                safe_pos_in_regions(bounds, &self.visible_bounds, marble.pos, marble.radius);
        }
    }

    pub fn set_radius_range(&mut self, min_radius: f32, max_radius: f32) {
        self.config.min_radius = min_radius.clamp(8.0, 128.0);
        self.config.max_radius = max_radius.max(self.config.min_radius).clamp(8.0, 128.0);
    }

    pub fn spawn_random(&mut self) -> u64 {
        let radius = self.rand_range(self.config.min_radius, self.config.max_radius);
        let pos = self.random_spawn_pos(radius);
        self.spawn_at(pos, radius)
    }

    pub fn spawn_at(&mut self, pos: Vec2, radius: f32) -> u64 {
        let radius = radius.clamp(self.config.min_radius, self.config.max_radius);
        let id = self.next_id;
        self.next_id += 1;
        let pattern = self.random_pattern();
        let mut marble = Marble::new(
            id,
            safe_pos_in_regions(self.bounds, &self.visible_bounds, pos, radius),
            radius,
            pattern,
        );
        let nudge_angle = self.rand_range(0.0, std::f32::consts::TAU);
        let nudge_speed = self.rand_range(80.0, 260.0);
        marble.vel = Vec2::new(nudge_angle.cos(), nudge_angle.sin()) * nudge_speed;
        self.marbles.push(marble);
        id
    }

    pub fn clear(&mut self) {
        self.marbles.clear();
        self.particles.clear();
        self.grabbed_id = None;
        self.samples.clear();
    }

    pub fn scatter(&mut self, speed: f32) {
        for i in 0..self.marbles.len() {
            let angle = self.rand_range(0.0, std::f32::consts::TAU);
            let sp = self.rand_range(speed * 0.55, speed);
            self.marbles[i].vel += Vec2::new(angle.cos(), angle.sin()) * sp;
        }
    }

    pub fn grab(&mut self, cursor: Vec2, now: f32) -> bool {
        self.update_cursor_sample(cursor, now);
        let Some(index) = self.hit_index(cursor) else {
            return false;
        };
        for marble in &mut self.marbles {
            marble.grabbed = false;
        }
        let marble = &mut self.marbles[index];
        marble.grabbed = true;
        self.grabbed_id = Some(marble.id);
        self.grab_offset = marble.pos - cursor;
        self.samples.clear();
        self.samples.push_back(PointerSample {
            pos: cursor,
            time: now,
        });
        true
    }

    pub fn move_cursor(&mut self, cursor: Vec2, now: f32) {
        self.update_cursor_sample(cursor, now);
        if self.grabbed_index().is_some() {
            self.push_grab_sample(cursor, now);
        }
    }

    pub fn begin_kick(&mut self, cursor: Vec2, now: f32) {
        self.cursor = cursor;
        self.cursor_time = Some(now);
        self.cursor_vel = Vec2::ZERO;
    }

    pub fn kick_cursor(&mut self, cursor: Vec2, now: f32) {
        let prev = self.cursor;
        self.update_cursor_sample(cursor, now);
        if self.grabbed_index().is_some() {
            self.push_grab_sample(cursor, now);
        } else {
            self.sweep_hit_marbles(prev, cursor);
        }
    }

    pub fn release(&mut self, now: f32) {
        let velocity = self.weighted_cursor_velocity(now) * self.config.cursor_throw_multiplier;
        if let Some(index) = self.grabbed_index() {
            let marble = &mut self.marbles[index];
            marble.grabbed = false;
            marble.vel = clamp_len(velocity, self.config.max_speed);
            marble.add_spin(velocity.perp_dot(self.grab_offset) / marble.radius.max(1.0) * 0.02);
            self.particles
                .emit_shards(marble.pos, marble.speed(), 8, marble.pattern.accent);
        }
        self.grabbed_id = None;
        self.samples.clear();
    }

    pub fn is_grabbed(&self) -> bool {
        self.grabbed_id.is_some()
    }

    pub fn hit_test(&self, cursor: Vec2) -> bool {
        self.hit_index(cursor).is_some()
    }

    pub fn advance(&mut self, frame_dt: f32) {
        self.accumulator = (self.accumulator + frame_dt).min(0.25);
        while self.accumulator >= FIXED_DT {
            self.step(FIXED_DT);
            self.accumulator -= FIXED_DT;
        }
        self.particles.update(frame_dt, self.config.particle_drag);
    }

    pub fn is_active(&self) -> bool {
        self.is_grabbed()
            || !self.particles.is_empty()
            || self
                .marbles
                .iter()
                .any(|m| m.speed() > 1.0 || m.spin.abs() > 0.01)
    }

    fn step(&mut self, dt: f32) {
        if let Some(index) = self.grabbed_index() {
            apply_grab_spring(&mut self.marbles[index], self.cursor + self.grab_offset, dt);
        }

        for marble in &mut self.marbles {
            let prev = marble.pos;
            if !marble.grabbed {
                marble.vel *= (1.0 - self.config.drag * dt).max(0.0);
            }
            marble.vel = clamp_len(marble.vel, self.config.max_speed);
            marble.pos += marble.vel * dt;
            marble.roll_by(marble.pos - prev);
            marble.spin_by(dt);
            marble.spin *= (-self.config.spin_drag * dt).exp();
            if marble.spin.abs() < 0.01 {
                marble.spin = 0.0;
            }
        }

        self.resolve_boundary_collisions();
        self.resolve_marble_collisions();
        self.remove_shattered();
    }

    fn resolve_boundary_collisions(&mut self) {
        let cfg = self.config.clone();
        let mut shard_events = Vec::new();
        for marble in &mut self.marbles {
            let r = marble.radius;
            for edge in &self.boundary_edges {
                let closest = nearest_point_on_segment(marble.pos, edge.start, edge.end);
                let to_inside = (marble.pos - closest).dot(edge.normal);
                if to_inside >= r || to_inside < -r {
                    continue;
                }
                let projection = edge.project(marble.pos);
                if projection < -r || projection > edge.length + r {
                    continue;
                }

                let penetration = r - to_inside;
                marble.pos += edge.normal * penetration;
                let incoming = -marble.vel.dot(edge.normal);
                if incoming > 0.0 {
                    marble.vel += edge.normal * ((1.0 + cfg.wall_restitution) * incoming);
                    let tangent = Vec2::new(-edge.normal.y, edge.normal.x);
                    let tangent_speed = marble.vel.dot(tangent);
                    marble.vel -= tangent * tangent_speed * cfg.friction;
                    marble.vel *= 0.90;
                    marble.add_spin(edge.normal.perp_dot(marble.vel) / r.max(1.0) * 0.18);
                    if marble.apply_damage(incoming, &cfg) {
                        shard_events.push((closest, incoming, marble.pattern.accent));
                    }
                }
            }

            if !point_in_regions(&self.visible_bounds, marble.pos) {
                let safe = safe_pos_in_regions(
                    self.bounds,
                    &self.visible_bounds,
                    marble.pos,
                    marble.radius,
                );
                let normal = (safe - marble.pos).normalize_or_zero();
                marble.pos = safe;
                let incoming = -marble.vel.dot(normal);
                if normal.length_squared() > 0.0 && incoming > 0.0 {
                    marble.vel += normal * ((1.0 + cfg.wall_restitution) * incoming);
                    if marble.apply_damage(incoming, &cfg) {
                        shard_events.push((safe, incoming, marble.pattern.accent));
                    }
                }
            }
        }

        for (point, speed, color) in shard_events {
            self.particles.emit_shards(
                point,
                speed,
                ((speed / 190.0) as usize).clamp(4, 24),
                color,
            );
        }
    }

    fn resolve_marble_collisions(&mut self) {
        let pairs = self.collision_pairs();
        let cfg = self.config.clone();
        let mut shard_events = Vec::new();

        for (a, b) in pairs {
            let (left, right) = self.marbles.split_at_mut(b);
            let ma = &mut left[a];
            let mb = &mut right[0];
            let delta = mb.pos - ma.pos;
            let dist_sq = delta.length_squared();
            let min_dist = ma.radius + mb.radius;
            if dist_sq >= min_dist * min_dist {
                continue;
            }

            let dist = dist_sq.sqrt();
            let normal = if dist > 0.001 { delta / dist } else { Vec2::X };
            let penetration = min_dist - dist;
            let inv_a = 1.0 / ma.mass.max(0.001);
            let inv_b = 1.0 / mb.mass.max(0.001);
            let inv_sum = inv_a + inv_b;
            if inv_sum > 0.0 {
                ma.pos -= normal * penetration * (inv_a / inv_sum);
                mb.pos += normal * penetration * (inv_b / inv_sum);
            }

            let rel_vel = mb.vel - ma.vel;
            let approach = rel_vel.dot(normal);
            if approach >= 0.0 {
                continue;
            }

            let impulse_mag = -(1.0 + cfg.marble_restitution) * approach / inv_sum;
            let impulse = normal * impulse_mag;
            ma.vel -= impulse * inv_a;
            mb.vel += impulse * inv_b;

            let tangent = Vec2::new(-normal.y, normal.x);
            let tangent_speed = rel_vel.dot(tangent);
            ma.vel += tangent * tangent_speed * cfg.friction * 0.5;
            mb.vel -= tangent * tangent_speed * cfg.friction * 0.5;
            ma.add_spin(-tangent_speed / ma.radius.max(1.0) * 0.2);
            mb.add_spin(tangent_speed / mb.radius.max(1.0) * 0.2);

            let impact_speed = -approach;
            let contact = ma.pos + normal * ma.radius;
            let damaged_a = ma.apply_damage(impact_speed, &cfg);
            let damaged_b = mb.apply_damage(impact_speed, &cfg);
            if damaged_a {
                shard_events.push((contact, impact_speed, ma.pattern.accent));
            }
            if damaged_b {
                shard_events.push((contact, impact_speed, mb.pattern.accent));
            }
        }

        for (point, speed, color) in shard_events {
            self.particles.emit_shards(
                point,
                speed,
                ((speed / 180.0) as usize).clamp(5, 26),
                color,
            );
        }
    }

    fn collision_pairs(&self) -> Vec<(usize, usize)> {
        let cell_size = (self.config.max_radius * 2.25).max(72.0);
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, marble) in self.marbles.iter().enumerate() {
            let cell = cell_for(marble.pos, cell_size);
            grid.entry(cell).or_default().push(i);
        }

        let mut seen = HashSet::new();
        let mut pairs = Vec::new();
        for (&cell, indices) in &grid {
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let other_cell = (cell.0 + dx, cell.1 + dy);
                    let Some(other_indices) = grid.get(&other_cell) else {
                        continue;
                    };
                    for &a in indices {
                        for &b in other_indices {
                            if a == b {
                                continue;
                            }
                            let pair = if a < b { (a, b) } else { (b, a) };
                            if seen.insert(pair) {
                                pairs.push(pair);
                            }
                        }
                    }
                }
            }
        }
        pairs
    }

    fn sweep_hit_marbles(&mut self, prev_cursor: Vec2, cursor: Vec2) {
        let cursor_speed = self.cursor_vel.length();
        if cursor_speed < self.config.cursor_sweep_min_speed {
            return;
        }

        let cfg = self.config.clone();
        let mut shard_events = Vec::new();
        for marble in &mut self.marbles {
            let hit_radius = marble.radius + cfg.cursor_radius;
            let closest = nearest_point_on_segment(marble.pos, prev_cursor, cursor);
            let delta = marble.pos - closest;
            let distance = delta.length();
            if distance > hit_radius {
                continue;
            }
            let normal = if distance > 0.001 {
                delta / distance
            } else {
                self.cursor_vel.normalize_or_zero()
            };
            if normal.length_squared() <= 0.0 {
                continue;
            }

            let relative = self.cursor_vel - marble.vel;
            let approach = relative.dot(normal);
            if approach <= 0.0 {
                continue;
            }

            let penetration = hit_radius - distance;
            let impulse = (normal * (approach * (1.0 + cfg.cursor_bounce) + penetration * 14.0)
                + self.cursor_vel * cfg.cursor_momentum_transfer)
                * cfg.cursor_sweep_impulse_scale;
            marble.vel = clamp_len(marble.vel + impulse / marble.mass.max(0.001), cfg.max_speed);
            let contact = closest - marble.pos;
            marble.add_spin(contact.perp_dot(relative) / marble.radius.max(1.0).powi(2) * 0.65);
            if marble.apply_damage(approach, &cfg) {
                shard_events.push((closest, approach, marble.pattern.accent));
            }
        }

        for (point, speed, color) in shard_events {
            self.particles.emit_shards(
                point,
                speed,
                ((speed / 240.0) as usize).clamp(4, 18),
                color,
            );
        }
    }

    fn remove_shattered(&mut self) {
        let cfg = self.config.clone();
        let mut shattered = Vec::new();
        self.marbles.retain(|marble| {
            if marble.health <= cfg.shatter_health {
                shattered.push((marble.pos, marble.speed(), marble.pattern.accent, marble.id));
                false
            } else {
                true
            }
        });
        for (pos, speed, color, id) in shattered {
            if self.grabbed_id == Some(id) {
                self.grabbed_id = None;
                self.samples.clear();
            }
            self.particles
                .emit_shards(pos, speed.max(1500.0), 72, color.lerp(Vec4::ONE, 0.35));
        }
    }

    fn hit_index(&self, cursor: Vec2) -> Option<usize> {
        self.marbles
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, marble)| cursor.distance(marble.pos) <= marble.radius + 6.0)
            .min_by(|(_, a), (_, b)| cursor.distance(a.pos).total_cmp(&cursor.distance(b.pos)))
            .map(|(index, _)| index)
    }

    fn grabbed_index(&self) -> Option<usize> {
        let id = self.grabbed_id?;
        self.marbles.iter().position(|marble| marble.id == id)
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

    fn push_grab_sample(&mut self, cursor: Vec2, now: f32) {
        self.samples.push_back(PointerSample {
            pos: cursor,
            time: now,
        });
        let cutoff = now - THROW_WINDOW * 2.0;
        while self.samples.len() > 2 && self.samples.front().is_some_and(|s| s.time < cutoff) {
            self.samples.pop_front();
        }
    }

    fn weighted_cursor_velocity(&self, now: f32) -> Vec2 {
        if self.samples.len() < 2 {
            return Vec2::ZERO;
        }
        let mut sum = Vec2::ZERO;
        let mut wsum = 0.0;
        let mut it = self.samples.iter();
        let mut prev = *it.next().unwrap();
        for sample in it {
            let dt = sample.time - prev.time;
            if dt > 1e-5 {
                let v = (sample.pos - prev.pos) / dt;
                let age = (now - sample.time).max(0.0);
                let w = (-age / THROW_WINDOW).exp();
                sum += v * w;
                wsum += w;
            }
            prev = *sample;
        }
        if wsum > 0.0 {
            sum / wsum
        } else {
            Vec2::ZERO
        }
    }

    fn random_spawn_pos(&mut self, radius: f32) -> Vec2 {
        let region_index = ((self.rand() * self.visible_bounds.len() as f32) as usize)
            .min(self.visible_bounds.len().saturating_sub(1));
        let region = self
            .visible_bounds
            .get(region_index)
            .copied()
            .unwrap_or(self.bounds);
        let margin = (radius * 1.25).max(8.0);
        let min_x = region.left + margin;
        let max_x = region.right - margin;
        let min_y = region.top + margin;
        let max_y = region.bottom - margin;

        let mut fallback =
            safe_pos_in_regions(self.bounds, &self.visible_bounds, region.center(), radius);
        for _ in 0..24 {
            let pos = Vec2::new(
                if min_x < max_x {
                    self.rand_range(min_x, max_x)
                } else {
                    region.center().x
                },
                if min_y < max_y {
                    self.rand_range(min_y, max_y)
                } else {
                    region.center().y
                },
            );
            fallback = pos;
            if self
                .marbles
                .iter()
                .all(|m| pos.distance(m.pos) >= radius + m.radius + 4.0)
            {
                return pos;
            }
        }
        fallback
    }

    fn random_pattern(&mut self) -> MarblePattern {
        let palette = (self.rand() * PALETTES.len() as f32) as usize % PALETTES.len();
        let [primary, secondary, accent] = PALETTES[palette];
        MarblePattern {
            seed: self.next_rand_u32(),
            primary,
            secondary,
            accent,
            ribbons: Vec4::new(
                self.rand_range(0.75, 2.25),
                self.rand_range(0.09, 0.16),
                self.rand_range(0.36, 0.82),
                self.rand_range(0.48, 1.0),
            ),
            glass: Vec4::new(
                self.rand_range(0.75, 1.55),
                self.rand_range(0.30, 0.70),
                self.rand_range(0.42, 0.72),
                self.rand_range(0.022, 0.052),
            ),
        }
    }

    fn next_rand_u32(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn rand(&mut self) -> f32 {
        self.next_rand_u32() as f32 / u32::MAX as f32
    }

    fn rand_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand().clamp(0.0, 1.0)
    }
}

const PALETTES: [[Vec4; 3]; 14] = [
    [
        Vec4::new(0.95, 0.05, 0.04, 1.0),
        Vec4::new(0.04, 0.18, 0.95, 1.0),
        Vec4::new(1.0, 0.58, 0.02, 1.0),
    ],
    [
        Vec4::new(0.80, 0.02, 0.16, 1.0),
        Vec4::new(0.02, 0.62, 0.22, 1.0),
        Vec4::new(0.36, 0.04, 0.78, 1.0),
    ],
    [
        Vec4::new(1.0, 0.36, 0.02, 1.0),
        Vec4::new(0.02, 0.46, 0.82, 1.0),
        Vec4::new(0.90, 0.02, 0.46, 1.0),
    ],
    [
        Vec4::new(0.92, 0.02, 0.58, 1.0),
        Vec4::new(0.05, 0.22, 0.86, 1.0),
        Vec4::new(0.54, 0.90, 0.06, 1.0),
    ],
    [
        Vec4::new(0.78, 0.06, 0.02, 1.0),
        Vec4::new(0.04, 0.12, 0.16, 1.0),
        Vec4::new(0.80, 0.84, 0.04, 1.0),
    ],
    [
        Vec4::new(0.04, 0.70, 0.62, 1.0),
        Vec4::new(0.54, 0.06, 0.88, 1.0),
        Vec4::new(1.0, 0.26, 0.14, 1.0),
    ],
    [
        Vec4::new(0.96, 0.03, 0.03, 1.0),
        Vec4::new(0.92, 0.82, 0.02, 1.0),
        Vec4::new(0.03, 0.16, 0.92, 1.0),
    ],
    [
        Vec4::new(0.04, 0.44, 0.16, 1.0),
        Vec4::new(0.02, 0.22, 0.76, 1.0),
        Vec4::new(1.0, 0.42, 0.04, 1.0),
    ],
    [
        Vec4::new(0.98, 0.04, 0.38, 1.0),
        Vec4::new(0.90, 0.14, 0.02, 1.0),
        Vec4::new(0.18, 0.04, 0.72, 1.0),
    ],
    [
        Vec4::new(0.02, 0.18, 0.88, 1.0),
        Vec4::new(0.86, 0.03, 0.08, 1.0),
        Vec4::new(0.08, 0.76, 0.36, 1.0),
    ],
    [
        Vec4::new(0.50, 0.03, 0.90, 1.0),
        Vec4::new(0.96, 0.18, 0.08, 1.0),
        Vec4::new(0.04, 0.54, 0.94, 1.0),
    ],
    [
        Vec4::new(0.96, 0.10, 0.02, 1.0),
        Vec4::new(0.18, 0.80, 0.08, 1.0),
        Vec4::new(0.04, 0.18, 0.96, 1.0),
    ],
    [
        Vec4::new(0.66, 0.02, 0.12, 1.0),
        Vec4::new(0.02, 0.34, 0.68, 1.0),
        Vec4::new(0.95, 0.50, 0.02, 1.0),
    ],
    [
        Vec4::new(0.04, 0.64, 0.34, 1.0),
        Vec4::new(0.82, 0.03, 0.54, 1.0),
        Vec4::new(0.08, 0.18, 0.86, 1.0),
    ],
];

fn apply_grab_spring(marble: &mut Marble, target: Vec2, dt: f32) {
    let mut force = GRAB_STIFFNESS * (target - marble.pos) - GRAB_DAMPING * marble.vel;
    let mag = force.length();
    if mag > GRAB_MAX_FORCE {
        force *= GRAB_MAX_FORCE / mag;
    }
    marble.vel += force / marble.mass.max(0.001) * dt;
}

#[derive(Debug, Clone, Copy)]
struct BoundaryEdge {
    start: Vec2,
    end: Vec2,
    normal: Vec2,
    axis: Vec2,
    length: f32,
}

impl BoundaryEdge {
    fn new(start: Vec2, end: Vec2, normal: Vec2) -> Option<Self> {
        let delta = end - start;
        let length = delta.length();
        if length <= 1.0 {
            return None;
        }
        Some(Self {
            start,
            end,
            normal: normal.normalize_or_zero(),
            axis: delta / length,
            length,
        })
    }

    fn project(&self, point: Vec2) -> f32 {
        (point - self.start).dot(self.axis)
    }
}

fn build_boundary_edges(regions: &[Bounds], fallback: Bounds) -> Vec<BoundaryEdge> {
    let regions: Vec<_> = regions
        .iter()
        .copied()
        .map(normalized_bounds)
        .filter(|bounds| bounds.width() > 1.0 && bounds.height() > 1.0)
        .collect();
    let regions = if regions.is_empty() {
        vec![fallback]
    } else {
        regions
    };

    let mut edges = Vec::new();
    for (i, region) in regions.iter().copied().enumerate() {
        let mut top_spans = vec![(region.left, region.right)];
        let mut bottom_spans = vec![(region.left, region.right)];
        let mut left_spans = vec![(region.top, region.bottom)];
        let mut right_spans = vec![(region.top, region.bottom)];

        for (j, other) in regions.iter().copied().enumerate() {
            if i == j {
                continue;
            }
            if other.bottom >= region.top - 1.0 && other.top < region.top - 1.0 {
                subtract_span(&mut top_spans, other.left, other.right);
            }
            if other.top <= region.bottom + 1.0 && other.bottom > region.bottom + 1.0 {
                subtract_span(&mut bottom_spans, other.left, other.right);
            }
            if other.right >= region.left - 1.0 && other.left < region.left - 1.0 {
                subtract_span(&mut left_spans, other.top, other.bottom);
            }
            if other.left <= region.right + 1.0 && other.right > region.right + 1.0 {
                subtract_span(&mut right_spans, other.top, other.bottom);
            }
        }

        for (left, right) in top_spans {
            if let Some(edge) = BoundaryEdge::new(
                Vec2::new(left, region.top),
                Vec2::new(right, region.top),
                Vec2::Y,
            ) {
                edges.push(edge);
            }
        }
        for (left, right) in bottom_spans {
            if let Some(edge) = BoundaryEdge::new(
                Vec2::new(left, region.bottom),
                Vec2::new(right, region.bottom),
                -Vec2::Y,
            ) {
                edges.push(edge);
            }
        }
        for (top, bottom) in left_spans {
            if let Some(edge) = BoundaryEdge::new(
                Vec2::new(region.left, top),
                Vec2::new(region.left, bottom),
                Vec2::X,
            ) {
                edges.push(edge);
            }
        }
        for (top, bottom) in right_spans {
            if let Some(edge) = BoundaryEdge::new(
                Vec2::new(region.right, top),
                Vec2::new(region.right, bottom),
                -Vec2::X,
            ) {
                edges.push(edge);
            }
        }
    }

    edges
}

fn subtract_span(spans: &mut Vec<(f32, f32)>, left: f32, right: f32) {
    let span_min = left.min(right);
    let span_max = left.max(right);
    let left = span_min;
    let right = span_max;
    if right - left <= 0.0 {
        return;
    }

    let mut remaining = Vec::with_capacity(spans.len() + 1);
    for (span_left, span_right) in spans.drain(..) {
        let overlap_left = span_left.max(left);
        let overlap_right = span_right.min(right);
        if overlap_right <= overlap_left {
            remaining.push((span_left, span_right));
            continue;
        }
        if span_left < overlap_left {
            remaining.push((span_left, overlap_left));
        }
        if overlap_right < span_right {
            remaining.push((overlap_right, span_right));
        }
    }
    *spans = remaining;
}

fn circle_center_in_regions(regions: &[Bounds], pos: Vec2, radius: f32) -> bool {
    regions
        .iter()
        .copied()
        .any(|region| circle_center_in_region(region, pos, radius))
}

fn point_in_regions(regions: &[Bounds], pos: Vec2) -> bool {
    regions.iter().copied().any(|region| {
        pos.x >= region.left
            && pos.x <= region.right
            && pos.y >= region.top
            && pos.y <= region.bottom
    })
}

fn circle_center_in_region(region: Bounds, pos: Vec2, radius: f32) -> bool {
    pos.x >= region.left + radius
        && pos.x <= region.right - radius
        && pos.y >= region.top + radius
        && pos.y <= region.bottom - radius
}

fn safe_pos_in_regions(bounds: Bounds, regions: &[Bounds], pos: Vec2, radius: f32) -> Vec2 {
    if circle_center_in_regions(regions, pos, radius) {
        return pos;
    }

    regions
        .iter()
        .copied()
        .map(|region| {
            let safe = safe_pos(region, pos, radius);
            (safe, safe.distance_squared(pos))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(safe, _)| safe)
        .unwrap_or_else(|| safe_pos(bounds, pos, radius))
}

fn normalized_bounds(bounds: Bounds) -> Bounds {
    Bounds::new(
        bounds.left.min(bounds.right),
        bounds.top.min(bounds.bottom),
        bounds.left.max(bounds.right),
        bounds.top.max(bounds.bottom),
    )
}

fn safe_pos(bounds: Bounds, pos: Vec2, radius: f32) -> Vec2 {
    let x_min = bounds.left + radius;
    let x_max = bounds.right - radius;
    let y_min = bounds.top + radius;
    let y_max = bounds.bottom - radius;
    Vec2::new(
        if x_min <= x_max {
            pos.x.clamp(x_min, x_max)
        } else {
            bounds.center().x
        },
        if y_min <= y_max {
            pos.y.clamp(y_min, y_max)
        } else {
            bounds.center().y
        },
    )
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

fn cell_for(pos: Vec2, cell_size: f32) -> (i32, i32) {
    (
        (pos.x / cell_size).floor() as i32,
        (pos.y / cell_size).floor() as i32,
    )
}
