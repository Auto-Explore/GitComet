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

#[test]
fn structural_rows_preserve_neutral_text_and_highlight_only_novel_tokens() {
    let old = "same\nf(1);\nclose\n";
    let new = "same\nwrap(f(1));\nclose\n";
    let diff = StructuralDiff {
        language: "JavaScript".into(),
        aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), Some(2))],
        old_changes: BTreeMap::new(),
        new_changes: BTreeMap::from([(1, vec![0..5, 9..10])]),
        blocks: std::iter::once(1..2).collect(),
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
    assert_eq!(
        split.source.split_row(1).unwrap().old.unwrap().as_ref(),
        "f(1);"
    );
    assert_eq!(inline.source.inline_row(1).unwrap().text.as_ref(), "-f(1);");
    assert_eq!(
        inline.source.inline_row(2).unwrap().text.as_ref(),
        "+wrap(f(1));"
    );
    assert_eq!(inline.visual_kind(1), Some(DiffLineKind::Context));
    assert_eq!(inline.visual_kind(2), Some(DiffLineKind::Add));
    assert!(split.structural_ranges(1, true).unwrap().is_empty());
    assert_eq!(
        split.structural_ranges(1, false).unwrap().as_ref(),
        &[0..5, 9..10]
    );
    assert_eq!(
        inline.structural_ranges(2).unwrap().as_ref(),
        &[0..5, 9..10]
    );
    assert_eq!(split.change_blocks(), vec![1..2]);
    assert_eq!(inline.change_blocks(), vec![1..3]);
}
