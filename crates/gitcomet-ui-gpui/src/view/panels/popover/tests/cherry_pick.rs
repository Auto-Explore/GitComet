use super::branch::{create_tracking_store, wait_until};
use super::*;
use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};

fn entry(commit_id: &str, action: InteractiveRebaseAction) -> InteractiveRebaseEntry {
    InteractiveRebaseEntry {
        action,
        commit_id: commit_id.to_string(),
        summary: commit_id.to_string(),
        message: commit_id.to_string(),
        new_message: (action == InteractiveRebaseAction::Reword).then(|| "reworded".to_string()),
    }
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

fn open_confirm<'a>(
    cx: &'a mut gpui::TestAppContext,
    label: &str,
    entries: Vec<InteractiveRebaseEntry>,
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    Arc<super::branch::TrackingRepo>,
) {
    let (store, events, repo, _workdir) = create_tracking_store(label);
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::InteractiveCherryPickConfirm { repo_id, entries },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx, repo)
}

#[gpui::test]
fn multi_cherry_pick_confirm_no_picks_without_committing(cx: &mut gpui::TestAppContext) {
    let (view, cx, repo) = open_confirm(
        cx,
        "multi-pick-no",
        vec![
            entry("1111111", InteractiveRebaseAction::Pick),
            entry("2222222", InteractiveRebaseAction::Drop),
            entry("3333333", InteractiveRebaseAction::Pick),
        ],
    );

    click(cx, "interactive_cherry_pick_no");

    wait_until("the uncommitted multi-pick to reach the backend", || {
        repo.actions()
            .iter()
            .any(|action| action == "interactive-cherry-pick:3:commit=false")
    });
    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "No closes the dialog");
}

#[gpui::test]
fn multi_cherry_pick_confirm_yes_commits(cx: &mut gpui::TestAppContext) {
    let (_view, cx, repo) = open_confirm(
        cx,
        "multi-pick-yes",
        vec![
            entry("1111111", InteractiveRebaseAction::Pick),
            entry("2222222", InteractiveRebaseAction::Pick),
        ],
    );

    click(cx, "interactive_cherry_pick_yes");

    wait_until("the committing multi-pick to reach the backend", || {
        repo.actions()
            .iter()
            .any(|action| action == "interactive-cherry-pick:2:commit=true")
    });
}

#[gpui::test]
fn multi_cherry_pick_confirm_disables_no_for_a_reword_plan(cx: &mut gpui::TestAppContext) {
    let (view, cx, repo) = open_confirm(
        cx,
        "multi-pick-reword",
        vec![
            entry("1111111", InteractiveRebaseAction::Pick),
            entry("2222222", InteractiveRebaseAction::Reword),
        ],
    );

    click(cx, "interactive_cherry_pick_no");

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(is_open, "a disabled No leaves the dialog open");
    assert!(
        !repo
            .actions()
            .iter()
            .any(|action| action.starts_with("interactive-cherry-pick")),
        "a reword plan cannot be applied uncommitted: {:?}",
        repo.actions()
    );
}

#[gpui::test]
fn start_cherry_pick_asks_whether_to_commit(cx: &mut gpui::TestAppContext) {
    use crate::view::panels::tests::{app_state_with_repo, push_test_state};
    use gitcomet_state::model::{InteractiveCherryPickSetup, RepoState};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let entries = vec![
        entry(
            "1111111111111111111111111111111111111111",
            InteractiveRebaseAction::Pick,
        ),
        entry(
            "2222222222222222222222222222222222222222",
            InteractiveRebaseAction::Pick,
        ),
    ];
    let mut repo = RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: std::env::temp_dir().join("gitcomet_ui_test_multi_pick_start"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.interactive_cherry_pick_setup = Some(InteractiveCherryPickSetup {
        entries: entries.clone(),
        source_colors: vec![],
        full_messages: Loadable::Ready(()),
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    click(cx, "irebase_start");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert_eq!(
        kind,
        Some(PopoverKind::InteractiveCherryPickConfirm { repo_id, entries }),
        "Start asks before picking"
    );
}

fn cherry_pick_colours(
    cx: &mut gpui::TestAppContext,
    scope: gitcomet_core::domain::LogScope,
    commits: &[(&str, &[&str], &str)],
    branches: &[(&str, &str)],
    stash: Option<&str>,
    picked: [&str; 2],
) -> Vec<(String, u8)> {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(31);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: std::env::temp_dir().join("gitcomet_ui_test_cherry_pick_colours"),
                },
            );
            repo.history_state.history_scope = scope;
            repo.head_branch = Loadable::Ready("main".into());
            repo.branches = Loadable::Ready(Arc::new(
                branches
                    .iter()
                    .map(|(name, target)| gitcomet_core::domain::Branch {
                        name: (*name).into(),
                        target: CommitId((*target).into()),
                        upstream: None,
                        divergence: None,
                    })
                    .collect(),
            ));
            repo.stashes = Loadable::Ready(Arc::new(
                stash
                    .map(|id| gitcomet_core::domain::StashEntry {
                        index: 0,
                        id: CommitId(id.into()),
                        message: "WIP on main: work".into(),
                        created_at: None,
                    })
                    .into_iter()
                    .collect(),
            ));
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: commits
                    .iter()
                    .map(|(id, parents, summary)| gitcomet_core::domain::Commit {
                        id: CommitId((*id).into()),
                        parent_ids: parents.iter().map(|p| CommitId((*p).into())).collect(),
                        summary: (*summary).into(),
                        author: "a".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    })
                    .collect(),
                next_cursor: None,
            }));
            repo.history_state.multi_selection.commits =
                Arc::new(picked.map(|id| CommitId(id.into())).to_vec());
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
        });
    });
    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: CommitId(picked[0].into()),
                        },
                        cx,
                    )
                })
            })
            .expect("expected a commit menu");
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { action, .. } => match action.as_ref() {
                    ContextMenuAction::OpenInteractiveCherryPickSetup { source_colors, .. } => {
                        Some(source_colors.clone())
                    }
                    _ => None,
                },
                _ => None,
            })
            .expect("expected Cherry-pick 2 commits")
    })
}

#[gpui::test]
fn multi_cherry_pick_colours_match_a_head_only_graph(cx: &mut gpui::TestAppContext) {
    // `topic` sits on the merged side; a HEAD-only graph draws no branch
    // heads, so it does not change that commit's colour.
    let colours = cherry_pick_colours(
        cx,
        gitcomet_core::domain::LogScope::FullReachable,
        &[
            ("m", &["a", "b"], "m"),
            ("b", &["base"], "b"),
            ("a", &["base"], "a"),
            ("base", &["root"], "base"),
            ("root", &[], "root"),
        ],
        &[("main", "m"), ("topic", "b")],
        None,
        ["b", "a"],
    );
    assert_eq!(colours, [("a".to_string(), 0), ("b".to_string(), 1)]);
}

#[gpui::test]
fn multi_cherry_pick_colours_skip_stash_helper_commits(cx: &mut gpui::TestAppContext) {
    // The list leaves the stash's index commit out of the graph; counting its
    // lane would shift the colour of every lane that starts below it.
    let colours = cherry_pick_colours(
        cx,
        gitcomet_core::domain::LogScope::AllBranches,
        &[
            ("s", &["m", "i"], "WIP on main: work"),
            ("i", &["m"], "index on main: work"),
            ("t", &["base"], "t"),
            ("m", &["base"], "m"),
            ("u", &["base"], "u"),
            ("base", &["root"], "base"),
            ("root", &[], "root"),
        ],
        &[("main", "m"), ("topic", "t"), ("other", "u")],
        Some("s"),
        ["t", "u"],
    );
    assert_eq!(colours, [("u".to_string(), 3), ("t".to_string(), 2)]);
}
