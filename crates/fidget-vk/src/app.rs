//! Glue between winit (window + input), the simulation, and the Vulkan
//! renderer. Owns the fixed-timestep loop and translates the simulation state
//! into renderer instances each frame.

use std::time::Instant;

use anyhow::Result;
use glam::{Vec2, Vec4};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
#[cfg(target_os = "linux")]
use winit::platform::x11::{WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

use fidget_sim::{Bounds, ParticleKind, World};

use crate::config::Settings;
use crate::renderer::{EguiDrawData, Instance, Renderer, RubberBandMesh};

pub struct App {
    settings: Settings,
    world: World,
    // `renderer` is declared before `window` so it (and its Vulkan surface) is
    // dropped before the window it was created from.
    renderer: Option<Renderer>,
    window: Option<Window>,
    start: Instant,
    last_frame: Instant,
    cursor: Vec2,
    instances: Vec<Instance>,
    rubber_band: RubberBandMesh,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    hud_visible: bool,
}

#[derive(Debug, Clone, Copy)]
struct OverlayGeometry {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

impl App {
    pub fn new(settings: Settings) -> Self {
        let bounds = Bounds::new(0.0, 0.0, 1280.0, 720.0);
        let world = World::new(settings.world_config(), bounds);
        Self {
            settings,
            world,
            renderer: None,
            window: None,
            start: Instant::now(),
            last_frame: Instant::now(),
            cursor: Vec2::ZERO,
            instances: Vec::with_capacity(4096),
            rubber_band: RubberBandMesh::with_capacity(2048, 8192),
            egui_ctx: egui::Context::default(),
            egui_state: None,
            hud_visible: true,
        }
    }

    fn now(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    fn build_instances(&mut self) {
        let world = &self.world;
        let cfg = &world.config;
        self.instances.clear();
        self.rubber_band.clear();

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
            rebuild_rubber_band(&mut self.rubber_band, world);
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

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let dt = {
            let now = Instant::now();
            let dt = (now - self.last_frame).as_secs_f32().min(0.1);
            self.last_frame = now;
            dt
        };
        self.world.advance(dt);

        let egui_output = {
            let raw_input = {
                let window = self.window.as_ref().expect("window exists while redrawing");
                self.egui_state
                    .as_mut()
                    .expect("egui state exists while redrawing")
                    .take_egui_input(window)
            };
            let ctx = self.egui_ctx.clone();
            let full_output = ctx.run(raw_input, |ctx| self.show_hud(ctx));
            {
                let window = self.window.as_ref().expect("window exists while redrawing");
                self.egui_state
                    .as_mut()
                    .expect("egui state exists while redrawing")
                    .handle_platform_output(window, full_output.platform_output);
            }
            let pixels_per_point = ctx.pixels_per_point();
            let clipped_primitives = ctx.tessellate(full_output.shapes, pixels_per_point);
            Some((
                full_output.textures_delta,
                clipped_primitives,
                pixels_per_point,
            ))
        };

        self.build_instances();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let egui_draw =
            egui_output
                .as_ref()
                .map(
                    |(textures_delta, clipped_primitives, pixels_per_point)| EguiDrawData {
                        textures_delta,
                        clipped_primitives,
                        pixels_per_point: *pixels_per_point,
                    },
                );
        match renderer.render(&self.instances, &self.rubber_band, egui_draw) {
            Ok(true) => {}
            Ok(false) => {
                // Swapchain out of date: recreate at the current window size.
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    if let Err(e) = renderer.resize((size.width, size.height)) {
                        log::error!("swapchain recreation failed: {e}");
                        event_loop.exit();
                    }
                    self.world.set_bounds(Bounds::new(
                        0.0,
                        0.0,
                        size.width as f32,
                        size.height as f32,
                    ));
                }
            }
            Err(e) => {
                log::error!("render error: {e}");
                event_loop.exit();
            }
        }
    }

    fn show_hud(&mut self, ctx: &egui::Context) {
        if !self.hud_visible {
            egui::Window::new("HUD")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .fixed_pos(egui::pos2(18.0, 18.0))
                .show(ctx, |ui| {
                    if ui.button("Show Fidget controls").clicked() {
                        self.hud_visible = true;
                    }
                });
            return;
        }

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
                        egui::Slider::new(&mut stiffness, 15.0..=420.0).text("rubber elasticity"),
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
                    ui.add(egui::Slider::new(&mut damping, 2.0..=90.0).text("rubber damping"));
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
                ui.small("Negative hook offset places the rubber-band hook above the desktop.");

                ui.horizontal(|ui| {
                    if ui.button("Reset ball").clicked() {
                        self.world.reset();
                    }
                    if ui.button("Cut/recall").clicked() {
                        self.world.toggle_spring();
                    }
                });
            });
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let overlay = overlay_geometry(event_loop);
        let attrs = Window::default_attributes()
            .with_title("Fidget-VK")
            .with_position(overlay.position)
            .with_inner_size(overlay.size)
            .with_transparent(true)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        #[cfg(target_os = "linux")]
        let attrs = attrs
            .with_override_redirect(true)
            .with_x11_window_type(vec![WindowType::Dock]);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                log::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };
        window.set_outer_position(overlay.position);
        let _ = window.request_inner_size(overlay.size);

        let size = window.inner_size();
        let display = window.display_handle().unwrap().as_raw();
        let win = window.window_handle().unwrap().as_raw();
        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );

        match Renderer::new(display, win, (size.width, size.height)) {
            Ok(renderer) => {
                self.world
                    .set_bounds(Bounds::new(0.0, 0.0, size.width as f32, size.height as f32));
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.egui_state = Some(egui_state);
                self.last_frame = Instant::now();
                log::info!(
                    "transparent overlay geometry: pos=({}, {}) size={}x{}",
                    overlay.position.x,
                    overlay.position.y,
                    size.width,
                    size.height
                );
                log::info!(
                    "Fidget-VK is running. Drag the ball; brush/sweep the rubber band to displace or entangle it; right-click or C=cut/recall rubber band, N=fling, R=reset, G=gravity, Esc=quit"
                );
            }
            Err(e) => {
                log::error!("failed to initialise renderer: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let h_pressed = matches!(
            &event,
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyH))
        );
        if h_pressed {
            self.hud_visible = !self.hud_visible;
        }

        let egui_consumed = if let (Some(window), Some(egui_state)) =
            (self.window.as_ref(), self.egui_state.as_mut())
        {
            egui_state.on_window_event(window, &event).consumed
        } else {
            false
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    let _ = renderer.resize((size.width, size.height));
                }
                self.world
                    .set_bounds(Bounds::new(0.0, 0.0, size.width as f32, size.height as f32));
            }
            WindowEvent::CursorMoved { position, .. } => {
                if egui_consumed {
                    return;
                }
                self.cursor = Vec2::new(position.x as f32, position.y as f32);
                let now = self.now();
                self.world.move_cursor(self.cursor, now);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if egui_consumed {
                    return;
                }
                if button == MouseButton::Left {
                    let now = self.now();
                    match state {
                        ElementState::Pressed => {
                            self.world.grab(self.cursor, now);
                        }
                        ElementState::Released => {
                            self.world.release(now);
                        }
                    }
                } else if button == MouseButton::Right && state == ElementState::Pressed {
                    self.world.toggle_spring();
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if egui_consumed {
                    return;
                }
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        PhysicalKey::Code(KeyCode::KeyR) | PhysicalKey::Code(KeyCode::Space) => {
                            self.world.reset();
                        }
                        PhysicalKey::Code(KeyCode::KeyC) => self.world.toggle_spring(),
                        PhysicalKey::Code(KeyCode::KeyG) => self.world.toggle_gravity(),
                        PhysicalKey::Code(KeyCode::KeyH) => {}
                        PhysicalKey::Code(KeyCode::KeyN) => self.world.nudge(2800.0),
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.settings.save();
    }
}

pub fn run() -> Result<()> {
    let settings = Settings::load();
    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App::new(settings);
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn overlay_geometry(event_loop: &ActiveEventLoop) -> OverlayGeometry {
    let mut monitors = event_loop.available_monitors();
    let Some(first) = monitors.next() else {
        return OverlayGeometry {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1280, 720),
        };
    };

    let pos = first.position();
    let size = first.size();
    let mut left = pos.x;
    let mut top = pos.y;
    let mut right = pos.x + size.width as i32;
    let mut bottom = pos.y + size.height as i32;

    for monitor in monitors {
        let pos = monitor.position();
        let size = monitor.size();
        left = left.min(pos.x);
        top = top.min(pos.y);
        right = right.max(pos.x + size.width as i32);
        bottom = bottom.max(pos.y + size.height as i32);
    }

    OverlayGeometry {
        position: PhysicalPosition::new(left, top),
        size: PhysicalSize::new((right - left).max(1) as u32, (bottom - top).max(1) as u32),
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
        for i in 0..=18 {
            let t = i as f32 / 18.0;
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
    let primary = Vec4::new(0.015, 0.05, 0.10, 1.0).lerp(world.config.color_outer, 0.78);
    let accent = Vec4::new(0.85, 0.96, 1.0, 1.0).lerp(world.config.color_inner, 0.72);
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
