use super::*;

#[gpui::test]
fn worktrees_section_shows_spinner_while_removing_worktree(cx: &mut gpui::TestAppContext) {
    // The removal must actually run: a repo the backend cannot open finishes
    // its actions at once with a missing-handle error, spinner and all.
    let backend = Arc::new(SubmoduleTestBackend::default());
    let gate = backend.gate.clone();
    let (store, events) = AppStore::new_test(backend);
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_worktrees_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let pending = gate.hold();
    store_for_test.dispatch(Msg::RemoveWorktree {
        repo_id,
        path: base.join("repo1").join("worktree_to_remove"),
        delete_branch: None,
    });

    let selector = worktrees_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        if cx.debug_bounds(selector).is_some() && pending.has_started() {
            break;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for worktrees spinner to render");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn submodules_section_shows_spinner_while_loading(cx: &mut gpui::TestAppContext) {
    let backend = Arc::new(SubmoduleTestBackend::default());
    let gate = backend.gate.clone();
    let (store, events) = AppStore::new_test(backend);
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    // Idle metadata prefetch can start as soon as the repository opens.
    // Hold it before restoring the session so expansion cannot race completion.
    let pending = gate.hold();
    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_submodules_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let section_ix = wait_for_debug_index(cx, &_view, "submodules_section", 64);
    let section_selector = debug_selector("submodules_section", section_ix);
    click_debug_selector(cx, section_selector, 1);

    let selector = submodules_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        let repo_requested = store_for_test
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.sidebar_data_request.submodules && matches!(repo.submodules, Loadable::Loading)
            });

        if repo_requested && cx.debug_bounds(selector).is_some() && pending.has_started() {
            break;
        }

        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for submodules spinner to render: {:?}",
                store_for_test
                    .snapshot()
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .map(|repo| (&repo.sidebar_data_request, &repo.submodules))
            );
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

#[gpui::test]
fn stash_section_shows_spinner_while_loading(cx: &mut gpui::TestAppContext) {
    let backend = Arc::new(StashTestBackend::default());
    let gate = backend.gate.clone();
    // The expanded sidebar requests stashes as soon as the repository opens.
    let pending = gate.hold();
    let (store, events) = AppStore::new_test(backend);
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_stash_spinner_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);

    let section_ix = wait_for_debug_index(cx, &_view, "stash_section", 64);
    let section_selector = debug_selector("stash_section", section_ix);
    click_debug_selector(cx, section_selector, 1);

    let selector = stash_spinner_selector(repo_id);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        sync_view_for_tests(cx, &_view);

        let repo_requested = store_for_test
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| {
                repo.sidebar_data_request.stashes && matches!(repo.stashes, Loadable::Loading)
            });

        if repo_requested && cx.debug_bounds(selector).is_some() && pending.has_started() {
            break;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for stash spinner to render");
        }

        cx.run_until_parked();
        std::thread::yield_now();
    }
}

fn worktrees_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("worktrees_spinner_{}", repo_id.0).into_boxed_str())
}

fn submodules_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("submodules_spinner_{}", repo_id.0).into_boxed_str())
}

fn stash_spinner_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("stash_spinner_{}", repo_id.0).into_boxed_str())
}
