use super::super::super::*;
use super::*;
use crate::view::panes::diff::input::DiffPaneInput;
use gitcomet_core::domain::FileDiffText;

#[gpui::test]
fn structural_selection_maps_collapsed_rows_and_never_expands_a_block(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::test_support::TestBackend;
    use std::collections::BTreeSet;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        root.read(app).diff_renderer.clone().update(app, |pane, _| {
            let old = "before\nold\nhidden\nhidden\nold tail\nafter\n";
            let new = "before\nnew\nhidden\nhidden\nnew tail\nafter\n";
            let diff = StructuralDiff { language: "Rust".into(), aligned_lines: (0..6).map(|i| (Some(i), Some(i))).collect(),
                old_changes: BTreeMap::from([(1, std::iter::once(0..3).collect()), (4, std::iter::once(0..3).collect())]),
                new_changes: BTreeMap::from([(1, std::iter::once(0..3).collect()), (4, std::iter::once(0..3).collect())]), blocks: vec![1..2, 4..5] };
            let plan = Arc::new(gitcomet_core::file_diff::structural::plan(&diff, old, new));
            let mut source = StreamedFileDiffSource::new(plan,
                IndexedFileDiffSource::from_shared(Some(&Arc::from(old))), IndexedFileDiffSource::from_shared(Some(&Arc::from(new))), DiffWhitespaceMode::Show);
            source.apply_structural(diff);
            let source = Arc::new(source);
            pane.file_diff_row_provider = Some(Arc::new(PagedFileDiffRows::new(source.clone(), 10)));
            pane.file_diff_inline_row_provider = Some(Arc::new(PagedFileDiffInlineRows::new(source, 10)));
            pane.diff_engine_status = DiffEngineStatus::Difftastic { syntactic_changes: true, text_differs: true };
            pane.diff_word_wrap = false;
            pane.diff_view = DiffViewMode::Split;
            pane.diff_visible_indices = Arc::from([1, 4]);
            let selected = pane.structural_line_selection(1, DiffTextRegion::SplitRight).unwrap();
            assert!(selected.old.is_empty());
            assert_eq!(selected.new, BTreeSet::from([5]));
            let first = pane.structural_block_lines(1).unwrap();
            assert_eq!(first.old, BTreeSet::from([2]));
            assert_eq!(first.new, BTreeSet::from([2]));
            assert!(pane.structural_block_lines(3).is_none());
            pane.diff_selection_range = Some((0, 1));
            let selected = pane.structural_line_selection(1, DiffTextRegion::SplitRight).unwrap();
            assert!(selected.old.is_empty());
            assert_eq!(selected.new, BTreeSet::from([2, 5]));
            pane.diff_selection_range = None;
            pane.diff_text_anchor = Some(DiffTextPos { source_visible_ix: 0, region: DiffTextRegion::SplitRight, offset: 1 });
            pane.diff_text_head = Some(DiffTextPos { source_visible_ix: 1, region: DiffTextRegion::SplitLeft, offset: 2 });
            let selected = pane.structural_line_selection(0, DiffTextRegion::SplitRight).unwrap();
            assert_eq!(selected.old, BTreeSet::from([5]));
            assert_eq!(selected.new, BTreeSet::from([2]));
            pane.diff_text_anchor = Some(DiffTextPos { source_visible_ix: 0, region: DiffTextRegion::SplitLeft, offset: 1 });
            pane.diff_text_head = Some(DiffTextPos { source_visible_ix: 0, region: DiffTextRegion::SplitRight, offset: 0 });
            let selected = pane.structural_line_selection(0, DiffTextRegion::SplitLeft).unwrap();
            assert_eq!(selected.old, BTreeSet::from([2]));
            assert!(selected.new.is_empty());
            pane.diff_text_anchor = Some(DiffTextPos { source_visible_ix: 0, region: DiffTextRegion::SplitLeft, offset: 1 });
            pane.diff_text_head = Some(DiffTextPos { source_visible_ix: 1, region: DiffTextRegion::SplitLeft, offset: 0 });
            let selected = pane.structural_line_selection(0, DiffTextRegion::SplitLeft).unwrap();
            assert_eq!(selected.old, BTreeSet::from([2]));
            assert!(selected.new.is_empty());
            pane.diff_text_anchor = None;
            pane.diff_text_head = None;
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_visible_indices = Arc::from([1, 2, 5, 6]);
            let selected = pane.structural_line_selection(3, DiffTextRegion::Inline).unwrap();
            assert!(selected.old.is_empty());
            assert_eq!(selected.new, BTreeSet::from([5]));
            let first = pane.structural_block_lines(2).unwrap();
            assert_eq!(first.old, BTreeSet::from([2]));
            assert_eq!(first.new, BTreeSet::from([2]));
            let target = DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged);
            let workdir = std::env::temp_dir().join("gitcomet-structural-selection");
            let mut repo = gitcomet_state::model::RepoState::new_opening(RepoId(811), gitcomet_core::domain::RepoSpec { workdir: workdir.clone() });
            repo.open = Loadable::Ready(());
            repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff::from_unified(target.clone(), "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1,6 +1,6 @@\n before\n-old\n+new\n hidden\n hidden\n-old tail\n+new tail\n after\n")));
            repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new("file.rs".into(), Some(old.to_owned()), Some(new.to_owned())))));
            pane.input = DiffPaneInput::selected(Arc::new(AppState { active_repo: Some(repo.id), repos: vec![repo], ..AppState::test_default() }));
            pane.diff_cache_repo_id = Some(RepoId(811).into());
            pane.diff_cache_rev = 1;
            pane.diff_cache_target = Some(target.clone());
            pane.file_diff_cache_repo_id = Some(RepoId(811).into());
            pane.file_diff_cache_rev = 1;
            pane.file_diff_cache_target = Some(target);
            pane.file_diff_cache_path = Some(workdir.join("file.rs"));
            pane.file_diff_cache_engine = pane.effective_diff_engine();
            pane.file_diff_cache_whitespace_mode = pane.diff_whitespace_mode;
            pane.diff_content_mode = DiffContentMode::Collapsed;
            pane.diff_surface_frame = pane.diff_surface_frame.wrapping_add(1);
            pane.collapsed_diff_visible_rows = Arc::from([
                CollapsedDiffVisibleRow::HunkHeader { src_ix: 1, expansion_kind: CollapsedDiffExpansionKind::None, display_src_ix: None, hidden_rows: 0 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 1 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 2 },
                CollapsedDiffVisibleRow::HunkHeader { src_ix: 5, expansion_kind: CollapsedDiffExpansionKind::None, display_src_ix: None, hidden_rows: 2 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 5 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 6 },
            ]);
            assert!(pane.is_collapsed_diff_projection_active());
            let selected = pane.structural_line_selection(5, DiffTextRegion::Inline).unwrap();
            assert!(selected.old.is_empty());
            assert_eq!(selected.new, BTreeSet::from([5]));
            pane.diff_text_anchor = Some(DiffTextPos { source_visible_ix: 1, region: DiffTextRegion::Inline, offset: 0 });
            pane.diff_text_head = Some(DiffTextPos { source_visible_ix: 5, region: DiffTextRegion::Inline, offset: 3 });
            let selected = pane.structural_line_selection(5, DiffTextRegion::Inline).unwrap();
            assert_eq!(selected.old, BTreeSet::from([2, 5]));
            assert_eq!(selected.new, BTreeSet::from([2, 5]));
            pane.diff_text_anchor = None;
            pane.diff_text_head = None;
            pane.diff_view = DiffViewMode::Split;
            pane.collapsed_diff_visible_rows = Arc::from([
                CollapsedDiffVisibleRow::HunkHeader { src_ix: 1, expansion_kind: CollapsedDiffExpansionKind::None, display_src_ix: None, hidden_rows: 0 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 1 },
                CollapsedDiffVisibleRow::HunkHeader { src_ix: 4, expansion_kind: CollapsedDiffExpansionKind::None, display_src_ix: None, hidden_rows: 2 },
                CollapsedDiffVisibleRow::FileRow { row_ix: 4 },
            ]);
            let selected = pane.structural_line_selection(3, DiffTextRegion::SplitRight).unwrap();
            assert!(selected.old.is_empty());
            assert_eq!(selected.new, BTreeSet::from([5]));
        });
    });
}

#[test]
fn identical_text_with_structural_novelty_has_two_inline_sides() {
    let text = "function f() {\n";
    let diff = StructuralDiff {
        language: "JavaScript".into(),
        aligned_lines: vec![(Some(0), Some(0))],
        old_changes: BTreeMap::from([(0, std::iter::once(13..14).collect())]),
        new_changes: BTreeMap::from([(0, std::iter::once(13..14).collect())]),
        blocks: std::iter::once(0..1).collect(),
    };
    let plan = Arc::new(gitcomet_core::file_diff::structural::plan(
        &diff, text, text,
    ));
    let source = IndexedFileDiffSource::from_shared(Some(&Arc::from(text)));
    let mut source =
        StreamedFileDiffSource::new(plan, source.clone(), source, DiffWhitespaceMode::Show);
    source.apply_structural(diff);
    assert_eq!(source.inline_len(), 2);
    assert_eq!(source.inline_visual_kind(0), Some(DiffLineKind::Remove));
    assert_eq!(source.inline_visual_kind(1), Some(DiffLineKind::Add));
}

/// A line changed on one side only is one row showing that side, with both
/// line numbers; a line changed on both sides keeps a row per side.
#[test]
#[allow(clippy::single_range_in_vec_init)]
fn a_line_changed_on_one_side_is_one_row_showing_that_side() {
    let old = "same\nf(1);\ng(a?);\nh(1);\nclose\n";
    let new = "same\nwrap(f(1));\ng(a);\nh(2);\nclose\n";
    let diff = StructuralDiff {
        language: "JavaScript".into(),
        aligned_lines: (0..5).map(|line| (Some(line), Some(line))).collect(),
        old_changes: BTreeMap::from([(2, vec![3..4]), (3, vec![2..3])]),
        new_changes: BTreeMap::from([(1, vec![0..5, 9..10]), (3, vec![2..3])]),
        blocks: std::iter::once(1..4).collect(),
    };
    let plan = Arc::new(gitcomet_core::file_diff::structural::plan(&diff, old, new));
    let mut source = StreamedFileDiffSource::new(
        plan,
        IndexedFileDiffSource::from_shared(Some(&Arc::from(old))),
        IndexedFileDiffSource::from_shared(Some(&Arc::from(new))),
        DiffWhitespaceMode::Show,
    );
    source.apply_structural(diff);
    let source = Arc::new(source);
    let split = PagedFileDiffRows::new(source.clone(), 10);
    let inline = PagedFileDiffInlineRows::new(source, 10);
    let rows: Vec<_> = (0..6)
        .map(|row| {
            let line = inline.source.inline_row(row).unwrap();
            let data = inline.source.inline_row_render_data(row).unwrap();
            assert_eq!(
                (data.kind, data.old_line, data.new_line),
                (line.kind, line.old_line, line.new_line)
            );
            assert_eq!(data.text.as_ref(), &line.text[1..]);
            (
                line.text.to_string(),
                line.old_line,
                line.new_line,
                inline.visual_kind(row),
            )
        })
        .collect();
    let row = |text: &str, old, new, kind| (text.to_owned(), old, new, Some(kind));
    assert_eq!(
        rows,
        [
            row(" same", Some(1), Some(1), DiffLineKind::Context),
            row("+wrap(f(1));", Some(2), Some(2), DiffLineKind::Add),
            row("-g(a?);", Some(3), Some(3), DiffLineKind::Remove),
            row("-h(1);", Some(4), None, DiffLineKind::Remove),
            row("+h(2);", None, Some(4), DiffLineKind::Add),
            row(" close", Some(5), Some(5), DiffLineKind::Context),
        ]
    );
    assert!(inline.source.inline_row(6).is_none());
    assert_eq!(
        inline.structural_ranges(1).unwrap().as_ref(),
        &[0..5, 9..10]
    );
    assert_eq!(inline.structural_ranges(2).unwrap().as_ref(), &[3..4]);
    assert_eq!(
        split.source.split_row(1).unwrap().old.unwrap().as_ref(),
        "f(1);"
    );
    assert!(split.structural_ranges(1, true).unwrap().is_empty());
    assert_eq!(
        split.structural_ranges(1, false).unwrap().as_ref(),
        &[0..5, 9..10]
    );
    assert_eq!(split.change_blocks(), vec![1..4]);
    assert_eq!(inline.change_blocks(), vec![1..5]);
}

#[test]
fn a_cancelled_difftastic_build_yields_no_rows() {
    let file = FileDiffText::new_shared(
        "app.js".into(),
        Some(Arc::from("call(a, b);\n")),
        Some(Arc::from("call(a, c);\n")),
    );
    let build = |cancellation: &CancellationToken| {
        super::super::file_diff::build_file_diff_cache_with_engine(
            &file,
            std::path::Path::new("/repo"),
            None,
            DiffWhitespaceMode::Show,
            DiffEngine::Difftastic,
            cancellation,
        )
    };
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(build(&cancelled).err(), Some(FileDiffCacheError::Cancelled));
    let rebuild = build(&CancellationToken::new()).expect("difftastic build");
    assert_eq!(
        rebuild.engine_status,
        DiffEngineStatus::Difftastic {
            syntactic_changes: true,
            text_differs: true,
        }
    );
}

/// A final newline added on the new side is no changed token, so the last
/// line stays one unmarked row; the line engine's newline step had drawn it
/// twice as plain context.
#[test]
#[allow(clippy::single_range_in_vec_init)]
fn a_final_newline_change_draws_the_last_line_once() {
    let old = "fn a() {}\nend";
    let new = "fn b() {}\nend\n";
    let diff = StructuralDiff {
        language: "Rust".into(),
        aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
        old_changes: BTreeMap::from([(0, vec![3..4])]),
        new_changes: BTreeMap::from([(0, vec![3..4])]),
        blocks: vec![0..1],
    };
    let plan = Arc::new(gitcomet_core::file_diff::structural::plan(&diff, old, new));
    let mut source = StreamedFileDiffSource::new(
        plan,
        IndexedFileDiffSource::from_shared(Some(&Arc::from(old))),
        IndexedFileDiffSource::from_shared(Some(&Arc::from(new))),
        DiffWhitespaceMode::Show,
    );
    source.apply_structural(diff);
    let rows: Vec<_> = (0..source.inline_len())
        .map(|row| {
            (
                source.inline_row(row).unwrap().text.to_string(),
                source.inline_visual_kind(row).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("-fn a() {}".to_owned(), DiffLineKind::Remove),
            ("+fn b() {}".to_owned(), DiffLineKind::Add),
            (" end".to_owned(), DiffLineKind::Context),
        ]
    );
}

fn show_structural_rows(pane: &mut DiffRenderer, old: &str, new: &str, diff: StructuralDiff) {
    let plan = Arc::new(gitcomet_core::file_diff::structural::plan(&diff, old, new));
    let mut source = StreamedFileDiffSource::new(
        plan,
        IndexedFileDiffSource::from_shared(Some(&Arc::from(old))),
        IndexedFileDiffSource::from_shared(Some(&Arc::from(new))),
        DiffWhitespaceMode::Show,
    );
    source.apply_structural(diff);
    let source = Arc::new(source);
    pane.diff_visible_indices = (0..source.split_len().max(source.inline_len())).collect();
    pane.file_diff_row_provider = Some(Arc::new(PagedFileDiffRows::new(source.clone(), 10)));
    pane.file_diff_inline_row_provider = Some(Arc::new(PagedFileDiffInlineRows::new(source, 10)));
    pane.diff_engine_status = DiffEngineStatus::Difftastic {
        syntactic_changes: true,
        text_differs: true,
    };
    pane.diff_word_wrap = false;
    // A current file identity, so rows read their text rather than none.
    let target = DiffTarget::working_tree("file.js".into(), DiffArea::Unstaged);
    let workdir = std::env::temp_dir().join("gitcomet-structural-rows");
    let mut repo = gitcomet_state::model::RepoState::new_opening(
        RepoId(812),
        gitcomet_core::domain::RepoSpec {
            workdir: workdir.clone(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff_file_rev = 1;
    repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new(
        "file.js".into(),
        Some(old.to_owned()),
        Some(new.to_owned()),
    ))));
    pane.input = DiffPaneInput::selected(Arc::new(AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..AppState::test_default()
    }));
    pane.file_diff_cache_repo_id = Some(RepoId(812).into());
    pane.file_diff_cache_rev = 1;
    pane.file_diff_cache_target = Some(target);
    pane.file_diff_cache_path = Some(workdir.join("file.js"));
    pane.file_diff_cache_engine = pane.effective_diff_engine();
    pane.file_diff_cache_whitespace_mode = pane.diff_whitespace_mode;
    pane.diff_content_mode = DiffContentMode::Full;
    assert!(pane.is_file_diff_view_active());
}

/// A selection starting at the very end of a line covers none of that line,
/// yet "Stage selected lines" took it with the rest. A blank line inside the
/// selection still counts, as it has nothing else to select.
#[gpui::test]
fn a_selection_from_a_line_end_leaves_that_line_out(cx: &mut gpui::TestAppContext) {
    use crate::view::test_support::TestBackend;
    use std::collections::BTreeSet;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        root.read(app).diff_renderer.clone().update(app, |pane, _| {
            pane.diff_view = DiffViewMode::Inline;
            show_structural_rows(
                pane,
                "f(a);\nh(a);\n",
                "f(a, b);\n\nh(a, b);\n",
                StructuralDiff {
                    language: "JavaScript".into(),
                    aligned_lines: vec![(Some(0), Some(0)), (None, Some(1)), (Some(1), Some(2))],
                    old_changes: BTreeMap::new(),
                    new_changes: BTreeMap::from([
                        (0, std::iter::once(3..6).collect()),
                        (2, std::iter::once(3..6).collect()),
                    ]),
                    blocks: std::iter::once(0..3).collect(),
                },
            );
            assert_eq!(
                pane.diff_text_full_line_for_region(0, DiffTextRegion::Inline)
                    .as_ref(),
                "f(a, b);"
            );
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix: 0,
                region: DiffTextRegion::Inline,
                offset: "f(a, b);".len(),
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix: 2,
                region: DiffTextRegion::Inline,
                offset: 3,
            });
            let selected = pane
                .structural_line_selection(2, DiffTextRegion::Inline)
                .unwrap();
            assert_eq!(selected.old, BTreeSet::from([2]));
            assert_eq!(selected.new, BTreeSet::from([2, 3]));
        });
    });
}

/// Right-clicking a Split pair changed on one side, with nothing selected,
/// acts on the row as its one gutter button does: both of its lines. Taking
/// only the clicked column staged the new line beside the old one.
#[gpui::test]
#[allow(clippy::single_range_in_vec_init)]
fn right_clicking_a_split_pair_changed_on_one_side_takes_both_lines(cx: &mut gpui::TestAppContext) {
    use crate::view::test_support::TestBackend;
    use std::collections::BTreeSet;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        root.read(app).diff_renderer.clone().update(app, |pane, _| {
            pane.diff_view = DiffViewMode::Split;
            show_structural_rows(
                pane,
                "f(a);\ng(a);\n",
                "f(a, b);\ng(b);\n",
                StructuralDiff {
                    language: "JavaScript".into(),
                    aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1))],
                    old_changes: BTreeMap::from([(1, std::iter::once(2..3).collect())]),
                    new_changes: BTreeMap::from([
                        (0, std::iter::once(3..6).collect()),
                        (1, std::iter::once(2..3).collect()),
                    ]),
                    blocks: vec![0..2],
                },
            );
            for region in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
                let selected = pane.structural_line_selection(0, region).unwrap();
                assert_eq!(selected.old, BTreeSet::from([1]), "{region:?}");
                assert_eq!(selected.new, BTreeSet::from([1]), "{region:?}");
            }
            let changed_on_both_sides = pane
                .structural_line_selection(1, DiffTextRegion::SplitRight)
                .unwrap();
            assert!(changed_on_both_sides.old.is_empty());
            assert_eq!(changed_on_both_sides.new, BTreeSet::from([2]));
        });
    });
}

/// A right-click acts on the text selection only inside the rows it takes:
/// not on a row it only touches at an edge, not outside it, and not in the
/// other column of a selection inside one Split column. Both engines decide
/// this here.
#[gpui::test]
fn a_right_click_takes_the_selection_only_inside_the_rows_it_takes(cx: &mut gpui::TestAppContext) {
    use crate::view::test_support::TestBackend;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        root.read(app).diff_renderer.clone().update(app, |pane, _| {
            pane.diff_view = DiffViewMode::Split;
            show_structural_rows(
                pane,
                "a(1);\nb(1);\nc(1);\nd(1);\n",
                "a(1, x);\nb(1, x);\nc(1, x);\nd(1, x);\n",
                StructuralDiff {
                    language: "JavaScript".into(),
                    aligned_lines: (0..4).map(|line| (Some(line), Some(line))).collect(),
                    old_changes: BTreeMap::new(),
                    new_changes: (0..4)
                        .map(|line| (line, std::iter::once(3..6).collect()))
                        .collect(),
                    blocks: std::iter::once(0..4).collect(),
                },
            );
            let at = |row: usize, region| DiffTextPos {
                source_visible_ix: row,
                region,
                offset: 0,
            };
            pane.diff_text_anchor = Some(at(1, DiffTextRegion::SplitLeft));
            pane.diff_text_head = Some(DiffTextPos {
                offset: 2,
                ..at(3, DiffTextRegion::SplitLeft)
            });
            let split = |row, region| {
                pane.diff_text_selection_line_rows_at(DiffViewMode::Split, row, region)
            };
            assert_eq!(split(2, DiffTextRegion::SplitLeft), Some((1, 3)));
            assert_eq!(split(2, DiffTextRegion::SplitRight), None);
            assert_eq!(split(0, DiffTextRegion::SplitLeft), None);
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_text_anchor = Some(DiffTextPos {
                offset: "a(1, x);".len(),
                ..at(0, DiffTextRegion::Inline)
            });
            pane.diff_text_head = Some(DiffTextPos {
                offset: 3,
                ..at(2, DiffTextRegion::Inline)
            });
            let inline = |row| {
                pane.diff_text_selection_line_rows_at(
                    DiffViewMode::Inline,
                    row,
                    DiffTextRegion::Inline,
                )
            };
            assert_eq!(inline(0), None);
            assert_eq!(inline(1), Some((1, 2)));
        });
    });
}
