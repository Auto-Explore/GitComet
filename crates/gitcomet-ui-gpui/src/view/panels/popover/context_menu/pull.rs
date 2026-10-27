use super::*;
use gitcomet_state::model::{AutoFetchStatus, RepoAutoFetchState};

fn auto_fetch_status_text(enabled: bool, state: &RepoAutoFetchState) -> &'static str {
    if !enabled {
        return "Automatic fetch: Off";
    }
    match &state.status {
        AutoFetchStatus::Idle => "Automatic fetch: Waiting",
        AutoFetchStatus::Running => "Fetching automatically…",
        AutoFetchStatus::Succeeded { .. } => "Automatic fetch: Succeeded",
        AutoFetchStatus::Failed { .. } => "Automatic fetch: Failed",
    }
}

pub(in super::super) fn auto_fetch_failure_description(
    error: &gitcomet_core::error::Error,
) -> String {
    error.to_string().replace(['\r', '\n'], " ")
}

pub(in super::super) fn auto_fetch_auth_recovery_hint(
    error: &gitcomet_core::error::Error,
) -> Option<&'static str> {
    let gitcomet_core::error::ErrorKind::Git(failure) = error.kind() else {
        return None;
    };
    let Some(
        gitcomet_core::auth::AuthOutcome::NeedsInput(challenge)
        | gitcomet_core::auth::AuthOutcome::Rejected(challenge),
    ) = failure.auth_outcome()
    else {
        return None;
    };
    Some(match challenge.kind {
        gitcomet_core::auth::GitAuthKind::HostVerification => "Use Fetch all to review the host.",
        gitcomet_core::auth::GitAuthKind::UsernamePassword
        | gitcomet_core::auth::GitAuthKind::Passphrase => "Use Fetch all to sign in.",
    })
}

fn auto_fetch_status_items(
    enabled: bool,
    state: &RepoAutoFetchState,
    format: DateTimeFormat,
    timezone: Timezone,
    show_timezone: bool,
) -> Vec<ContextMenuItem> {
    let mut items = vec![ContextMenuItem::Label(
        auto_fetch_status_text(enabled, state).into(),
    )];
    if enabled && let AutoFetchStatus::Failed { error, .. } = &state.status {
        items.push(ContextMenuItem::Description(
            components::ContextMenuText::new(auto_fetch_failure_description(error))
                .max_lines(1)
                .tooltip_mode(components::TruncatedTextTooltipMode::FullTextIfTruncated),
        ));
        if let Some(hint) = auto_fetch_auth_recovery_hint(error) {
            items.push(ContextMenuItem::Description(hint.into()));
        }
    }
    if enabled && !state.paused_remotes.is_empty() {
        items.push(ContextMenuItem::Description(
            format!(
                "Paused for {} until Fetch all succeeds.",
                state.paused_remotes.join(", ")
            )
            .into(),
        ));
    } else if enabled
        && matches!(state.status, AutoFetchStatus::Failed { .. })
        && let Some(at) = state.retry_at
    {
        let mut timestamp = String::new();
        format_datetime_into(&mut timestamp, at, format, timezone, show_timezone);
        items.push(ContextMenuItem::Label(
            format!("Next attempt: {timestamp}").into(),
        ));
    }
    if let Some(at) = state.last_success_at {
        let mut timestamp = String::new();
        format_datetime_into(&mut timestamp, at, format, timezone, show_timezone);
        items.push(ContextMenuItem::Label(
            format!("Last successful fetch: {timestamp}").into(),
        ));
    }
    items
}

pub(super) fn model(this: &PopoverHost) -> ContextMenuModel {
    let active_repo_id = this.active_repo_id();
    let repo_disabled = active_repo_id.is_none();
    let pull_disabled = this
        .active_repo()
        .is_none_or(|repo| !pull_enabled(repo, &this.state.large_file_settings));
    let repo_id = active_repo_id.unwrap_or(RepoId(0));
    let upstream = super::active_branch_tracking_upstream(this);

    let mut model = ContextMenuModel::new(vec![
        ContextMenuItem::Header(super::action_menu_title("Pull", upstream).into()),
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Pull (default)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: None,
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::Default,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (fast-forward if possible)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("F".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::FastForwardIfPossible,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (fast-forward only)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("O".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::FastForwardOnly,
            }),
        },
        ContextMenuItem::Entry {
            label: "Pull (rebase)".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("R".into()),
            disabled: pull_disabled,
            action: Box::new(ContextMenuAction::Pull {
                repo_id,
                mode: PullMode::Rebase,
            }),
        },
        ContextMenuItem::Separator,
        ContextMenuItem::Entry {
            label: "Fetch all".into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("A".into()),
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        },
    ]);
    if let Some(repo) = this.active_repo() {
        model.items.extend(auto_fetch_status_items(
            this.state.remote_settings.auto_fetch.enabled,
            &repo.auto_fetch,
            this.date_time_format,
            this.timezone,
            this.show_timezone,
        ));
    }
    model.items.extend([
        ContextMenuItem::Entry {
            label: "Prune merged branches".into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        },
        ContextMenuItem::Entry {
            label: "Prune local tags".into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: repo_disabled,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        },
    ]);
    model.items.extend(super::large_file::pull_items(
        &this.state,
        this.active_repo(),
    ));
    model
        .items
        .extend(super::annex::pull_items(&this.state, this.active_repo()));
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_fetch_failure_description_is_single_line_without_losing_details() {
        let state = RepoAutoFetchState {
            last_success_at: None,
            status: AutoFetchStatus::Failed {
                at: std::time::UNIX_EPOCH,
                error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
                    "fatal: remote unavailable\r\nCheck access.\nRepository was not found.".into(),
                )),
            },
            ..RepoAutoFetchState::default()
        };
        let items =
            auto_fetch_status_items(true, &state, DateTimeFormat::YmdHm, Timezone::Utc, false);
        let ContextMenuItem::Description(reason) = &items[1] else {
            panic!("failure details should be a description");
        };
        assert_eq!(
            reason.as_ref(),
            "fatal: remote unavailable  Check access. Repository was not found."
        );
    }

    #[test]
    fn automatic_fetch_failure_keeps_the_last_success_visible() {
        let success = std::time::UNIX_EPOCH;
        let state = RepoAutoFetchState {
            last_success_at: Some(success),
            status: AutoFetchStatus::Failed {
                at: success + std::time::Duration::from_secs(1800),
                error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
                    "offline".into(),
                )),
            },
            ..RepoAutoFetchState::default()
        };
        let labels: Vec<_> =
            auto_fetch_status_items(true, &state, DateTimeFormat::YmdHm, Timezone::Utc, false)
                .into_iter()
                .map(|item| match item {
                    ContextMenuItem::Label(label) | ContextMenuItem::Description(label) => {
                        label.as_ref().to_owned()
                    }
                    _ => panic!("fetch status must be informational"),
                })
                .collect();
        assert_eq!(
            labels,
            [
                "Automatic fetch: Failed",
                "offline",
                "Last successful fetch: 1970-01-01 00:00",
            ]
        );
        assert_eq!(
            auto_fetch_status_text(false, &state),
            "Automatic fetch: Off"
        );
    }

    #[test]
    fn automatic_fetch_failures_show_when_it_retries_or_what_waits_for_fetch_all() {
        let failed = AutoFetchStatus::Failed {
            at: std::time::UNIX_EPOCH,
            error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
                "offline".into(),
            )),
        };
        let rows = |state: &RepoAutoFetchState| -> Vec<String> {
            auto_fetch_status_items(true, state, DateTimeFormat::YmdHm, Timezone::Utc, false)
                .into_iter()
                .map(|item| match item {
                    ContextMenuItem::Label(label) | ContextMenuItem::Description(label) => {
                        label.as_ref().to_owned()
                    }
                    _ => panic!("fetch status must be informational"),
                })
                .collect()
        };
        let retrying = RepoAutoFetchState {
            status: failed.clone(),
            retry_at: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(3600)),
            ..RepoAutoFetchState::default()
        };
        assert_eq!(
            rows(&retrying),
            [
                "Automatic fetch: Failed",
                "offline",
                "Next attempt: 1970-01-01 01:00"
            ]
        );
        let paused = RepoAutoFetchState {
            paused_remotes: vec!["origin".into(), "fork".into()],
            ..retrying
        };
        assert_eq!(
            rows(&paused),
            [
                "Automatic fetch: Failed",
                "offline",
                "Paused for origin, fork until Fetch all succeeds.",
            ]
        );
    }

    #[test]
    fn automatic_fetch_authentication_recovery_stays_visible_beside_the_truncated_reason() {
        let failure = gitcomet_core::error::GitFailure::new(
            "git fetch --all",
            gitcomet_core::error::GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            Vec::new(),
            Some("authentication required".into()),
        )
        .with_auth_outcome(gitcomet_core::auth::AuthOutcome::NeedsInput(
            gitcomet_core::auth::AuthChallenge::from_prompt(
                gitcomet_core::auth::GitAuthKind::UsernamePassword,
                "Password for test remote:",
            ),
        ));
        let state = RepoAutoFetchState {
            last_success_at: None,
            status: AutoFetchStatus::Failed {
                at: std::time::UNIX_EPOCH,
                error: gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Git(
                    failure,
                )),
            },
            ..RepoAutoFetchState::default()
        };
        let items =
            auto_fetch_status_items(true, &state, DateTimeFormat::YmdHm, Timezone::Utc, false);
        let [
            ContextMenuItem::Label(status),
            ContextMenuItem::Description(reason),
            ContextMenuItem::Description(recovery),
        ] = items.as_slice()
        else {
            panic!("status, reason and authentication recovery should be separate rows");
        };
        assert_eq!(status.as_ref(), "Automatic fetch: Failed");
        assert_eq!(
            reason,
            &components::ContextMenuText::new("git fetch --all failed: authentication required")
                .max_lines(1)
                .tooltip_mode(components::TruncatedTextTooltipMode::FullTextIfTruncated)
        );
        assert_eq!(recovery.as_ref(), "Use Fetch all to sign in.");
    }
}
