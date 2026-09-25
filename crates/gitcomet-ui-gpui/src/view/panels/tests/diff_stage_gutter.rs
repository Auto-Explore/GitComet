use super::*;

use super::shortcuts::{app_state_with_active_repo, apply_state};
use crate::view::rows::{DiffStageHover, DiffStageSlot};
use crate::view::test_support::{drain_store_worker, repo_ops_rev};
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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

/// The button sits in the line-number gutter's empty slack, but hiding line
/// numbers must not take it away with them: it then overlaps the first
/// characters of the line instead, which is the lesser cost of the two. Sizing
/// the button out of a row it has no room in is the failure this guards.
#[gpui::test]
fn stage_gutter_is_painted_whether_or_not_line_numbers_are_shown(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_stage_gutter_view(
        cx,
        worktree_target(DiffArea::Unstaged),
        DiffViewMode::Inline,
    );

    // With line numbers shown the button fits in the gutter, clear of the text.
    let (visible_ix, cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    let text_left =
        diff_row_text_left(cx, &view, visible_ix).expect("the row must have painted a text hitbox");
    assert!(
        cell.right() <= text_left,
        "with a gutter to sit in the button must stay left of the diff text: cell ends at {:?}, text starts at {text_left:?}",
        cell.right(),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_show_line_numbers = false;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let (_, narrow_cell) = stage_gutter_cell(cx, &view, "+new one", DiffStageSlot::Inline);
    assert_eq!(
        narrow_cell.size, cell.size,
        "the button must still be painted, at full size, with the gutter gone"
    );
}

/// Left edge of a row's diff text, as its hitbox reports it.
fn diff_row_text_left(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    visible_ix: usize,
) -> Option<Pixels> {
    cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_text_hitboxes
            .iter()
            .filter(|((row_ix, _), _)| *row_ix == visible_ix)
            .map(|(_, hitbox)| hitbox.bounds.left())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    })
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
    let ops_rev_before = crate::view::test_support::repo_ops_rev(&view, cx, RepoId(70910));
    simulate_counted_click(cx, cell.center(), 1);
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_selection_range),
        Some(selection),
        "clicking the gutter button must not move the diff row selection"
    );

    // The reducer bumps the repo's ops revision as soon as it accepts the patch
    // message, which is as far as this backend-less harness can follow it.
    crate::view::test_support::drain_store_worker(&view, cx);
    assert!(
        crate::view::test_support::repo_ops_rev(&view, cx, RepoId(70910)) > ops_rev_before,
        "the store must accept the staging command"
    );
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

#[gpui::test]
fn stage_gutter_cancels_presses_released_on_another_button(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for area in [DiffArea::Unstaged, DiffArea::Staged] {
        for mode in [DiffViewMode::Inline, DiffViewMode::Split] {
            let (view, cx) = open_stage_gutter_view(cx, worktree_target(area), mode);
            let slot = if mode == DiffViewMode::Inline {
                DiffStageSlot::Inline
            } else {
                DiffStageSlot::SplitRight
            };
            let (_, first) = stage_gutter_cell(cx, &view, "+new one", slot);
            let (_, second) = stage_gutter_cell(cx, &view, "+new two", slot);
            // This backend can finish an action before the next snapshot. Check
            // the persistent revision after draining the worker, so even a
            // completed accidental action fails the cancellation assertions.
            let repo_id = RepoId(70910);
            drain_store_worker(&view, cx);
            let ops_rev_before = repo_ops_rev(&view, cx, repo_id);
            cx.simulate_mouse_down(first.center(), MouseButton::Left, Modifiers::default());
            draw_and_drain_test_window(cx);
            drain_store_worker(&view, cx);
            assert_eq!(
                repo_ops_rev(&view, cx, repo_id),
                ops_rev_before,
                "pressing a gutter must not stage yet ({area:?}, {mode:?})"
            );
            cx.simulate_mouse_move(
                second.center(),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            cx.simulate_mouse_up(second.center(), MouseButton::Left, Modifiers::default());
            draw_and_drain_test_window(cx);
            drain_store_worker(&view, cx);
            assert_eq!(
                repo_ops_rev(&view, cx, repo_id),
                ops_rev_before,
                "releasing on another gutter must activate neither ({area:?}, {mode:?})"
            );
            simulate_counted_click(cx, second.center(), 1);
            draw_and_drain_test_window(cx);
            drain_store_worker(&view, cx);
            assert!(
                repo_ops_rev(&view, cx, repo_id) > ops_rev_before,
                "a completed gutter click must dispatch an action ({area:?}, {mode:?})"
            );
        }
    }
}

/// A window showing a generated `src/big.rs` diff of `lines` lines, every
/// fifth one changed, drawn in `mode`.
fn open_generated_diff_view(
    cx: &mut gpui::TestAppContext,
    repo_id: RepoId,
    lines: usize,
    mode: DiffViewMode,
) -> (
    gpui::Entity<super::super::GitCometView>,
    &mut gpui::VisualTestContext,
    std::path::PathBuf,
) {
    use std::fmt::Write as _;

    let (mut old_text, mut new_text, mut body) = (String::new(), String::new(), String::new());
    for ix in 0..lines {
        let line = format!("let value_{ix} = compute(\"the quick brown fox\", {ix}, &state);");
        let _ = writeln!(old_text, "{line}");
        if ix % 5 == 0 {
            let changed = format!("let value_{ix} = compute(\"the lazy dog\", {ix} + 1, &state);");
            let _ = writeln!(new_text, "{changed}");
            let _ = writeln!(body, "-{line}\n+{changed}");
        } else {
            let _ = writeln!(new_text, "{line}");
            let _ = writeln!(body, " {line}");
        }
    }
    let path = std::path::PathBuf::from("src/big.rs");
    let target = worktree_target_at("src/big.rs", DiffArea::Unstaged);
    let unified = format!(
        "diff --git a/src/big.rs b/src/big.rs\n--- a/src/big.rs\n+++ b/src/big.rs\n\
         @@ -1,{lines} +1,{lines} @@\n{body}"
    );

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_generated_diff_{}",
        std::process::id(),
        repo_id.0
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let mut repo = opening_repo_state(repo_id, &workdir);
    repo.open = Loadable::Ready(());
    repo.head_branch = Loadable::Ready("main".into());
    set_test_file_status(
        &mut repo,
        path.clone(),
        gitcomet_core::domain::FileStatusKind::Modified,
        DiffArea::Unstaged,
    );
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff_state_rev = 1;
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff::from_unified(
        target, &unified,
    )));
    repo.diff_state.diff_file_rev = 1;
    repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(
        gitcomet_core::domain::FileDiffText::new(path, Some(old_text), Some(new_text)),
    )));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = mode;
            cx.notify();
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "the file diff view to render its rows",
        |pane| pane.is_file_diff_view_active() && pane.diff_visible_len() > 0,
        |pane| pane.diff_visible_len(),
    );
    (view, cx, workdir)
}

/// Frame cost of a large text diff. Ignored: a measurement, not a check.
///
/// `GITCOMET_BENCH_DIFF_LINES` (3000) lines, every fifth one changed;
/// `GITCOMET_BENCH_DIFF_SPLIT` set draws the split view. Run with
/// `--ignored --nocapture` and one test thread for clean allocation counts.
#[gpui::test]
#[ignore]
fn diff_view_real_frame_benchmark(cx: &mut gpui::TestAppContext) {
    use std::time::Instant;

    let _visual_guard = crate::test_support::lock_visual_test();
    let lines: usize = std::env::var("GITCOMET_BENCH_DIFF_LINES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3000);
    let mode = if std::env::var_os("GITCOMET_BENCH_DIFF_SPLIT").is_some_and(|v| !v.is_empty()) {
        DiffViewMode::Split
    } else {
        DiffViewMode::Inline
    };
    let (view, cx, workdir) = open_generated_diff_view(cx, RepoId(70930), lines, mode);
    let _cached_views = std::env::var_os("GITCOMET_BENCH_CACHED_VIEWS")
        .map(|_| crate::view::enable_stable_cached_views_for_test());
    for _ in 0..3 {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
    let row = cx
        .update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_hitbox_bounds_for_tests(
                    10,
                    if mode == DiffViewMode::Split {
                        DiffTextRegion::SplitLeft
                    } else {
                        DiffTextRegion::Inline
                    },
                )
        })
        .expect("row 10 is drawn");

    const FRAMES: usize = 40;
    let percentile = |samples: &mut Vec<f64>, p: usize| {
        samples.sort_by(f64::total_cmp);
        samples[(samples.len() - 1) * p / 100]
    };
    let mut rebuild_ms = Vec::new();
    let mut rebuild_allocs = crate::perf_alloc::PerfAllocMetrics::default();
    for _ in 0..FRAMES {
        let started = Instant::now();
        let (_, allocations) = crate::perf_alloc::measure_allocations(|| {
            cx.update(|window, app| {
                window.refresh();
                let _ = window.draw(app);
            })
        });
        rebuild_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        rebuild_allocs = rebuild_allocs.saturating_add(allocations);
    }
    let mut move_us = Vec::new();
    for step in 0..FRAMES {
        let x = row.left() + px(4.0 + (step % 8) as f32);
        let started = Instant::now();
        cx.simulate_mouse_move(point(x, row.center().y), None, gpui::Modifiers::none());
        move_us.push(started.elapsed().as_secs_f64() * 1e6);
    }
    eprintln!(
        "diff view lines={lines} mode={mode:?} profile={} rebuild_ms_p50={:.2} \
         rebuild_ms_p95={:.2} allocs_per_rebuild={:.0} mouse_move_us_p50={:.1} \
         mouse_move_us_p95={:.1}",
        if cfg!(debug_assertions) {
            "test"
        } else {
            "release"
        },
        percentile(&mut rebuild_ms, 50),
        percentile(&mut rebuild_ms, 95),
        rebuild_allocs.alloc_ops as f64 / FRAMES as f64,
        percentile(&mut move_us, 50),
        percentile(&mut move_us, 95),
    );
    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn a_frame_of_a_text_diff_resolves_what_the_pane_shows_a_bounded_number_of_times(
    cx: &mut gpui::TestAppContext,
) {
    // Whether a preview covers the diff was asked per painted row, and each
    // answer stats the file; a frame answers it once.
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx, workdir) =
        open_generated_diff_view(cx, RepoId(70931), 400, DiffViewMode::Inline);
    for _ in 0..2 {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }

    crate::view::panes::main::take_file_preview_active_checks_for_tests();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |_pane, cx| cx.notify());
        });
        window.refresh();
        let _ = window.draw(app);
    });
    let checks = crate::view::panes::main::take_file_preview_active_checks_for_tests();
    assert!(
        checks <= 2,
        "a frame of a text diff checks the preview surface {checks} times"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}
