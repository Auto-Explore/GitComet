use super::TerminalModes;
use gpui::{Pixels, px};

#[derive(Clone, Copy, Debug)]
enum MouseButtonCode {
    LeftButton = 0,
    MiddleButton = 1,
    RightButton = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoneMove = 35,
    ScrollUp = 64,
    ScrollDown = 65,
}

impl MouseButtonCode {
    /// Returns `None` for buttons the mouse protocol does not report (Navigate
    /// back/forward), so they are suppressed rather than reported as left-clicks.
    fn from_button(e: gpui::MouseButton) -> Option<Self> {
        match e {
            gpui::MouseButton::Left => Some(MouseButtonCode::LeftButton),
            gpui::MouseButton::Middle => Some(MouseButtonCode::MiddleButton),
            gpui::MouseButton::Right => Some(MouseButtonCode::RightButton),
            gpui::MouseButton::Navigate(_) => None,
        }
    }

    /// `None` for the held button means "suppress" (Navigate); `Some(NoneMove)`
    /// means no button is held (plain motion).
    fn from_move_button(e: Option<gpui::MouseButton>) -> Option<Self> {
        match e {
            Some(gpui::MouseButton::Left) => Some(MouseButtonCode::LeftMove),
            Some(gpui::MouseButton::Middle) => Some(MouseButtonCode::MiddleMove),
            Some(gpui::MouseButton::Right) => Some(MouseButtonCode::RightMove),
            Some(gpui::MouseButton::Navigate(_)) => None,
            None => Some(MouseButtonCode::NoneMove),
        }
    }

    fn from_scroll(delta_y: Pixels) -> Self {
        if delta_y > px(0.0) {
            MouseButtonCode::ScrollUp
        } else {
            MouseButtonCode::ScrollDown
        }
    }
}

#[derive(Clone, Copy)]
enum MouseFormat {
    Sgr,
    Normal,
}

impl MouseFormat {
    fn from_mode(mode: TerminalModes) -> Self {
        if mode.contains(TerminalModes::SGR_MOUSE) {
            MouseFormat::Sgr
        } else {
            MouseFormat::Normal
        }
    }
}

pub(crate) fn terminal_mouse_button_report(
    grid_row: i32,
    grid_col: usize,
    button: gpui::MouseButton,
    modifiers: gpui::Modifiers,
    pressed: bool,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let code = MouseButtonCode::from_button(button)?;
    mouse_report(
        grid_row,
        grid_col,
        code,
        pressed,
        modifiers,
        MouseFormat::from_mode(mode),
    )
}

pub(crate) fn terminal_mouse_moved_report(
    grid_row: i32,
    grid_col: usize,
    held_button: Option<gpui::MouseButton>,
    modifiers: gpui::Modifiers,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    if !mode.intersects(TerminalModes::MOUSE_MOTION | TerminalModes::MOUSE_DRAG) {
        return None;
    }
    let code = MouseButtonCode::from_move_button(held_button)?;
    if mode.contains(TerminalModes::MOUSE_DRAG) && matches!(code, MouseButtonCode::NoneMove) {
        return None;
    }
    mouse_report(
        grid_row,
        grid_col,
        code,
        true,
        modifiers,
        MouseFormat::from_mode(mode),
    )
}

pub(crate) fn terminal_scroll_report(
    grid_row: i32,
    grid_col: usize,
    modifiers: gpui::Modifiers,
    delta_y: Pixels,
    step_rows: usize,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let code = MouseButtonCode::from_scroll(delta_y);
    let report = mouse_report(
        grid_row,
        grid_col,
        code,
        true,
        modifiers,
        MouseFormat::from_mode(mode),
    )?;
    Some(if step_rows == 1 {
        report
    } else {
        report.repeat(step_rows)
    })
}

fn mouse_report(
    grid_row: i32,
    grid_col: usize,
    button: MouseButtonCode,
    pressed: bool,
    modifiers: gpui::Modifiers,
    format: MouseFormat,
) -> Option<Vec<u8>> {
    if grid_row < 0 {
        return None;
    }
    let mut mods: u8 = 0;
    if modifiers.shift {
        mods += 4;
    }
    if modifiers.alt {
        mods += 8;
    }
    if modifiers.control {
        mods += 16;
    }
    match format {
        MouseFormat::Sgr => {
            let c = if pressed { 'M' } else { 'm' };
            Some(
                format!(
                    "\x1b[<{};{};{}{}",
                    button as u8 + mods,
                    grid_col + 1,
                    grid_row + 1,
                    c
                )
                .into_bytes(),
            )
        }
        MouseFormat::Normal => {
            if pressed {
                normal_mouse_report(grid_row, grid_col, button as u8 + mods)
            } else {
                normal_mouse_report(grid_row, grid_col, 3 + mods)
            }
        }
    }
}

fn normal_mouse_report(grid_row: i32, grid_col: usize, button: u8) -> Option<Vec<u8>> {
    let max_point = 223;
    if grid_row >= max_point || grid_col >= max_point as usize {
        return None;
    }
    let mut msg = vec![b'\x1b', b'[', b'M', 32 + button];
    msg.push(32 + 1 + grid_col as u8);
    msg.push(32 + 1 + grid_row as u8);
    Some(msg)
}

#[cfg(test)]
mod tests;
