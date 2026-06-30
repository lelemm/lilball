//! Native Win32 overlay shell.

use std::mem::size_of;
use std::num::NonZeroIsize;

use anyhow::{anyhow, Context, Result};
use egui::{Event, Modifiers, PointerButton, RawInput};
use fidget_sim::Bounds;
use glam::Vec2;
use image::imageops::FilterType;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, ReleaseDC, ScreenToClient, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HGDIOBJ, HMONITOR, MONITORINFO,
    SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, RegisterHotKey, ReleaseCapture, SetCapture, UnregisterHotKey,
    MOD_ALT, MOD_CONTROL, VK_MENU, VK_RBUTTON, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NOTIFYICONDATAW, NOTIFYICON_VERSION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, KillTimer,
    LoadCursorW, LoadIconW, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, GWL_EXSTYLE, HCURSOR, HICON, HMENU,
    HTCLIENT, HTTRANSPARENT, HWND_TOPMOST, ICONINFO, IDC_ARROW, IDI_APPLICATION, LWA_ALPHA,
    MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SM_CXSMICON, SM_CXVIRTUALSCREEN,
    SM_CYSMICON, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, TRACK_POPUP_MENU_FLAGS, WM_APP, WM_CHAR,
    WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCHITTEST,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SIZE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::app::core::{AppAction, Core};
use crate::config::{PlayMode, Settings, ToySize};
use crate::renderer::{DesktopSnapshot, EguiDrawData, Renderer};

const CLASS_NAME: &str = "FidgetVkOverlayWindow";
const WINDOW_TITLE: &str = "Fidget-VK";
const TIMER_ID_REDRAW: usize = 1;
const TIMER_MS: u32 = 8;
const TRAY_ID: u32 = 1;
const WM_TRAY: u32 = WM_APP + 1;
const VK_C: u32 = b'C' as u32;
const VK_G: u32 = b'G' as u32;
const VK_H: u32 = b'H' as u32;
const VK_N: u32 = b'N' as u32;
const VK_P: u32 = b'P' as u32;
const VK_R: u32 = b'R' as u32;
const VK_SPACE: u32 = 0x20;
const VK_ESCAPE: u32 = 0x1B;

const HOTKEY_RESET: i32 = 101;
const HOTKEY_TOGGLE_SPRING: i32 = 102;
const HOTKEY_TOGGLE_GRAVITY: i32 = 103;
const HOTKEY_TOGGLE_HUD: i32 = 104;
const HOTKEY_NUDGE: i32 = 105;
const HOTKEY_QUIT: i32 = 106;
const HOTKEY_TOGGLE_MODE: i32 = 107;

const MENU_TOGGLE_HUD: usize = 201;
const MENU_RESET: usize = 202;
const MENU_TOGGLE_SPRING: usize = 203;
const MENU_TOGGLE_GRAVITY: usize = 204;
const MENU_NUDGE: usize = 205;
const MENU_QUIT: usize = 206;
const MENU_TOGGLE_RUBBER_BAND: usize = 207;
const MENU_TOGGLE_BOTTOM_BOUNCE: usize = 208;
const MENU_TOGGLE_SINGLE_MONITOR: usize = 209;
const MENU_SIZE_SMALL: usize = 210;
const MENU_SIZE_MEDIUM: usize = 211;
const MENU_SIZE_LARGE: usize = 212;
const MENU_TOGGLE_MODE: usize = 213;
const MENU_SPAWN_MARBLE: usize = 214;
const MENU_CLEAR_MARBLES: usize = 215;
const MENU_SCATTER_MARBLES: usize = 216;
const SOCCER_ICON_PNG: &[u8] = include_bytes!("../../../../assets/soccer_ball_material.png");
const MONITORINFOF_PRIMARY: u32 = 1;

pub(super) fn run() -> Result<()> {
    let instance = module_instance()?;
    let class_name = wide_null(CLASS_NAME);
    let window_title = wide_null(WINDOW_TITLE);
    register_window_class(instance, &class_name)?;

    let geometry = OverlayGeometry::virtual_desktop();
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_title.as_ptr()),
            WS_POPUP,
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            None,
            None,
            Some(instance),
            None,
        )
        .context("failed to create Win32 overlay window")?
    };

    let mut shell = Box::new(Win32App::new(hwnd, instance, geometry.clone()));
    unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWLP_USERDATA,
            shell.as_mut() as *mut Win32App as isize,
        );
        SetLayeredWindowAttributes(hwnd, Default::default(), 255, LWA_ALPHA)
            .context("failed to configure layered overlay window")?;
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            geometry.x,
            geometry.y,
            geometry.width,
            geometry.height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .context("failed to position Win32 overlay window")?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    shell
        .init_renderer()
        .context("failed to initialise Win32 overlay renderer")?;
    shell.add_tray_icon();
    shell.register_hotkeys();
    shell.start_timer();

    log::info!(
        "Win32 overlay geometry: pos=({}, {}) size={}x{}",
        geometry.x,
        geometry.y,
        geometry.width,
        geometry.height
    );
    log::info!(
        "Fidget-VK Win32 shell is running. Tray menu and Ctrl+Alt+H/C/G/N/R/Esc hotkeys are active."
    );

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0).as_bool() } {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    shell.shutdown();
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

struct Win32App {
    core: Core,
    hwnd: HWND,
    instance: HINSTANCE,
    geometry: OverlayGeometry,
    renderer: Option<Renderer>,
    egui_input: RawInput,
    cursor: Vec2,
    modifiers: Modifiers,
    tray_added: bool,
    left_down: bool,
    right_down: bool,
    click_through: bool,
    snapshotter: DesktopSnapshotter,
}

impl Win32App {
    fn new(hwnd: HWND, instance: HINSTANCE, geometry: OverlayGeometry) -> Self {
        let snapshot_geometry = geometry.clone();
        let egui_input = RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(geometry.width as f32, geometry.height as f32),
            )),
            ..Default::default()
        };
        Self {
            core: Core::new(Settings::load()),
            hwnd,
            instance,
            geometry,
            renderer: None,
            egui_input,
            cursor: Vec2::ZERO,
            modifiers: Modifiers::default(),
            tray_added: false,
            left_down: false,
            right_down: false,
            click_through: true,
            snapshotter: DesktopSnapshotter::new(snapshot_geometry),
        }
    }

    fn init_renderer(&mut self) -> Result<()> {
        self.core
            .resize(self.geometry.width as u32, self.geometry.height as u32);
        self.core.set_monitor_layout(
            self.geometry.monitor_bounds.iter().copied(),
            self.geometry.primary_monitor,
        );
        let display = RawDisplayHandle::Windows(WindowsDisplayHandle::new());
        let hwnd = NonZeroIsize::new(self.hwnd.0 as isize).ok_or_else(|| anyhow!("null HWND"))?;
        let hinstance = NonZeroIsize::new(self.instance.0 as isize);
        let mut window = Win32WindowHandle::new(hwnd);
        window.hinstance = hinstance;
        let window = RawWindowHandle::Win32(window);
        let renderer = Renderer::new(
            display,
            window,
            (self.geometry.width as u32, self.geometry.height as u32),
        )?;
        self.renderer = Some(renderer);
        self.core.reset_frame_clock();
        Ok(())
    }

    fn start_timer(&self) {
        let timer = unsafe { SetTimer(Some(self.hwnd), TIMER_ID_REDRAW, TIMER_MS, None) };
        if timer == 0 {
            log::warn!("failed to start Win32 redraw timer");
        }
    }

    fn redraw(&mut self) {
        self.poll_cursor();
        self.core.advance_frame();
        self.update_egui_screen_rect();

        let ctx = self.core.egui_context();
        let mut input = self.egui_input.take();
        input.time = Some(self.core.now() as f64);
        input.modifiers = self.modifiers;
        let full_output = ctx.run(input, |ctx| self.core.show_hud(ctx));
        for command in full_output.platform_output.commands {
            log::debug!("unhandled egui platform command from Win32 shell: {command:?}");
        }
        let pixels_per_point = ctx.pixels_per_point();
        let clipped_primitives = ctx.tessellate(full_output.shapes, pixels_per_point);
        self.core.build_instances();
        self.update_click_through_for_cursor();
        let desktop_snapshot = self.refresh_desktop_snapshot();

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let egui_draw = EguiDrawData {
            textures_delta: &full_output.textures_delta,
            clipped_primitives: &clipped_primitives,
            pixels_per_point,
        };
        match renderer.render(
            self.core.instances(),
            self.core.rubber_band(),
            self.core.marble_instances(),
            desktop_snapshot.as_ref(),
            Some(egui_draw),
        ) {
            Ok(true) => {}
            Ok(false) => {
                if let Err(e) =
                    renderer.resize((self.geometry.width as u32, self.geometry.height as u32))
                {
                    log::error!("Win32 swapchain recreation failed: {e}");
                    self.quit();
                }
            }
            Err(e) => {
                log::error!("Win32 render error: {e}");
                self.quit();
            }
        }
    }

    fn resize(&mut self, width: i32, height: i32) {
        self.geometry.width = width.max(1);
        self.geometry.height = height.max(1);
        self.snapshotter.set_geometry(self.geometry.clone());
        self.core
            .resize(self.geometry.width as u32, self.geometry.height as u32);
        self.core.set_monitor_layout(
            self.geometry.monitor_bounds.iter().copied(),
            self.geometry.primary_monitor,
        );
        self.update_egui_screen_rect();
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(e) =
                renderer.resize((self.geometry.width as u32, self.geometry.height as u32))
            {
                log::error!("Win32 resize failed: {e}");
            }
        }
    }

    fn refresh_desktop_snapshot(&mut self) -> Option<DesktopSnapshot> {
        if self.core.play_mode() != PlayMode::Marbles || !self.core.take_desktop_snapshot_request()
        {
            return None;
        }
        match self.snapshotter.capture() {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                log::debug!("desktop snapshot capture failed: {e}");
                None
            }
        }
    }

    fn update_egui_screen_rect(&mut self) {
        self.egui_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(self.geometry.width as f32, self.geometry.height as f32),
        ));
    }

    fn update_modifiers(&mut self) {
        let shift = key_is_down(VK_SHIFT.0 as i32);
        let ctrl = key_is_down(0x11);
        let alt = key_is_down(VK_MENU.0 as i32);
        self.modifiers = Modifiers {
            alt,
            ctrl,
            shift,
            mac_cmd: false,
            command: ctrl,
        };
    }

    fn poll_cursor(&mut self) {
        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return;
        }
        self.poll_right_button();
        if unsafe { ScreenToClient(self.hwnd, &mut point) }.as_bool() {
            let pos = Vec2::new(point.x as f32, point.y as f32);
            if pos != self.cursor {
                self.cursor = pos;
                self.egui_input
                    .events
                    .push(Event::PointerMoved(egui::pos2(pos.x, pos.y)));
                self.core.on_cursor_moved(pos);
            }
            self.update_click_through_for_cursor();
        }
    }

    fn poll_right_button(&mut self) {
        let down = key_is_down_async(VK_RBUTTON.0 as i32);
        match (self.right_down, down) {
            (false, true) => {
                self.right_down = true;
                self.core.on_right_pressed();
            }
            (true, false) => {
                self.right_down = false;
                self.core.on_right_released();
            }
            _ => {}
        }
    }

    fn update_click_through_for_cursor(&mut self) {
        let cursor_inside = self.cursor.x >= 0.0
            && self.cursor.y >= 0.0
            && self.cursor.x < self.geometry.width as f32
            && self.cursor.y < self.geometry.height as f32;
        let interactive =
            cursor_inside && (self.left_down || self.core.hit_test_interactive(self.cursor));
        self.set_click_through(!interactive);
    }

    fn set_click_through(&mut self, enabled: bool) {
        if self.click_through == enabled {
            return;
        }

        unsafe {
            let style = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let new_style = if enabled {
                style | WS_EX_TRANSPARENT.0 as isize
            } else {
                style & !(WS_EX_TRANSPARENT.0 as isize)
            };
            SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, new_style);
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        self.click_through = enabled;
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) {
        self.update_modifiers();
        let pos = client_pos_from_lparam(lparam);
        self.cursor = pos;
        self.egui_input
            .events
            .push(Event::PointerMoved(egui::pos2(pos.x, pos.y)));
        if !self.core.egui_context().wants_pointer_input() || self.left_down {
            self.core.on_cursor_moved(pos);
        }
    }

    fn on_left_button(&mut self, lparam: LPARAM, pressed: bool) {
        self.update_modifiers();
        let pos = client_pos_from_lparam(lparam);
        self.cursor = pos;
        self.egui_input.events.push(Event::PointerButton {
            pos: egui::pos2(pos.x, pos.y),
            button: PointerButton::Primary,
            pressed,
            modifiers: self.modifiers,
        });
        if pressed {
            if self.core.egui_context().wants_pointer_input() {
                return;
            }
            self.core.on_cursor_moved(pos);
            if self.core.on_left_pressed() {
                self.left_down = true;
                unsafe {
                    let _ = SetCapture(self.hwnd);
                }
            }
        } else {
            if self.left_down {
                self.core.on_cursor_moved(pos);
                self.core.on_left_released();
                self.left_down = false;
                let _ = unsafe { ReleaseCapture() };
            }
        }
    }

    fn on_right_button_down(&mut self, lparam: LPARAM) {
        self.update_modifiers();
        let pos = client_pos_from_lparam(lparam);
        self.cursor = pos;
        if !self.right_down {
            self.right_down = true;
            self.core.on_cursor_moved(pos);
            self.core.on_right_pressed();
            unsafe {
                let _ = SetCapture(self.hwnd);
            }
        }
        self.egui_input.events.push(Event::PointerButton {
            pos: egui::pos2(pos.x, pos.y),
            button: PointerButton::Secondary,
            pressed: true,
            modifiers: self.modifiers,
        });
    }

    fn on_right_button_up(&mut self, lparam: LPARAM) {
        self.update_modifiers();
        let pos = client_pos_from_lparam(lparam);
        self.cursor = pos;
        if self.right_down {
            self.core.on_cursor_moved(pos);
            self.right_down = false;
            self.core.on_right_released();
            if !self.left_down {
                let _ = unsafe { ReleaseCapture() };
            }
        }
        self.egui_input.events.push(Event::PointerButton {
            pos: egui::pos2(pos.x, pos.y),
            button: PointerButton::Secondary,
            pressed: false,
            modifiers: self.modifiers,
        });
        self.update_click_through_for_cursor();
    }

    fn on_key(&mut self, wparam: WPARAM, pressed: bool) {
        self.update_modifiers();
        let Some(key) = egui_key_from_vk(wparam.0 as u32) else {
            return;
        };
        self.egui_input.events.push(Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: self.modifiers,
        });
        if !pressed || self.core.egui_context().wants_keyboard_input() {
            return;
        }
        match wparam.0 as u32 {
            VK_ESCAPE => self.quit(),
            VK_R | VK_SPACE => self.core.apply_action(AppAction::Reset),
            VK_C => self.core.apply_action(AppAction::ToggleSpring),
            VK_G => self.core.apply_action(AppAction::ToggleGravity),
            VK_H => self.core.apply_action(AppAction::ToggleHud),
            VK_N => self.core.apply_action(AppAction::Nudge),
            VK_P => self.core.apply_action(AppAction::ToggleMode),
            _ => {}
        }
    }

    fn on_char(&mut self, wparam: WPARAM) {
        let Some(ch) = char::from_u32(wparam.0 as u32) else {
            return;
        };
        if !ch.is_control() {
            self.egui_input.events.push(Event::Text(ch.to_string()));
        }
    }

    fn hit_test(&mut self, lparam: LPARAM) -> LRESULT {
        let mut point = POINT {
            x: signed_low_word(lparam.0 as usize as u32) as i32,
            y: signed_high_word(lparam.0 as usize as u32) as i32,
        };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut point);
        }
        let cursor = Vec2::new(point.x as f32, point.y as f32);
        if cursor.x < 0.0
            || cursor.y < 0.0
            || cursor.x >= self.geometry.width as f32
            || cursor.y >= self.geometry.height as f32
        {
            return unsafe { DefWindowProcW(self.hwnd, WM_NCHITTEST, WPARAM(0), lparam) };
        }
        if self.core.hit_test_interactive(cursor) {
            LRESULT(HTCLIENT as isize)
        } else {
            // This is the per-object pass-through behavior: empty overlay
            // pixels do not activate this HWND and fall through to the desktop.
            LRESULT(HTTRANSPARENT as isize)
        }
    }

    fn add_tray_icon(&mut self) {
        let mut data = self.tray_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        data.hIcon = app_icon();
        fill_wide_buf(&mut data.szTip, "Fidget-VK");
        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                data.Anonymous.uVersion = NOTIFYICON_VERSION;
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
                self.tray_added = true;
            } else {
                log::warn!("failed to add Fidget-VK tray icon");
            }
        }
    }

    fn remove_tray_icon(&mut self) {
        if self.tray_added {
            let data = self.tray_data();
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.tray_added = false;
        }
    }

    fn tray_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_ID,
            ..Default::default()
        }
    }

    fn show_tray_menu(&mut self) {
        let menu = match unsafe { CreatePopupMenu() } {
            Ok(menu) => menu,
            Err(e) => {
                log::warn!("failed to create tray menu: {e}");
                return;
            }
        };
        unsafe {
            append_checked_menu(menu, MENU_TOGGLE_HUD, "Show HUD", self.core.hud_visible());
            append_checked_menu(
                menu,
                MENU_TOGGLE_MODE,
                "Marble mode",
                self.core.play_mode() == PlayMode::Marbles,
            );
            append_menu(
                menu,
                MENU_RESET,
                if self.core.play_mode() == PlayMode::Marbles {
                    "Spawn marble"
                } else {
                    "Spawn ball"
                },
            );
            append_menu(menu, MENU_SPAWN_MARBLE, "Spawn marble");
            append_menu(menu, MENU_SCATTER_MARBLES, "Scatter marbles");
            append_menu(
                menu,
                MENU_CLEAR_MARBLES,
                &format!("Clear marbles ({})", self.core.marble_count()),
            );
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            append_checked_menu(
                menu,
                MENU_TOGGLE_SPRING,
                "Spring attached",
                self.core.spring_attached(),
            );
            append_checked_menu(
                menu,
                MENU_TOGGLE_RUBBER_BAND,
                "Rubber band visual",
                self.core.rubber_band_visual_enabled(),
            );
            append_checked_menu(
                menu,
                MENU_SIZE_SMALL,
                "Ball + string: S",
                self.core.toy_size() == ToySize::Small,
            );
            append_checked_menu(
                menu,
                MENU_SIZE_MEDIUM,
                "Ball + string: M",
                self.core.toy_size() == ToySize::Medium,
            );
            append_checked_menu(
                menu,
                MENU_SIZE_LARGE,
                "Ball + string: L",
                self.core.toy_size() == ToySize::Large,
            );
            append_checked_menu(
                menu,
                MENU_TOGGLE_GRAVITY,
                "Gravity enabled",
                self.core.gravity_enabled(),
            );
            append_checked_menu(
                menu,
                MENU_TOGGLE_BOTTOM_BOUNCE,
                "Bounce bottom edge",
                self.core.bottom_bounce_enabled(),
            );
            append_checked_menu(
                menu,
                MENU_TOGGLE_SINGLE_MONITOR,
                "Single monitor bounds",
                self.core.single_monitor_bounds_enabled(),
            );
            append_menu(menu, MENU_NUDGE, "Fling ball");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            append_menu(menu, MENU_QUIT, "Quit");

            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_ok() {
                let _ = SetForegroundWindow(self.hwnd);
                let flags: TRACK_POPUP_MENU_FLAGS =
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON;
                let _ = windows::Win32::UI::WindowsAndMessaging::TrackPopupMenu(
                    menu, flags, point.x, point.y, None, self.hwnd, None,
                );
            }
            let _ = DestroyMenu(menu);
        }
    }

    fn handle_command(&mut self, id: usize) {
        match id {
            MENU_TOGGLE_HUD => self.core.apply_action(AppAction::ToggleHud),
            MENU_TOGGLE_MODE => self.core.apply_action(AppAction::ToggleMode),
            MENU_RESET => self.core.apply_action(AppAction::Reset),
            MENU_SPAWN_MARBLE => self.core.apply_action(AppAction::SpawnMarble),
            MENU_CLEAR_MARBLES => self.core.apply_action(AppAction::ClearMarbles),
            MENU_SCATTER_MARBLES => self.core.apply_action(AppAction::ScatterMarbles),
            MENU_TOGGLE_SPRING => self.core.apply_action(AppAction::ToggleSpring),
            MENU_TOGGLE_RUBBER_BAND => self.core.apply_action(AppAction::ToggleSpringVisual),
            MENU_TOGGLE_GRAVITY => self.core.apply_action(AppAction::ToggleGravity),
            MENU_TOGGLE_BOTTOM_BOUNCE => self.core.apply_action(AppAction::ToggleBottomBounce),
            MENU_TOGGLE_SINGLE_MONITOR => {
                self.core.apply_action(AppAction::ToggleSingleMonitorBounds)
            }
            MENU_SIZE_SMALL => self
                .core
                .apply_action(AppAction::SetToySize(ToySize::Small)),
            MENU_SIZE_MEDIUM => self
                .core
                .apply_action(AppAction::SetToySize(ToySize::Medium)),
            MENU_SIZE_LARGE => self
                .core
                .apply_action(AppAction::SetToySize(ToySize::Large)),
            MENU_NUDGE => self.core.apply_action(AppAction::Nudge),
            MENU_QUIT => self.quit(),
            _ => {}
        }
    }

    fn register_hotkeys(&self) {
        let modifiers = MOD_CONTROL | MOD_ALT;
        let hotkeys = [
            (HOTKEY_RESET, VK_R),
            (HOTKEY_TOGGLE_SPRING, VK_C),
            (HOTKEY_TOGGLE_GRAVITY, VK_G),
            (HOTKEY_TOGGLE_HUD, VK_H),
            (HOTKEY_NUDGE, VK_N),
            (HOTKEY_TOGGLE_MODE, VK_P),
            (HOTKEY_QUIT, VK_ESCAPE),
        ];
        for (id, vk) in hotkeys {
            if let Err(e) = unsafe { RegisterHotKey(Some(self.hwnd), id, modifiers, vk) } {
                log::warn!("failed to register Win32 hotkey id {id}: {e}");
            }
        }
    }

    fn unregister_hotkeys(&self) {
        for id in [
            HOTKEY_RESET,
            HOTKEY_TOGGLE_SPRING,
            HOTKEY_TOGGLE_GRAVITY,
            HOTKEY_TOGGLE_HUD,
            HOTKEY_NUDGE,
            HOTKEY_TOGGLE_MODE,
            HOTKEY_QUIT,
        ] {
            let _ = unsafe { UnregisterHotKey(Some(self.hwnd), id) };
        }
    }

    fn handle_hotkey(&mut self, id: i32) {
        match id {
            HOTKEY_RESET => self.core.apply_action(AppAction::Reset),
            HOTKEY_TOGGLE_SPRING => self.core.apply_action(AppAction::ToggleSpring),
            HOTKEY_TOGGLE_GRAVITY => self.core.apply_action(AppAction::ToggleGravity),
            HOTKEY_TOGGLE_HUD => self.core.apply_action(AppAction::ToggleHud),
            HOTKEY_NUDGE => self.core.apply_action(AppAction::Nudge),
            HOTKEY_TOGGLE_MODE => self.core.apply_action(AppAction::ToggleMode),
            HOTKEY_QUIT => self.quit(),
            _ => {}
        }
    }

    fn quit(&self) {
        unsafe {
            PostQuitMessage(0);
        }
    }

    fn shutdown(&mut self) {
        self.set_click_through(false);
        let _ = unsafe { KillTimer(Some(self.hwnd), TIMER_ID_REDRAW) };
        if self.left_down {
            let _ = unsafe { ReleaseCapture() };
            self.left_down = false;
        }
        self.unregister_hotkeys();
        self.remove_tray_icon();
        self.core.save_settings();
        self.renderer = None;
    }
}

#[derive(Debug, Clone)]
struct OverlayGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    monitor_bounds: Vec<Bounds>,
    primary_monitor: usize,
}

impl OverlayGeometry {
    fn virtual_desktop() -> Self {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let (monitor_bounds, primary_monitor) = collect_monitor_bounds(x, y);
            let monitor_bounds = if monitor_bounds.is_empty() {
                vec![Bounds::new(0.0, 0.0, width as f32, height as f32)]
            } else {
                monitor_bounds
            };
            let primary_monitor = primary_monitor.min(monitor_bounds.len().saturating_sub(1));
            Self {
                x,
                y,
                width,
                height,
                monitor_bounds,
                primary_monitor,
            }
        }
    }
}

#[derive(Debug, Clone)]
struct DesktopSnapshotter {
    geometry: OverlayGeometry,
}

impl DesktopSnapshotter {
    fn new(geometry: OverlayGeometry) -> Self {
        Self { geometry }
    }

    fn set_geometry(&mut self, geometry: OverlayGeometry) {
        self.geometry = geometry;
    }

    fn capture(&mut self) -> Result<DesktopSnapshot> {
        let width = self.geometry.width.max(1) as u32;
        let height = self.geometry.height.max(1) as u32;
        let width_i32 = width as i32;
        let height_i32 = height as i32;

        let bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width_i32,
                biHeight: -height_i32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return Err(anyhow!("failed to get screen DC"));
        }
        let mem_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if mem_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, screen_dc);
            }
            return Err(anyhow!("failed to create compatible DC"));
        }

        let mut bits = std::ptr::null_mut();
        let bitmap = unsafe {
            CreateDIBSection(
                Some(mem_dc),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            )
            .context("failed to create desktop capture DIB")?
        };

        let result = (|| {
            let old = unsafe { SelectObject(mem_dc, HGDIOBJ::from(bitmap)) };
            if old.is_invalid() {
                return Err(anyhow!("failed to select capture bitmap"));
            }
            unsafe {
                BitBlt(
                    mem_dc,
                    0,
                    0,
                    width_i32,
                    height_i32,
                    Some(screen_dc),
                    self.geometry.x,
                    self.geometry.y,
                    SRCCOPY,
                )
            }
            .context("desktop BitBlt failed")?;
            unsafe {
                let _ = SelectObject(mem_dc, old);
            }

            let bgra = unsafe {
                std::slice::from_raw_parts(bits as *const u8, width as usize * height as usize * 4)
            };
            let mut rgba = vec![0_u8; bgra.len()];
            for (src, dst) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                dst[0] = src[2];
                dst[1] = src[1];
                dst[2] = src[0];
                dst[3] = 255;
            }
            Ok(DesktopSnapshot {
                width,
                height,
                rgba,
            })
        })();

        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(bitmap));
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
        }
        result
    }
}

struct MonitorEntry {
    bounds: Bounds,
    primary: bool,
}

struct MonitorEdgeCollector {
    origin_x: i32,
    origin_y: i32,
    entries: Vec<MonitorEntry>,
}

fn collect_monitor_bounds(origin_x: i32, origin_y: i32) -> (Vec<Bounds>, usize) {
    let mut collector = MonitorEdgeCollector {
        origin_x,
        origin_y,
        entries: Vec::new(),
    };
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor_edge),
            LPARAM(&mut collector as *mut MonitorEdgeCollector as isize),
        )
    };
    if !ok.as_bool() {
        log::warn!("failed to enumerate display monitors");
    }
    let primary_monitor = collector
        .entries
        .iter()
        .position(|entry| entry.primary)
        .unwrap_or(0);
    (
        collector
            .entries
            .into_iter()
            .map(|entry| entry.bounds)
            .collect(),
        primary_monitor,
    )
}

unsafe extern "system" fn collect_monitor_edge(
    monitor: HMONITOR,
    _hdc: HDC,
    rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let collector = unsafe { &mut *(data.0 as *mut MonitorEdgeCollector) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let monitor_rect = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        info.rcMonitor
    } else if !rect.is_null() {
        unsafe { *rect }
    } else {
        return BOOL(1);
    };

    collector.entries.push(MonitorEntry {
        bounds: Bounds::new(
            (monitor_rect.left - collector.origin_x) as f32,
            (monitor_rect.top - collector.origin_y) as f32,
            (monitor_rect.right - collector.origin_x) as f32,
            (monitor_rect.bottom - collector.origin_y) as f32,
        ),
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    BOOL(1)
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let app_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Win32App };
    if app_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let app = unsafe { &mut *app_ptr };

    match msg {
        WM_TIMER if wparam.0 == TIMER_ID_REDRAW => {
            app.redraw();
            LRESULT(0)
        }
        WM_SIZE => {
            let width = low_word(lparam.0 as usize as u32) as i32;
            let height = high_word(lparam.0 as usize as u32) as i32;
            app.resize(width, height);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            app.on_mouse_move(lparam);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            app.on_left_button(lparam, true);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            app.on_left_button(lparam, false);
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            app.on_right_button_down(lparam);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            app.on_right_button_up(lparam);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            app.on_key(wparam, true);
            LRESULT(0)
        }
        WM_KEYUP => {
            app.on_key(wparam, false);
            LRESULT(0)
        }
        WM_CHAR => {
            app.on_char(wparam);
            LRESULT(0)
        }
        WM_NCHITTEST => app.hit_test(lparam),
        WM_HOTKEY => {
            app.handle_hotkey(wparam.0 as i32);
            LRESULT(0)
        }
        WM_COMMAND => {
            app.handle_command(low_word(wparam.0 as u32) as usize);
            LRESULT(0)
        }
        WM_TRAY => {
            let tray_msg = lparam.0 as u32;
            if tray_msg == WM_LBUTTONUP || tray_msg == WM_LBUTTONDBLCLK {
                app.core.apply_action(AppAction::Reset);
            } else if tray_msg == WM_RBUTTONUP || tray_msg == WM_CONTEXTMENU {
                app.show_tray_menu();
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            app.quit();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn register_window_class(instance: HINSTANCE, class_name: &[u16]) -> Result<()> {
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default()) };
    let icon = app_icon();
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance,
        hIcon: icon,
        hCursor: cursor,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        Err(anyhow!("failed to register Win32 window class"))
    } else {
        Ok(())
    }
}

fn module_instance() -> Result<HINSTANCE> {
    let module = unsafe { GetModuleHandleW(None).context("failed to get module handle")? };
    Ok(HINSTANCE(module.0))
}

fn app_icon() -> HICON {
    match soccer_ball_icon() {
        Ok(icon) => icon,
        Err(e) => {
            log::warn!("failed to create soccer ball tray icon: {e}");
            unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or(HICON::default()) }
        }
    }
}

fn soccer_ball_icon() -> Result<HICON> {
    let width = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    let height = unsafe { GetSystemMetrics(SM_CYSMICON) }.max(16);
    let size = width.min(height).clamp(16, 64) as u32;
    let image = image::load_from_memory(SOCCER_ICON_PNG)
        .context("failed to decode embedded soccer ball texture")?
        .resize_to_fill(size, size, FilterType::Lanczos3)
        .to_rgba8();
    let size_i32 = size as i32;

    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size_i32,
            biHeight: -size_i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let mut bits = std::ptr::null_mut();
    let color = unsafe {
        CreateDIBSection(None, &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
            .context("failed to create icon color bitmap")?
    };

    let result = (|| {
        let pixels =
            unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize) };
        for y in 0..size {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                let px = image.get_pixel(x, y);
                let nx = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let ny = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
                let r2 = nx * nx + ny * ny;
                let edge = (1.0 - ((r2.sqrt() - 0.88) / 0.12).clamp(0.0, 1.0)).clamp(0.0, 1.0);
                let z = (1.0 - r2).max(0.0).sqrt();
                let shade = (0.66 + z * 0.34).clamp(0.0, 1.0);
                pixels[i] = (px[2] as f32 * shade) as u8;
                pixels[i + 1] = (px[1] as f32 * shade) as u8;
                pixels[i + 2] = (px[0] as f32 * shade) as u8;
                pixels[i + 3] = (px[3] as f32 * edge) as u8;
            }
        }

        let mask_stride = size.div_ceil(32) * 4;
        let mask_bits = vec![0_u8; (mask_stride * size) as usize];
        let mask = unsafe {
            CreateBitmap(
                size_i32,
                size_i32,
                1,
                1,
                Some(mask_bits.as_ptr() as *const std::ffi::c_void),
            )
        };
        if mask.is_invalid() {
            return Err(anyhow!("failed to create icon mask bitmap"));
        }

        let icon_info = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon_result =
            unsafe { windows::Win32::UI::WindowsAndMessaging::CreateIconIndirect(&icon_info) }
                .context("failed to create soccer ball icon");
        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(mask));
        }
        icon_result
    })();

    unsafe {
        let _ = DeleteObject(HGDIOBJ::from(color));
    }
    result
}

unsafe fn append_menu(menu: HMENU, id: usize, text: &str) {
    let wide = wide_null(text);
    let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
}

unsafe fn append_checked_menu(menu: HMENU, id: usize, text: &str, checked: bool) {
    let wide = wide_null(text);
    let state = if checked { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING | state, id, PCWSTR(wide.as_ptr()));
}

fn fill_wide_buf(buf: &mut [u16], text: &str) {
    let wide = wide_null(text);
    let count = wide.len().min(buf.len());
    buf[..count].copy_from_slice(&wide[..count]);
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn key_is_down(vk: i32) -> bool {
    unsafe { GetKeyState(vk) < 0 }
}

fn key_is_down_async(vk: i32) -> bool {
    unsafe { GetAsyncKeyState(vk) < 0 }
}

fn client_pos_from_lparam(lparam: LPARAM) -> Vec2 {
    let packed = lparam.0 as usize as u32;
    Vec2::new(
        signed_low_word(packed) as f32,
        signed_high_word(packed) as f32,
    )
}

fn low_word(value: u32) -> u16 {
    (value & 0xffff) as u16
}

fn high_word(value: u32) -> u16 {
    ((value >> 16) & 0xffff) as u16
}

fn signed_low_word(value: u32) -> i16 {
    low_word(value) as i16
}

fn signed_high_word(value: u32) -> i16 {
    high_word(value) as i16
}

fn egui_key_from_vk(vk: u32) -> Option<egui::Key> {
    Some(match vk {
        VK_ESCAPE => egui::Key::Escape,
        VK_SPACE => egui::Key::Space,
        VK_R => egui::Key::R,
        VK_C => egui::Key::C,
        VK_G => egui::Key::G,
        VK_H => egui::Key::H,
        VK_N => egui::Key::N,
        VK_P => egui::Key::P,
        0x25 => egui::Key::ArrowLeft,
        0x26 => egui::Key::ArrowUp,
        0x27 => egui::Key::ArrowRight,
        0x28 => egui::Key::ArrowDown,
        0x09 => egui::Key::Tab,
        0x0D => egui::Key::Enter,
        0x08 => egui::Key::Backspace,
        0x2E => egui::Key::Delete,
        _ => return None,
    })
}
