use super::*;

impl GitCometView {
    pub(super) fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        self.ui_settings_persist_seq = self.ui_settings_persist_seq.wrapping_add(1);
        let seq = self.ui_settings_persist_seq;

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| {
                Timer::after(Duration::from_millis(250)).await;
                let settings = view
                    .update(cx, |this, cx| {
                        if this.ui_settings_persist_seq != seq {
                            return None;
                        }

                        let ww: f32 = this.last_window_size.width.round().into();
                        let wh: f32 = this.last_window_size.height.round().into();
                        let window_width = (ww.is_finite() && ww >= 1.0).then_some(ww as u32);
                        let window_height = (wh.is_finite() && wh >= 1.0).then_some(wh as u32);

                        let sidebar_width: f32 = this.sidebar_width.round().into();
                        let details_width: f32 = this.details_width.round().into();

                        let (history_show_author, history_show_date, history_show_sha) = this
                            .main_pane
                            .read(cx)
                            .history_visible_column_preferences(cx);

                        let settings = session::UiSettings {
                            window_width,
                            window_height,
                            sidebar_width: (sidebar_width.is_finite() && sidebar_width >= 1.0)
                                .then_some(sidebar_width as u32),
                            details_width: (details_width.is_finite() && details_width >= 1.0)
                                .then_some(details_width as u32),
                            theme_mode: Some(this.theme_mode.key().to_string()),
                            date_time_format: Some(this.date_time_format.key().to_string()),
                            timezone: Some(this.timezone.key()),
                            show_timezone: Some(this.show_timezone),
                            history_show_author: Some(history_show_author),
                            history_show_date: Some(history_show_date),
                            history_show_sha: Some(history_show_sha),
                        };

                        Some(settings)
                    })
                    .ok()
                    .flatten();

                let Some(settings) = settings else {
                    return;
                };

                let _ = smol::unblock(move || session::persist_ui_settings(settings)).await;
            },
        )
        .detach();
    }

    pub(super) fn clamp_pane_widths_to_window(&mut self) {
        let total_w = self.last_window_size.width;
        if total_w.is_zero() {
            return;
        }

        let handles_w = px(PANE_RESIZE_HANDLE_PX) * 2.0;
        let main_min = px(MAIN_MIN_PX);
        let sidebar_min = px(SIDEBAR_MIN_PX);
        let details_min = px(DETAILS_MIN_PX);

        let max_sidebar = (total_w - self.details_width - main_min - handles_w).max(sidebar_min);
        self.sidebar_width = self.sidebar_width.max(sidebar_min).min(max_sidebar);

        let max_details = (total_w - self.sidebar_width - main_min - handles_w).max(details_min);
        self.details_width = self.details_width.max(details_min).min(max_details);
    }
}
