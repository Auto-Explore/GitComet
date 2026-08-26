use super::*;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct SurveyPromptSession {
    pub(super) survey_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) opened_at_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) postponed_until_unix_seconds: Option<u64>,
}

pub fn should_show_survey_prompt(survey_id: &str) -> bool {
    let Some(session_file_path) = default_session_file_path() else {
        return false;
    };
    should_show_survey_prompt_from_path(&session_file_path, survey_id, current_unix_seconds())
}

pub fn should_show_survey_prompt_from_path(
    session_file_path: &Path,
    survey_id: &str,
    now_unix_seconds: u64,
) -> bool {
    let Some(file) = load_file(session_file_path) else {
        return false;
    };
    if !has_recorded_session_repository(&file) {
        return false;
    }

    let Some(prompt) = file.survey_prompt else {
        return true;
    };
    if prompt.survey_id != survey_id {
        return true;
    }
    if prompt.opened_at_unix_seconds.is_some() {
        return false;
    }

    prompt
        .postponed_until_unix_seconds
        .is_none_or(|postponed_until| postponed_until <= now_unix_seconds)
}

pub fn persist_survey_prompt_opened(survey_id: &str) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_survey_prompt_opened_to_path(&session_file_path, survey_id, current_unix_seconds())
}

pub fn persist_survey_prompt_opened_to_path(
    session_file_path: &Path,
    survey_id: &str,
    now_unix_seconds: u64,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        file.survey_prompt = Some(SurveyPromptSession {
            survey_id: survey_id.to_string(),
            opened_at_unix_seconds: Some(now_unix_seconds),
            postponed_until_unix_seconds: None,
        });

        persist_to_path(session_file_path, &file)
    })
}

pub fn persist_survey_prompt_postponed(survey_id: &str, postpone_seconds: u64) -> io::Result<()> {
    let Some(session_file_path) = default_session_file_path() else {
        return Ok(());
    };
    persist_survey_prompt_postponed_to_path(
        &session_file_path,
        survey_id,
        postpone_seconds,
        current_unix_seconds(),
    )
}

pub fn persist_survey_prompt_postponed_to_path(
    session_file_path: &Path,
    survey_id: &str,
    postpone_seconds: u64,
    now_unix_seconds: u64,
) -> io::Result<()> {
    with_session_file_persist_lock(|| {
        let mut file = load_file(session_file_path).unwrap_or_default();
        file.version = CURRENT_SESSION_FILE_VERSION;
        file.survey_prompt = Some(SurveyPromptSession {
            survey_id: survey_id.to_string(),
            opened_at_unix_seconds: None,
            postponed_until_unix_seconds: Some(now_unix_seconds.saturating_add(postpone_seconds)),
        });

        persist_to_path(session_file_path, &file)
    })
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// Survey eligibility only needs a usage signal. A recorded repository means the user has used
// GitComet before; it does not need to prove the repository still exists on disk.
