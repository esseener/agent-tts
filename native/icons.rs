use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

/// Keep the title bar and taskbar at their physical pixel sizes. Windows must
/// not enlarge the 16px title icon for a 32/40/48px taskbar button.
#[derive(Default)]
pub struct WindowIcons {
    large: HICON,
    small: HICON,
    dpi: u32,
}

impl WindowIcons {
    pub unsafe fn refresh(&mut self, window: HWND, instance: HINSTANCE) {
        let dpi = GetDpiForWindow(window).max(96);
        if self.dpi == dpi {
            return;
        }
        let load = |metric| {
            LoadImageW(
                instance,
                1usize as *const u16,
                IMAGE_ICON,
                GetSystemMetricsForDpi(metric, dpi),
                GetSystemMetricsForDpi(metric + 1, dpi),
                0,
            ) as HICON
        };
        let large = load(SM_CXICON);
        let small = load(SM_CXSMICON);
        if large.is_null() || small.is_null() {
            if !large.is_null() {
                DestroyIcon(large);
            }
            if !small.is_null() {
                DestroyIcon(small);
            }
            return;
        }
        SendMessageW(window, WM_SETICON, ICON_BIG as usize, large as isize);
        SendMessageW(window, WM_SETICON, ICON_SMALL as usize, small as isize);
        let previous = std::mem::replace(self, Self { large, small, dpi });
        drop(previous);
    }
}

impl Drop for WindowIcons {
    fn drop(&mut self) {
        unsafe {
            if self.large != null_mut() {
                DestroyIcon(self.large);
            }
            if self.small != null_mut() {
                DestroyIcon(self.small);
            }
        }
    }
}
