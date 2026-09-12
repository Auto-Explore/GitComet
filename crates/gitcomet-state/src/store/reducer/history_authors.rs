use super::*;
use crate::history_authors::{HistoryAuthorsEffect, HistoryAuthorsMsg, HistoryAuthorsState};
use gitcomet_core::services::CancellationToken;

pub(super) fn reduce(state: &mut AppState, event: HistoryAuthorsMsg) -> Vec<Effect> {
    let repo_id = match &event {
        HistoryAuthorsMsg::Ensure { repo_id, .. } | HistoryAuthorsMsg::Loaded { repo_id, .. } => {
            *repo_id
        }
    };
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    match event {
        HistoryAuthorsMsg::Ensure { retry, .. } => {
            if !repo.history_state.authors.needs_load(repo)
                && !(retry && matches!(repo.history_state.authors.names, Loadable::Error(_)))
            {
                return Vec::new();
            }
            let source = HistoryAuthorsState::source_for(repo);
            let authors = &mut repo.history_state.authors;
            authors.cancellation.cancel();
            authors.cancellation = CancellationToken::new();
            authors.seq = authors.seq.wrapping_add(1);
            authors.rev = authors.rev.wrapping_add(1);
            authors.source = Some(source);
            authors.names = Loadable::Loading;
            vec![Effect::HistoryAuthors(HistoryAuthorsEffect {
                repo_id,
                seq: authors.seq,
                mode: repo.history_state.history_scope,
                cancellation: authors.cancellation.clone(),
            })]
        }
        HistoryAuthorsMsg::Loaded { seq, result, .. } => {
            if repo.history_state.authors.seq != seq || repo.history_state.authors.needs_load(repo)
            {
                return Vec::new();
            }
            let authors = &mut repo.history_state.authors;
            authors.names = match result {
                Ok(names) => Loadable::Ready(names),
                Err(error) => Loadable::Error(error.to_string()),
            };
            authors.rev = authors.rev.wrapping_add(1);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::{HistoryMode, RepoSpec};
    use gitcomet_core::services::HistorySnapshot;

    fn fixture() -> AppState {
        AppState {
            repos: vec![RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: "/tmp/authors".into(),
                },
            )],
            ..Default::default()
        }
    }

    fn request(state: &mut AppState) -> HistoryAuthorsEffect {
        let effect = reduce(
            state,
            HistoryAuthorsMsg::Ensure {
                repo_id: RepoId(1),
                retry: false,
            },
        )
        .remove(0);
        let Effect::HistoryAuthors(work) = effect else {
            panic!("expected authors")
        };
        work
    }

    fn complete(state: &mut AppState, work: &HistoryAuthorsEffect, names: &[&str]) {
        reduce(
            state,
            HistoryAuthorsMsg::Loaded {
                repo_id: work.repo_id,
                seq: work.seq,
                result: Ok(names
                    .iter()
                    .copied()
                    .map(Arc::from)
                    .collect::<Vec<_>>()
                    .into()),
            },
        );
    }

    #[test]
    fn author_catalog_is_on_demand_and_retained_across_author_filters_and_progress() {
        let mut state = fixture();
        assert!(matches!(
            state.repos[0].history_state.authors.names,
            Loadable::NotLoaded
        ));
        let work = request(&mut state);
        assert!(
            reduce(
                &mut state,
                HistoryAuthorsMsg::Ensure {
                    repo_id: RepoId(1),
                    retry: true
                }
            )
            .is_empty()
        );
        complete(&mut state, &work, &["Alice", "Older author"]);
        let names = match &state.repos[0].history_state.authors.names {
            Loadable::Ready(names) => names.clone(),
            _ => panic!(),
        };
        state.repos[0].set_history_author_filter(Some("Alice".into()));
        state.repos[0].history_state.indexed.rev += 1;
        state.repos[0].history_state.log_rev += 1;
        assert!(
            reduce(
                &mut state,
                HistoryAuthorsMsg::Ensure {
                    repo_id: RepoId(1),
                    retry: true
                }
            )
            .is_empty()
        );
        assert!(
            matches!(&state.repos[0].history_state.authors.names, Loadable::Ready(current) if Arc::ptr_eq(current, &names))
        );
        assert!(state.repos[0].history_state.indexed.ranges.is_empty());
    }

    #[test]
    fn scope_changes_cancel_and_reject_old_author_results() {
        let mut state = fixture();
        let old = request(&mut state);
        state.repos[0].set_log_scope(HistoryMode::MergesOnly);
        assert!(old.cancellation.is_cancelled());
        let next = request(&mut state);
        assert_eq!(next.mode, HistoryMode::MergesOnly);
        complete(&mut state, &old, &["stale"]);
        assert!(matches!(
            state.repos[0].history_state.authors.names,
            Loadable::Loading
        ));
        complete(&mut state, &next, &["Merger"]);
        assert!(
            matches!(&state.repos[0].history_state.authors.names, Loadable::Ready(names) if names[0].as_ref() == "Merger")
        );
    }

    #[test]
    fn refresh_and_snapshot_changes_invalidate_author_requests() {
        let mut state = fixture();
        let old = request(&mut state);
        state.repos[0].history_state.log_snapshot = Some(HistorySnapshot("new heads".into()));
        complete(&mut state, &old, &["stale"]);
        assert!(matches!(
            state.repos[0].history_state.authors.names,
            Loadable::Loading
        ));
        let next = request(&mut state);
        assert!(old.cancellation.is_cancelled());
        state.repos[0].bump_load_epoch();
        assert!(next.cancellation.is_cancelled());
        complete(&mut state, &next, &["also stale"]);
        assert!(matches!(
            state.repos[0].history_state.authors.names,
            Loadable::Loading
        ));
    }

    #[test]
    fn author_errors_are_retried_only_when_requested() {
        let mut state = fixture();
        let work = request(&mut state);
        reduce(
            &mut state,
            work.failed(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Cancelled,
            )),
        );
        assert!(
            reduce(
                &mut state,
                HistoryAuthorsMsg::Ensure {
                    repo_id: RepoId(1),
                    retry: false
                }
            )
            .is_empty()
        );
        assert_eq!(
            reduce(
                &mut state,
                HistoryAuthorsMsg::Ensure {
                    repo_id: RepoId(1),
                    retry: true
                }
            )
            .len(),
            1
        );
    }
}
