//! Shared application state used by platform shells.

use std::time::Instant;

use fidget_sim::{Bounds, ParticleKind, World};
use glam::{Vec2, Vec4};

use crate::config::Settings;
use crate::renderer::Instance;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppAction {
    Reset,
    ToggleSpring,
    ToggleGravity,
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
            AppAction::ToggleGravity => self.world.toggle_gravity(),
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
        self.instances.clear();

        // Trail (drawn first, faint and soft).
        if cfg.trail_enabled {
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
            push_spring_instances(&mut self.instances, world);
        }

        // Particles.
        if cfg.particles_enabled {
            for p in world.particles.iter() {
                let lf = p.life_frac();
                let base = match p.kind {
                    ParticleKind::Spark => Vec4::new(1.0, 0.85, 0.5, 1.0),
                    ParticleKind::Burst => cfg.color_outer,
                    ParticleKind::Mote => cfg.color_inner,
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
        let ball = &world.ball;
        let s = ball.squash_scale(cfg.max_speed);
        let v = ball.vel;
        let roll_dir = if v.length_squared() > 1.0 {
            v.normalize()
        } else {
            ball.roll_dir
        };
        let r = ball.radius;
        let outer = cfg.color_outer;
        let c = ball.pos.to_array();
        let roll = [roll_dir.x, roll_dir.y, ball.roll_angle, 0.0];

        self.instances.push(Instance {
            center: c,
            half: [r * 2.4 * s.x, r * 2.4 * s.y],
            color: [outer.x, outer.y, outer.z, 0.08],
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
