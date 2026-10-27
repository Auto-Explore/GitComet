use super::*;
use gitcomet_extension_api::Registry;

use crate::view::test_support::{drain_store_worker, redraw};

/// A window showing one repository's History of `commits` commits, the log's
/// last page, in `scope`.
fn history_of(
    cx: &mut gpui::TestAppContext,
    commits: usize,
    scope: LogScope,
) -> (Entity<GitCometView>, AppStore, &mut gpui::VisualTestContext) {
    // A window gets an extension host once some extension is installed.
    cx.update(|cx| {
        let registry = Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .unwrap();
        crate::view::extension_host::install(registry, cx);
    });
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let commits = (0..commits)
        .map(|ix| commit(&format!("c{ix:03}"), &[], &format!("commit {ix:03}")))
        .collect::<Vec<_>>();
    let page = Arc::new(log_page(commits, None));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-host-reveal"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.history_state.history_scope = scope;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "c000")]));
    repo.branches_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    });
    store.replace_snapshot_for_test(Arc::clone(&state));
    cx.update(|_window, app| {
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| model.set_state(Arc::clone(&state), cx));
    });
    cx.run_until_parked();
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_3").is_some()
    });
    (view, store, cx)
}

fn reveal(view: &Entity<GitCometView>, commit: &str, cx: &mut gpui::VisualTestContext) {
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let repository = cx
        .update(|_window, app| host.active_repository(app))
        .unwrap()
        .expect("the repository is open");
    cx.update(|_window, app| host.reveal_commit(&repository, &CommitId(commit.into()), app))
        .expect("the repository is open");
    for _ in 0..4 {
        cx.run_until_parked();
        redraw(cx);
    }
}

/// An extension shows a commit as the sidebar shows a branch's tip: History
/// in place of the view it was on, the commit selected and scrolled to, far
/// down the log, and History's mode as it was.
#[gpui::test]
fn an_extension_reveals_a_commit_selected_and_in_view(cx: &mut gpui::TestAppContext) {
    let (view, store, cx) = history_of(cx, 200, LogScope::FullReachable);
    assert!(
        cx.debug_bounds("history_row_150").is_none(),
        "not drawn yet"
    );
    let tab = cx
        .debug_bounds("repository_view_0")
        .expect("the example's view tab")
        .center();
    cx.simulate_click(tab, gpui::Modifiers::default());
    cx.run_until_parked();
    redraw(cx);
    assert!(cx.debug_bounds("example_review_view").is_some());

    reveal(&view, "c150", cx);
    assert!(
        cx.debug_bounds("example_review_view").is_none(),
        "History is shown again"
    );
    drain_store_worker(&view, cx);
    let selected = store.snapshot().repos[0]
        .history_state
        .selected_commit
        .clone();
    assert_eq!(selected, Some(CommitId("c150".into())));
    assert!(
        cx.debug_bounds("history_row_150").is_some(),
        "its row is scrolled into view"
    );
    assert_eq!(
        store.snapshot().repos[0].history_state.history_scope,
        LogScope::FullReachable,
        "a commit History shows leaves its mode alone"
    );
}

/// A commit outside History's mode, with every page of it loaded, switches
/// History to all branches, as a sidebar click on another branch does.
#[gpui::test]
fn a_commit_history_leaves_out_shows_all_branches(cx: &mut gpui::TestAppContext) {
    let (view, store, cx) = history_of(cx, 6, LogScope::FullReachable);

    reveal(&view, "elsewhere", cx);
    drain_store_worker(&view, cx);
    assert_eq!(
        store.snapshot().repos[0].history_state.history_scope,
        LogScope::AllBranches
    );
}
