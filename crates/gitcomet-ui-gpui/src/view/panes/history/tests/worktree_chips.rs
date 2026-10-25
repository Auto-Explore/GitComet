use super::*;
use crate::view::caches::history_worktree_chips_by_head;
use gitcomet_core::domain::{Worktree, WorktreeDirtySummary};

fn worktree(path: impl Into<PathBuf>, head: Option<&str>, branch: Option<&str>) -> Worktree {
    Worktree {
        path: path.into(),
        head: head.map(|head| CommitId(head.into())),
        branch: branch.map(str::to_owned),
        detached: false,
    }
}

#[test]
fn other_worktrees_are_grouped_by_the_commit_they_have_checked_out() {
    let chips = history_worktree_chips_by_head(
        &[
            worktree("/wt/own", Some("c0"), Some("main")),
            worktree("/wt/bare", None, None),
            worktree("/wt/a", Some("c1"), Some("side")),
            worktree("/wt/b", Some("c1"), None),
        ],
        Path::new("/wt/own"),
    );

    assert!(
        !chips.contains_key(&CommitId("c0".into())),
        "this worktree's HEAD is the HEAD row, not a chip"
    );
    let on_c1: Vec<_> = chips[&CommitId("c1".into())]
        .iter()
        .map(|chip| (chip.path.clone(), chip.label.to_string()))
        .collect();
    assert_eq!(
        on_c1,
        vec![
            (PathBuf::from("/wt/a"), "side · a".to_string()),
            (PathBuf::from("/wt/b"), "b".to_string()),
        ]
    );
    assert_eq!(chips.len(), 1, "a worktree with no HEAD gets no chip");
}

struct Mounted<'a> {
    cx: &'a mut gpui::VisualTestContext,
    view: Entity<GitCometView>,
    history: Entity<HistoryView>,
    store: AppStore,
    tip_row: usize,
    base_row: usize,
}

fn worktree_path(folder: &str) -> PathBuf {
    // `/tmp/...` has no drive prefix on Windows, so opening it adds one.
    // Use absolute fixture paths that stay the same when opened.
    std::env::temp_dir()
        .join("history-worktree-chips")
        .join(folder)
}

/// `tip` on `base`, opened at `main` on `tip`; `others` are
/// `(folder, branch)` worktrees on `base`, and `dirty` the folders with changes.
fn mount<'a>(
    cx: &'a mut gpui::TestAppContext,
    others: &[(&str, &str)],
    dirty: &[&str],
) -> Mounted<'a> {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(
        vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
        None,
    ));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: worktree_path("main"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let mut worktrees = vec![worktree(worktree_path("main"), Some("tip"), Some("main"))];
    worktrees.extend(
        others
            .iter()
            .map(|(folder, branch)| worktree(worktree_path(folder), Some("base"), Some(branch))),
    );
    repo.worktrees = Loadable::Ready(Arc::new(worktrees));
    repo.worktrees_rev = 1;
    repo.worktree_dirty = Loadable::Ready(Arc::new(
        dirty
            .iter()
            .map(|folder| WorktreeDirtySummary {
                path: worktree_path(folder),
                head: Some(CommitId("base".into())),
                modified: 1,
                ..Default::default()
            })
            .collect(),
    ));
    repo.worktree_dirty_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    });

    ensure_history_cache_for_tests(cx, &view, state);
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    wait_until(cx, "history rows", |cx| {
        cx.update(|_, app| {
            history
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.row_vms.len() == 2)
        })
    });
    let (tip_row, base_row) = cx.update(|_, app| {
        history.update(app, |history, _| {
            let plan = history.ensure_history_list_plan();
            (plan.list_ix_for_visible(0), plan.list_ix_for_visible(1))
        })
    });
    for _ in 0..2 {
        crate::view::test_support::redraw(cx);
    }
    Mounted {
        cx,
        view,
        history,
        store: store_for_assert,
        tip_row,
        base_row,
    }
}

fn bounds_of(cx: &mut gpui::VisualTestContext, selector: String) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(Box::leak(selector.into_boxed_str()))
}

fn wait_for_open(cx: &mut gpui::VisualTestContext, store: &AppStore, folder: &str) {
    let path = worktree_path(folder);
    wait_until(cx, &format!("{folder} to open"), |_| {
        store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == path)
    });
}

/// A clean worktree has no row of its own, so its chip sits on the commit it
/// has checked out. A dirty one keeps its chip on its changes row.
#[gpui::test]
fn a_clean_worktree_gets_a_chip_on_its_head_commit(cx: &mut gpui::TestAppContext) {
    let Mounted {
        cx,
        view,
        history,
        store,
        tip_row,
        base_row,
    } = mount(cx, &[("clean", "side"), ("dirty", "fix")], &["dirty"]);

    let row = bounds_of(cx, format!("history_row_{base_row}")).expect("the base commit's row");
    let chip = bounds_of(cx, format!("history_worktree_chip_{base_row}"))
        .expect("the clean worktree should have a chip on the commit it has checked out");
    assert!(
        row.contains(&chip.center()) && chip.center().x > row.center().x,
        "the chip belongs at the end of its commit's message, got {chip:?} in {row:?}"
    );
    assert!(
        bounds_of(cx, format!("history_worktree_more_{base_row}")).is_none(),
        "the dirty worktree's chip is on its changes row, so nothing is folded here"
    );
    assert!(
        bounds_of(cx, format!("history_worktree_chip_{tip_row}")).is_none(),
        "this worktree gets no chip"
    );

    // The message ends where the chip starts, so pointing at the chip is not
    // pointing at the message.
    let hover_card_at = |position: Point<Pixels>, cx: &mut gpui::VisualTestContext| {
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        crate::view::test_support::redraw(cx);
        let shown = cx.debug_bounds("commit_message_hover").is_some();
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
        shown
    };
    assert!(
        !hover_card_at(chip.center(), cx),
        "the commit message card must not open over the chip"
    );
    assert!(
        hover_card_at(point(chip.left() - px(40.0), chip.center().y), cx),
        "the message beside the chip still opens its card"
    );

    cx.simulate_click(chip.center(), gpui::Modifiers::default());
    assert!(
        cx.update(|_, app| history.read(app).pending_history_selections.is_empty()),
        "clicking the chip must not also select the commit under it"
    );
    wait_for_open(cx, &store, "clean");

    // Control: the same row, clicked beside the chip, selects its commit.
    cx.simulate_click(
        point(chip.left() - px(40.0), chip.center().y),
        gpui::Modifiers::default(),
    );
    assert!(
        cx.update(|_, app| !history.read(app).pending_history_selections.is_empty()),
        "a click on the row itself should select the commit"
    );
}

/// A commit shows one chip and "+N"; the "+N" opens a menu of every clean
/// worktree there, so the folded ones stay reachable. The chips take at most
/// half the message cell.
#[gpui::test]
fn more_worktrees_on_a_commit_fold_into_a_menu(cx: &mut gpui::TestAppContext) {
    let Mounted {
        cx,
        view,
        store,
        base_row,
        ..
    } = mount(
        cx,
        &[
            ("a-clean-worktree-with-a-long-folder-name", "first"),
            ("another-clean-worktree-with-a-long-name", "second"),
            ("dirty", "fix"),
        ],
        &["dirty"],
    );
    cx.simulate_resize(gpui::size(px(1100.0), px(700.0)));
    crate::view::test_support::redraw(cx);

    let message = cx.debug_bounds("history_message_header_cell").unwrap();
    let chip = bounds_of(cx, format!("history_worktree_chip_{base_row}"))
        .expect("the first worktree's chip");
    let more = bounds_of(cx, format!("history_worktree_more_{base_row}"))
        .expect("the rest fold into a +N chip");
    assert!(
        chip.right() <= more.left() && more.right() <= message.right(),
        "+N follows the chip at the end of the message cell"
    );
    assert!(
        chip.left() >= message.center().x - px(8.0),
        "the chips take at most half the message cell: {chip:?} in {message:?}"
    );

    cx.simulate_click(more.center(), gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert_eq!(
        cx.update(|_, app| crate::view::test_support::popover_kind(view.read(app), app)),
        Some(PopoverKind::worktree(
            RepoId(1),
            WorktreePopoverKind::CommitMenu {
                commit_id: CommitId("base".into()),
            },
        ))
    );
    assert!(cx.debug_bounds("history_worktree_menu_0").is_some());
    assert!(
        cx.debug_bounds("history_worktree_menu_2").is_none(),
        "the dirty worktree is not on this row, so not in its menu"
    );
    let second = cx
        .debug_bounds("history_worktree_menu_1")
        .expect("the folded worktree is in the menu");
    cx.simulate_click(second.center(), gpui::Modifiers::default());
    wait_for_open(cx, &store, "another-clean-worktree-with-a-long-name");
}
