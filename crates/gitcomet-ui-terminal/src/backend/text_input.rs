use super::PtySender;
use gpui::{App, Bounds, Pixels, Point, Window};

pub(crate) struct TerminalTextInputHandler {
    focus_handle: gpui::FocusHandle,
    pty_sender: Option<PtySender>,
    marked_text: String,
}

impl TerminalTextInputHandler {
    pub(crate) fn new(focus_handle: gpui::FocusHandle, pty_sender: Option<PtySender>) -> Self {
        Self {
            focus_handle,
            pty_sender,
            marked_text: String::new(),
        }
    }
}

impl gpui::InputHandler for TerminalTextInputHandler {
    fn accepts_text_input(&mut self, window: &mut Window, _cx: &mut App) -> bool {
        gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window)
            && self.pty_sender.is_some()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<std::ops::Range<usize>> {
        (!self.marked_text.is_empty()).then(|| 0..self.marked_text.encode_utf16().count())
    }

    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if !gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window) {
            return;
        }
        if let Some(ref pty) = self.pty_sender {
            self.marked_text.clear();
            pty.write(text.as_bytes().to_vec());
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if !gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window) {
            return;
        }
        self.marked_text.clear();
        self.marked_text.push_str(new_text);
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut App) {
        self.marked_text.clear();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests;
