use super::*;

impl GitCometView {
    pub(in super::super) fn open_repo_panel(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        if !self.open_repo_panel {
            return div();
        }

        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .bg(theme.colors.surface_bg)
            .border_1()
            .border_color(theme.colors.border)
            .rounded(px(theme.radii.panel))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child("Path"),
            )
            .child(div().flex_1().child(self.open_repo_input.clone()))
            .child(
                components::Button::new("open_repo_go", "Open")
                    .separated_end_slot(popover::hotkey_hint(theme, "open_repo_go_hint", "Enter"))
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.submit_open_repo_panel(cx);
                    }),
            )
            .child(
                popover::cancel_button("open_repo_cancel", "open_repo_cancel_hint", theme)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.open_repo_panel = false;
                        cx.notify();
                    }),
            )
    }

    pub(in super::super) fn submit_open_repo_panel(&mut self, cx: &mut gpui::Context<Self>) {
        let path = self
            .open_repo_input
            .read_with(cx, |input, _| input.text().trim().to_string());
        if !path.is_empty() {
            self.store.dispatch(Msg::OpenRepo(path.into()));
            self.open_repo_panel = false;
        }
        cx.notify();
    }
}
