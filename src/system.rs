use std::collections::HashMap;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    ShowWindow,
    ToggleWindow,
    Refresh,
    Quit,
    SwitchSlot(usize),
}

#[cfg(windows)]
pub struct SingleInstanceGuard(Option<windows::Win32::Foundation::HANDLE>);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0 {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
        }
    }
}

#[cfg(not(windows))]
pub struct SingleInstanceGuard;

#[cfg(windows)]
pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    use windows::core::{PCWSTR, w};

    let handle = match unsafe {
        CreateMutexW(
            None,
            false,
            w!("Local\\tkpdx01.lg-monitor-control.single-instance"),
        )
    } {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(%error, "Could not create the single-instance mutex");
            return Some(SingleInstanceGuard(None));
        }
    };

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            if let Ok(hwnd) = FindWindowW(PCWSTR::null(), w!("LG Monitor Control")) {
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
            let _ = CloseHandle(handle);
        }
        None
    } else {
        Some(SingleInstanceGuard(Some(handle)))
    }
}

#[cfg(not(windows))]
pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
    Some(SingleInstanceGuard)
}

pub struct SystemIntegration {
    _tray_icon: Option<TrayIcon>,
    hotkey_manager: Option<GlobalHotKeyManager>,
    bindings: Vec<(HotKey, SystemAction)>,
    active_hotkeys: HashMap<u32, SystemAction>,
    warning: Option<String>,
}

impl SystemIntegration {
    pub fn new(enable_hotkeys: bool) -> Self {
        let mut warnings = Vec::new();
        let tray_icon = match create_tray_icon() {
            Ok(icon) => Some(icon),
            Err(error) => {
                warnings.push(format!("System tray is unavailable: {error}"));
                None
            }
        };

        let hotkey_manager = match GlobalHotKeyManager::new() {
            Ok(manager) => Some(manager),
            Err(error) => {
                warnings.push(format!("Global hotkeys are unavailable: {error}"));
                None
            }
        };

        let bindings = vec![
            (
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM),
                SystemAction::ToggleWindow,
            ),
            (
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Digit1),
                SystemAction::SwitchSlot(0),
            ),
            (
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Digit2),
                SystemAction::SwitchSlot(1),
            ),
            (
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Digit3),
                SystemAction::SwitchSlot(2),
            ),
            (
                HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Digit4),
                SystemAction::SwitchSlot(3),
            ),
        ];

        let mut integration = Self {
            _tray_icon: tray_icon,
            hotkey_manager,
            bindings,
            active_hotkeys: HashMap::new(),
            warning: (!warnings.is_empty()).then(|| warnings.join(" ")),
        };

        if enable_hotkeys && let Err(error) = integration.set_hotkeys_enabled(true) {
            integration.warning = Some(match integration.warning.take() {
                Some(existing) => format!("{existing} {error}"),
                None => error,
            });
        }

        integration
    }

    pub fn poll_actions(&self) -> Vec<SystemAction> {
        let mut actions = Vec::new();

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = menu_action(&event.id.0) {
                actions.push(action);
            }
        }

        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                actions.push(SystemAction::ToggleWindow);
            }
        }

        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state == HotKeyState::Pressed
                && let Some(action) = self.active_hotkeys.get(&event.id)
            {
                actions.push(*action);
            }
        }

        actions
    }

    pub fn set_hotkeys_enabled(&mut self, enabled: bool) -> Result<(), String> {
        let Some(manager) = &self.hotkey_manager else {
            return Err("Global hotkey manager could not be initialized.".into());
        };

        if !enabled {
            for (hotkey, _) in &self.bindings {
                if self.active_hotkeys.contains_key(&hotkey.id()) {
                    let _ = manager.unregister(*hotkey);
                }
            }
            self.active_hotkeys.clear();
            return Ok(());
        }

        self.active_hotkeys.clear();
        let mut failed = Vec::new();
        for (hotkey, action) in &self.bindings {
            match manager.register(*hotkey) {
                Ok(()) => {
                    self.active_hotkeys.insert(hotkey.id(), *action);
                }
                Err(error) => failed.push(format!("{hotkey}: {error}")),
            }
        }

        if failed.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Some shortcuts are already in use: {}",
                failed.join(", ")
            ))
        }
    }

    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
    }
}

#[cfg(windows)]
fn native_window_handle(frame: &eframe::Frame) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;

    let window_handle = frame.window_handle().ok()?;
    let RawWindowHandle::Win32(window_handle) = window_handle.as_raw() else {
        return None;
    };
    Some(HWND(window_handle.hwnd.get() as *mut _))
}

#[cfg(windows)]
pub fn is_window_visible(frame: &eframe::Frame) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    native_window_handle(frame).is_none_or(|hwnd| unsafe { IsWindowVisible(hwnd).as_bool() })
}

#[cfg(not(windows))]
pub fn is_window_visible(_frame: &eframe::Frame) -> bool {
    true
}

#[cfg(windows)]
pub fn apply_dark_window_chrome(frame: &eframe::Frame) -> bool {
    use std::mem::size_of;

    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    };
    use windows::core::BOOL;

    let Some(hwnd) = native_window_handle(frame) else {
        return false;
    };

    let dark_mode = BOOL(1);
    let caption_color = 0x0018_0D0A_u32;
    let text_color = 0x00FA_F2F0_u32;
    let border_color = 0x0058_3C37_u32;

    let results = unsafe {
        [
            (
                "immersive dark mode",
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    std::ptr::from_ref(&dark_mode).cast(),
                    size_of::<BOOL>() as u32,
                ),
            ),
            (
                "caption color",
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_CAPTION_COLOR,
                    std::ptr::from_ref(&caption_color).cast(),
                    size_of::<u32>() as u32,
                ),
            ),
            (
                "text color",
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_TEXT_COLOR,
                    std::ptr::from_ref(&text_color).cast(),
                    size_of::<u32>() as u32,
                ),
            ),
            (
                "border color",
                DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_BORDER_COLOR,
                    std::ptr::from_ref(&border_color).cast(),
                    size_of::<u32>() as u32,
                ),
            ),
        ]
    };

    for (attribute, result) in results {
        match result {
            Ok(()) => tracing::debug!(attribute, "Applied DWM window attribute"),
            Err(error) => {
                tracing::debug!(attribute, %error, "DWM window attribute is unavailable")
            }
        }
    }

    true
}

#[cfg(not(windows))]
pub fn apply_dark_window_chrome(_frame: &eframe::Frame) -> bool {
    true
}

fn menu_action(id: &str) -> Option<SystemAction> {
    match id {
        "open" => Some(SystemAction::ShowWindow),
        "refresh" => Some(SystemAction::Refresh),
        "quit" => Some(SystemAction::Quit),
        _ => id
            .strip_prefix("switch-slot-")
            .and_then(|value| value.parse().ok())
            .map(SystemAction::SwitchSlot),
    }
}

fn create_tray_icon() -> Result<TrayIcon, Box<dyn std::error::Error>> {
    let menu = Menu::new();
    let open = MenuItem::with_id("open", "Open LG Monitor Control", true, None);
    let refresh = MenuItem::with_id("refresh", "Refresh monitors", true, None);
    let quick_switch = Submenu::with_id("quick-switch", "Quick switch", true);
    let slot_one = MenuItem::with_id("switch-slot-0", "Input 1    Ctrl+Alt+1", true, None);
    let slot_two = MenuItem::with_id("switch-slot-1", "Input 2    Ctrl+Alt+2", true, None);
    let slot_three = MenuItem::with_id("switch-slot-2", "Input 3    Ctrl+Alt+3", true, None);
    let slot_four = MenuItem::with_id("switch-slot-3", "Input 4    Ctrl+Alt+4", true, None);
    let separator = PredefinedMenuItem::separator();
    let quit = MenuItem::with_id("quit", "Quit", true, None);

    quick_switch.append_items(&[&slot_one, &slot_two, &slot_three, &slot_four])?;
    menu.append_items(&[&open, &refresh, &quick_switch, &separator, &quit])?;

    let size = 32;
    let icon = Icon::from_rgba(brand_icon_rgba(size), size, size)?;
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("LG Monitor Control")
        .with_icon(icon)
        .build()
        .map_err(Into::into)
}

pub fn brand_icon_rgba(size: u32) -> Vec<u8> {
    let mut pixels = vec![0; (size * size * 4) as usize];
    let scale = size as f32;

    for y in 0..size {
        for x in 0..size {
            let nx = (x as f32 + 0.5) / scale;
            let ny = (y as f32 + 0.5) / scale;
            let rounded_square = rounded_rect_contains(nx, ny, 0.06, 0.06, 0.94, 0.94, 0.22);
            let index = ((y * size + x) * 4) as usize;

            if rounded_square {
                let blend = (nx * 0.45 + ny * 0.55).clamp(0.0, 1.0);
                pixels[index] = (96.0 + 34.0 * blend) as u8;
                pixels[index + 1] = (52.0 + 36.0 * blend) as u8;
                pixels[index + 2] = (210.0 + 32.0 * (1.0 - blend)) as u8;
                pixels[index + 3] = 255;
            }

            let screen_outer = rounded_rect_contains(nx, ny, 0.18, 0.22, 0.82, 0.67, 0.08);
            let screen_inner = rounded_rect_contains(nx, ny, 0.235, 0.275, 0.765, 0.615, 0.045);
            let stand = (0.46..=0.54).contains(&nx) && (0.66..=0.78).contains(&ny);
            let base = (0.35..=0.65).contains(&nx) && (0.76..=0.82).contains(&ny);

            if (screen_outer && !screen_inner) || stand || base {
                pixels[index] = 222;
                pixels[index + 1] = 249;
                pixels[index + 2] = 255;
                pixels[index + 3] = 255;
            } else if screen_inner {
                pixels[index] = 18;
                pixels[index + 1] = 25;
                pixels[index + 2] = 48;
                pixels[index + 3] = 255;
            }
        }
    }

    pixels
}

fn rounded_rect_contains(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> bool {
    let closest_x = x.clamp(left + radius, right - radius);
    let closest_y = y.clamp(top + radius, bottom - radius);
    let dx = x - closest_x;
    let dy = y - closest_y;
    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_tray_menu_ids_to_actions() {
        assert_eq!(menu_action("open"), Some(SystemAction::ShowWindow));
        assert_eq!(menu_action("refresh"), Some(SystemAction::Refresh));
        assert_eq!(menu_action("quit"), Some(SystemAction::Quit));
        assert_eq!(
            menu_action("switch-slot-3"),
            Some(SystemAction::SwitchSlot(3))
        );
        assert_eq!(menu_action("unknown"), None);
    }
}
