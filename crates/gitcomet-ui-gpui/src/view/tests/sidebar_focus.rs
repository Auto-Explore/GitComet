use super::*;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};

/// Renders the branch sidebar for a repository holding `main`, a `feat/` group
/// with `feat/a` and `feat/b`, and `origin/main`, with the history focused on
/// `focus`.
fn render_branches_with_focus(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    focus: HistoryFocusSet,
) {
    let branch = |name: &str| Branch {
        name: name.to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    };
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("feat/b"),
    ]));
    state.repos[0].remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    state.repos[0].history_state.history_focus = focus;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, view);
}

/// The list indexes of the rows matching `selector_for(ix)`.
fn rendered_rows(
    cx: &mut gpui::VisualTestContext,
    selector_for: impl Fn(usize) -> String,
) -> Vec<usize> {
    (0..40)
        .filter(|&ix| cx.debug_bounds(selector_for(ix).leak()).is_some())
        .collect()
}

fn eye_rows(cx: &mut gpui::VisualTestContext) -> Vec<usize> {
    rendered_rows(cx, |ix| format!("sidebar_focus_eye_{ix}"))
}

#[gpui::test]
fn focus_eyes_appear_only_while_the_history_is_focused(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store_for_view, events, None, window, cx)
    });

    render_branches_with_focus(&store, &view, cx, HistoryFocusSet::default());
    assert!(
        !rendered_rows(cx, |ix| format!("branch_row_1_{ix}")).is_empty(),
        "expected the fixture's branch rows to render"
    );
    assert_eq!(eye_rows(cx), Vec::<usize>::new(), "no eyes without a focus");

    render_branches_with_focus(
        &store,
        &view,
        cx,
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("main")]),
    );
    let eyes = eye_rows(cx);
    let branch_rows = rendered_rows(cx, |ix| format!("branch_row_1_{ix}"));
    let group_rows = rendered_rows(cx, |ix| format!("branch_group_{ix}"));
    assert!(!group_rows.is_empty(), "expected the feat/ group to render");
    for ix in branch_rows.iter().chain(group_rows.iter()) {
        assert!(
            eyes.contains(ix),
            "row {ix} has no eye (eyes={eyes:?}, branches={branch_rows:?}, groups={group_rows:?})"
        );
    }
    assert!(
        eyes.len() > branch_rows.len() + group_rows.len(),
        "the origin header carries an eye too (eyes={eyes:?})"
    );

    // Every eye sits in the same column, whatever the row's depth.
    let lefts: Vec<_> = eyes
        .iter()
        .map(|ix| {
            cx.debug_bounds(format!("sidebar_focus_eye_{ix}").leak())
                .expect("eye bounds")
                .left()
        })
        .collect();
    assert!(
        lefts.windows(2).all(|pair| pair[0] == pair[1]),
        "eyes must line up in one column, got {lefts:?}"
    );
}

/// The group's eye focuses every member at once, leaving what was already
/// focused in place.
#[gpui::test]
fn a_group_eye_focuses_every_branch_in_the_group(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store_for_view, events, None, window, cx)
    });
    render_branches_with_focus(
        &store,
        &view,
        cx,
        HistoryFocusSet::from_iter([HistoryFocus::local_branch("main")]),
    );

    let group_ix = *rendered_rows(cx, |ix| format!("branch_group_{ix}"))
        .first()
        .expect("expected the feat/ group to render");
    let eye = cx
        .debug_bounds(format!("sidebar_focus_eye_{group_ix}").leak())
        .expect("expected the group's eye");
    cx.simulate_click(eye.center(), gpui::Modifiers::default());

    let expected = HistoryFocusSet::from_iter([
        HistoryFocus::local_branch("main"),
        HistoryFocus::local_branch("feat/a"),
        HistoryFocus::local_branch("feat/b"),
    ]);
    wait_until("the group's branches to be focused", || {
        store.snapshot().repos[0].history_state.history_focus == expected
    });
}

/// A branch's eye toggles just that branch.
#[gpui::test]
fn a_branch_eye_unfocuses_its_branch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store_for_view, events, None, window, cx)
    });
    render_branches_with_focus(
        &store,
        &view,
        cx,
        HistoryFocusSet::from_iter([
            HistoryFocus::local_branch("main"),
            HistoryFocus::local_branch("feat/a"),
        ]),
    );

    // Groups sort before branches, so the branch rows come in the order
    // feat/a, feat/b, main, origin/main.
    let branch_rows = rendered_rows(cx, |ix| format!("branch_row_1_{ix}"));
    assert_eq!(
        branch_rows.len(),
        4,
        "unexpected branch rows {branch_rows:?}"
    );
    let main_ix = branch_rows[2];
    let eye = cx
        .debug_bounds(format!("sidebar_focus_eye_{main_ix}").leak())
        .expect("expected main's eye");
    cx.simulate_click(eye.center(), gpui::Modifiers::default());

    let expected = HistoryFocusSet::from_iter([HistoryFocus::local_branch("feat/a")]);
    wait_until("main to leave the focus", || {
        store.snapshot().repos[0].history_state.history_focus == expected
    });
}
