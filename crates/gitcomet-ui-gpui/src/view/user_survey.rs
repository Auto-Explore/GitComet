use super::*;

pub(super) const SURVEY_ID: &str = "gitcomet_user_survey_2026_04";
pub(super) const SURVEY_NAME: &str = "GitComet User Survey";
pub(super) const SURVEY_MESSAGE: &str = "Help shape GitComet by taking a short user survey.";
pub(super) const SURVEY_URL: &str = "https://docs.google.com/forms/d/e/1FAIpQLSd8DKIl222UomSXrpv1q9rWodRlBSQo9pJDD62GbZEANTgD1A/viewform";
pub(super) const SURVEY_OPEN_LABEL: &str = "Open Survey";
pub(super) const SURVEY_POSTPONE_LABEL: &str = "Later";
pub(super) const SURVEY_POSTPONE_SECONDS: u64 = 60 * 60 * 24 * 7;

impl GitCometView {
    pub(in crate::view) fn maybe_show_user_survey_on_startup(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.view_mode != GitCometViewMode::Normal
            || !session::should_show_survey_prompt(SURVEY_ID)
        {
            return;
        }

        self.toast_host.update(cx, |host, cx| {
            host.push_survey_toast(
                SURVEY_ID,
                SURVEY_NAME,
                SURVEY_MESSAGE,
                SURVEY_URL,
                SURVEY_OPEN_LABEL,
                SURVEY_POSTPONE_LABEL,
                SURVEY_POSTPONE_SECONDS,
                cx,
            );
        });
    }
}
