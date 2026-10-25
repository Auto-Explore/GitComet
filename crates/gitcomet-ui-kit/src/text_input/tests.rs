use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::wrap::*;
use super::*;
use crate::text_layout::TextLayoutExt as _;

mod editing_history;
mod focus_events;
mod highlighting;
mod layout;
mod selection_ime;

fn multiline_input(
    cx: &mut gpui::TestAppContext,
) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    cx.add_window_view(|window, cx| {
        window.activate();
        let input = TextInput::new(
            TextInputOptions {
                multiline: true,
                ..Default::default()
            },
            window,
            cx,
        );
        window.focus(&input.focus_handle, cx);
        input
    })
}
