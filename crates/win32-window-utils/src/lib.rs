use std::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, ShowWindowAsync};

/// Restore a Win32 window from the maximized state.
pub fn restore_window(hwnd: isize) -> bool {
    let hwnd = HWND(hwnd as *mut c_void);
    unsafe { ShowWindowAsync(hwnd, SW_RESTORE).as_bool() }
}
