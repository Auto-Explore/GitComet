use super::*;

impl GitCometView {
    pub(in crate::view) fn maybe_show_user_survey_on_startup(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.view_mode != GitCometViewMode::Normal || !session::should_show_user_survey_prompt()
        {
            return;
        }

        self.toast_host
            .update(cx, |host, cx| host.push_user_survey_toast(cx));
    }
}
