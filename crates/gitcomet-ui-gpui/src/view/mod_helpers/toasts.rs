use super::*;

#[derive(Clone)]
pub(crate) struct ToastState {
    pub(crate) id: u64,
    pub(crate) kind: components::ToastKind,
    pub(crate) input: Entity<components::TextInput>,
    pub(crate) is_code_message: bool,
    pub(crate) actions: Vec<ToastAction>,
    pub(crate) dismiss_behavior: ToastDismissBehavior,
    pub(crate) ttl: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToastAction {
    OpenUrl {
        url: String,
        label: String,
    },
    OpenSurvey {
        survey_id: String,
        survey_name: String,
        url: String,
        label: String,
    },
    PostponeSurvey {
        survey_id: String,
        survey_name: String,
        postpone_seconds: u64,
        label: String,
    },
    OpenHookActivity {
        repo_id: RepoId,
        operation_id: GitOperationId,
        label: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ToastDismissBehavior {
    #[default]
    Remove,
    PostponeSurvey {
        survey_id: String,
        survey_name: String,
        postpone_seconds: u64,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct CommitDetailsDelayState {
    pub(crate) repo_id: RepoId,
    pub(crate) commit_id: CommitId,
    pub(crate) show_loading: bool,
}
