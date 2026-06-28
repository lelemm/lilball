//! Glue between winit (window + input), the simulation, and the Vulkan
//! renderer. Owns the fixed-timestep loop and translates the simulation state
//! into renderer instances each frame.

use std::time::Instant;

use anyhow::Result;
use glam::{Vec2, Vec4};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use fidget_sim::{Bounds, ParticleKind, World};

use crate::config::Settings;
use crate::renderer::{Instance, Renderer};

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
        }
    }

    fn now(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    fn build_instances(&mut self) {
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
                    _pad: [0.0; 3],
                });
            }
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
                    _pad: [0.0; 3],
                });
            }
        }

        // Ball: glow halo, outer body, inner core, specular highlight.
        let ball = &world.ball;
        let s = ball.squash_scale(cfg.max_speed);
        let v = ball.vel;
        let (sx, sy) = if v.x.abs() >= v.y.abs() {
            (s.x, s.y)
        } else {
            (s.y, s.x)
        };
        let r = ball.radius;
        let inner = cfg.color_inner;
        let outer = cfg.color_outer;
        let c = ball.pos.to_array();

        self.instances.push(Instance {
            center: c,
            half: [r * 2.4 * sx, r * 2.4 * sy],
            color: [outer.x, outer.y, outer.z, 0.16],
            softness: 1.0,
            _pad: [0.0; 3],
        });
        self.instances.push(Instance {
            center: c,
            half: [r * 1.05 * sx, r * 1.05 * sy],
            color: [outer.x, outer.y, outer.z, 0.9],
            softness: 0.55,
            _pad: [0.0; 3],
        });
        self.instances.push(Instance {
            center: c,
            half: [r * 0.72 * sx, r * 0.72 * sy],
            color: [inner.x, inner.y, inner.z, 1.0],
            softness: 0.45,
            _pad: [0.0; 3],
        });
        let hl = ball.pos + Vec2::new(-0.32, -0.36) * r;
        self.instances.push(Instance {
            center: hl.to_array(),
            half: [r * 0.3, r * 0.3],
            color: [1.0, 1.0, 1.0, 0.7],
            softness: 0.6,
            _pad: [0.0; 3],
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
        self.build_instances();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        match renderer.render(&self.instances) {
            Ok(true) => {}
            Ok(false) => {
                // Swapchain out of date: recreate at the current window size.
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    if let Err(e) = renderer.resize((size.width, size.height)) {
                        log::error!("swapchain recreation failed: {e}");
                        event_loop.exit();
                    }
                    self.world
                        .set_bounds(Bounds::new(0.0, 0.0, size.width as f32, size.height as f32));
                }
            }
            Err(e) => {
                log::error!("render error: {e}");
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Fidget-VK")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                log::error!("failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let display = window.display_handle().unwrap().as_raw();
        let win = window.window_handle().unwrap().as_raw();

        match Renderer::new(display, win, (size.width, size.height)) {
            Ok(renderer) => {
                self.world
                    .set_bounds(Bounds::new(0.0, 0.0, size.width as f32, size.height as f32));
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.last_frame = Instant::now();
                log::info!("Fidget-VK is running. Drag the ball; keys: R=reset, G=gravity, Esc=quit");
            }
            Err(e) => {
                log::error!("failed to initialise renderer: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
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
                self.cursor = Vec2::new(position.x as f32, position.y as f32);
                let now = self.now();
                self.world.move_cursor(self.cursor, now);
            }
            WindowEvent::MouseInput { state, button, .. } => {
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
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                        PhysicalKey::Code(KeyCode::KeyR) | PhysicalKey::Code(KeyCode::Space) => {
                            self.world.reset();
                        }
                        PhysicalKey::Code(KeyCode::KeyG) => self.world.toggle_gravity(),
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
