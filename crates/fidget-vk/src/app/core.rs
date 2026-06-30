//! Shared application state used by platform shells.

use std::sync::OnceLock;
use std::time::Instant;

use fidget_sim::{BottomEdge, Bounds, MarbleWorld, ParticleKind, World, DEFAULT_RECALL_MARGIN};
use glam::{Vec2, Vec3, Vec4};

use crate::config::{PlayMode, Settings, ToySize};
use crate::renderer::{Instance, MarbleInstance, RubberBandMesh};

const SOCCER_GLOW_TEXTURE_PNG: &[u8] =
    include_bytes!("../../../../assets/soccer_ball_material.png");
static SOCCER_GLOW_TEXTURE: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
const INTRO_HINT_VISIBLE_SECS: f32 = 5.0;
const INTRO_HINT_FADE_SECS: f32 = 1.65;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppAction {
    Reset,
    ToggleSpring,
    ToggleSpringVisual,
    ToggleGravity,
    ToggleBottomBounce,
    ToggleSingleMonitorBounds,
    ToggleHud,
    Nudge,
    ToggleMode,
    SpawnMarble,
    ClearMarbles,
    ScatterMarbles,
    SetToySize(ToySize),
}

pub(super) struct Core {
    settings: Settings,
    world: World,
    marble_world: MarbleWorld,
    start: Instant,
    last_frame: Instant,
    virtual_bounds: Bounds,
    monitor_bounds: Vec<Bounds>,
    primary_monitor: usize,
    active_monitor: usize,
    monitor_layout_known: bool,
    cursor: Vec2,
    spring_interaction_active: bool,
    marble_kick_active: bool,
    instances: Vec<Instance>,
    marble_instances: Vec<MarbleInstance>,
    rubber_band: RubberBandMesh,
    egui_ctx: egui::Context,
    hud_visible: bool,
    hud_rect: Option<egui::Rect>,
    intro_hint: IntroHint,
    spawn_seed: u32,
    desktop_snapshot_requested: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct IntroHint {
    fade_started_at: Option<f32>,
    done: bool,
}

#[derive(Debug, Clone, Copy)]
struct IntroHintVisual {
    alpha: f32,
    fade: f32,
    time: f32,
}

impl Core {
    pub(super) fn new(settings: Settings) -> Self {
        let bounds = Bounds::new(0.0, 0.0, 1280.0, 720.0);
        let world = World::new(settings.world_config(), bounds);
        let marble_world = MarbleWorld::new(settings.marble_config(), bounds);
        let desktop_snapshot_requested = settings.mode == PlayMode::Marbles;
        Self {
            settings,
            world,
            marble_world,
            start: Instant::now(),
            last_frame: Instant::now(),
            virtual_bounds: bounds,
            monitor_bounds: vec![bounds],
            primary_monitor: 0,
            active_monitor: 0,
            monitor_layout_known: false,
            cursor: Vec2::ZERO,
            spring_interaction_active: false,
            marble_kick_active: false,
            instances: Vec::with_capacity(4096),
            marble_instances: Vec::with_capacity(512),
            rubber_band: RubberBandMesh::with_capacity(2048, 8192),
            egui_ctx: egui::Context::default(),
            hud_visible: false,
            hud_rect: None,
            intro_hint: IntroHint::default(),
            spawn_seed: 0xA53C_92D1,
            desktop_snapshot_requested,
        }
    }

    pub(super) fn egui_context(&self) -> egui::Context {
        self.egui_ctx.clone()
    }

    pub(super) fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub(super) fn marble_instances(&self) -> &[MarbleInstance] {
        &self.marble_instances
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
        match self.settings.mode {
            PlayMode::Fidget => self.world.advance(dt),
            PlayMode::Marbles => {
                self.ensure_marble_mode_seeded();
                self.marble_world.advance(dt);
            }
        }
        self.update_intro_hint();
    }

    pub(super) fn resize(&mut self, width: u32, height: u32) {
        self.virtual_bounds = Bounds::new(0.0, 0.0, width as f32, height as f32);
        if self.settings.mode == PlayMode::Marbles {
            self.desktop_snapshot_requested = true;
        }
        if !self.monitor_layout_known {
            self.monitor_bounds.clear();
            self.monitor_bounds.push(self.virtual_bounds);
            self.primary_monitor = 0;
            self.active_monitor = 0;
        }
        self.apply_monitor_bounds(false);
    }

    pub(super) fn set_monitor_layout<I>(&mut self, monitors: I, primary_monitor: usize)
    where
        I: IntoIterator<Item = Bounds>,
    {
        let had_layout = self.monitor_layout_known;
        self.monitor_bounds = monitors
            .into_iter()
            .filter(|bounds| bounds.width() > 1.0 && bounds.height() > 1.0)
            .collect();
        if self.monitor_bounds.is_empty() {
            self.monitor_bounds.push(self.virtual_bounds);
        }
        self.primary_monitor = primary_monitor.min(self.monitor_bounds.len() - 1);
        if self.settings.sim.single_monitor_bounds && !had_layout {
            self.active_monitor = self.primary_monitor;
        } else {
            self.active_monitor = self.active_monitor.min(self.monitor_bounds.len() - 1);
        }
        self.monitor_layout_known = true;
        if self.settings.mode == PlayMode::Marbles {
            self.desktop_snapshot_requested = true;
        }
        self.apply_monitor_bounds(self.settings.sim.single_monitor_bounds && !had_layout);
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
    pub(super) fn toy_size(&self) -> ToySize {
        self.settings.sim.toy_size
    }

    #[cfg(target_os = "windows")]
    pub(super) fn rubber_band_visual_enabled(&self) -> bool {
        self.settings.visuals.spring_visual.is_rubber_band()
    }

    #[cfg(target_os = "windows")]
    pub(super) fn bottom_bounce_enabled(&self) -> bool {
        self.world.bottom_bounce_enabled()
    }

    #[cfg(target_os = "windows")]
    pub(super) fn single_monitor_bounds_enabled(&self) -> bool {
        self.settings.sim.single_monitor_bounds
    }

    pub(super) fn play_mode(&self) -> PlayMode {
        self.settings.mode
    }

    pub(super) fn take_desktop_snapshot_request(&mut self) -> bool {
        std::mem::take(&mut self.desktop_snapshot_requested)
    }

    #[cfg(target_os = "windows")]
    pub(super) fn marble_count(&self) -> usize {
        self.marble_world.marbles.len()
    }

    pub(super) fn on_cursor_moved(&mut self, cursor: Vec2) {
        self.cursor = cursor;
        let now = self.now();
        if self.settings.mode == PlayMode::Marbles {
            if self.marble_kick_active && !self.marble_world.is_grabbed() {
                self.marble_world.kick_cursor(cursor, now);
            } else {
                self.marble_world.move_cursor(cursor, now);
            }
            return;
        }
        if self.spring_interaction_active {
            self.world.interact_spring(cursor, now);
            if self.spring_is_cursor_dragged() {
                self.update_single_monitor_from_cursor(cursor);
            }
        } else {
            self.world.move_cursor(cursor, now);
        }
    }

    pub(super) fn on_left_pressed(&mut self) -> bool {
        let now = self.now();
        if self.settings.mode == PlayMode::Marbles {
            return self.marble_world.grab(self.cursor, now);
        }
        let grabbed = self.world.grab(self.cursor, now);
        if grabbed {
            self.start_intro_hint_fade(now);
        }
        grabbed
    }

    pub(super) fn on_left_released(&mut self) {
        let now = self.now();
        if self.settings.mode == PlayMode::Marbles {
            self.marble_world.release(now);
            return;
        }
        self.world.release(now);
    }

    pub(super) fn on_right_pressed(&mut self) {
        if self.settings.mode == PlayMode::Marbles {
            let now = self.now();
            self.marble_kick_active = true;
            self.marble_world.begin_kick(self.cursor, now);
            return;
        }
        self.spring_interaction_active = true;
        let now = self.now();
        self.start_intro_hint_fade(now);
        self.world.interact_spring(self.cursor, now);
    }

    pub(super) fn on_right_released(&mut self) {
        if self.settings.mode == PlayMode::Marbles {
            self.marble_kick_active = false;
            return;
        }
        if self.spring_interaction_active {
            self.spring_interaction_active = false;
            self.world.stop_spring_interaction();
        }
    }

    pub(super) fn apply_action(&mut self, action: AppAction) {
        match action {
            AppAction::Reset => {
                self.start_intro_hint_fade(self.now());
                match self.settings.mode {
                    PlayMode::Fidget => self.spawn_ball_random(),
                    PlayMode::Marbles => self.spawn_marble_random(),
                }
            }
            AppAction::ToggleSpring => {
                if self.settings.mode == PlayMode::Marbles {
                    return;
                }
                self.start_intro_hint_fade(self.now());
                if self.world.ball_visible() {
                    self.world.toggle_spring();
                } else {
                    self.spawn_ball_random();
                }
            }
            AppAction::ToggleSpringVisual => {
                self.settings.visuals.spring_visual = self.settings.visuals.spring_visual.toggled();
            }
            AppAction::ToggleGravity => {
                if self.settings.mode == PlayMode::Fidget {
                    self.world.toggle_gravity();
                }
            }
            AppAction::ToggleBottomBounce => {
                let enabled = !self.world.bottom_bounce_enabled();
                self.world.set_bottom_bounce_enabled(enabled);
                self.settings.sim.bounce_bottom_edge = enabled;
                self.apply_pit_recall_margin();
            }
            AppAction::ToggleSingleMonitorBounds => {
                let enabled = !self.settings.sim.single_monitor_bounds;
                self.settings.sim.single_monitor_bounds = enabled;
                if enabled {
                    self.active_monitor = self.primary_monitor;
                }
                self.apply_monitor_bounds(enabled);
            }
            AppAction::ToggleHud => self.hud_visible = !self.hud_visible,
            AppAction::Nudge => {
                self.start_intro_hint_fade(self.now());
                match self.settings.mode {
                    PlayMode::Fidget => self.world.nudge(2800.0),
                    PlayMode::Marbles => self.marble_world.scatter(2500.0),
                }
            }
            AppAction::ToggleMode => self.toggle_mode(),
            AppAction::SpawnMarble => {
                self.settings.mode = PlayMode::Marbles;
                self.spawn_marble_random();
            }
            AppAction::ClearMarbles => self.marble_world.clear(),
            AppAction::ScatterMarbles => self.marble_world.scatter(2500.0),
            AppAction::SetToySize(size) => self.apply_toy_size(size),
        }
    }

    pub(super) fn show_hud(&mut self, ctx: &egui::Context) {
        self.show_intro_hints(ctx);
        if !self.hud_visible {
            self.hud_rect = None;
            return;
        }

        let window_response = egui::Window::new("Fidget controls")
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

                ui.horizontal(|ui| {
                    ui.label("mode");
                    if ui
                        .selectable_label(self.settings.mode == PlayMode::Fidget, "Fidget")
                        .clicked()
                    {
                        self.settings.mode = PlayMode::Fidget;
                    }
                    if ui
                        .selectable_label(self.settings.mode == PlayMode::Marbles, "Marbles")
                        .clicked()
                    {
                        self.settings.mode = PlayMode::Marbles;
                        self.ensure_marble_mode_seeded();
                        self.desktop_snapshot_requested = true;
                    }
                });

                if self.settings.mode == PlayMode::Marbles {
                    ui.horizontal(|ui| {
                        if ui.button("Spawn marble").clicked() {
                            self.spawn_marble_random();
                        }
                        if ui.button("Scatter").clicked() {
                            self.marble_world.scatter(2500.0);
                        }
                        if ui.button("Clear").clicked() {
                            self.marble_world.clear();
                        }
                    });
                    ui.label(format!("marbles: {}", self.marble_world.marbles.len()));
                    return;
                }

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
                        egui::Slider::new(&mut stiffness, 15.0..=420.0).text("string elasticity"),
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

                ui.horizontal(|ui| {
                    ui.label("size");
                    for (label, size) in [
                        ("S", ToySize::Small),
                        ("M", ToySize::Medium),
                        ("L", ToySize::Large),
                    ] {
                        if ui
                            .selectable_label(self.settings.sim.toy_size == size, label)
                            .clicked()
                        {
                            self.apply_toy_size(size);
                        }
                    }
                });

                let old_thickness = self.settings.visuals.rubber_band_thickness;
                let mut thickness = old_thickness;
                ui.horizontal(|ui| {
                    if ui.button("thinner").clicked() {
                        thickness -= 0.08;
                    }
                    ui.add(egui::Slider::new(&mut thickness, 0.4..=1.25).text("rubber thickness"));
                    if ui.button("fuller").clicked() {
                        thickness += 0.08;
                    }
                });
                self.settings.visuals.rubber_band_thickness = thickness.clamp(0.4, 1.25);

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
                    if ui.button("Spawn ball").clicked() {
                        self.start_intro_hint_fade(self.now());
                        self.spawn_ball_random();
                    }
                    if ui.button("Cut/recall").clicked() {
                        self.start_intro_hint_fade(self.now());
                        self.world.toggle_spring();
                    }
                });
            });

        self.hud_rect = window_response.map(|response| response.response.rect.expand(8.0));
    }

    fn apply_monitor_bounds(&mut self, reset_ball: bool) {
        let bounds = self.active_sim_bounds();
        self.world.set_bounds(bounds);
        self.marble_world
            .set_visible_bounds(bounds, self.marble_visible_bounds(bounds));
        if self.settings.sim.single_monitor_bounds {
            self.world
                .set_bottom_edges([BottomEdge::from_bounds(bounds)]);
        } else {
            self.world.set_bottom_edges(BottomEdge::exposed_from_bounds(
                &self.monitor_bounds,
                self.virtual_bounds,
            ));
        }
        if reset_ball {
            self.world.reset();
        }
        self.apply_pit_recall_margin();
    }

    fn active_sim_bounds(&self) -> Bounds {
        if self.settings.sim.single_monitor_bounds {
            self.monitor_bounds
                .get(self.active_monitor)
                .copied()
                .unwrap_or(self.virtual_bounds)
        } else {
            self.virtual_bounds
        }
    }

    fn marble_visible_bounds(&self, bounds: Bounds) -> Vec<Bounds> {
        if self.settings.sim.single_monitor_bounds {
            return vec![bounds];
        }

        let monitors: Vec<_> = self
            .monitor_bounds
            .iter()
            .copied()
            .filter(|bounds| bounds.width() > 1.0 && bounds.height() > 1.0)
            .collect();
        if monitors.is_empty() {
            vec![bounds]
        } else {
            monitors
        }
    }

    fn update_single_monitor_from_cursor(&mut self, cursor: Vec2) {
        if !self.settings.sim.single_monitor_bounds {
            return;
        }
        let Some(index) = self.monitor_index_at(cursor) else {
            return;
        };
        if index != self.active_monitor {
            self.active_monitor = index;
            self.apply_monitor_bounds(false);
        }
    }

    fn apply_pit_recall_margin(&mut self) {
        let margin =
            if self.settings.sim.single_monitor_bounds && !self.world.bottom_bounce_enabled() {
                0.0
            } else {
                DEFAULT_RECALL_MARGIN
            };
        self.world.set_recall_margin(margin);
    }

    fn apply_toy_size(&mut self, size: ToySize) {
        if self.settings.sim.toy_size == size {
            return;
        }
        self.settings.sim.toy_size = size;
        self.marble_world
            .set_radius_range(ToySize::Small.ball_radius(), ToySize::Large.ball_radius());
        self.world.set_size(
            size.ball_radius(),
            size.interaction_scale(),
            size.length_scale(),
        );
    }

    fn toggle_mode(&mut self) {
        self.settings.mode = match self.settings.mode {
            PlayMode::Fidget => PlayMode::Marbles,
            PlayMode::Marbles => PlayMode::Fidget,
        };
        self.spring_interaction_active = false;
        self.marble_kick_active = false;
        self.world.stop_spring_interaction();
        if self.settings.mode == PlayMode::Marbles {
            self.ensure_marble_mode_seeded();
            self.desktop_snapshot_requested = true;
        }
    }

    fn ensure_marble_mode_seeded(&mut self) {
        if self.marble_world.marbles.is_empty() {
            self.spawn_marble_random();
        }
    }

    fn spawn_marble_random(&mut self) {
        let bounds = self.random_spawn_bounds();
        self.marble_world.set_visible_bounds(bounds, [bounds]);
        self.marble_world.spawn_random();
        let active_bounds = self.active_sim_bounds();
        self.marble_world
            .set_visible_bounds(active_bounds, self.marble_visible_bounds(active_bounds));
        self.desktop_snapshot_requested = true;
    }

    fn spawn_ball_random(&mut self) {
        let bounds = self.random_spawn_bounds();
        let radius = self.world.ball.radius;
        let margin = (radius * 2.8).clamp(72.0, 180.0);
        let min_x = bounds.left + margin;
        let max_x = bounds.right - margin;
        let min_y = bounds.top + margin;
        let max_y = bounds.bottom - margin;
        let pos = Vec2::new(
            if min_x < max_x {
                min_x + (max_x - min_x) * self.next_spawn_rand()
            } else {
                bounds.center().x
            },
            if min_y < max_y {
                min_y + (max_y - min_y) * self.next_spawn_rand()
            } else {
                bounds.center().y
            },
        );
        self.world.spawn_attached_at(pos);
    }

    fn random_spawn_bounds(&mut self) -> Bounds {
        if self.settings.sim.single_monitor_bounds {
            return self.active_sim_bounds();
        }

        let monitors: Vec<Bounds> = self
            .monitor_bounds
            .iter()
            .copied()
            .filter(|bounds| bounds.width() > 1.0 && bounds.height() > 1.0)
            .collect();
        if monitors.is_empty() {
            return self.virtual_bounds;
        }

        let index = ((self.next_spawn_rand() * monitors.len() as f32) as usize)
            .min(monitors.len().saturating_sub(1));
        monitors[index]
    }

    fn next_spawn_rand(&mut self) -> f32 {
        let mut x = self.spawn_seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.spawn_seed = x;
        (x as f32 / u32::MAX as f32).clamp(0.0, 1.0)
    }

    fn spring_is_cursor_dragged(&self) -> bool {
        self.world.spring.attached
            && (self.world.spring.entanglement.is_some()
                || self
                    .world
                    .spring
                    .intersection
                    .is_some_and(|intersection| !intersection.max_age.is_finite()))
    }

    fn monitor_index_at(&self, point: Vec2) -> Option<usize> {
        self.monitor_bounds
            .iter()
            .position(|bounds| bounds_contains(*bounds, point))
    }

    fn update_intro_hint(&mut self) {
        if self.settings.mode != PlayMode::Fidget {
            return;
        }
        if self.intro_hint.done {
            return;
        }
        let now = self.now();
        if self.intro_hint.fade_started_at.is_none() && now >= INTRO_HINT_VISIBLE_SECS {
            self.intro_hint.fade_started_at = Some(now);
        }
        if self
            .intro_hint
            .fade_started_at
            .is_some_and(|started| now - started >= INTRO_HINT_FADE_SECS)
        {
            self.intro_hint.done = true;
        }
    }

    fn start_intro_hint_fade(&mut self, now: f32) {
        if self.settings.mode != PlayMode::Fidget {
            return;
        }
        if !self.intro_hint.done && self.intro_hint.fade_started_at.is_none() {
            self.intro_hint.fade_started_at = Some(now);
        }
    }

    fn intro_hint_visual(&self) -> Option<IntroHintVisual> {
        if self.settings.mode != PlayMode::Fidget {
            return None;
        }
        if self.intro_hint.done {
            return None;
        }
        let time = self.now();
        let fade = self
            .intro_hint
            .fade_started_at
            .map(|started| ((time - started) / INTRO_HINT_FADE_SECS).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        if fade >= 1.0 {
            return None;
        }
        let eased = fade * fade * (3.0 - 2.0 * fade);
        Some(IntroHintVisual {
            alpha: (1.0 - eased).powf(1.25),
            fade: eased,
            time,
        })
    }

    fn show_intro_hints(&self, ctx: &egui::Context) {
        if self.settings.mode != PlayMode::Fidget {
            return;
        }
        let Some(visual) = self.intro_hint_visual() else {
            return;
        };
        if !self.world.ball_visible() {
            return;
        }
        ctx.request_repaint();

        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("intro_action_hints"),
        ));
        let ball = &self.world.ball;
        let bounds = self.world.bounds;
        let label_y =
            (ball.pos.y + ball.radius + 30.0).clamp(bounds.top + 26.0, bounds.bottom - 38.0);
        let x_guard = (bounds.width() * 0.38).clamp(120.0, 240.0);
        let left_label = Vec2::new(
            (ball.pos.x - 12.0).clamp(bounds.left + x_guard, bounds.right - 16.0),
            label_y,
        );
        let right_label = Vec2::new(
            (ball.pos.x + 12.0).clamp(bounds.left + 16.0, bounds.right - x_guard),
            label_y,
        );

        if self.world.spring.attached {
            let (start, end) = spring_hint_segment(&self.world);
            let delta = end - start;
            if delta.length_squared() > 1.0 {
                let normal = Vec2::new(-delta.y, delta.x).normalize_or_zero();
                let center = start + delta * 0.43;
                let half_len = (self.world.ball.radius * 2.1).clamp(58.0, 110.0);
                draw_dashed_hint_line(
                    &painter,
                    center - normal * half_len,
                    center + normal * half_len,
                    visual,
                );
                let cut_pos = center + normal * (half_len + 18.0);
                draw_hint_text(
                    &painter,
                    "cut",
                    cut_pos,
                    egui::Align2::CENTER_CENTER,
                    17.0,
                    visual,
                    3.0,
                );
            }
        }

        draw_hint_text(
            &painter,
            "left click mouse = holds the ball",
            left_label,
            egui::Align2::RIGHT_TOP,
            15.0,
            visual,
            11.0,
        );
        draw_hint_text(
            &painter,
            "right click = kick the ball",
            right_label,
            egui::Align2::LEFT_TOP,
            15.0,
            visual,
            17.0,
        );
    }

    #[cfg(target_os = "windows")]
    pub(super) fn hit_test_interactive(&self, cursor: Vec2) -> bool {
        // The Win32 shell asks this during `WM_NCHITTEST`; returning false lets
        // normal desktop windows receive clicks through transparent overlay space.
        if self
            .hud_rect
            .is_some_and(|rect| rect.contains(egui::pos2(cursor.x, cursor.y)))
        {
            return true;
        }
        if self.settings.mode == PlayMode::Marbles {
            return self.marble_world.is_grabbed() || self.marble_world.hit_test(cursor);
        }
        if self.world.is_grabbed() {
            return true;
        }
        let ball = &self.world.ball;
        if !self.world.ball_visible() {
            return false;
        }
        if cursor.distance(ball.pos) <= ball.radius + 12.0 {
            return true;
        }
        false
    }

    pub(super) fn build_instances(&mut self) {
        self.instances.clear();
        self.marble_instances.clear();
        self.rubber_band.clear();
        if self.settings.mode == PlayMode::Marbles {
            self.build_marble_instances();
            return;
        }

        let world = &self.world;
        let cfg = &world.config;
        let rubber_visual = self.settings.visuals.spring_visual.is_rubber_band();

        if !world.ball_visible() {
            return;
        }

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
                rebuild_rubber_band(
                    &mut self.rubber_band,
                    world,
                    self.settings.visuals.rubber_band_thickness
                        * self.settings.sim.toy_size.band_scale(),
                );
            } else {
                push_spring_instances(
                    &mut self.instances,
                    world,
                    self.settings.sim.toy_size.interaction_scale(),
                );
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
                    ParticleKind::Shard => Vec4::new(0.82, 0.96, 1.0, 1.0),
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
        if let Some(hint) = self.intro_hint_visual() {
            push_intro_hint_dust_instances(&mut self.instances, world, hint);
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

    fn build_marble_instances(&mut self) {
        for marble in &self.marble_world.marbles {
            let health = (marble.health / marble.max_health).clamp(0.0, 1.0);
            let crack = marble.crack.clamp(0.0, 1.0);
            let shine_alpha = (0.095 + (marble.radius / ToySize::Large.ball_radius()) * 0.030)
                * (0.82 + health * 0.18)
                * (1.0 - crack * 0.22);
            self.instances.push(Instance {
                center: [
                    marble.pos.x,
                    marble.pos.y + marble.radius * (0.36 + crack * 0.10),
                ],
                half: [marble.radius * 1.22, marble.radius * 0.32],
                color: [1.0, 0.98, 0.90, shine_alpha],
                softness: 0.94,
                material: 0.0,
                roll: [1.0, 0.0, 0.0, 0.0],
            });
            let caustic_alpha = (0.105 + (marble.radius / ToySize::Large.ball_radius()) * 0.045)
                * (0.82 + health * 0.18)
                * (1.0 - crack * 0.18);
            let caustic_seed = (marble.pattern.seed % 10_000) as f32 * 0.01;
            self.instances.push(Instance {
                center: [
                    marble.pos.x + marble.radius * 0.05,
                    marble.pos.y + marble.radius * (0.50 + crack * 0.08),
                ],
                half: [marble.radius * 1.88, marble.radius * 0.58],
                color: [1.0, 1.0, 1.0, caustic_alpha],
                softness: 0.9,
                material: 3.0,
                roll: [0.94, 0.34, caustic_seed, marble.roll_angle],
            });
        }

        if self.settings.visuals.particles {
            for p in self.marble_world.particles.iter() {
                let lf = p.life_frac();
                let base = match p.kind {
                    ParticleKind::Spark => Vec4::new(1.0, 0.85, 0.5, 1.0),
                    ParticleKind::Burst => p.color,
                    ParticleKind::Mote => p.color,
                    ParticleKind::Shard => p.color.lerp(Vec4::ONE, 0.24),
                };
                self.instances.push(Instance {
                    center: p.pos.to_array(),
                    half: [p.size, p.size],
                    color: [base.x, base.y, base.z, lf * base.w * 0.92],
                    softness: if p.kind == ParticleKind::Shard {
                        0.34
                    } else {
                        0.82
                    },
                    material: if p.kind == ParticleKind::Shard {
                        2.0
                    } else {
                        0.0
                    },
                    roll: [1.0, 0.0, self.now() + p.pos.x * 0.01, p.pos.y * 0.01],
                });
            }
        }

        for marble in &self.marble_world.marbles {
            let speed = marble.speed();
            let roll_dir = if speed > 1.0 {
                marble.vel / speed
            } else {
                marble.roll_dir
            };
            let pattern = marble.pattern;
            let crack = marble.crack.clamp(0.0, 1.0);

            self.marble_instances.push(MarbleInstance {
                center: marble.pos.to_array(),
                radius: marble.radius,
                health: (marble.health / marble.max_health).clamp(0.0, 1.0),
                crack,
                seed: pattern.seed,
                roll: [roll_dir.x, roll_dir.y, marble.roll_angle, 0.0],
                primary: pattern.primary.to_array(),
                secondary: pattern.secondary.to_array(),
                accent: pattern.accent.to_array(),
                ribbons: pattern.ribbons.to_array(),
                glass: pattern.glass.to_array(),
            });
        }
    }
}

fn spring_hint_segment(world: &World) -> (Vec2, Vec2) {
    let start = world.spring.anchor;
    let end = world.ball.pos;
    if start.y >= world.bounds.top || (end.y - start.y).abs() <= 1.0 {
        return (start, end);
    }
    let t = ((world.bounds.top - start.y) / (end.y - start.y)).clamp(0.0, 1.0);
    (start.lerp(end, t), end)
}

fn draw_dashed_hint_line(painter: &egui::Painter, start: Vec2, end: Vec2, visual: IntroHintVisual) {
    let delta = end - start;
    let len = delta.length();
    if len <= 2.0 {
        return;
    }

    let dir = delta / len;
    let dash = 18.0;
    let gap = 10.0;
    let mut cursor = 0.0;
    let mut index = 0.0;
    while cursor < len {
        let next = (cursor + dash).min(len);
        let offset = dust_wind_offset(index + 5.0, visual, 30.0);
        let a = start + dir * cursor + offset;
        let b = start + dir * next + offset;
        let alpha = visual.alpha * (1.0 - visual.fade * 0.35);
        let stroke = egui::Stroke::new(2.0 - visual.fade * 0.7, hint_color(alpha));
        painter.line_segment([pos2(a), pos2(b)], stroke);
        cursor += dash + gap;
        index += 1.0;
    }
}

fn draw_hint_text(
    painter: &egui::Painter,
    text: &str,
    pos: Vec2,
    align: egui::Align2,
    size: f32,
    visual: IntroHintVisual,
    seed: f32,
) {
    let font = egui::FontId::proportional(size);
    let shadow = Vec2::new(0.0, 1.5);
    painter.text(
        pos2(pos + shadow),
        align,
        text,
        font.clone(),
        egui::Color32::from_rgba_unmultiplied(18, 15, 10, (110.0 * visual.alpha) as u8),
    );

    if visual.fade > 0.01 {
        for i in 0..7 {
            let n = seed + i as f32 * 2.71;
            let offset = dust_wind_offset(n, visual, 42.0 + i as f32 * 3.0);
            let alpha = visual.alpha * visual.fade * (0.18 - i as f32 * 0.016).max(0.04);
            painter.text(
                pos2(pos + offset),
                align,
                text,
                font.clone(),
                hint_color(alpha),
            );
        }
    }

    painter.text(pos2(pos), align, text, font, hint_color(visual.alpha));
}

fn push_intro_hint_dust_instances(
    instances: &mut Vec<Instance>,
    world: &World,
    visual: IntroHintVisual,
) {
    if visual.fade <= 0.01 {
        return;
    }

    let ball = &world.ball;
    let label_y = (ball.pos.y + ball.radius + 42.0)
        .clamp(world.bounds.top + 30.0, world.bounds.bottom - 34.0);
    let left_center = Vec2::new(ball.pos.x - ball.radius * 1.9, label_y + 7.0);
    let right_center = Vec2::new(ball.pos.x + ball.radius * 1.75, label_y + 7.0);
    push_dust_cloud(
        instances,
        left_center,
        Vec2::new(155.0, 17.0),
        46,
        visual,
        23.0,
    );
    push_dust_cloud(
        instances,
        right_center,
        Vec2::new(128.0, 17.0),
        40,
        visual,
        47.0,
    );

    if world.spring.attached {
        let (start, end) = spring_hint_segment(world);
        let center = start.lerp(end, 0.43);
        let extent = Vec2::new(start.distance(end).min(180.0) * 0.45, 20.0);
        push_dust_cloud(instances, center, extent, 34, visual, 71.0);
    }
}

fn push_dust_cloud(
    instances: &mut Vec<Instance>,
    center: Vec2,
    half: Vec2,
    count: usize,
    visual: IntroHintVisual,
    seed: f32,
) {
    let fade = visual.fade;
    for i in 0..count {
        let id = seed + i as f32;
        let local = Vec2::new(hash_signed(id * 3.1), hash_signed(id * 5.7));
        let wind = Vec2::new(72.0, -26.0) * fade.powf(1.15);
        let flutter = Vec2::new(
            (visual.time * (1.8 + hash01(id) * 2.5) + id).sin(),
            (visual.time * (1.4 + hash01(id + 9.0) * 2.1) + id * 0.7).cos(),
        ) * (8.0 + hash01(id + 13.0) * 18.0)
            * fade;
        let pos = center + local * half + wind * (0.35 + hash01(id + 4.0) * 0.9) + flutter;
        let size = 1.2 + hash01(id + 19.0) * 4.8;
        let alpha = visual.alpha * fade * (0.12 + hash01(id + 29.0) * 0.22);
        instances.push(Instance {
            center: pos.to_array(),
            half: [size, size],
            color: [0.94, 0.88, 0.74, alpha],
            softness: 0.82,
            material: 2.0,
            roll: [1.0, 0.0, visual.time + id * 0.13, id],
        });
    }
}

fn dust_wind_offset(seed: f32, visual: IntroHintVisual, strength: f32) -> Vec2 {
    if visual.fade <= 0.0 {
        return Vec2::ZERO;
    }
    let wind = Vec2::new(strength, -strength * 0.36) * visual.fade.powf(1.18);
    let flutter = Vec2::new(
        (visual.time * 2.1 + seed * 1.7).sin(),
        (visual.time * 1.7 + seed * 2.3).cos(),
    ) * (visual.fade * strength * 0.18);
    wind * (0.45 + hash01(seed) * 0.8) + flutter
}

fn hint_color(alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(244, 230, 196, (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

fn pos2(v: Vec2) -> egui::Pos2 {
    egui::pos2(v.x, v.y)
}

fn bounds_contains(bounds: Bounds, point: Vec2) -> bool {
    point.x >= bounds.left
        && point.x <= bounds.right
        && point.y >= bounds.top
        && point.y <= bounds.bottom
}

fn hash01(seed: f32) -> f32 {
    let x = (seed * 12.9898 + 78.233).sin() * 43_758.547;
    x - x.floor()
}

fn hash_signed(seed: f32) -> f32 {
    hash01(seed) * 2.0 - 1.0
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

fn rebuild_rubber_band(mesh: &mut RubberBandMesh, world: &World, thickness: f32) {
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
    let thickness = thickness.clamp(0.12, 1.25);
    let radius = (8.8 * thickness / stretch.sqrt()).clamp(1.1, 10.5);
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

fn push_spring_instances(instances: &mut Vec<Instance>, world: &World, scale: f32) {
    let scale = scale.clamp(0.45, 1.25);
    let anchor = world.spring.anchor;
    let ball = world.ball.pos;
    let outer = world.config.color_outer;
    let inner = world.config.color_inner;

    instances.push(Instance {
        center: anchor.to_array(),
        half: [8.0 * scale, 8.0 * scale],
        color: [inner.x, inner.y, inner.z, 0.85],
        softness: 0.75,
        material: 0.0,
        roll: [1.0, 0.0, 0.0, 0.0],
    });

    if let Some(entanglement) = world.spring.entanglement {
        push_coil_instances(
            instances,
            anchor,
            entanglement.center,
            outer,
            0.5,
            3.8 * scale,
            9.0 * scale,
        );
        push_coil_instances(
            instances,
            entanglement.center,
            ball,
            outer,
            0.72,
            4.8 * scale,
            13.0 * scale,
        );
        push_entangle_loop(
            instances,
            entanglement.center,
            entanglement.radius,
            inner,
            outer,
            scale,
        );
    } else if let Some(intersection) = world.spring.intersection {
        push_coil_instances(
            instances,
            anchor,
            intersection.point,
            outer,
            0.54,
            4.2 * scale,
            10.0 * scale,
        );
        push_coil_instances(
            instances,
            intersection.point,
            ball,
            outer,
            0.68,
            4.8 * scale,
            13.0 * scale,
        );
        instances.push(Instance {
            center: intersection.point.to_array(),
            half: [13.0 * scale, 13.0 * scale],
            color: [inner.x, inner.y, inner.z, 0.34 * intersection.strength()],
            softness: 0.9,
            material: 0.0,
            roll: [1.0, 0.0, 0.0, 0.0],
        });
    } else {
        push_coil_instances(
            instances,
            anchor,
            ball,
            outer,
            0.62,
            4.5 * scale,
            12.0 * scale,
        );
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
    scale: f32,
) {
    let loop_radius = radius.clamp(46.0, 96.0);
    for i in 0..28 {
        let t = i as f32 / 28.0;
        let angle = t * std::f32::consts::TAU;
        let pos = center + Vec2::new(angle.cos(), angle.sin()) * loop_radius;
        let color = if i % 2 == 0 { inner } else { outer };
        instances.push(Instance {
            center: pos.to_array(),
            half: [3.8 * scale, 3.8 * scale],
            color: [color.x, color.y, color.z, 0.58],
            softness: 0.72,
            material: 0.0,
            roll: [1.0, 0.0, 0.0, 0.0],
        });
    }
}
