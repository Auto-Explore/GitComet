use super::*;
use gitcomet_core::services::CancellationToken;

#[test]
fn author_catalog_includes_unloaded_history_and_reuses_unchanged_inputs() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history_with_authors(dir.path(), 600, |row| {
        if row < 300 {
            "Older author <old@example.com>"
        } else {
            "Recent author <recent@example.com>"
        }
    });
    for graph in [false, true] {
        if graph {
            run_git(dir.path(), &["commit-graph", "write", "--reachable"]);
        }
        let repo = GixBackend.open(dir.path()).unwrap();
        let cancel = CancellationToken::new();
        let page = repo
            .log_history_mode_page(HistoryMode::FullReachable, 200, None)
            .unwrap();
        assert!(
            page.commits
                .iter()
                .all(|commit| commit.author.as_ref() == "Recent author")
        );
        let names = repo
            .history_authors(HistoryMode::FullReachable, &cancel)
            .unwrap();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|name| name.as_ref() == "Older author"));
        repo.log_history_mode_page_filtered(
            HistoryMode::FullReachable,
            Some("Recent author"),
            200,
            None,
        )
        .unwrap();
        let again = repo
            .history_authors(HistoryMode::FullReachable, &cancel)
            .unwrap();
        assert!(Arc::ptr_eq(&names, &again));
        cancel.cancel();
        assert!(matches!(
            repo.history_authors(HistoryMode::FullReachable, &cancel)
                .unwrap_err()
                .kind(),
            ErrorKind::Cancelled
        ));
    }
}

#[test]
fn author_catalog_matches_history_scopes_and_refreshes_when_heads_move() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history_with_authors(dir.path(), 3, |_| "Base <base@example.com>");
    run_git(dir.path(), &["checkout", "-qb", "feature", "HEAD~1"]);
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Feature",
            "-c",
            "user.email=feature@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "feature",
        ],
    );
    run_git(dir.path(), &["checkout", "-q", "master"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    let cancel = CancellationToken::new();
    let names = |mode| {
        repo.history_authors(mode, &cancel)
            .unwrap()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(HistoryMode::FullReachable), ["Base"]);
    assert!(names(HistoryMode::AllBranches).contains(&"Feature".to_string()));
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Merger",
            "-c",
            "user.email=merge@example.com",
            "merge",
            "--no-ff",
            "-qm",
            "merge",
            "feature",
        ],
    );
    // Re-query the cached scope after moving a tip, before changing modes.
    assert!(names(HistoryMode::AllBranches).contains(&"Merger".to_string()));
    assert_eq!(names(HistoryMode::MergesOnly), ["Merger"]);
    assert_eq!(names(HistoryMode::FirstParent), ["Merger", "Base"]);
    let full = names(HistoryMode::FullReachable);
    assert_eq!(full.len(), 3);
    let no_merges = names(HistoryMode::NoMerges);
    assert_eq!(no_merges.len(), 2);
    assert!(!no_merges.contains(&"Merger".to_string()));
}

#[test]
fn author_catalog_handles_empty_repositories() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q"]);
    let repo = GixBackend.open(dir.path()).unwrap();
    for mode in [
        HistoryMode::AllBranches,
        HistoryMode::FullReachable,
        HistoryMode::FirstParent,
    ] {
        assert!(
            repo.history_authors(mode, &CancellationToken::new())
                .unwrap()
                .is_empty()
        );
    }
}
