//! Follows a dispatched commit or amend to its result through the command log.

use super::*;
use gitcomet_state::model::{AuthRetryOperation, CommandLogEntry};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct PendingCommit {
    pub(in crate::view) repo_id: RepoId,
    /// The repository's `command_log_seq` when the commit started; its result
    /// is a later entry.
    command_log_seq: u64,
    pub(in crate::view) amend: bool,
    /// The trimmed message that was submitted.
    pub(in crate::view) message: SharedString,
    /// The commit failed while a retry took over. The store drops the failed
    /// entry once the window has shown it, so the outcome is kept here.
    failed_awaiting_retry: bool,
}

impl PendingCommit {
    pub(in crate::view) fn new(
        state: &AppState,
        repo_id: RepoId,
        amend: bool,
        message: SharedString,
    ) -> Self {
        let command_log_seq = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map_or(0, |repo| repo.feedback.command_log_seq);
        Self {
            repo_id,
            command_log_seq,
            amend,
            message,
            failed_awaiting_retry: false,
        }
    }

    /// False while a retry will produce another result.
    pub(in crate::view) fn settled(&self) -> bool {
        !self.failed_awaiting_retry
    }

    fn command(&self) -> &'static str {
        if self.amend { "Amend" } else { "Commit" }
    }

    fn completed_entry<'a>(&self, repo: &'a RepoState) -> Option<&'a CommandLogEntry> {
        repo.feedback
            .command_log_after(self.command_log_seq)
            .iter()
            .rfind(|entry| entry.command == self.command())
    }

    fn retry_pending(&self, state: &AppState) -> bool {
        let prompt_retries = state.auth_prompt.as_ref().is_some_and(|prompt| {
            matches!(
                &prompt.operation,
                AuthRetryOperation::Commit {
                    repo_id,
                    amend,
                    ..
                } if *repo_id == self.repo_id && *amend == self.amend
            )
        });
        let retry_in_flight = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .is_some_and(|repo| {
                repo.pending
                    .commit_retry
                    .as_ref()
                    .is_some_and(|pending| pending.amend == self.amend)
            });
        prompt_retries || retry_in_flight
    }

    /// Follows the commit into `state`: `Some(ok)` once its result is in. It
    /// stays unsettled only while a retry will produce another result.
    pub(in crate::view) fn advance(&mut self, state: &AppState) -> Option<bool> {
        let repo = state.repos.iter().find(|repo| repo.id == self.repo_id)?;
        let entry_ok = self
            .completed_entry(repo)
            .map(|entry| entry.ok)
            .or(self.failed_awaiting_retry.then_some(false))?;
        self.failed_awaiting_retry = !entry_ok && self.retry_pending(state);
        Some(entry_ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_state::model::{AuthPromptState, PendingCommitRetry};
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    fn repo_state(id: RepoId) -> RepoState {
        RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    fn entry(command: &str, ok: bool, seconds: u64) -> CommandLogEntry {
        CommandLogEntry {
            time: UNIX_EPOCH + Duration::from_secs(seconds),
            ok,
            command: command.to_string(),
            summary: format!("{command}: test"),
            stdout: "".into(),
            stderr: "".into(),
            announce_success: true,
            handled_by_auth_prompt: false,
            failure_id: None,
            hook_operation_id: None,
        }
    }

    fn state_with(repo: RepoState) -> AppState {
        AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        }
    }

    fn pending(state: &AppState, amend: bool) -> PendingCommit {
        PendingCommit::new(state, RepoId(1), amend, "message".into())
    }

    fn retry_prompt(amend: bool) -> AuthPromptState {
        AuthPromptState {
            generation: 1,
            challenge: None,
            status: AuthPromptStatus::AwaitingInput,
            kind: AuthPromptKind::Passphrase,
            reason: "auth required".into(),
            operation: AuthRetryOperation::Commit {
                repo_id: RepoId(1),
                message: "message".into(),
                amend,
                push_after_commit: false,
            },
        }
    }

    /// A commit that failed into a passphrase prompt, as the window saw it.
    fn failed_awaiting_retry(amend: bool) -> (PendingCommit, AppState) {
        let command = if amend { "Amend" } else { "Commit" };
        let mut repo = repo_state(RepoId(1));
        let mut state = state_with(repo.clone());
        let mut pending = pending(&state, amend);
        repo.feedback_mut()
            .push_command_result(entry(command, false, 1));
        state = AppState {
            auth_prompt: Some(retry_prompt(amend)),
            ..state_with(repo)
        };
        assert_eq!(pending.advance(&state), Some(false));
        assert!(!pending.settled(), "the prompt will retry the commit");
        (pending, state)
    }

    #[test]
    fn ignores_previous_entry() {
        let mut repo = repo_state(RepoId(1));
        repo.feedback_mut()
            .push_command_result(entry("Amend", true, 1));
        let state = state_with(repo);
        assert_eq!(pending(&state, true).advance(&state), None);
    }

    #[test]
    fn amend_observes_later_entry_before_push_entry() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), true);
        repo.feedback_mut()
            .push_command_result(entry("Amend", true, 2));
        repo.feedback_mut()
            .push_command_result(entry("Push after commit", true, 3));
        assert_eq!(p.advance(&state_with(repo)), Some(true));
    }

    #[test]
    fn sees_successful_retry_in_the_same_snapshot_as_the_failure() {
        let mut repo = repo_state(RepoId(1));
        repo.feedback_mut()
            .push_command_result(entry("Commit", true, 1));
        let mut p = pending(&state_with(repo.clone()), true);
        repo.feedback_mut()
            .push_command_result(entry("Amend", false, 2));
        repo.feedback_mut()
            .push_command_result(entry("Amend", true, 3));
        assert_eq!(p.advance(&state_with(repo)), Some(true));
        assert!(p.settled());
    }

    #[test]
    fn sees_retry_after_shown_entries_were_dropped() {
        let mut repo = repo_state(RepoId(1));
        repo.feedback_mut()
            .push_command_result(entry("Commit", true, 1));
        let mut p = pending(&state_with(repo.clone()), true);
        repo.feedback_mut()
            .push_command_result(entry("Amend", false, 2));
        // The window acknowledged everything up to the failed attempt.
        repo.feedback_mut().command_log.clear();
        assert_eq!(p.advance(&state_with(repo.clone())), None);

        repo.feedback_mut()
            .push_command_result(entry("Amend", true, 3));
        assert_eq!(p.advance(&state_with(repo)), Some(true));
    }

    #[test]
    fn failed_commit_is_settled() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), false);
        repo.feedback_mut()
            .push_command_result(entry("Commit", false, 1));
        assert_eq!(p.advance(&state_with(repo)), Some(false));
        assert!(p.settled());
    }

    #[test]
    fn successful_commit_is_settled() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), false);
        repo.feedback_mut()
            .push_command_result(entry("Commit", true, 1));
        assert_eq!(p.advance(&state_with(repo)), Some(true));
        assert!(p.settled());
    }

    #[test]
    fn amend_entry_does_not_settle_commit() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), false);
        repo.feedback_mut()
            .push_command_result(entry("Amend", true, 1));
        assert_eq!(p.advance(&state_with(repo)), None);
    }

    #[test]
    fn amend_retry_does_not_keep_failed_commit_pending() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), false);
        repo.feedback_mut()
            .push_command_result(entry("Commit", false, 1));
        let state = AppState {
            auth_prompt: Some(retry_prompt(true)),
            ..state_with(repo)
        };
        assert_eq!(p.advance(&state), Some(false));
        assert!(p.settled());
    }

    #[test]
    fn commit_retry_keeps_failed_commit_pending() {
        let (p, _) = failed_awaiting_retry(false);
        assert!(!p.settled());
    }

    #[test]
    fn in_flight_retry_keeps_failure_pending() {
        let mut repo = repo_state(RepoId(1));
        let mut p = pending(&state_with(repo.clone()), true);
        repo.feedback_mut()
            .push_command_result(entry("Amend", false, 1));
        repo.pending.commit_retry = Some(PendingCommitRetry {
            message: "message".into(),
            amend: true,
            push_after_commit: false,
        });
        assert_eq!(p.advance(&state_with(repo)), Some(false));
        assert!(!p.settled());
    }

    #[test]
    fn abandoned_retry_settles_after_failure_was_dropped() {
        let (mut p, mut state) = failed_awaiting_retry(true);
        state.repos[0].feedback_mut().acknowledge(0, 1);
        state.auth_prompt = None;
        assert_eq!(p.advance(&state), Some(false));
        assert!(p.settled());
    }

    #[test]
    fn sees_successful_retry_after_failure_was_dropped() {
        let (mut p, mut state) = failed_awaiting_retry(true);
        state.repos[0].feedback_mut().acknowledge(0, 1);
        assert_eq!(p.advance(&state), Some(false));
        assert!(!p.settled(), "the prompt is still open");

        state.auth_prompt = None;
        state.repos[0]
            .feedback_mut()
            .push_command_result(entry("Amend", true, 2));
        assert_eq!(p.advance(&state), Some(true));
        assert!(p.settled());
    }
}
