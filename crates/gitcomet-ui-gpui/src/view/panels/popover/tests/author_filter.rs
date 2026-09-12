use super::*;
// Explicit: `use super::*` would otherwise resolve `author_filter` to this test
// module rather than the popover panel it exercises.
use crate::view::panels::popover::author_filter;
use crate::view::panels::tests::{app_state_with_repo, push_test_state};
use gitcomet_core::domain::{Commit, CommitId, CommitParentIds, LogPage};

fn repo_with_authors(repo_id: RepoId, log_rev: u64, authors: &[&str]) -> RepoState {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_author_filter_memo",
        std::process::id()
    ));
    let mut repo = RepoState::new_opening(repo_id, gitcomet_core::domain::RepoSpec { workdir });
    let page: Loadable<Arc<LogPage>> = Loadable::Ready(
        LogPage {
            commits: authors
                .iter()
                .enumerate()
                .map(|(ix, author)| Commit {
                    id: CommitId(format!("{ix:016x}").into()),
                    parent_ids: CommitParentIds::new(),
                    summary: "msg".into(),
                    author: (*author).into(),
                    time: SystemTime::UNIX_EPOCH,
                })
                .collect(),
            next_cursor: None,
        }
        .into(),
    );
    repo.log = page.clone();
    repo.history_state.log = page;
    repo.history_state.log_rev = log_rev;
    repo
}

fn suggestion_names(authors: &[SharedString]) -> Vec<String> {
    authors.iter().map(|a| a.to_string()).collect()
}

/// The dropdown re-renders on every mouse move over it, so collecting authors —
/// a walk of the whole accumulated log — must happen once per log revision, not
/// once per frame.
#[gpui::test]
fn author_suggestions_are_reused_until_the_log_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9001);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_authors(repo_id, 1, &["Bob", "Alice", "bob"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    let (first, second) = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, _cx| {
                (
                    author_filter::suggestions(host, repo_id),
                    author_filter::suggestions(host, repo_id),
                )
            })
        })
    });

    assert_eq!(suggestion_names(&first), vec!["Alice", "Bob"]);
    assert!(
        Arc::ptr_eq(&first, &second),
        "a second call at the same log revision must reuse the collected list"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_authors(repo_id, 2, &["Bob", "Alice", "Carol"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    let third = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, _cx| author_filter::suggestions(host, repo_id))
        })
    });

    assert_eq!(
        suggestion_names(&third),
        vec!["Alice", "Bob", "Carol"],
        "a new log revision must refresh the suggestions"
    );
}

/// Once a filter is applied the log holds that author's commits alone.
/// Recollecting from it would leave the dropdown showing only the name already
/// selected, with no way to switch to anyone else.
#[gpui::test]
fn author_suggestions_survive_an_applied_filter(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9002);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_authors(repo_id, 1, &["Alice", "Bob"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, _cx| author_filter::suggestions(host, repo_id));
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = repo_with_authors(repo_id, 2, &["Alice"]);
            repo.history_state.history_author_filter = Some("Alice".into());
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    let filtered = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, _cx| author_filter::suggestions(host, repo_id))
        })
    });

    assert_eq!(suggestion_names(&filtered), vec!["Alice", "Bob"]);
}

#[gpui::test]
fn completed_author_catalog_replaces_bootstrap_suggestions_without_a_log_change(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9003);
    let mut repo = repo_with_authors(repo_id, 1, &["Recent author"]);
    repo.history_state.history_author_filter = Some("Recent author".into());
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo.clone(), repo_id), cx)
        })
    });
    let before = cx.update(|_, app| {
        view.read(app)
            .popover_host
            .clone()
            .update(app, |host, _| author_filter::suggestions(host, repo_id))
    });
    assert_eq!(suggestion_names(&before), ["Recent author"]);

    repo.history_state.authors =
        gitcomet_state::history_authors::HistoryAuthorsState::loaded_for_test(
            &repo,
            vec![
                Arc::from("Older author"),
                Arc::from("Recent author"),
                Arc::from("older author"),
            ]
            .into(),
        );
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo.clone(), repo_id), cx)
        })
    });
    let (after, again) = cx.update(|_, app| {
        view.read(app).popover_host.clone().update(app, |host, _| {
            (
                author_filter::suggestions(host, repo_id),
                author_filter::suggestions(host, repo_id),
            )
        })
    });
    assert_eq!(suggestion_names(&after), ["Older author", "Recent author"]);
    assert!(Arc::ptr_eq(&after, &again));
    assert!(!Arc::ptr_eq(&before, &after));

    // The complete list survives a filtered page update and a pending rescan.
    repo.history_state.log_rev += 1;
    repo.history_state.authors.names = Loadable::Loading;
    repo.history_state.authors.rev += 1;
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx)
        })
    });
    let pending = cx.update(|_, app| {
        view.read(app)
            .popover_host
            .clone()
            .update(app, |host, _| author_filter::suggestions(host, repo_id))
    });
    assert!(Arc::ptr_eq(&after, &pending));
}

#[gpui::test]
fn author_catalog_from_another_scope_is_not_offered(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9004);
    let mut repo = repo_with_authors(repo_id, 1, &["Current branch author"]);
    repo.history_state.history_scope = gitcomet_core::domain::HistoryMode::AllBranches;
    repo.history_state.authors =
        gitcomet_state::history_authors::HistoryAuthorsState::loaded_for_test(
            &repo,
            vec![Arc::from("Other branch author")].into(),
        );
    repo.history_state.history_scope = gitcomet_core::domain::HistoryMode::FirstParent;
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx)
        })
    });
    let names = cx.update(|_, app| {
        view.read(app)
            .popover_host
            .clone()
            .update(app, |host, _| author_filter::suggestions(host, repo_id))
    });
    assert_eq!(suggestion_names(&names), ["Current branch author"]);
}
