use super::*;
use crate::history_find::{HistoryFindEffect, HistoryFindMsg, HistoryFindState};

pub(super) fn reduce(state: &mut AppState, event: HistoryFindMsg) -> Vec<Effect> {
    let repo_id = match &event {
        HistoryFindMsg::Find { repo_id, .. } | HistoryFindMsg::Found { repo_id, .. } => *repo_id,
        HistoryFindMsg::Close => {
            for repo in &mut state.repos {
                repo.history_state.find.close();
            }
            return Vec::new();
        }
    };
    // Keep decoded text for the active search only. Switching among large
    // repositories must not retain an unbounded cache per visited tab.
    if matches!(
        &event,
        HistoryFindMsg::Find {
            query: Some(_),
            index: Some(_),
            ..
        }
    ) {
        for repo in &mut state.repos {
            if repo.id != repo_id {
                repo.history_state.find.close();
            }
        }
    }
    let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    let stashes = match &repo.stashes {
        Loadable::Ready(stashes) => Arc::clone(stashes),
        _ => Default::default(),
    };
    let find = &mut repo.history_state.find;
    match event {
        HistoryFindMsg::Find { query, index, .. } => {
            if let (Some(query), Some(index)) = (&query, &index)
                && find.is_for(query, index, repo.stashes_rev)
                && find.error.is_none()
            {
                return Vec::new();
            }
            find.cancellation.cancel();
            // Stopping keeps the commit text: the view stops for passing
            // states (a regex mid-typing, a scope change, a tab switch).
            *find = HistoryFindState {
                seq: find.seq.wrapping_add(1),
                rev: find.rev.wrapping_add(1),
                cache: Arc::clone(&find.cache),
                ..Default::default()
            };
            let (Some(query), Some(index)) = (query, index) else {
                return Vec::new();
            };
            find.query = Some(query.clone());
            find.stashes_rev = repo.stashes_rev;
            find.index = Some(Arc::downgrade(&index));
            // An invalid regex matches nothing.
            if index.is_empty() || query.regex_error().is_some() {
                find.done = true;
                return Vec::new();
            }
            vec![Effect::HistoryFind(HistoryFindEffect {
                repo_id,
                seq: find.seq,
                index,
                query,
                stashes,
                cancellation: find.cancellation.clone(),
                cache: Arc::clone(&find.cache),
            })]
        }
        HistoryFindMsg::Found { seq, result, .. } => {
            if seq != find.seq || find.done || find.error.is_some() {
                return Vec::new();
            }
            match result {
                Ok(chunk) => {
                    if !chunk.matches.is_empty() {
                        Arc::make_mut(&mut find.matches).push(chunk.matches.into());
                    }
                    find.done = chunk.done;
                }
                Err(error) => {
                    find.error = Some(error.to_string());
                }
            }
            find.rev = find.rev.wrapping_add(1);
            Vec::new()
        }
        // Handled for every repo above.
        HistoryFindMsg::Close => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history_find::HistoryFindChunk;
    use crate::model::RepoState;
    use gitcomet_core::domain::{LogScope, RepoSpec};
    use gitcomet_core::history_find::HistoryFindQuery;
    use gitcomet_core::history_index::{HistoryIndexBuilder, HistoryIndexHandle};
    use gitcomet_core::services::{CancellationToken, HistorySnapshot};
    use gitcomet_core::text_search::TextSearchOptions;

    fn fixture() -> AppState {
        AppState {
            repos: vec![RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: "/tmp/find".into(),
                },
            )],
            ..Default::default()
        }
    }

    fn index(rows: usize) -> HistoryIndexHandle {
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("heads".into()), LogScope::AllBranches, 20)
                .unwrap();
        for row in 0..rows as u32 {
            let mut id = [0u8; 20];
            id[..4].copy_from_slice(&row.to_be_bytes());
            builder.push(&id, [], false).unwrap();
        }
        builder.finish(&CancellationToken::new()).unwrap()
    }

    fn find(state: &mut AppState, query: &str, index: &HistoryIndexHandle) -> Vec<Effect> {
        find_with(state, query, TextSearchOptions::default(), index)
    }

    fn find_with(
        state: &mut AppState,
        query: &str,
        options: TextSearchOptions,
        index: &HistoryIndexHandle,
    ) -> Vec<Effect> {
        reduce(
            state,
            HistoryFindMsg::Find {
                repo_id: RepoId(1),
                query: HistoryFindQuery::new(query, options),
                index: Some(index.clone()),
            },
        )
    }

    fn work(effects: Vec<Effect>) -> HistoryFindEffect {
        match effects.into_iter().next() {
            Some(Effect::HistoryFind(work)) => work,
            other => panic!("expected a find effect, got {other:?}"),
        }
    }

    fn found(state: &mut AppState, seq: u64, matches: Vec<u32>, done: bool) {
        reduce(
            state,
            HistoryFindMsg::Found {
                repo_id: RepoId(1),
                seq,
                result: Ok(HistoryFindChunk { matches, done }),
            },
        );
    }

    #[test]
    fn results_accumulate_until_the_scan_is_done() {
        let mut state = fixture();
        let index = index(8);
        let work = work(find(&mut state, "fix", &index));
        found(&mut state, work.seq, vec![1, 3], false);
        found(&mut state, work.seq, vec![6], true);
        let find = &state.repos[0].history_state.find;
        assert_eq!(find.match_rows().collect::<Vec<_>>().as_slice(), &[1, 3, 6]);
        assert!(find.done);
    }

    #[test]
    fn the_same_query_over_the_same_index_is_not_rescanned() {
        let mut state = fixture();
        let index = index(8);
        let _ = work(find(&mut state, "fix", &index));
        assert!(find(&mut state, "fix", &index).is_empty());
    }

    #[test]
    fn history_find_restarts_when_stash_rows_change_without_a_new_index() {
        let mut state = fixture();
        let index = index(8);
        let first = work(find(&mut state, "main", &index));
        state.repos[0].stashes =
            Loadable::Ready(Arc::new(vec![gitcomet_core::domain::StashEntry {
                index: 0,
                id: gitcomet_core::domain::CommitId("0000000".into()),
                message: "On main: saved work".into(),
                created_at: None,
            }]));
        state.repos[0].stashes_rev += 1;
        let next = work(find(&mut state, "main", &index));
        assert!(first.cancellation.is_cancelled());
        assert_ne!(next.seq, first.seq);
        assert!(Arc::ptr_eq(&first.cache, &next.cache));
        found(&mut state, first.seq, vec![7], true);
        assert!(state.repos[0].history_state.find.matches.is_empty());
        assert_eq!(next.stashes.len(), 1);
    }

    /// The options are part of the query: toggling one searches again.
    #[test]
    fn a_changed_option_cancels_and_restarts_the_scan() {
        let mut state = fixture();
        let index = index(8);
        let mut scan = work(find(&mut state, "fix", &index));
        for options in [
            TextSearchOptions {
                match_case: true,
                ..TextSearchOptions::default()
            },
            TextSearchOptions {
                whole_word: true,
                ..TextSearchOptions::default()
            },
            TextSearchOptions {
                regex: true,
                ..TextSearchOptions::default()
            },
        ] {
            let restarted = work(find_with(&mut state, "fix", options, &index));
            assert!(scan.cancellation.is_cancelled(), "{options:?}");
            assert_ne!(restarted.seq, scan.seq);
            assert_eq!(restarted.query.options(), options);
            assert!(find_with(&mut state, "fix", options, &index).is_empty());
            scan = restarted;
        }
    }

    #[test]
    fn a_new_query_cancels_and_ignores_the_old_scan() {
        let mut state = fixture();
        let index = index(8);
        let old = work(find(&mut state, "fix", &index));
        let new = work(find(&mut state, "feat", &index));
        assert!(old.cancellation.is_cancelled());
        found(&mut state, old.seq, vec![2], true);
        let find = &state.repos[0].history_state.find;
        assert!(find.matches.is_empty());
        assert!(!find.done);
        found(&mut state, new.seq, vec![5], true);
        assert_eq!(
            state.repos[0]
                .history_state
                .find
                .match_rows()
                .collect::<Vec<_>>()
                .as_slice(),
            &[5]
        );
    }

    #[test]
    fn a_replacement_index_starts_a_new_scan() {
        let mut state = fixture();
        let _ = work(find(&mut state, "fix", &index(8)));
        let replacement = index(9);
        let work = work(find(&mut state, "fix", &replacement));
        assert!(Arc::ptr_eq(&work.index, &replacement));
    }

    #[test]
    fn clearing_the_query_cancels_and_empties_results() {
        let mut state = fixture();
        let index = index(8);
        let work = work(find(&mut state, "fix", &index));
        found(&mut state, work.seq, vec![1], false);
        assert!(find(&mut state, "  ", &index).is_empty());
        assert!(work.cancellation.is_cancelled());
        let find = &state.repos[0].history_state.find;
        assert!(find.query.is_none() && find.matches.is_empty());
    }

    /// The view stops the search for passing states: a blank query, a regex
    /// mid-typing, a scope change, a tab switch. None of them may throw away
    /// the decoded commit text the next query reuses.
    #[test]
    fn stopping_the_search_keeps_the_commit_text() {
        let mut state = fixture();
        let index = index(8);
        let first = work(find(&mut state, "fix", &index));
        reduce(
            &mut state,
            HistoryFindMsg::Find {
                repo_id: RepoId(1),
                query: None,
                index: None,
            },
        );
        assert!(first.cancellation.is_cancelled());
        let next = work(find(&mut state, "fix", &index));
        assert!(Arc::ptr_eq(&first.cache, &next.cache));
    }

    #[test]
    fn history_find_searching_another_repo_releases_the_previous_text_cache() {
        let mut state = fixture();
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: "/tmp/find-other".into(),
            },
        ));
        let index = index(8);
        let first = work(find(&mut state, "fix", &index));
        let cache = Arc::downgrade(&first.cache);
        let cancellation = first.cancellation.clone();
        drop(first);
        reduce(
            &mut state,
            HistoryFindMsg::Find {
                repo_id: RepoId(2),
                query: HistoryFindQuery::new("fix", TextSearchOptions::default()),
                index: Some(index),
            },
        );
        assert!(cancellation.is_cancelled());
        assert!(cache.upgrade().is_none());
        assert!(state.repos[0].history_state.find.query.is_none());
    }

    #[test]
    fn closing_the_find_bar_releases_the_commit_text_of_every_repo() {
        let mut state = fixture();
        let index = index(8);
        let scan = work(find(&mut state, "fix", &index));
        let cache = Arc::downgrade(&scan.cache);
        drop(scan);
        reduce(&mut state, HistoryFindMsg::Close);
        let find = &state.repos[0].history_state.find;
        assert!(find.query.is_none() && find.matches.is_empty());
        assert!(cache.upgrade().is_none(), "the closed bar keeps its cache");
    }

    /// An invalid regex matches nothing; reading all of history to say so
    /// would be wasted work.
    #[test]
    fn an_invalid_regex_is_answered_without_a_scan() {
        let mut state = fixture();
        let index = index(8);
        let options = TextSearchOptions {
            regex: true,
            ..TextSearchOptions::default()
        };
        assert!(find_with(&mut state, "fix(", options, &index).is_empty());
        let find = &state.repos[0].history_state.find;
        let query = HistoryFindQuery::new("fix(", options).unwrap();
        assert!(find.is_for(&query, &index, state.repos[0].stashes_rev));
        assert!(find.done && find.matches.is_empty() && find.error.is_none());
    }

    #[test]
    fn cancelled_repo_loads_clear_an_unfinished_scan_so_it_can_restart() {
        let mut state = fixture();
        let index = index(8);
        let scan = work(find(&mut state, "fix", &index));
        found(&mut state, scan.seq, vec![1], false);
        // A tab switch or a finished repo action cancels repo loads, and
        // with them the scan and its undelivered replies.
        state.repos[0].bump_load_epoch();
        assert!(scan.cancellation.is_cancelled());
        let results = &state.repos[0].history_state.find;
        assert!(results.query.is_none() && results.matches.is_empty());
        found(&mut state, scan.seq, vec![5], true);
        assert!(state.repos[0].history_state.find.matches.is_empty());
        // The same query over the same index is no longer answered, so the
        // view's next request starts a fresh scan.
        let restarted = work(find(&mut state, "fix", &index));
        assert!(Arc::ptr_eq(&restarted.index, &index));
    }

    #[test]
    fn cancelled_repo_loads_keep_a_finished_scan() {
        let mut state = fixture();
        let index = index(8);
        let scan = work(find(&mut state, "fix", &index));
        found(&mut state, scan.seq, vec![2], true);
        state.repos[0].bump_load_epoch();
        assert_eq!(
            state.repos[0]
                .history_state
                .find
                .match_rows()
                .collect::<Vec<_>>()
                .as_slice(),
            &[2]
        );
        assert!(find(&mut state, "fix", &index).is_empty());
    }

    #[test]
    fn a_failed_scan_keeps_partial_results_and_reports_the_error() {
        let mut state = fixture();
        let index = index(8);
        let work = work(find(&mut state, "fix", &index));
        found(&mut state, work.seq, vec![1], false);
        reduce(
            &mut state,
            work.failed(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("boom".into()),
            )),
        );
        let find = &state.repos[0].history_state.find;
        assert_eq!(find.match_rows().collect::<Vec<_>>().as_slice(), &[1]);
        assert!(find.error.is_some());
    }

    #[test]
    fn a_failed_history_find_can_retry_the_same_query_and_index() {
        let mut state = fixture();
        let index = index(8);
        let first = work(find(&mut state, "fix", &index));
        let first_seq = first.seq;
        reduce(
            &mut state,
            first.failed(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("transient read failure".into()),
            )),
        );
        let retry = work(find(&mut state, "fix", &index));
        assert_ne!(retry.seq, first_seq);
        assert!(state.repos[0].history_state.find.error.is_none());
        found(&mut state, retry.seq, vec![2], true);
        assert_eq!(
            state.repos[0]
                .history_state
                .find
                .match_rows()
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn finished_history_find_does_not_keep_an_obsolete_index_alive() {
        let mut state = fixture();
        let index = index(8);
        let weak = Arc::downgrade(&index);
        let scan = work(find(&mut state, "fix", &index));
        found(&mut state, scan.seq, vec![2], true);
        drop(scan);
        state.repos[0].bump_load_epoch();
        drop(index);
        assert!(
            weak.upgrade().is_none(),
            "inactive find results retain the entire index"
        );
    }
}
