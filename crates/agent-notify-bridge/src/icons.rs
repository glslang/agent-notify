//! Tray icon assets: the per-state `.ico` files are embedded at build time and
//! decoded to `tray_icon::Icon`s at startup, picking the light or dark set to
//! match the Windows system theme.

use crate::worker::IconState;
use anyhow::Context;
use tray_icon::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

/// Raw `.ico` bytes for one theme, one per [`IconState`].
struct ThemeBytes {
    idle: &'static [u8],
    running: &'static [u8],
    waiting: &'static [u8],
    done: &'static [u8],
    failed: &'static [u8],
    paused: &'static [u8],
}

const DARK: ThemeBytes = ThemeBytes {
    idle: include_bytes!("../assets/ico/dark/agent-notify-idle.ico"),
    running: include_bytes!("../assets/ico/dark/agent-notify-running.ico"),
    waiting: include_bytes!("../assets/ico/dark/agent-notify-waiting.ico"),
    done: include_bytes!("../assets/ico/dark/agent-notify-done.ico"),
    failed: include_bytes!("../assets/ico/dark/agent-notify-failed.ico"),
    paused: include_bytes!("../assets/ico/dark/agent-notify-paused.ico"),
};

const LIGHT: ThemeBytes = ThemeBytes {
    idle: include_bytes!("../assets/ico/light/agent-notify-idle.ico"),
    running: include_bytes!("../assets/ico/light/agent-notify-running.ico"),
    waiting: include_bytes!("../assets/ico/light/agent-notify-waiting.ico"),
    done: include_bytes!("../assets/ico/light/agent-notify-done.ico"),
    failed: include_bytes!("../assets/ico/light/agent-notify-failed.ico"),
    paused: include_bytes!("../assets/ico/light/agent-notify-paused.ico"),
};

/// The decoded tray icon for every [`IconState`], built once at startup.
pub struct IconSet {
    idle: Icon,
    running: Icon,
    waiting: Icon,
    done: Icon,
    failed: Icon,
    paused: Icon,
}

impl IconSet {
    /// Clone is cheap: `tray_icon::Icon` is reference-counted internally.
    pub fn get(&self, state: IconState) -> Icon {
        match state {
            IconState::Idle => self.idle.clone(),
            IconState::Running => self.running.clone(),
            IconState::Waiting => self.waiting.clone(),
            IconState::Done => self.done.clone(),
            IconState::Failed => self.failed.clone(),
            IconState::Paused => self.paused.clone(),
        }
    }
}

/// Decode the largest image in an `.ico` to a `tray_icon::Icon`. The tray scales
/// it to the system tray size, so the highest-resolution entry looks best.
fn decode(bytes: &'static [u8]) -> anyhow::Result<Icon> {
    let dir = ico::IconDir::read(std::io::Cursor::new(bytes)).context("failed to read .ico")?;
    let entry = dir
        .entries()
        .iter()
        .max_by_key(|entry| entry.width() * entry.height())
        .context(".ico has no image entries")?;
    let image = entry.decode().context("failed to decode .ico entry")?;
    Icon::from_rgba(image.rgba_data().to_vec(), image.width(), image.height())
        .context("failed to build tray icon from rgba")
}

pub fn load_icon_set(theme: Theme) -> anyhow::Result<IconSet> {
    let bytes = match theme {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    };
    Ok(IconSet {
        idle: decode(bytes.idle)?,
        running: decode(bytes.running)?,
        waiting: decode(bytes.waiting)?,
        done: decode(bytes.done)?,
        failed: decode(bytes.failed)?,
        paused: decode(bytes.paused)?,
    })
}

/// Detect the active tray theme from the Windows registry. `SystemUsesLightTheme`
/// is `1` in light mode and `0` in dark mode; a missing value falls back to dark,
/// the Windows default. Read once at startup (a trayless winit loop receives no
/// live theme-change events).
pub fn detect_theme() -> Theme {
    match read_system_uses_light_theme() {
        Some(1) => Theme::Light,
        _ => Theme::Dark,
    }
}

fn read_system_uses_light_theme() -> Option<u32> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut data: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            std::ptr::from_mut(&mut data).cast(),
            &mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

/// UTF-16, null-terminated, for the wide Win32 registry APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_icon_sets_decode() {
        // Every embedded .ico must decode and build a tray icon, or the tray
        // would fail to start.
        load_icon_set(Theme::Dark).expect("dark icon set decodes");
        load_icon_set(Theme::Light).expect("light icon set decodes");
    }

    #[test]
    fn detect_theme_does_not_panic() {
        let _ = detect_theme();
    }
}
