use super::*;

pub(super) const SURVEY_OPEN_LABEL: &str = "Open Survey";
pub(super) const SURVEY_POSTPONE_LABEL: &str = "Later";
pub(super) const SURVEY_POSTPONE_SECONDS: u64 = 60 * 60 * 24 * 7;

impl GitCometView {
    pub(in crate::view) fn maybe_show_user_survey_on_startup(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        // The identity names the survey; a product without one never prompts.
        let Some(survey) = gitcomet_core::identity::current().links().survey.as_ref() else {
            return;
        };
        if self.view_mode != GitCometViewMode::Normal
            || !session::should_show_survey_prompt(&survey.id)
        {
            return;
        }

        let survey_name = format!("{} User Survey", crate::view::product_name());
        self.toast_host.update(cx, |host, cx| {
            host.push_survey_toast(
                &survey.id,
                &survey_name,
                &survey.message,
                &survey.url,
                SURVEY_OPEN_LABEL,
                SURVEY_POSTPONE_LABEL,
                SURVEY_POSTPONE_SECONDS,
                cx,
            );
        });
    }
}
