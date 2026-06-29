//! Shared application state used by platform shells.

use std::sync::OnceLock;
use std::time::Instant;

use fidget_sim::{BottomEdge, Bounds, ParticleKind, World};
use glam::{Vec2, Vec3, Vec4};

use crate::config::Settings;
use crate::renderer::{Instance, RubberBandMesh};

const SOCCER_GLOW_TEXTURE_PNG: &[u8] =
    include_bytes!("../../../../assets/soccer_ball_material.png");
static SOCCER_GLOW_TEXTURE: OnceLock<Option<image::RgbaImage>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppAction {
    Reset,
    ToggleSpring,
    ToggleSpringVisual,
    ToggleGravity,
    ToggleBottomBounce,
    ToggleHud,
    Nudge,
}

pub(super) struct Core {
    settings: Settings,
    world: World,
    start: Instant,
    last_frame: Instant,
    cursor: Vec2,
    spring_interaction_active: bool,
    instances: Vec<Instance>,
    rubber_band: RubberBandMesh,
    egui_ctx: egui::Context,
    hud_visible: bool,
    hud_rect: Option<egui::Rect>,
}

impl Core {
    pub(super) fn new(settings: Settings) -> Self {
        let bounds = Bounds::new(0.0, 0.0, 1280.0, 720.0);
        let world = World::new(settings.world_config(), bounds);
        Self {
            settings,
            world,
            start: Instant::now(),
            last_frame: Instant::now(),
            cursor: Vec2::ZERO,
            spring_interaction_active: false,
            instances: Vec::with_capacity(4096),
            rubber_band: RubberBandMesh::with_capacity(2048, 8192),
            egui_ctx: egui::Context::default(),
            hud_visible: false,
            hud_rect: None,
        }
    }

    pub(super) fn egui_context(&self) -> egui::Context {
        self.egui_ctx.clone()
    }

    pub(super) fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub(super) fn rubber_band(&self) -> &RubberBandMesh {
        &self.rubber_band
    }

    pub(super) fn now(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    pub(super) fn reset_frame_clock(&mut self) {
        self.last_frame = Instant::now();
    }

    pub(super) fn advance_frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.world.advance(dt);
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.world
            .set_bounds(Bounds::new(0.0, 0.0, width as f32, height as f32));
    }

    pub(super) fn set_bottom_edges<I>(&mut self, edges: I)
    where
        I: IntoIterator<Item = BottomEdge>,
    {
        self.world.set_bottom_edges(edges);
    }

    pub(super) fn save_settings(&self) {
        self.settings.save();
    }

    #[cfg(target_os = "windows")]
    pub(super) fn hud_visible(&self) -> bool {
        self.hud_visible
    }

    #[cfg(target_os = "windows")]
    pub(super) fn gravity_enabled(&self) -> bool {
        self.world.gravity_strength() > 0.0
    }

    #[cfg(target_os = "windows")]
    pub(super) fn spring_attached(&self) -> bool {
        self.world.spring_attached()
    }

    #[cfg(target_os = "windows")]
    pub(super) fn rubber_band_visual_enabled(&self) -> bool {
        self.settings.visuals.spring_visual.is_rubber_band()
    }

    #[cfg(target_os = "windows")]
    pub(super) fn bottom_bounce_enabled(&self) -> bool {
        self.world.bottom_bounce_enabled()
    }

    pub(super) fn on_cursor_moved(&mut self, cursor: Vec2) {
        self.cursor = cursor;
        let now = self.now();
        if self.spring_interaction_active {
            self.world.interact_spring(cursor, now);
        } else {
            self.world.move_cursor(cursor, now);
        }
    }

    pub(super) fn on_left_pressed(&mut self) -> bool {
        let now = self.now();
        self.world.grab(self.cursor, now)
    }

    pub(super) fn on_left_released(&mut self) {
        let now = self.now();
        self.world.release(now);
    }

    pub(super) fn on_right_pressed(&mut self) {
        self.spring_interaction_active = true;
        let now = self.now();
        self.world.interact_spring(self.cursor, now);
    }

    pub(super) fn on_right_released(&mut self) {
        if self.spring_interaction_active {
            self.spring_interaction_active = false;
            self.world.stop_spring_interaction();
        }
    }

    pub(super) fn apply_action(&mut self, action: AppAction) {
        match action {
            AppAction::Reset => self.world.reset(),
            AppAction::ToggleSpring => self.world.toggle_spring(),
            AppAction::ToggleSpringVisual => {
                self.settings.visuals.spring_visual = self.settings.visuals.spring_visual.toggled();
            }
            AppAction::ToggleGravity => self.world.toggle_gravity(),
            AppAction::ToggleBottomBounce => {
                let enabled = !self.world.bottom_bounce_enabled();
                self.world.set_bottom_bounce_enabled(enabled);
                self.settings.sim.bounce_bottom_edge = enabled;
            }
            AppAction::ToggleHud => self.hud_visible = !self.hud_visible,
            AppAction::Nudge => self.world.nudge(2800.0),
        }
    }

    pub(super) fn show_hud(&mut self, ctx: &egui::Context) {
        let window_response = if !self.hud_visible {
            egui::Window::new("HUD")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .fixed_pos(egui::pos2(18.0, 18.0))
                .show(ctx, |ui| {
                    if ui.button("Show Fidget controls").clicked() {
                        self.hud_visible = true;
                    }
                })
        } else {
            egui::Window::new("Fidget controls")
                .default_pos(egui::pos2(18.0, 18.0))
                .default_width(290.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("H toggles this HUD");
                        if ui.button("Hide").clicked() {
                            self.hud_visible = false;
                        }
                    });
                    ui.separator();

                    let old_gravity = self.world.gravity_strength();
                    let mut gravity = old_gravity;
                    ui.horizontal(|ui| {
                        if ui.button("-").clicked() {
                            gravity -= 150.0;
                        }
                        ui.add(egui::Slider::new(&mut gravity, 0.0..=2400.0).text("gravity"));
                        if ui.button("+").clicked() {
                            gravity += 150.0;
                        }
                    });
                    if (gravity - old_gravity).abs() > f32::EPSILON {
                        self.world.set_gravity_strength(gravity);
                    }

                    let old_stiffness = self.world.spring_stiffness();
                    let mut stiffness = old_stiffness;
                    ui.horizontal(|ui| {
                        if ui.button("soft").clicked() {
                            stiffness -= 25.0;
                        }
                        ui.add(
                            egui::Slider::new(&mut stiffness, 15.0..=420.0)
                                .text("string elasticity"),
                        );
                        if ui.button("stiff").clicked() {
                            stiffness += 25.0;
                        }
                    });
                    if (stiffness - old_stiffness).abs() > f32::EPSILON {
                        self.world.set_spring_stiffness(stiffness);
                    }

                    let old_damping = self.world.spring_damping();
                    let mut damping = old_damping;
                    ui.horizontal(|ui| {
                        if ui.button("-").clicked() {
                            damping -= 6.0;
                        }
                        ui.add(egui::Slider::new(&mut damping, 2.0..=90.0).text("string damping"));
                        if ui.button("+").clicked() {
                            damping += 6.0;
                        }
                    });
                    if (damping - old_damping).abs() > f32::EPSILON {
                        self.world.set_spring_damping(damping);
                    }

                    let old_hook = self.world.hook_offset_y();
                    let mut hook = old_hook;
                    ui.horizontal(|ui| {
                        if ui.button("Hook higher").clicked() {
                            hook -= 60.0;
                        }
                        if ui.button("Hook lower").clicked() {
                            hook += 60.0;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(&mut hook, -600.0..=260.0).text("hook y offset"));
                    });
                    if (hook - old_hook).abs() > f32::EPSILON {
                        self.world.set_hook_offset_y(hook);
                    }
                    ui.small("Negative hook offset places the string hook above the desktop.");

                    ui.horizontal(|ui| {
                        if ui.button("Reset ball").clicked() {
                            self.world.reset();
                        }
                        if ui.button("Cut/recall").clicked() {
                            self.world.toggle_spring();
                        }
                    });
                })
        };

        self.hud_rect = window_response.map(|response| response.response.rect.expand(8.0));
    }

    #[cfg(target_os = "windows")]
    pub(super) fn hit_test_interactive(&self, cursor: Vec2) -> bool {
        if self.world.is_grabbed() {
            return true;
        }
        // The Win32 shell asks this during `WM_NCHITTEST`; returning false lets
        // normal desktop windows receive clicks through transparent overlay space.
        if self
            .hud_rect
            .is_some_and(|rect| rect.contains(egui::pos2(cursor.x, cursor.y)))
        {
            return true;
        }
        let ball = &self.world.ball;
        if cursor.distance(ball.pos) <= ball.radius + 12.0 {
            return true;
        }
        false
    }

    pub(super) fn build_instances(&mut self) {
        let world = &self.world;
        let cfg = &world.config;
        let rubber_visual = self.settings.visuals.spring_visual.is_rubber_band();
        self.instances.clear();
        self.rubber_band.clear();

        // Trail (drawn first, faint and soft). Rubber-band mode uses faded
        // ball ghosts instead of the default blue glow trail.
        if cfg.trail_enabled && !rubber_visual {
            for p in world.trail.iter() {
                let a = world.trail.alpha_for(p);
                if a <= 0.01 {
                    continue;
                }
                let col = cfg.color_outer;
                self.instances.push(Instance {
                    center: p.pos.to_array(),
                    half: [p.radius * 0.55, p.radius * 0.55],
                    color: [col.x, col.y, col.z, a * 0.35],
                    softness: 1.0,
                    material: 0.0,
                    roll: [1.0, 0.0, 0.0, 0.0],
                });
            }
        }

        if world.spring.attached {
            if rubber_visual {
                rebuild_rubber_band(&mut self.rubber_band, world);
            } else {
                push_spring_instances(&mut self.instances, world);
            }
        }

        let ball = &world.ball;
        let s = ball.squash_scale(cfg.max_speed);
        let v = ball.vel;
        let roll_dir = if v.length_squared() > 1.0 {
            v.normalize()
        } else {
            ball.roll_dir
        };
        let r = ball.radius;
        let glow_outer = if rubber_visual {
            rubber_ball_glow_color(roll_dir, ball.roll_angle)
        } else {
            cfg.color_outer
        };
        let glow_inner = if rubber_visual {
            glow_outer.lerp(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.28)
        } else {
            cfg.color_inner
        };
        let c = ball.pos.to_array();
        let roll = [roll_dir.x, roll_dir.y, ball.roll_angle, 0.0];

        // Particles.
        if cfg.particles_enabled {
            for p in world.particles.iter() {
                let lf = p.life_frac();
                let base = match p.kind {
                    ParticleKind::Spark => Vec4::new(1.0, 0.85, 0.5, 1.0),
                    ParticleKind::Burst => glow_outer,
                    ParticleKind::Mote => glow_inner,
                };
                self.instances.push(Instance {
                    center: p.pos.to_array(),
                    half: [p.size, p.size],
                    color: [base.x, base.y, base.z, lf * base.w * 0.9],
                    softness: 0.85,
                    material: 0.0,
                    roll: [1.0, 0.0, 0.0, 0.0],
                });
            }
        }

        // Ball: faint glow halo plus matte textured body.
        if rubber_visual && cfg.trail_enabled {
            push_ball_trail_instances(&mut self.instances, world, roll_dir, ball.roll_angle);
        }

        self.instances.push(Instance {
            center: c,
            half: [r * 2.4 * s.x, r * 2.4 * s.y],
            color: [
                glow_outer.x,
                glow_outer.y,
                glow_outer.z,
                if rubber_visual { 0.09 } else { 0.08 },
            ],
            softness: 1.0,
            material: 0.0,
            roll,
        });
        self.instances.push(Instance {
            center: c,
            half: [r * 1.05 * s.x, r * 1.05 * s.y],
            color: [1.0, 1.0, 1.0, 1.0],
            softness: 0.08,
            material: 1.0,
            roll,
        });
    }
}

fn rubber_ball_glow_color(roll_dir: Vec2, roll_angle: f32) -> Vec4 {
    let Some(texture) = soccer_glow_texture() else {
        return Vec4::new(0.82, 0.80, 0.74, 1.0);
    };

    let axis = if roll_dir.length_squared() > 0.001 {
        roll_dir.normalize()
    } else {
        Vec2::X
    };
    let tangent = Vec2::new(-axis.y, axis.x);
    let (sin, cos) = roll_angle.sin_cos();
    let light_dir = Vec3::new(-0.35, -0.45, 0.82).normalize();
    let mut sum = Vec3::ZERO;
    let mut weight_sum = 0.0_f32;

    const GRID: usize = 17;
    for y in 0..GRID {
        for x in 0..GRID {
            let local = Vec2::new(
                (x as f32 + 0.5) / GRID as f32 * 2.0 - 1.0,
                (y as f32 + 0.5) / GRID as f32 * 2.0 - 1.0,
            );
            let r2 = local.length_squared();
            if r2 > 1.0 {
                continue;
            }

            let z = (1.0 - r2).sqrt();
            let along = local.dot(axis);
            let across = local.dot(tangent);
            let rolled_along = along * cos - z * sin;
            let material_local = axis * rolled_along + tangent * across;
            let uv = material_local * 0.5 + Vec2::splat(0.5);
            let tex = sample_soccer_texture(texture, uv);

            let normal = Vec3::new(local.x, -local.y, z).normalize();
            let diffuse = normal.dot(light_dir).max(0.0);
            let shade = 0.70 + diffuse * 0.30;
            let screen_weight = 0.28 + z * 0.72;
            sum += tex * shade * screen_weight;
            weight_sum += screen_weight;
        }
    }

    if weight_sum <= f32::EPSILON {
        return Vec4::new(0.82, 0.80, 0.74, 1.0);
    }

    let avg = sum / weight_sum;
    let glow = (avg * 1.15 + Vec3::splat(0.03)).clamp(Vec3::ZERO, Vec3::ONE);
    Vec4::new(glow.x, glow.y, glow.z, 1.0)
}

fn soccer_glow_texture() -> Option<&'static image::RgbaImage> {
    SOCCER_GLOW_TEXTURE
        .get_or_init(|| {
            image::load_from_memory(SOCCER_GLOW_TEXTURE_PNG)
                .ok()
                .map(|image| image.to_rgba8())
        })
        .as_ref()
}

fn sample_soccer_texture(texture: &image::RgbaImage, uv: Vec2) -> Vec3 {
    let width = texture.width().max(1);
    let height = texture.height().max(1);
    let u = uv.x.rem_euclid(1.0);
    let v = uv.y.rem_euclid(1.0);
    let x = (u * (width - 1) as f32).round() as u32;
    let y = (v * (height - 1) as f32).round() as u32;
    let pixel = texture.get_pixel(x, y);
    Vec3::new(
        pixel[0] as f32 / 255.0,
        pixel[1] as f32 / 255.0,
        pixel[2] as f32 / 255.0,
    )
}

fn push_ball_trail_instances(
    instances: &mut Vec<Instance>,
    world: &World,
    roll_dir: Vec2,
    roll_angle: f32,
) {
    let ball = &world.ball;
    for p in world.trail.iter() {
        let age_alpha = world.trail.alpha_for(p);
        let dist = p.pos.distance(ball.pos);
        if age_alpha <= 0.04 || dist <= ball.radius * 0.35 {
            continue;
        }

        let distance_t = (dist / (ball.radius * 8.0)).clamp(0.0, 1.0);
        let scale = (0.32 + age_alpha * 0.54 - distance_t * 0.20).clamp(0.24, 0.82);
        let alpha = (age_alpha.powf(1.35) * (0.46 - distance_t * 0.10)).clamp(0.0, 0.46);
        let axis = (ball.pos - p.pos).normalize_or_zero();
        let axis = if axis.length_squared() > 0.0 {
            axis
        } else {
            roll_dir
        };
        let ghost_roll = (roll_angle - dist / ball.radius).rem_euclid(std::f32::consts::TAU);
        instances.push(Instance {
            center: p.pos.to_array(),
            half: [p.radius * scale, p.radius * scale],
            color: [1.0, 1.0, 1.0, alpha],
            softness: 0.08,
            material: 1.0,
            roll: [axis.x, axis.y, ghost_roll, 0.0],
        });
    }
}

fn rebuild_rubber_band(mesh: &mut RubberBandMesh, world: &World) {
    let anchor = world.spring.anchor;
    let ball = world.ball.pos;
    let mut path = Vec::with_capacity(32);
    let mut joints = Vec::with_capacity(6);

    path.push(anchor);
    joints.push(anchor);
    if let Some(entanglement) = world.spring.entanglement {
        let loop_radius = entanglement.radius.clamp(world.ball.radius * 0.8, 150.0);
        let start_angle = (anchor - entanglement.center).to_angle();
        let spin = entanglement.angular_velocity.signum();
        let spin = if spin.abs() > 0.0 { spin } else { 1.0 };
        let loop_segments = ((loop_radius * 0.42).round() as usize).clamp(28, 48);
        for i in 0..=loop_segments {
            let t = i as f32 / loop_segments as f32;
            let angle = start_angle + spin * std::f32::consts::TAU * 1.08 * t;
            path.push(entanglement.center + Vec2::new(angle.cos(), angle.sin()) * loop_radius);
        }
        if let Some(&loop_end) = path.last() {
            joints.push(loop_end);
        }
    } else if let Some(intersection) = world.spring.intersection {
        path.push(intersection.point);
        joints.push(intersection.point);
    }

    let last = path.last().copied().unwrap_or(anchor);
    let ball_joint = ball_band_attach(ball, last, world.ball.radius);
    path.push(ball_joint);
    joints.push(ball_joint);

    let stretch = anchor.distance(ball).max(1.0) / world.spring.rest_length.max(1.0);
    let radius = (8.8 / stretch.sqrt()).clamp(4.8, 10.5);
    let primary = Vec4::new(0.56, 0.36, 0.17, 1.0);
    let accent = Vec4::new(0.94, 0.73, 0.43, 1.0);
    mesh.rebuild(&path, &joints, primary, accent, radius);
}

fn ball_band_attach(ball: Vec2, from: Vec2, radius: f32) -> Vec2 {
    let dir = (from - ball).normalize_or_zero();
    if dir.length_squared() > 0.0 {
        ball + dir * radius * 0.88
    } else {
        ball
    }
}

fn push_spring_instances(instances: &mut Vec<Instance>, world: &World) {
    let anchor = world.spring.anchor;
    let ball = world.ball.pos;
    let outer = world.config.color_outer;
    let inner = world.config.color_inner;

    instances.push(Instance {
        center: anchor.to_array(),
        half: [8.0, 8.0],
        color: [inner.x, inner.y, inner.z, 0.85],
        softness: 0.75,
        material: 0.0,
        roll: [1.0, 0.0, 0.0, 0.0],
    });

    if let Some(entanglement) = world.spring.entanglement {
        push_coil_instances(instances, anchor, entanglement.center, outer, 0.5, 3.8, 9.0);
        push_coil_instances(instances, entanglement.center, ball, outer, 0.72, 4.8, 13.0);
        push_entangle_loop(
            instances,
            entanglement.center,
            entanglement.radius,
            inner,
            outer,
        );
    } else if let Some(intersection) = world.spring.intersection {
        push_coil_instances(
            instances,
            anchor,
            intersection.point,
            outer,
            0.54,
            4.2,
            10.0,
        );
        push_coil_instances(instances, intersection.point, ball, outer, 0.68, 4.8, 13.0);
        instances.push(Instance {
            center: intersection.point.to_array(),
            half: [13.0, 13.0],
            color: [inner.x, inner.y, inner.z, 0.34 * intersection.strength()],
            softness: 0.9,
            material: 0.0,
            roll: [1.0, 0.0, 0.0, 0.0],
        });
    } else {
        push_coil_instances(instances, anchor, ball, outer, 0.62, 4.5, 12.0);
    }
}

fn push_coil_instances(
    instances: &mut Vec<Instance>,
    start: Vec2,
    end: Vec2,
    color: Vec4,
    alpha: f32,
    dot_radius: f32,
    wave_radius: f32,
) {
    let delta = end - start;
    let len = delta.length();
    if len <= 1.0 {
        return;
    }

    let dir = delta / len;
    let normal = Vec2::new(-dir.y, dir.x);
    let coils = (len / 34.0).round().clamp(6.0, 22.0);
    let segments = (coils as usize * 8).max(2);

    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let base = start + delta * t;
        let end_fade = (t * (1.0 - t) * 4.0).clamp(0.0, 1.0);
        let wave = (t * coils * std::f32::consts::TAU).sin() * wave_radius * end_fade;
        let pos = base + normal * wave;
        instances.push(Instance {
            center: pos.to_array(),
            half: [dot_radius, dot_radius],
            color: [color.x, color.y, color.z, alpha],
            softness: 0.7,
            material: 0.0,
            roll: [1.0, 0.0, 0.0, 0.0],
        });
    }
}

fn push_entangle_loop(
    instances: &mut Vec<Instance>,
    center: Vec2,
    radius: f32,
    inner: Vec4,
    outer: Vec4,
) {
    let loop_radius = radius.clamp(46.0, 96.0);
    for i in 0..28 {
        let t = i as f32 / 28.0;
        let angle = t * std::f32::consts::TAU;
        let pos = center + Vec2::new(angle.cos(), angle.sin()) * loop_radius;
        let color = if i % 2 == 0 { inner } else { outer };
        instances.push(Instance {
            center: pos.to_array(),
            half: [3.8, 3.8],
            color: [color.x, color.y, color.z, 0.58],
            softness: 0.72,
            material: 0.0,
            roll: [1.0, 0.0, 0.0, 0.0],
        });
    }
}
