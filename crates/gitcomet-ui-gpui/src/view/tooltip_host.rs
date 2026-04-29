use super::*;

pub(crate) struct TooltipHost {
    theme: AppTheme,

    tooltip_text: Option<SharedString>,
    tooltip_candidate_last: Option<SharedString>,
    tooltip_visible_text: Option<SharedString>,
    tooltip_pending_pos: Option<Point<Pixels>>,
    tooltip_visible_pos: Option<Point<Pixels>>,
    tooltip_delay_seq: u64,
    last_mouse_pos: Point<Pixels>,
}

impl TooltipHost {
    pub(crate) fn new(theme: AppTheme) -> Self {
        Self {
            theme,
            tooltip_text: None,
            tooltip_candidate_last: None,
            tooltip_visible_text: None,
            tooltip_pending_pos: None,
            tooltip_visible_pos: None,
            tooltip_delay_seq: 0,
            last_mouse_pos: point(px(0.0), px(0.0)),
        }
    }

    pub(crate) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(crate) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.tooltip_text == next {
            return false;
        }

        self.tooltip_text = next;
        self.sync_tooltip_state(cx);
        cx.notify();
        true
    }

    pub(crate) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.tooltip_text.as_ref() != Some(tooltip) {
            return false;
        }

        self.tooltip_text = None;
        self.sync_tooltip_state(cx);
        cx.notify();
        true
    }

    pub(crate) fn clear_tooltip(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let had_tooltip = self.tooltip_text.is_some()
            || self.tooltip_candidate_last.is_some()
            || self.tooltip_visible_text.is_some()
            || self.tooltip_pending_pos.is_some()
            || self.tooltip_visible_pos.is_some();
        if !had_tooltip {
            return false;
        }

        self.tooltip_text = None;
        self.tooltip_candidate_last = None;
        self.tooltip_visible_text = None;
        self.tooltip_pending_pos = None;
        self.tooltip_visible_pos = None;
        self.tooltip_delay_seq = self.tooltip_delay_seq.wrapping_add(1);
        cx.notify();
        true
    }

    pub(crate) fn on_mouse_moved(&mut self, pos: Point<Pixels>, cx: &mut gpui::Context<Self>) {
        self.last_mouse_pos = pos;
        self.maybe_restart_tooltip_delay(cx);
    }

    fn sync_tooltip_state(&mut self, cx: &mut gpui::Context<Self>) {
        if self.tooltip_text == self.tooltip_candidate_last {
            return;
        }

        self.tooltip_candidate_last = self.tooltip_text.clone();
        self.tooltip_visible_text = None;
        self.tooltip_visible_pos = None;
        self.tooltip_pending_pos = None;
        self.tooltip_delay_seq = self.tooltip_delay_seq.wrapping_add(1);

        let Some(text) = self.tooltip_text.clone() else {
            return;
        };

        let anchor = self.last_mouse_pos;
        self.tooltip_pending_pos = Some(anchor);

        if !crate::ui_runtime::current().uses_tooltip_delay() {
            self.tooltip_visible_text = Some(text);
            self.tooltip_visible_pos = Some(anchor);
            return;
        }

        let seq = self.tooltip_delay_seq;

        cx.spawn(
            async move |view: WeakEntity<TooltipHost>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(500)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.tooltip_delay_seq != seq {
                        return;
                    }
                    if this.tooltip_text.as_ref() != Some(&text) {
                        return;
                    }
                    let Some(pending_pos) = this.tooltip_pending_pos else {
                        return;
                    };
                    let dx = (this.last_mouse_pos.x - pending_pos.x).abs();
                    let dy = (this.last_mouse_pos.y - pending_pos.y).abs();
                    if dx > px(2.0) || dy > px(2.0) {
                        return;
                    }
                    this.tooltip_visible_text = Some(text.clone());
                    this.tooltip_visible_pos = Some(pending_pos);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn maybe_restart_tooltip_delay(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(candidate) = self.tooltip_text.clone() else {
            if self.tooltip_visible_text.is_some() {
                self.tooltip_visible_text = None;
                self.tooltip_visible_pos = None;
                cx.notify();
            }
            return;
        };

        if let Some(visible_anchor) = self.tooltip_visible_pos {
            let dx = (self.last_mouse_pos.x - visible_anchor.x).abs();
            let dy = (self.last_mouse_pos.y - visible_anchor.y).abs();
            if dx <= px(6.0) && dy <= px(6.0) {
                return;
            }
        }

        let should_restart = match self.tooltip_pending_pos {
            None => true,
            Some(pending_anchor) => {
                let dx = (self.last_mouse_pos.x - pending_anchor.x).abs();
                let dy = (self.last_mouse_pos.y - pending_anchor.y).abs();
                dx > px(2.0) || dy > px(2.0)
            }
        };

        if !should_restart {
            return;
        }

        self.tooltip_visible_text = None;
        self.tooltip_visible_pos = None;
        self.tooltip_pending_pos = Some(self.last_mouse_pos);
        self.tooltip_delay_seq = self.tooltip_delay_seq.wrapping_add(1);

        if !crate::ui_runtime::current().uses_tooltip_delay() {
            self.tooltip_visible_text = Some(candidate);
            self.tooltip_visible_pos = self.tooltip_pending_pos;
            cx.notify();
            return;
        }

        let seq = self.tooltip_delay_seq;

        cx.spawn(
            async move |view: WeakEntity<TooltipHost>, cx: &mut gpui::AsyncApp| {
                smol::Timer::after(Duration::from_millis(500)).await;
                let _ = view.update(cx, |this, cx| {
                    if this.tooltip_delay_seq != seq {
                        return;
                    }
                    if this.tooltip_text.as_ref() != Some(&candidate) {
                        return;
                    }
                    let Some(pending_pos) = this.tooltip_pending_pos else {
                        return;
                    };
                    let dx = (this.last_mouse_pos.x - pending_pos.x).abs();
                    let dy = (this.last_mouse_pos.y - pending_pos.y).abs();
                    if dx > px(2.0) || dy > px(2.0) {
                        return;
                    }
                    this.tooltip_visible_text = Some(candidate.clone());
                    this.tooltip_visible_pos = Some(pending_pos);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    #[cfg(test)]
    pub(crate) fn tooltip_text_for_test(&self) -> Option<SharedString> {
        self.tooltip_text.clone()
    }
}

impl Render for TooltipHost {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.sync_tooltip_state(cx);

        let theme = self.theme;
        let mut layer = div()
            .id("tooltip_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full();

        if let Some(text) = self.tooltip_visible_text.clone() {
            let tooltip_bg = theme.colors.tooltip_bg;
            let tooltip_text_color = theme.colors.tooltip_text;
            let anchor = self.tooltip_visible_pos.unwrap_or(self.last_mouse_pos);
            let pos = point(anchor.x + px(12.0), anchor.y + px(18.0));

            layer = layer.child(
                anchored()
                    .position(pos)
                    .anchor(Corner::TopLeft)
                    .offset(point(px(0.0), px(0.0)))
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .bg(tooltip_bg)
                            .rounded(px(theme.radii.row))
                            .shadow_sm()
                            .text_xs()
                            .text_color(tooltip_text_color)
                            .child(text),
                    ),
            );
        }

        layer
    }
}
