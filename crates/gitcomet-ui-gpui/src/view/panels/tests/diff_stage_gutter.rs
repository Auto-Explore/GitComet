use super::*;

use super::shortcuts::{app_state_with_active_repo, apply_state, wait_until};
use crate::view::rows::{DiffStageHover, DiffStageSlot};
use gitcomet_core::domain::DiffLineKind;

const STAGE_GUTTER_PATH: &str = "src/lib.rs";
/// A path git cannot write unambiguously on the `diff --git` line, taken from a
/// real repository where staging single lines used to fail because of it.
const STAGE_GUTTER_SPACED_PATH: &str = "src/rules - Copy - Copy - Copy.rs";
const STAGE_GUTTER_OLD_TEXT: &str = "context one\nold one\nold two\ncontext two\n";
const STAGE_GUTTER_NEW_TEXT: &str = "context one\nnew one\nnew two\ncontext two\n";

/// One hunk with two removals and two additions, so a per-line patch has to
/// prove it dropped the other addition and demoted the other removal. Shaped
/// exactly like `git diff` writes it, including the TAB it appends after a name
/// containing a space so the two halves of the header can be told apart.
fn stage_gutter_unified(path: &str) -> String {
    let tab = if path.contains(' ') { "\t" } else { "" };
    format!(
        "diff --git a/{path} b/{path}\n\
         --- a/{path}{tab}\n\
         +++ b/{path}{tab}\n\
         @@ -1,4 +1,4 @@\n\
         \x20context one\n\
         -old one\n\
         -old two\n\
         +new one\n\
         +new two\n\
         \x20context two\n"
    )
}

fn stage_gutter_repo(
    repo_id: RepoId,
    workdir: &Path,
    target: DiffTarget,
) -> gitcomet_state::model::RepoState {
    let path = match &target {
        DiffTarget::WorkingTree { path, .. } => path.clone(),
        DiffTarget::Commit { path, .. } => path.clone().unwrap_or_default(),
        DiffTarget::CommitRange { path, .. } => path.clone().unwrap_or_default(),
    };
    let unified = stage_gutter_unified(&path.to_string_lossy());
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());

    let area = match &target {
        DiffTarget::WorkingTree { area, .. } => *area,
        _ => DiffArea::Unstaged,
    };
    set_test_file_status(
        &mut repo,
        path.clone(),
        gitcomet_core::domain::FileStatusKind::Modified,
        area,
    );
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff_state_rev = 1;
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff::from_unified(
        target, &unified,
    )));
    repo.diff_state.diff_file_rev = 1;
    repo.diff_state.diff_file =
        Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
            path,
            Some(STAGE_GUTTER_OLD_TEXT.to_string()),
            Some(STAGE_GUTTER_NEW_TEXT.to_string()),
        ))));
    repo
}

fn worktree_target(area: DiffArea) -> DiffTarget {
    worktree_target_at(STAGE_GUTTER_PATH, area)
}

fn worktree_target_at(path: &str, area: DiffArea) -> DiffTarget {
    DiffTarget::WorkingTree {
        path: std::path::PathBuf::from(path),
        area,
    }
}

/// Open a window showing the fixture diff in the default (whole-file) view and
/// wait until its rows are rendered.
fn open_stage_gutter_view(
    cx: &mut gpui::TestAppContext,
    target: DiffTarget,
    diff_view: DiffViewMode,
) -> (
    gpui::Entity<super::super::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_stage_gutter",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let repo = stage_gutter_repo(RepoId(70910), &workdir, target);
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "the file diff view to render its rows",
        |pane| pane.is_file_diff_view_active() && pane.diff_visible_len() > 0,
        |pane| {
            format!(
                "file_diff_active={} visible_len={}",
                pane.is_file_diff_view_active(),
                pane.diff_visible_len(),
            )
        },
    );
    (view, cx)
}

/// Source index of the patch line with this exact unified text.
fn src_ix_for_text(pane: &MainPaneView, text: &str) -> usize {
    (0..pane.patch_diff_row_len())
        .find(|src_ix| {
            pane.patch_diff_row(*src_ix)
                .is_some_and(|line| line.text.as_ref() == text)
        })
        .unwrap_or_else(|| panic!("expected a patch line reading {text:?}"))
}

/// Visible row rendering the patch line with this text, as the gutter button's
/// click handler sees it.
fn visible_ix_for_text(pane: &MainPaneView, text: &str) -> usize {
    let src_ix = src_ix_for_text(pane, text);
    (0..pane.diff_visible_len())
        .find(|visible_ix| {
            pane.diff_src_ixs_for_visible_ix(*visible_ix)
                .contains(&src_ix)
        })
        .unwrap_or_else(|| panic!("expected a visible row for {text:?}"))
}

fn stage_gutter_patch(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    text: &str,
    kind: DiffLineKind,
) -> Option<String> {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = visible_ix_for_text(&pane, text);
        pane.diff_stage_gutter_patch(visible_ix, kind)
    })
}

fn stage_gutter_cell(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    text: &str,
    slot: DiffStageSlot,
) -> (usize, gpui::Bounds<Pixels>) {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = visible_ix_for_text(&pane, text);
        let cell = *pane
            .diff_stage_gutter_cells
            .get(&(visible_ix, slot))
            .unwrap_or_else(|| {
                panic!("expected {text:?} to paint a stage gutter cell in {slot:?}")
            });
        (visible_ix, cell)
    })
}

#[gpui::test]
fn stage_gutter_patch_keeps_only_the_clicked_added_line(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let patch = stage_gutter_patch(cx, &view, "+new two", DiffLineKind::Add)
        .expect("expected a patch for the clicked added line");

    assert_eq!(
        patch,
        concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,4 +1,4 @@\n",
            " context one\n",
            " old one\n",
            " old two\n",
            "+new two\n",
            " context two\n",
        ),
        "the other addition must be dropped and both removals kept as context"
    );
}

/// Regression: the whole-file view matches a rendered row back to its patch
/// line by file path, so a path the header parser could not read left every
/// lookup empty and the gutter button could only report that it had failed.
#[gpui::test]
fn stage_gutter_patch_works_for_a_path_containing_spaces(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target_at(STAGE_GUTTER_SPACED_PATH, DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let path = STAGE_GUTTER_SPACED_PATH;
    assert_eq!(
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.diff_file_for_src_ix
                .iter()
                .filter_map(|file| file.as_deref().map(str::to_string))
                .collect::<std::collections::BTreeSet<_>>()
        }),
        std::collections::BTreeSet::from([path.to_string()]),
        "every patch line must resolve to the spaced path"
    );

    let patch = stage_gutter_patch(cx, &view, "+new two", DiffLineKind::Add)
        .expect("a spaced path must still build a per-line patch");

    assert_eq!(
        patch,
        format!(
            "diff --git a/{path} b/{path}\n\
             --- a/{path}\t\n\
             +++ b/{path}\t\n\
             @@ -1,4 +1,4 @@\n\
             \x20context one\n\
             \x20old one\n\
             \x20old two\n\
             +new two\n\
             \x20context two\n"
        ),
        "the header lines must be copied through verbatim, tabs included"
    );
}

#[gpui::test]
fn stage_gutter_patch_keeps_only_the_clicked_removed_line(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let patch = stage_gutter_patch(cx, &view, "-old one", DiffLineKind::Remove)
        .expect("expected a patch for the clicked removed line");

    assert_eq!(
        patch,
        concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,4 +1,4 @@\n",
            " context one\n",
            "-old one\n",
            " old two\n",
            " context two\n",
        ),
        "the other removal must be kept as context and both additions dropped"
    );
}

#[gpui::test]
fn stage_gutter_resolves_each_split_column_to_its_own_line(cx: &mut gpui::TestAppContext) {
    let (view, cx) =
        open_stage_gutter_view(cx, worktree_target(DiffArea::Unstaged), DiffViewMode::Split);

    // A split row aligns a removal with its replacement, so both columns paint a
    // button on the same row: each must act on the change its own side shows.
    let (removed_ix, _) = stage_gutter_cell(cx, &view, "-old one", DiffStageSlot::SplitLeft);
    let (added_ix, _) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::SplitRight);
    assert_eq!(
        removed_ix, added_ix,
        "the fixture's first change should render as one aligned split row"
    );

    let added = stage_gutter_patch(cx, &view, "+new one", DiffLineKind::Add)
        .expect("expected a patch for the added line in the right column");
    let removed = stage_gutter_patch(cx, &view, "-old one", DiffLineKind::Remove)
        .expect("expected a patch for the removed line in the left column");

    assert!(
        added.contains("+new one\n") && !added.contains("-old one\n"),
        "right column staged the wrong line: {added}"
    );
    assert!(
        removed.contains("-old one\n") && !removed.contains("+new one\n"),
        "left column staged the wrong line: {removed}"
    );
}

#[gpui::test]
fn stage_gutter_builds_a_reverse_appliable_patch_for_a_staged_diff(cx: &mut gpui::TestAppContext) {
    let (view, cx) =
        open_stage_gutter_view(cx, worktree_target(DiffArea::Staged), DiffViewMode::Inline);

    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_stage_gutter_area()),
        Some(DiffArea::Staged)
    );

    let patch = stage_gutter_patch(cx, &view, "+new one", DiffLineKind::Add)
        .expect("expected a patch for the clicked added line");

    // Unstaging applies this in reverse, so the side it has to match is the
    // index: the addition left alone stays as context and the removals, which
    // the index does not contain, are dropped. Building it the staging way
    // instead makes `git apply --cached --reverse` reject the patch.
    assert_eq!(
        patch,
        concat!(
            "diff --git a/src/lib.rs b/src/lib.rs\n",
            "--- a/src/lib.rs\n",
            "+++ b/src/lib.rs\n",
            "@@ -1,4 +1,4 @@\n",
            " context one\n",
            "+new one\n",
            " new two\n",
            " context two\n",
        ),
    );
}

#[gpui::test]
fn stage_gutter_is_disabled_for_commit_diffs(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        DiffTarget::Commit {
            commit_id: CommitId("abcdef00112233bb".into()),
            path: Some(std::path::PathBuf::from("src/lib.rs")),
        },
        DiffViewMode::Inline,
    );

    let (area, cells) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.diff_stage_gutter_area(),
            pane.diff_stage_gutter_cells.len(),
        )
    });

    assert_eq!(area, None, "a commit diff has no index to stage lines into");
    assert_eq!(cells, 0, "no stage button may be painted for a commit diff");
}

#[gpui::test]
fn stage_gutter_button_hover_follows_the_pointer(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let (visible_ix, cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    cx.simulate_mouse_move(cell.center(), None, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_stage_gutter_hover),
        Some(DiffStageHover {
            visible_ix,
            slot: DiffStageSlot::Inline,
            on_button: true,
        }),
        "the pointer on the button should mark it as the active target"
    );

    // Anywhere else in the same row still shows the button, just not lit up.
    let text_position = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_text_hitboxes
            .get(&(visible_ix, DiffTextRegion::Inline))
            .expect("expected an inline text hitbox for the added line")
            .bounds
            .center()
    });
    cx.simulate_mouse_move(text_position, None, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_stage_gutter_hover),
        Some(DiffStageHover {
            visible_ix,
            slot: DiffStageSlot::Inline,
            on_button: false,
        }),
        "hovering the line should reveal its button without lighting it up"
    );

    // A context row has no button, so leaving the change row hides it again.
    let context_position = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let context_ix = visible_ix_for_text(&pane, " context one");
        pane.diff_text_hitboxes
            .get(&(context_ix, DiffTextRegion::Inline))
            .expect("expected an inline text hitbox for the context line")
            .bounds
            .center()
    });
    cx.simulate_mouse_move(context_position, None, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_stage_gutter_hover),
        None,
        "moving to a line that cannot be staged should hide the button"
    );
}

#[gpui::test]
fn clicking_stage_gutter_button_stages_without_moving_the_row_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let selection = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            let anchor = visible_ix_for_text(pane, "-old one");
            pane.diff_selection_anchor = Some(anchor);
            pane.diff_selection_range = Some((anchor, anchor));
            cx.notify();
            (anchor, anchor)
        })
    });
    draw_and_drain_test_window(cx);

    let (_, cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    simulate_counted_click(cx, cell.center(), 1);
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_selection_range),
        Some(selection),
        "clicking the gutter button must not move the diff row selection"
    );

    // The reducer marks a local action in flight as soon as it accepts the patch
    // message, which is as far as this backend-less harness can follow it.
    wait_until(cx, "the store to accept the staging command", |cx| {
        cx.update(|_window, app| {
            let snapshot = view.read(app).store.snapshot();
            snapshot
                .repos
                .first()
                .is_some_and(|repo| repo.local_actions_in_flight > 0)
        })
    });
}

#[gpui::test]
fn stage_gutter_hover_clears_when_its_button_stops_being_painted(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let (visible_ix, cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    cx.simulate_mouse_move(cell.center(), None, Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .main_pane
            .read(app)
            .diff_stage_gutter_hover
            .map(|hover| hover.visible_ix)),
        Some(visible_ix),
    );

    // Scrolling under a still pointer delivers no mouse move, so the hovered row
    // never gets to clear itself; here the row simply stops painting a button.
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_stage_gutter_cells.clear();
        });
    });
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_stage_gutter_hover),
        None,
        "a hover whose button is no longer painted must not stay pinned"
    );
}

#[gpui::test]
fn releasing_a_stage_gutter_press_does_not_click_the_row(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    let (visible_ix, cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    cx.simulate_mouse_move(cell.center(), None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: cell.center(),
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.update(|_window, app| crate::press_gesture::is_press_claimed(app)),
        "the button must own the press so other release handlers stand down"
    );

    // Park a sentinel selection and disarm the reload autoscroll, so the only
    // thing that can move the selection from here is a row click.
    let sentinel = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            let sentinel = (0, 0);
            pane.diff_selection_anchor = Some(sentinel.0);
            pane.diff_selection_range = Some(sentinel);
            pane.diff_autoscroll_pending = false;
            cx.notify();
            sentinel
        })
    });

    // Staging reloads the diff, so the release lands on a repainted row whose
    // handlers know nothing about the press. Releasing over the row's text
    // stands in for that: unguarded, it would select the row.
    let text_position = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_text_hitboxes
            .get(&(visible_ix, DiffTextRegion::Inline))
            .expect("expected an inline text hitbox for the added line")
            .bounds
            .center()
    });
    cx.simulate_event(MouseUpEvent {
        position: text_position,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_selection_range),
        Some(sentinel),
        "a release that belongs to the gutter button must not select a row"
    );
}
