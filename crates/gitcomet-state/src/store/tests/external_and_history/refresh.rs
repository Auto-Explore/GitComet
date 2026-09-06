use super::*;
use gitcomet_core::services::{HistoryReadResult, HistorySnapshot};
use rustc_hash::FxHashSet;

fn reply(
    state: &mut AppState,
    seq: u64,
    cursor: Option<LogCursor>,
    result: Result<HistoryReadResult>,
) -> Vec<Effect> {
    history_message(
        state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor,
            result,
        }),
    )
}

fn complete_state(count: usize) -> AppState {
    let commits = commits_named("c", count);
    let mut state = paginated_repo_state(&commits);
    state.repos[0].set_log(Loadable::Ready(Arc::new(LogPage {
        commits,
        next_cursor: None,
    })));
    state.repos[0].history_state.log_snapshot = Some(HistorySnapshot("original".into()));
    state
}

fn failed_detached_checkout_restores_head_after_unchanged_refresh(scope: LogScope) {
    for head_reply_first in [true, false] {
        let mut state = complete_state(2);
        let repo_id = RepoId(1);
        let repo = &mut state.repos[0];
        repo.history_state.history_scope = scope;
        repo.set_head_branch(Loadable::Ready("HEAD".into()));
        let Loadable::Ready(before) = repo.log.clone() else {
            unreachable!()
        };
        let actual_head = before.commits[0].id.clone();
        let rejected_target = before.commits[1].id.clone();
        repo.set_detached_head_commit(Some(actual_head.clone()));
        repo.set_selected_commit(Some(rejected_target.clone()));
        let rev = repo.log_rev;
        repo.set_commit_multi_selection(crate::model::CommitMultiSelection {
            commits: vec![rejected_target.clone()],
            anchor: Some(rejected_target.clone()),
            anchor_index: Some(1),
            anchor_log_rev: Some(rev),
        });
        let selection = repo.history_state.multi_selection.clone();

        let effects = history_message(
            &mut state,
            Msg::CheckoutCommit {
                repo_id,
                commit_id: rejected_target.clone(),
            },
        );
        assert!(matches!(
            effects.as_slice(),
            [Effect::CheckoutCommit { commit_id, .. }] if commit_id == &rejected_target
        ));
        assert_eq!(state.repos[0].detached_head_commit, Some(rejected_target));

        let mut effects = history_message(
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id,
                action: RepoActionKind::CheckoutCommit,
                result: Err(Error::new(ErrorKind::Backend("checkout rejected".into()))),
            }),
        );
        assert!(state.repos[0].feedback.last_error.is_some());

        for refresh in 0..2 {
            if refresh > 0 {
                effects = refresh_history(&mut state);
            }
            assert!(effects.iter().any(|effect| matches!(
                effect,
                Effect::LoadLog { scope: loaded_scope, .. } if *loaded_scope == scope
            )));
            let head_reply = Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Ok("HEAD".into()),
            });
            let log_reply = Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                repo_id,
                seq: log_effect(&effects).0,
                scope,
                cursor: None,
                result: Ok(HistoryReadResult::Unchanged),
            });
            let replies = if head_reply_first {
                [head_reply, log_reply]
            } else {
                [log_reply, head_reply]
            };
            for message in replies {
                history_message(&mut state, message);
            }

            let repo = &state.repos[0];
            assert_eq!(
                repo.detached_head_commit.as_ref(),
                Some(&actual_head),
                "{scope:?}, head_reply_first={head_reply_first}: unchanged history must restore the actual detached HEAD"
            );
            assert!(matches!(&repo.log, Loadable::Ready(page) if Arc::ptr_eq(page, &before)));
            assert_eq!(repo.log_rev, rev);
            assert_eq!(repo.history_state.multi_selection, selection);
            assert_eq!(
                repo.history_state.log_snapshot,
                Some(HistorySnapshot("original".into()))
            );
            assert!(repo.loads_in_flight.active_log_seq().is_none());
        }
    }
}

#[test]
fn failed_detached_checkout_restores_head_after_unchanged_full_reachable_refresh() {
    failed_detached_checkout_restores_head_after_unchanged_refresh(LogScope::FullReachable);
}

#[test]
fn failed_detached_checkout_restores_head_after_unchanged_first_parent_refresh() {
    failed_detached_checkout_restores_head_after_unchanged_refresh(LogScope::FirstParent);
}

#[test]
fn unchanged_focus_keeps_the_entire_page_selection_and_revisions() {
    let mut state = complete_state(50_000);
    let Loadable::Ready(before) = state.repos[0].log.clone() else {
        unreachable!()
    };
    let selected = before.commits.last().unwrap().id.clone();
    state.repos[0].set_selected_commit(Some(selected.clone()));
    let rev = state.repos[0].log_rev;
    state.repos[0].set_commit_multi_selection(crate::model::CommitMultiSelection {
        commits: vec![selected.clone()],
        anchor: Some(selected),
        anchor_index: Some(49_999),
        anchor_log_rev: Some(rev),
    });
    let selection = state.repos[0].history_state.multi_selection.clone();
    for _ in 0..5 {
        let effects = history_message(&mut state, Msg::SetActiveRepo { repo_id: RepoId(1) });
        reply(
            &mut state,
            log_effect(&effects).0,
            None,
            Ok(HistoryReadResult::Unchanged),
        );
        let repo = &state.repos[0];
        assert!(matches!(&repo.log, Loadable::Ready(page) if Arc::ptr_eq(page, &before)));
        assert_eq!(repo.log_rev, rev);
        assert_eq!(repo.history_state.multi_selection, selection);
        assert_eq!(
            repo.history_state.log_snapshot,
            Some(HistorySnapshot("original".into()))
        );
    }
}

#[test]
fn complete_deep_history_keeps_its_root_after_hundreds_of_new_commits() {
    let mut state = complete_state(50_000);
    let Loadable::Ready(before) = state.repos[0].log.clone() else {
        unreachable!()
    };
    let selected = before.commits.last().unwrap().id.clone();
    state.repos[0].set_selected_commit(Some(selected.clone()));
    state.repos[0].set_commit_multi_selection(crate::model::CommitMultiSelection {
        commits: vec![selected.clone()],
        anchor: Some(selected.clone()),
        ..Default::default()
    });
    let history: Vec<_> = commits_named("new", 601)
        .into_iter()
        .chain(before.commits.iter().cloned())
        .collect();
    let effects = refresh_history(&mut state);
    answer_log(&mut state, &effects, &history);
    assert!(
        matches!(&state.repos[0].log, Loadable::Ready(page) if page.commits == history && page.next_cursor.is_none())
    );
    assert_eq!(state.repos[0].history_state.selected_commit, Some(selected));
}

#[test]
fn failed_or_cancelled_refresh_preserves_ready_history() {
    for error in [
        ErrorKind::Cancelled,
        ErrorKind::Backend("refresh failed".into()),
    ] {
        let mut state = complete_state(6_000);
        let before = state.repos[0].log.clone();
        let rev = state.repos[0].log_rev;
        let effects = refresh_history(&mut state);
        reply(
            &mut state,
            log_effect(&effects).0,
            None,
            Err(Error::new(error)),
        );
        assert_eq!(state.repos[0].log, before);
        assert_eq!(state.repos[0].log_rev, rev);
        assert!(state.repos[0].loads_in_flight.active_log_seq().is_none());
        assert!(state.repos[0].history_state.log_snapshot.is_some());
    }
}

#[test]
fn late_and_duplicate_replies_cannot_overwrite_a_completed_newer_request() {
    let mut state = complete_state(6_000);
    let old = log_effect(&refresh_history(&mut state)).0;
    let new = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);
    reply(&mut state, new, None, Ok(HistoryReadResult::Unchanged));
    let before = state.repos[0].log.clone();
    let rev = state.repos[0].log_rev;
    for seq in [old, new] {
        assert!(
            reply(
                &mut state,
                seq,
                None,
                Ok(LogPage {
                    commits: vec![],
                    next_cursor: None
                }
                .into())
            )
            .is_empty()
        );
        assert_eq!(state.repos[0].log, before);
        assert_eq!(state.repos[0].log_rev, rev);
    }
}

#[test]
fn invalidated_pagination_refreshes_without_appending_incompatible_rows() {
    let old = commits_named("c", 800);
    let mut state = paginated_repo_state(&old[..600]);
    let before = state.repos[0].log.clone();
    let effects = history_message(&mut state, Msg::LoadMoreHistory { repo_id: RepoId(1) });
    let (seq, _, cursor) = log_effect(&effects);
    let next = reply(&mut state, seq, cursor, Ok(HistoryReadResult::Invalidated));
    assert_eq!(state.repos[0].log, before);
    assert!(log_effect(&next).2.is_none());
    let history: Vec<_> = commits_named("new", 401).into_iter().chain(old).collect();
    answer_log(&mut state, &next, &history);
    let Loadable::Ready(page) = &state.repos[0].log else {
        unreachable!()
    };
    let ids: FxHashSet<_> = page.commits.iter().map(|c| &c.id).collect();
    assert_eq!(ids.len(), page.commits.len());
    assert!(ids.contains(&CommitId("c0599".into())));
}
