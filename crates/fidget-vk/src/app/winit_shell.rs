//! Current winit-based overlay shell used by Linux and preview builds.

use anyhow::Result;
use glam::Vec2;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
#[cfg(target_os = "linux")]
use winit::platform::x11::{WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

use crate::app::core::{AppAction, Core};
use crate::config::Settings;
use crate::renderer::{EguiDrawData, Renderer};

pub(super) struct WinitApp {
    core: Core,
    // `renderer` is declared before `window` so it (and its Vulkan surface) is
    // dropped before the window it was created from.
    renderer: Option<Renderer>,
    window: Option<Window>,
    egui_state: Option<egui_winit::State>,
}

#[derive(Debug, Clone, Copy)]
struct OverlayGeometry {
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

impl WinitApp {
    fn new(settings: Settings) -> Self {
        Self {
            core: Core::new(settings),
            renderer: None,
            window: None,
            egui_state: None,
        }
    }

    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        self.core.advance_frame();

        let egui_output = {
            let raw_input = {
                let window = self.window.as_ref().expect("window exists while redrawing");
                self.egui_state
                    .as_mut()
                    .expect("egui state exists while redrawing")
                    .take_egui_input(window)
            };
            let ctx = self.core.egui_context();
            let full_output = ctx.run(raw_input, |ctx| self.core.show_hud(ctx));
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

        self.core.build_instances();

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
        match renderer.render(self.core.instances(), egui_draw) {
            Ok(true) => {}
            Ok(false) => {
                // Swapchain out of date: recreate at the current window size.
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    if let Err(e) = renderer.resize((size.width, size.height)) {
                        log::error!("swapchain recreation failed: {e}");
                        event_loop.exit();
                    }
                    self.core.resize(size.width, size.height);
                }
            }
            Err(e) => {
                log::error!("render error: {e}");
                event_loop.exit();
            }
        }
    }
}

impl ApplicationHandler for WinitApp {
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
            self.core.egui_context(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );

        match Renderer::new(display, win, (size.width, size.height)) {
            Ok(renderer) => {
                self.core.resize(size.width, size.height);
                self.renderer = Some(renderer);
                self.window = Some(window);
                self.egui_state = Some(egui_state);
                self.core.reset_frame_clock();
                log::info!(
                    "transparent overlay geometry: pos=({}, {}) size={}x{}",
                    overlay.position.x,
                    overlay.position.y,
                    size.width,
                    size.height
                );
                log::info!(
                    "Fidget-VK is running. Drag the ball; hold right-click and brush/sweep the spring to displace or entangle it; C=cut/recall spring, N=fling, R=reset, G=gravity, Esc=quit"
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
            self.core.apply_action(AppAction::ToggleHud);
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
                self.core.resize(size.width, size.height);
            }
            WindowEvent::CursorMoved { position, .. } => {
                if egui_consumed {
                    return;
                }
                self.core
                    .on_cursor_moved(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if egui_consumed {
                    return;
                }
                if button == MouseButton::Left {
                    match state {
                        ElementState::Pressed => {
                            self.core.on_left_pressed();
                        }
                        ElementState::Released => {
                            self.core.on_left_released();
                        }
                    }
                } else if button == MouseButton::Right {
                    match state {
                        ElementState::Pressed => self.core.on_right_pressed(),
                        ElementState::Released => self.core.on_right_released(),
                    }
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
                            self.core.apply_action(AppAction::Reset);
                        }
                        PhysicalKey::Code(KeyCode::KeyC) => {
                            self.core.apply_action(AppAction::ToggleSpring);
                        }
                        PhysicalKey::Code(KeyCode::KeyG) => {
                            self.core.apply_action(AppAction::ToggleGravity);
                        }
                        PhysicalKey::Code(KeyCode::KeyH) => {}
                        PhysicalKey::Code(KeyCode::KeyN) => {
                            self.core.apply_action(AppAction::Nudge);
                        }
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
        self.core.save_settings();
    }
}

pub(super) fn run() -> Result<()> {
    let settings = Settings::load();
    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = WinitApp::new(settings);
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
