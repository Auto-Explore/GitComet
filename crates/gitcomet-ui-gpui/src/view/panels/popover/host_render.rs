use super::*;

impl Render for PopoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(kind) = self.popover.clone() else {
            return div().into_any_element();
        };

        let history_refs_menu_active = self.history_refs_menu_active(cx);
        let close = cx.listener(|this, _e: &MouseDownEvent, window, cx| {
            this.close_popover_and_restore_focus(window, cx);
        });

        let popover = self.popover_view(kind, window, cx).into_any_element();
        let is_centered = matches!(self.popover_anchor, Some(PopoverAnchor::Centered));
        let mut layer = div()
            .id("popover_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full();
        if !history_refs_menu_active && !is_centered {
            let scrim = div()
                .id("popover_scrim")
                .debug_selector(|| "repo_popover_close".to_string())
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(gpui::rgba(0x00000000))
                .occlude()
                .on_any_mouse_down(close);
            layer = layer.child(scrim);
        }
        layer = layer.child(popover);
        // Painted after the popover, so it hit-tests above the picker it floats
        // over and its own scrim intercepts the click that would otherwise
        // reach `popover_scrim` and close the whole picker.
        if let Some(row_menu) = picker_row_menu::layer(self, window, cx) {
            layer = layer.child(row_menu);
        }
        layer.into_any_element()
    }
}
