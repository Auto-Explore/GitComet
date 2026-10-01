//! Window chrome platform glue: zoom, the system menu, and native move/resize.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowZoomAction {
    Zoom,
    Restore,
}

pub(crate) fn window_zoom_action(is_maximized: bool) -> WindowZoomAction {
    if cfg!(target_os = "windows") && is_maximized {
        WindowZoomAction::Restore
    } else {
        WindowZoomAction::Zoom
    }
}

pub(crate) fn toggle_window_zoom(window: &Window) {
    match window_zoom_action(window.is_maximized()) {
        WindowZoomAction::Zoom => window.zoom_window(),
        WindowZoomAction::Restore => {
            #[cfg(target_os = "windows")]
            if restore_maximized_window(window) {
                return;
            }

            window.zoom_window();
        }
    }
}

pub(crate) fn show_window_system_menu(window: &Window, position: Point<Pixels>) {
    #[cfg(target_os = "windows")]
    if show_windows_window_system_menu(window, position) {
        return;
    }

    window.show_window_menu(position);
}

pub(crate) fn application() -> gpui::Application {
    gpui_platform::application()
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn window_menu_position(position: Point<Pixels>, scale_factor: f32) -> (i32, i32) {
    (
        (f32::from(position.x) * scale_factor).round() as i32,
        (f32::from(position.y) * scale_factor).round() as i32,
    )
}

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WindowSystemMenuRequest {
    pub hwnd: isize,
    pub x: i32,
    pub y: i32,
}

#[cfg(target_os = "windows")]
pub(super) fn window_hwnd(window: &Window) -> Option<isize> {
    let Ok(handle) = raw_window_handle::HasWindowHandle::window_handle(window) else {
        return None;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };

    Some(handle.hwnd.get())
}

thread_local! {
    /// When we last asked the compositor/window manager for an interactive move
    /// or resize grab.
    static LAST_WINDOW_GRAB_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Record that we just requested an interactive move/resize grab.
///
/// The grab is executed by the compositor/WM, which takes the input focus for
/// the duration of the drag. GPUI surfaces that as a plain window deactivate →
/// activate pair — on Wayland via `wl_keyboard` Leave/Enter, on X11 via
/// FocusOut/FocusIn (which are not filtered on `mode`, so NotifyGrab and
/// NotifyUngrab arrive too) — indistinguishable from the user alt-tabbing away
/// and back. This marker lets the activation observer ignore its own echo
/// instead of treating it as a return to the app and refreshing the repo.
pub(crate) fn note_window_grab_started() {
    LAST_WINDOW_GRAB_AT.with(|cell| cell.set(Some(Instant::now())));
}

/// Whether a grab was requested no more than `max_age` ago. Always consumes the
/// marker, so a grab the compositor silently dropped cannot arm suppression for
/// an unrelated activation minutes later.
pub(crate) fn take_window_grab_started_within(now: Instant, max_age: Duration) -> bool {
    LAST_WINDOW_GRAB_AT.with(|cell| match cell.take() {
        Some(at) => now.saturating_duration_since(at) <= max_age,
        None => false,
    })
}

/// Hand the title-bar drag to the platform. On Windows this goes through GPUI's
/// `start_window_move` (a posted `WM_NCLBUTTONDOWN`/`HTCAPTION`) rather than the
/// `SC_MOVE` system command GitComet used to post itself: GPUI tracks that drag
/// and synthesizes the `WM_LBUTTONUP` the modal move loop swallows, so the app
/// sees a complete press/release pair after every move, and the native
/// restore-on-drag for maximized windows works the same either way.
pub(crate) fn begin_window_move(window: &Window) {
    note_window_grab_started();
    window.start_window_move();
}

pub(crate) fn begin_window_resize(window: &Window, edge: gpui::ResizeEdge) {
    note_window_grab_started();
    window.start_window_resize(edge);
}

#[cfg(target_os = "windows")]
pub(super) fn restore_maximized_window(window: &Window) -> bool {
    let Some(hwnd) = window_hwnd(window) else {
        return false;
    };

    // GPUI's Windows zoom path currently maps directly to SW_MAXIMIZE, so
    // restore must go through the native Win32 API until upstream toggles.
    gitcomet_win32_window_utils::restore_window(hwnd)
}

#[cfg(target_os = "windows")]
pub(super) fn show_windows_window_system_menu(window: &Window, position: Point<Pixels>) -> bool {
    let Some(request) = window_system_menu_request(window, position) else {
        return false;
    };

    gitcomet_win32_window_utils::show_window_system_menu(request.hwnd, request.x, request.y);
    true
}

#[cfg(target_os = "windows")]
pub(crate) fn window_system_menu_request(
    window: &Window,
    position: Point<Pixels>,
) -> Option<WindowSystemMenuRequest> {
    let (x, y) = window_menu_position(position, window.scale_factor());
    let hwnd = window_hwnd(window)?;
    Some(WindowSystemMenuRequest { hwnd, x, y })
}
