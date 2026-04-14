use std::ptr::null;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnableMenuItem, GWL_STYLE, GetSystemMenu, GetWindowLongPtrW, HMENU, IsIconic, IsZoomed,
    MENU_ITEM_FLAGS, MF_BYCOMMAND, MF_ENABLED, MF_GRAYED, PostMessageW, SC_CLOSE, SC_MAXIMIZE,
    SC_MINIMIZE, SC_MOVE, SC_RESTORE, SC_SIZE, SW_RESTORE, SetForegroundWindow, ShowWindowAsync,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenuEx, WINDOW_STYLE,
    WM_NULL, WM_SYSCOMMAND, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

/// Restore a Win32 window from the maximized state.
pub fn restore_window(hwnd: isize) -> bool {
    let hwnd = hwnd as HWND;
    unsafe { ShowWindowAsync(hwnd, SW_RESTORE) != 0 }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct SystemMenuState {
    restore: bool,
    move_window: bool,
    size: bool,
    minimize: bool,
    maximize: bool,
    close: bool,
}

fn has_style(style: WINDOW_STYLE, flag: WINDOW_STYLE) -> bool {
    style & flag == flag
}

fn system_menu_state(
    style: WINDOW_STYLE,
    is_minimized: bool,
    is_maximized: bool,
) -> SystemMenuState {
    let has_system_menu = has_style(style, WS_SYSMENU);
    let is_restored = !is_minimized && !is_maximized;
    let can_resize = has_style(style, WS_THICKFRAME);
    let has_minimize = has_style(style, WS_MINIMIZEBOX);
    let has_maximize = has_style(style, WS_MAXIMIZEBOX);

    SystemMenuState {
        restore: has_system_menu && !is_restored,
        move_window: has_system_menu && is_restored,
        size: has_system_menu && can_resize && is_restored,
        minimize: has_system_menu && has_minimize && !is_minimized,
        maximize: has_system_menu && has_maximize && !is_maximized,
        close: has_system_menu,
    }
}

fn enable_menu_item(menu: HMENU, command: u32, enabled: bool) {
    let flags: MENU_ITEM_FLAGS = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
    unsafe {
        let _ = EnableMenuItem(menu, command, flags);
    }
}

fn sync_system_menu_state(hwnd: HWND, menu: HMENU) {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as WINDOW_STYLE };
    let state = system_menu_state(style, unsafe { IsIconic(hwnd) != 0 }, unsafe {
        IsZoomed(hwnd) != 0
    });

    enable_menu_item(menu, SC_RESTORE, state.restore);
    enable_menu_item(menu, SC_MOVE, state.move_window);
    enable_menu_item(menu, SC_SIZE, state.size);
    enable_menu_item(menu, SC_MINIMIZE, state.minimize);
    enable_menu_item(menu, SC_MAXIMIZE, state.maximize);
    enable_menu_item(menu, SC_CLOSE, state.close);
}

/// Show the native Win32 system menu for a window at the given client-area position.
pub fn show_window_system_menu(hwnd: isize, x: i32, y: i32) {
    let hwnd = hwnd as HWND;
    let mut position = POINT { x, y };

    unsafe {
        if ClientToScreen(hwnd, &mut position) == 0 {
            return;
        }

        let menu = GetSystemMenu(hwnd, 0);
        if menu.is_null() {
            return;
        }

        sync_system_menu_state(hwnd, menu);
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenuEx(
            menu,
            TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            position.x,
            position.y,
            hwnd,
            null(),
        ) as usize;

        let _ = PostMessageW(hwnd, WM_NULL, WPARAM::default(), LPARAM::default());
        if command != 0 {
            let _ = PostMessageW(hwnd, WM_SYSCOMMAND, command as WPARAM, LPARAM::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_window_menu_state_enables_move_size_minimize_and_maximize() {
        let style = WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let state = system_menu_state(style, false, false);

        assert_eq!(
            state,
            SystemMenuState {
                restore: false,
                move_window: true,
                size: true,
                minimize: true,
                maximize: true,
                close: true,
            }
        );
    }

    #[test]
    fn maximized_window_menu_state_enables_restore_and_minimize_only() {
        let style = WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let state = system_menu_state(style, false, true);

        assert_eq!(
            state,
            SystemMenuState {
                restore: true,
                move_window: false,
                size: false,
                minimize: true,
                maximize: false,
                close: true,
            }
        );
    }

    #[test]
    fn minimized_window_menu_state_enables_restore_and_maximize_only() {
        let style = WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
        let state = system_menu_state(style, true, false);

        assert_eq!(
            state,
            SystemMenuState {
                restore: true,
                move_window: false,
                size: false,
                minimize: false,
                maximize: true,
                close: true,
            }
        );
    }
}
