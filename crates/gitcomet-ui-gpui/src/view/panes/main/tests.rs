use super::core_impl::resolved_output_highlight_provider_binding_key;
use super::{
    ClearDiffSelectionAction, RenderableConflictFile, ResolvedOutputConflictMarker,
    VersionedCachedDiffStyledText, apply_conflict_choice_provenance_hints,
    apply_three_way_empty_base_provenance_hints, build_focused_mergetool_save_payload,
    build_line_starts, build_resolved_output_conflict_markers,
    build_resolved_output_conflict_markers_from_block_ranges,
    build_resolved_output_syntax_state_for_snapshot,
    build_resolved_output_syntax_state_for_snapshot_with_budget, clear_diff_selection_action,
    conflict_file_is_binary, conflict_marker_nav_entries_from_markers,
    conflict_resolver_output_context_line, dirty_byte_range_to_line_range,
    first_output_marker_line_for_conflict, focused_mergetool_save_exit_code,
    output_line_range_for_conflict_block_in_text, pane_content_width_for_layout,
    parse_conflict_canvas_rows_env, remap_line_keyed_cache_for_delta, renderable_conflict_file,
    replace_output_lines_in_range, resolved_outline_delta_between_texts,
    resolved_outline_delta_for_snapshot_transition, resolved_output_conflict_block_ranges_in_text,
    resolved_output_marker_for_line, resolved_output_markers_for_text,
    split_target_conflict_block_into_subchunks, versioned_cached_diff_styled_text_is_current,
    versioned_query_cached_diff_styled_text_is_current,
};
use crate::kit::text_model::TextModel;
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
    ResolvedLineSource, SourceLines,
};
use crate::view::rows;
use crate::view::{ConflictResolverUiState, GitCometViewMode};
use gitcomet_core::domain::RepoSpec;
use gitcomet_state::model::{ConflictFile, Loadable, RepoId, RepoState};
use rustc_hash::FxHashMap as HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn clear_diff_selection_action_is_clear_for_normal_mode() {
    assert_eq!(
        clear_diff_selection_action(GitCometViewMode::Normal),
        ClearDiffSelectionAction::ClearSelection
    );
}

#[test]
fn clear_diff_selection_action_exits_focused_mergetool_mode() {
    assert_eq!(
        clear_diff_selection_action(GitCometViewMode::FocusedMergetool),
        ClearDiffSelectionAction::ExitFocusedMergetool
    );
}

#[test]
fn focused_mergetool_save_exit_code_is_success_when_all_resolved() {
    assert_eq!(focused_mergetool_save_exit_code(0, 0), 0);
    assert_eq!(focused_mergetool_save_exit_code(3, 3), 0);
}

#[test]
fn focused_mergetool_save_exit_code_is_canceled_when_unresolved_remain() {
    assert_eq!(focused_mergetool_save_exit_code(3, 2), 1);
}

fn focused_mergetool_marker_labels() -> gitcomet_core::conflict_output::ConflictMarkerLabels<'static>
{
    gitcomet_core::conflict_output::ConflictMarkerLabels {
        local: "LOCAL",
        remote: "REMOTE",
        base: "BASE",
    }
}

fn repo_with_conflict_file(
    repo_id: RepoId,
    target_path: &Path,
    conflict_file: Loadable<Option<ConflictFile>>,
) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.conflict_state.conflict_file_path = Some(target_path.to_path_buf());
    repo.conflict_state.conflict_file = conflict_file;
    repo
}

fn text_conflict_file(path: &Path, current: &str) -> ConflictFile {
    ConflictFile {
        path: path.to_path_buf(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: Some(Arc::<str>::from("base\n")),
        ours: Some(Arc::<str>::from("ours\n")),
        theirs: Some(Arc::<str>::from("theirs\n")),
        current: Some(Arc::<str>::from(current)),
    }
}

fn binary_conflict_file(path: &Path) -> ConflictFile {
    ConflictFile {
        path: path.to_path_buf(),
        base_bytes: Some(Arc::from(&b"base"[..])),
        ours_bytes: Some(Arc::from(&b"ours"[..])),
        theirs_bytes: Some(Arc::from(&b"theirs"[..])),
        current_bytes: None,
        base: None,
        ours: None,
        theirs: None,
        current: None,
    }
}

#[test]
fn renderable_conflict_file_reuses_cached_loaded_file_while_store_loading_same_target() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let cached_file = text_conflict_file(&target_path, "cached current\n");
    let repo = repo_with_conflict_file(repo_id, &target_path, Loadable::Loading);
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(target_path.clone()),
        loaded_file: Some(cached_file),
        ..ConflictResolverUiState::default()
    };

    let renderable = renderable_conflict_file(&repo, &conflict_resolver, &target_path);

    assert!(matches!(
        renderable,
        RenderableConflictFile::File(file)
            if file.current.as_deref() == Some("cached current\n")
    ));
}

#[test]
fn renderable_conflict_file_does_not_reuse_cached_file_for_different_path() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let repo = repo_with_conflict_file(repo_id, &target_path, Loadable::Loading);
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(PathBuf::from("other.html")),
        loaded_file: Some(text_conflict_file(
            Path::new("other.html"),
            "cached current\n",
        )),
        ..ConflictResolverUiState::default()
    };

    assert_eq!(
        renderable_conflict_file(&repo, &conflict_resolver, &target_path),
        RenderableConflictFile::Loading
    );
}

#[test]
fn renderable_conflict_file_prefers_store_ready_file_over_cached_file() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let store_file = text_conflict_file(&target_path, "store current\n");
    let repo = repo_with_conflict_file(repo_id, &target_path, Loadable::Ready(Some(store_file)));
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(target_path.clone()),
        loaded_file: Some(text_conflict_file(&target_path, "cached current\n")),
        ..ConflictResolverUiState::default()
    };

    let renderable = renderable_conflict_file(&repo, &conflict_resolver, &target_path);

    assert!(matches!(
        renderable,
        RenderableConflictFile::File(file)
            if file.current.as_deref() == Some("store current\n")
    ));
}

#[test]
fn renderable_conflict_file_preserves_store_error_over_cached_file() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let repo = repo_with_conflict_file(
        repo_id,
        &target_path,
        Loadable::Error("load failed".to_string()),
    );
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(target_path.clone()),
        loaded_file: Some(text_conflict_file(&target_path, "cached current\n")),
        ..ConflictResolverUiState::default()
    };

    assert_eq!(
        renderable_conflict_file(&repo, &conflict_resolver, &target_path),
        RenderableConflictFile::Error("load failed".into())
    );
}

#[test]
fn renderable_conflict_file_preserves_missing_store_result_over_cached_file() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let repo = repo_with_conflict_file(repo_id, &target_path, Loadable::Ready(None));
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(target_path.clone()),
        loaded_file: Some(text_conflict_file(&target_path, "cached current\n")),
        ..ConflictResolverUiState::default()
    };

    assert_eq!(
        renderable_conflict_file(&repo, &conflict_resolver, &target_path),
        RenderableConflictFile::Missing
    );
}

#[test]
fn binary_conflict_detection_uses_cached_loaded_file_during_loading() {
    let repo_id = RepoId(7);
    let target_path = PathBuf::from("index.html");
    let repo = repo_with_conflict_file(repo_id, &target_path, Loadable::Loading);
    let conflict_resolver = ConflictResolverUiState {
        repo_id: Some(repo_id),
        path: Some(target_path.clone()),
        loaded_file: Some(binary_conflict_file(&target_path)),
        ..ConflictResolverUiState::default()
    };

    let renderable = renderable_conflict_file(&repo, &conflict_resolver, &target_path);

    assert!(matches!(
        renderable,
        RenderableConflictFile::File(file) if conflict_file_is_binary(&file)
    ));
}

#[test]
fn focused_mergetool_save_payload_rehydrates_unedited_materialized_conflicts() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "ours\n".to_string().into(),
        theirs: "theirs\n".to_string().into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0],
        Some("ours\n"),
        focused_mergetool_marker_labels(),
    );

    assert_eq!(
        payload.output,
        "<<<<<<< LOCAL\nours\n=======\ntheirs\n>>>>>>> REMOTE\n"
    );
    assert_eq!(payload.total_conflicts, 1);
    assert_eq!(payload.resolved_conflicts, 0);
}

#[test]
fn focused_mergetool_save_payload_keeps_manual_edits_and_unedited_markers() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-1\n".to_string().into(),
            theirs: "theirs-1\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("middle\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-2\n".to_string().into(),
            theirs: "theirs-2\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("bottom\n".to_string().into()),
    ];

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0, 1],
        Some("top\nmanual-1\nmiddle\nours-2\nbottom\n"),
        focused_mergetool_marker_labels(),
    );

    assert_eq!(
        payload.output,
        concat!(
            "top\n",
            "manual-1\n",
            "middle\n",
            "<<<<<<< LOCAL\n",
            "ours-2\n",
            "=======\n",
            "theirs-2\n",
            ">>>>>>> REMOTE\n",
            "bottom\n"
        )
    );
    assert_eq!(payload.total_conflicts, 1);
    assert_eq!(payload.resolved_conflicts, 0);
}

#[test]
fn focused_mergetool_save_payload_marks_manual_output_as_resolved() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "ours\n".to_string().into(),
        theirs: "theirs\n".to_string().into(),
        choice: ConflictChoice::Ours,
        resolved: false,
    })];

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0],
        Some("manual\n"),
        focused_mergetool_marker_labels(),
    );

    assert_eq!(payload.output, "manual\n");
    assert_eq!(
        focused_mergetool_save_exit_code(payload.total_conflicts, payload.resolved_conflicts),
        0
    );
}

#[test]
fn parse_conflict_canvas_rows_env_accepts_truthy_values() {
    assert!(parse_conflict_canvas_rows_env("1"));
    assert!(parse_conflict_canvas_rows_env("true"));
    assert!(parse_conflict_canvas_rows_env("on"));
    assert!(parse_conflict_canvas_rows_env("yes"));
    assert!(parse_conflict_canvas_rows_env("maybe"));
}

#[test]
fn parse_conflict_canvas_rows_env_rejects_falsey_values() {
    assert!(!parse_conflict_canvas_rows_env("0"));
    assert!(!parse_conflict_canvas_rows_env("false"));
    assert!(!parse_conflict_canvas_rows_env("off"));
    assert!(!parse_conflict_canvas_rows_env("no"));
}

#[test]
fn replace_output_lines_in_range_replaces_only_target_chunk() {
    let output = "top\nkeep\nalso-keep\nbottom";
    let replacement = vec!["picked".to_string()];
    let next = replace_output_lines_in_range(output, 1..3, &replacement);
    assert_eq!(next, "top\npicked\nbottom");
}

#[test]
fn replace_output_lines_in_range_preserves_trailing_newline() {
    let output = "a\nb\n";
    let replacement = vec!["x".to_string(), "y".to_string()];
    let next = replace_output_lines_in_range(output, 1..2, &replacement);
    assert_eq!(next, "a\nx\ny\n");
}

#[test]
fn resolved_outline_delta_between_texts_clamps_to_utf8_boundaries() {
    let old_text = "prefix ä\nsuffix";
    let new_text = "prefix ö\nsuffix";
    let delta = resolved_outline_delta_between_texts(old_text, new_text).expect("delta");
    assert_eq!(old_text.get(delta.old_range.clone()), Some("ä"));
    assert_eq!(new_text.get(delta.new_range.clone()), Some("ö"));
}

#[test]
fn resolved_outline_delta_for_snapshot_transition_prefers_recent_edit_delta() {
    let mut model = TextModel::from("prefix value\nsuffix");
    let old_snapshot = model.snapshot();
    let new_range = model.replace_range(7..12, "token");
    let new_snapshot = model.snapshot();

    let delta = resolved_outline_delta_for_snapshot_transition(
        &old_snapshot,
        &new_snapshot,
        Some((7..12, new_range)),
    )
    .expect("delta");

    assert_eq!(delta.old_range, 7..12);
    assert_eq!(delta.new_range, 7..12);
}

#[test]
fn resolved_outline_delta_for_snapshot_transition_falls_back_after_multiple_revisions() {
    let mut model = TextModel::from("abcdef");
    let old_snapshot = model.snapshot();
    let _first = model.replace_range(1..2, "B");
    let latest = model.replace_range(4..5, "E");
    let new_snapshot = model.snapshot();

    let delta = resolved_outline_delta_for_snapshot_transition(
        &old_snapshot,
        &new_snapshot,
        Some((4..5, latest)),
    )
    .expect("delta");

    assert_eq!(delta.old_range, 1..5);
    assert_eq!(delta.new_range, 1..5);
}

#[test]
fn dirty_byte_range_to_line_range_includes_line_join_delete() {
    let text = "a\nb\nc";
    let line_starts = build_line_starts(text);
    // Delete the newline between "a" and "b".
    let dirty = dirty_byte_range_to_line_range(&line_starts, text.len(), 1..2);
    assert_eq!(dirty, 0..2);
}

#[test]
fn remap_line_keyed_cache_for_delta_shifts_suffix_entries() {
    let mut cache: HashMap<usize, usize> = HashMap::default();
    cache.insert(0, 10);
    cache.insert(4, 40);
    cache.insert(7, 70);

    remap_line_keyed_cache_for_delta(&mut cache, 2..5, 2..3);
    assert_eq!(cache.get(&0), Some(&10));
    assert_eq!(cache.get(&4), None);
    assert_eq!(cache.get(&5), Some(&70));
}

#[test]
fn remap_line_keyed_cache_for_delta_preserves_versioned_preview_entries() {
    let mut cache: HashMap<usize, VersionedCachedDiffStyledText> = HashMap::default();
    let make_entry = |text: &str| VersionedCachedDiffStyledText {
        syntax_epoch: 7,
        query_generation: 0,
        styled: crate::view::diff_text_model::CachedDiffStyledText {
            text: text.to_string().into(),
            highlights: Arc::from(Vec::new()),
            highlights_hash: 11,
            text_hash: 22,
        },
    };
    cache.insert(0, make_entry("keep"));
    cache.insert(7, make_entry("shift"));

    remap_line_keyed_cache_for_delta(&mut cache, 2..5, 2..3);

    let keep = versioned_cached_diff_styled_text_is_current(cache.get(&0), 7)
        .expect("unchanged prefix entry should stay current");
    assert_eq!(keep.text.as_ref(), "keep");

    let shifted = versioned_cached_diff_styled_text_is_current(cache.get(&5), 7)
        .expect("suffix entry should move and keep its syntax epoch");
    assert_eq!(shifted.text.as_ref(), "shift");
    assert!(cache.get(&7).is_none());
}

#[test]
fn versioned_diff_style_cache_entry_only_matches_current_epoch() {
    let styled = crate::view::diff_text_model::CachedDiffStyledText {
        text: "styled".into(),
        highlights: Arc::from(Vec::new()),
        highlights_hash: 11,
        text_hash: 22,
    };
    let entry = VersionedCachedDiffStyledText {
        syntax_epoch: 7,
        query_generation: 0,
        styled: styled.clone(),
    };

    let current = versioned_cached_diff_styled_text_is_current(Some(&entry), 7)
        .expect("matching epoch should return cached styled text");
    assert_eq!(current.text, styled.text);
    assert_eq!(current.highlights_hash, styled.highlights_hash);
    assert_eq!(current.text_hash, styled.text_hash);

    assert!(
        versioned_cached_diff_styled_text_is_current(Some(&entry), 8).is_none(),
        "stale cache entries should be ignored when syntax epoch advances"
    );
    assert!(
        versioned_cached_diff_styled_text_is_current(None, 7).is_none(),
        "missing cache entries should stay missing"
    );
}

#[test]
fn versioned_query_diff_style_cache_entry_only_matches_current_generation() {
    let styled = crate::view::diff_text_model::CachedDiffStyledText {
        text: "styled".into(),
        highlights: Arc::from(Vec::new()),
        highlights_hash: 11,
        text_hash: 22,
    };
    let entry = VersionedCachedDiffStyledText {
        syntax_epoch: 7,
        query_generation: 3,
        styled: styled.clone(),
    };

    let current = versioned_query_cached_diff_styled_text_is_current(Some(&entry), 7, 3)
        .expect("matching syntax epoch and query generation should return cached styled text");
    assert_eq!(current.text, styled.text);

    assert!(
        versioned_query_cached_diff_styled_text_is_current(Some(&entry), 8, 3).is_none(),
        "stale syntax epochs should invalidate query cache entries"
    );
    assert!(
        versioned_query_cached_diff_styled_text_is_current(Some(&entry), 7, 4).is_none(),
        "stale query generations should invalidate query cache entries"
    );
}

#[test]
fn resolved_output_conflict_block_ranges_match_point_lookup() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "y\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let ranges =
        resolved_output_conflict_block_ranges_in_text(&segments, &output).expect("block ranges");
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, &output, 0),
        ranges.first().cloned()
    );
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, &output, 1),
        ranges.get(1).cloned()
    );
}

#[test]
fn output_line_range_for_conflict_block_in_text_maps_middle_blocks_exactly() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\ny\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "z\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, &output, 0),
        Some(1..2)
    );
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, &output, 1),
        Some(3..5)
    );
}

#[test]
fn output_line_range_for_conflict_block_in_text_maps_eof_block_without_newline() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "tail".to_string().into(),
            theirs: "other".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, &output, 0),
        Some(1..2)
    );
}

#[test]
fn output_line_range_for_conflict_block_in_text_returns_none_when_output_drifts() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\n".to_string().into(),
            theirs: "y\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
    ];

    let drifted_output = "top\ndrift\nmid\nb\n";
    assert_eq!(
        output_line_range_for_conflict_block_in_text(&segments, drifted_output, 1),
        None
    );
}

#[test]
fn build_resolved_output_conflict_markers_maps_chunk_boundaries() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\ny\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "z\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

    assert_eq!(
        markers[1],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 2,
            is_start: true,
            is_end: true,
            unresolved: false,
        })
    );
    assert_eq!(
        markers[3],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 1,
            range_start: 3,
            range_end: 5,
            is_start: true,
            is_end: false,
            unresolved: false,
        })
    );
    assert_eq!(
        markers[4],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 1,
            range_start: 3,
            range_end: 5,
            is_start: false,
            is_end: true,
            unresolved: false,
        })
    );
}

#[test]
fn build_resolved_output_conflict_markers_anchors_zero_length_ranges() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some(String::new().into()),
            ours: String::new().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

    assert_eq!(
        markers[1],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 1,
            is_start: true,
            is_end: true,
            unresolved: false,
        })
    );
}

#[test]
fn build_resolved_output_conflict_markers_marks_unresolved_blocks() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

    assert_eq!(
        markers[1],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 2,
            is_start: true,
            is_end: true,
            unresolved: true,
        })
    );
}

#[test]
fn conflict_marker_nav_entries_include_only_marker_starts() {
    let markers = vec![
        None,
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: true,
            is_end: false,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: false,
            is_end: true,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 1,
            range_start: 3,
            range_end: 4,
            is_start: true,
            is_end: true,
            unresolved: false,
        }),
    ];
    assert_eq!(
        conflict_marker_nav_entries_from_markers(&markers),
        vec![1, 3]
    );
}

#[test]
fn conflict_marker_nav_entries_dedup_conflicts_with_multiple_start_ranges() {
    let markers = vec![
        None,
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: true,
            is_end: false,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: false,
            is_end: true,
            unresolved: true,
        }),
        None,
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 4,
            range_end: 5,
            is_start: true,
            is_end: true,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 1,
            range_start: 5,
            range_end: 6,
            is_start: true,
            is_end: true,
            unresolved: false,
        }),
    ];
    assert_eq!(
        conflict_marker_nav_entries_from_markers(&markers),
        vec![1, 5]
    );
}

#[test]
fn first_output_marker_line_for_conflict_returns_first_start() {
    let markers = vec![
        None,
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: true,
            is_end: false,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 3,
            is_start: false,
            is_end: true,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 3,
            range_end: 4,
            is_start: true,
            is_end: true,
            unresolved: true,
        }),
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 1,
            range_start: 4,
            range_end: 5,
            is_start: true,
            is_end: true,
            unresolved: false,
        }),
    ];

    assert_eq!(first_output_marker_line_for_conflict(&markers, 0), Some(1));
    assert_eq!(first_output_marker_line_for_conflict(&markers, 1), Some(4));
    assert_eq!(first_output_marker_line_for_conflict(&markers, 2), None);
}

#[test]
fn conflict_resolver_output_context_line_prefers_clicked_offset() {
    let content = "top\nmiddle\nbottom\n";
    let cursor_offset = 0usize;
    let clicked_offset = "top\nmiddle\n".len();
    assert_eq!(
        conflict_resolver_output_context_line(content, cursor_offset, Some(clicked_offset)),
        2
    );
    assert_eq!(
        conflict_resolver_output_context_line(content, "top\n".len(), None),
        1
    );
}

#[test]
fn clicked_unresolved_line_maps_to_chunk_marker() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-1\nours-2\n".to_string().into(),
            theirs: "theirs-1\ntheirs-2\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let cursor_offset = 0usize;
    let clicked_offset = "top\nours-1\n".len();
    let clicked_line =
        conflict_resolver_output_context_line(&output, cursor_offset, Some(clicked_offset));
    let marker = resolved_output_marker_for_line(&segments, &output, clicked_line).expect("marker");
    assert!(marker.unresolved);
    assert_eq!(marker.conflict_ix, 0);
}

#[test]
fn build_resolved_output_conflict_markers_splits_unresolved_subchunks() {
    let segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("a\ncommon\nb\n".to_string().into()),
            ours: "ao\ncommon\nbo\n".to_string().into(),
            theirs: "at\ncommon\nbt\n".to_string().into(),
            choice: ConflictChoice::Base,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

    let starts = markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(starts, 2, "expected two unresolved subchunk starts");
    assert!(
        markers.get(2).is_some_and(|m| m.is_none()),
        "resolved middle line should not be marked as conflict"
    );
}

#[test]
fn build_resolved_output_conflict_markers_splits_method_edit_and_trailing_insertion() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
            base: Some(
                "pub fn opposite(self) -> Color {\n    match self {\n        Color::White => Color::Black,\n        Color::Black => Color::White,\n    }\n}\n"
                    .to_string()
                    .into(),
            ),
            ours: "pub fn opposite(self) -> Color {\n    match self {\n        Color::White => Color::Black,\n        Color::Black => Color::White,\n    }\n}\n"
                .to_string()
                .into(),
            theirs: "pub fn opposite(self) -> Self {\n    match self {\n        Self::White => Self::Black,\n        Self::Black => Self::White,\n    }\n}\n\npub fn name(self) -> &'static str {\n    match self {\n        Self::White => \"White\",\n        Self::Black => \"Black\",\n    }\n}\n"
                .to_string()
                .into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        })];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);

    let starts = markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(starts, 2, "expected two decision marker starts");
}

#[test]
fn build_resolved_output_conflict_markers_matches_combined_conflict_marker_case() {
    let conflict_text = "impl Color {\n<<<<<<< HEAD\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n=======\n    pub fn opposite(self) -> Self {\n        match self {\n            Self::White => Self::Black,\n            Self::Black => Self::White,\n        }\n    }\n\n    pub fn name(self) -> &'static str {\n        match self {\n            Self::White => \"White\",\n            Self::Black => \"Black\",\n>>>>>>> origin/version2\n        }\n    }\n}\n";
    let base_text = "impl Color {\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n        }\n    }\n}\n";
    let mut segments = conflict_resolver::parse_conflict_markers(conflict_text);
    conflict_resolver::populate_block_bases_from_ancestor(&mut segments, base_text);

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(&segments, &output, line_count);
    let starts = markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(starts, 2, "expected two marker starts for impl Color case");
}

#[test]
fn split_target_conflict_block_into_subchunks_isolates_close_markers() {
    let conflict_text = "impl Color {\n<<<<<<< HEAD\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n=======\n    pub fn opposite(self) -> Self {\n        match self {\n            Self::White => Self::Black,\n            Self::Black => Self::White,\n        }\n    }\n\n    pub fn name(self) -> &'static str {\n        match self {\n            Self::White => \"White\",\n            Self::Black => \"Black\",\n>>>>>>> origin/version2\n        }\n    }\n}\n";
    let base_text = "impl Color {\n    pub fn opposite(self) -> Color {\n        match self {\n            Color::White => Color::Black,\n            Color::Black => Color::White,\n        }\n    }\n}\n";
    let mut segments = conflict_resolver::parse_conflict_markers(conflict_text);
    conflict_resolver::populate_block_bases_from_ancestor(&mut segments, base_text);
    let mut region_indices = conflict_resolver::sequential_conflict_region_indices(&segments);
    let output_before = conflict_resolver::generate_resolved_text(&segments);
    let projection_before = conflict_resolver::ResolvedOutputProjection::from_segments(&segments);

    let before_markers = resolved_output_markers_for_text(&segments, &output_before);
    let before_starts = before_markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(
        before_starts, 2,
        "fixture should begin with two close markers"
    );
    let streamed_markers_before = build_resolved_output_conflict_markers_from_block_ranges(
        &segments,
        projection_before.conflict_line_ranges(),
        projection_before.len(),
    );
    let streamed_starts_before = streamed_markers_before
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(
        streamed_starts_before, 1,
        "streamed bootstrap should keep one coarse marker start per unsplit block"
    );

    assert!(
        split_target_conflict_block_into_subchunks(&mut segments, &mut region_indices, 0),
        "expected target block to split"
    );

    assert_eq!(conflict_resolver::conflict_count(&segments), 2);
    assert_eq!(region_indices, vec![0, 0]);
    let output_after = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(
        output_after, output_before,
        "split should preserve output text"
    );
    let projection_after = conflict_resolver::ResolvedOutputProjection::from_segments(&segments);
    let streamed_markers_after = build_resolved_output_conflict_markers_from_block_ranges(
        &segments,
        projection_after.conflict_line_ranges(),
        projection_after.len(),
    );
    let streamed_starts_after = streamed_markers_after
        .iter()
        .flatten()
        .filter(|m| m.is_start)
        .count();
    assert_eq!(
        streamed_starts_after, 2,
        "lazy split should expose one coarse marker start per resulting subchunk block"
    );

    let after_markers = resolved_output_markers_for_text(&segments, &output_after);
    let mut starts_by_conflict: std::collections::BTreeMap<usize, usize> =
        std::collections::BTreeMap::new();
    for marker in after_markers.iter().flatten().filter(|m| m.is_start) {
        *starts_by_conflict.entry(marker.conflict_ix).or_default() += 1;
    }
    assert_eq!(starts_by_conflict.get(&0).copied(), Some(1));
    assert_eq!(starts_by_conflict.get(&1).copied(), Some(1));
}

#[test]
fn conflict_region_index_is_unique_detects_split_subchunk_duplicates() {
    assert!(super::conflict_region_index_is_unique(&[0], 0));
    assert!(super::conflict_region_index_is_unique(&[0, 1], 0));
    assert!(!super::conflict_region_index_is_unique(&[0, 0], 0));
}

#[test]
fn append_choice_after_conflict_block_appends_selected_order_for_single_marker() {
    let mut segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];
    let mut region_indices = vec![0];

    let inserted_ix = super::append_choice_after_conflict_block(
        &mut segments,
        &mut region_indices,
        0,
        ConflictChoice::Theirs,
    );

    assert_eq!(inserted_ix, Some(1));
    assert_eq!(conflict_resolver::conflict_count(&segments), 2);
    assert_eq!(region_indices, vec![0, 0]);
    let output = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(output, "pre\nours\ntheirs\npost\n");
}

#[test]
fn append_choice_after_conflict_block_from_same_marker_keeps_single_choice_per_side() {
    let mut segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Base,
            resolved: true,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];
    let mut region_indices = vec![0];

    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            0,
            ConflictChoice::Ours,
        ),
        Some(1)
    );
    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            0,
            ConflictChoice::Theirs,
        ),
        Some(2)
    );
    // Picking C again from the same marker should not append duplicate chunks.
    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            0,
            ConflictChoice::Theirs,
        ),
        None
    );

    assert_eq!(
        super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 0),
        vec![
            ConflictChoice::Base,
            ConflictChoice::Ours,
            ConflictChoice::Theirs
        ]
    );
    assert_eq!(conflict_resolver::conflict_count(&segments), 3);
    assert_eq!(
        conflict_resolver::generate_resolved_text(&segments),
        "pre\nbase\nours\ntheirs\npost\n"
    );
}

#[test]
fn non_contiguous_matching_blocks_do_not_share_choice_group() {
    let mut segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Text("middle\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];
    // Simulate subchunk-derived duplicate region ids while preserving a text boundary.
    let mut region_indices = vec![0, 0];

    assert_eq!(
        super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 1),
        Vec::<ConflictChoice>::new()
    );

    assert!(
        super::reset_conflict_block_selection(&mut segments, &mut region_indices, 0),
        "resetting first block should not remove it due later non-contiguous match"
    );
    assert_eq!(conflict_resolver::conflict_count(&segments), 2);
}

#[test]
fn adjacent_markers_with_same_text_but_different_regions_do_not_interfere() {
    let mut segments = vec![
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Theirs,
            resolved: true,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
    ];
    let mut region_indices = vec![10, 11];

    assert_eq!(
        super::conflict_group_selected_choices_for_ix(&segments, &region_indices, 1),
        Vec::<ConflictChoice>::new()
    );
    assert_eq!(
        super::conflict_group_indices_for_choice(
            &segments,
            &region_indices,
            1,
            ConflictChoice::Theirs
        ),
        Vec::<usize>::new()
    );

    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            1,
            ConflictChoice::Theirs,
        ),
        None
    );
    assert_eq!(conflict_resolver::conflict_count(&segments), 2);
}

#[test]
fn pick_sequence_is_reversible_to_original_unpicked_state() {
    let mut segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];
    let original = segments.clone();
    let mut region_indices = vec![0];

    // Pick A.
    let target = segments.iter_mut().find_map(|seg| match seg {
        ConflictSegment::Block(block) => Some(block),
        _ => None,
    });
    if let Some(block) = target {
        block.choice = ConflictChoice::Base;
        block.resolved = true;
    } else {
        panic!("expected conflict block");
    }
    // Pick B then C in order.
    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            0,
            ConflictChoice::Ours,
        ),
        Some(1)
    );
    assert_eq!(
        super::append_choice_after_conflict_block(
            &mut segments,
            &mut region_indices,
            1,
            ConflictChoice::Theirs,
        ),
        Some(2)
    );
    assert_eq!(
        conflict_resolver::generate_resolved_text(&segments),
        "pre\nbase\nours\ntheirs\npost\n"
    );

    // Deselect A, then B, then C.
    assert!(super::reset_conflict_block_selection(
        &mut segments,
        &mut region_indices,
        0
    ));
    assert!(super::reset_conflict_block_selection(
        &mut segments,
        &mut region_indices,
        0
    ));
    assert!(super::reset_conflict_block_selection(
        &mut segments,
        &mut region_indices,
        0
    ));

    assert_eq!(segments, original);
    assert_eq!(region_indices, vec![0]);
    assert_eq!(
        conflict_resolver::generate_resolved_text(&segments),
        conflict_resolver::generate_resolved_text(&original)
    );
}

#[test]
fn pick_and_deselect_multiple_orders_always_restore_original_state() {
    fn initial_segments() -> Vec<ConflictSegment> {
        vec![
            ConflictSegment::Text("pre\n".to_string().into()),
            ConflictSegment::Block(ConflictBlock {
                base: Some("base\n".to_string().into()),
                ours: "ours\n".to_string().into(),
                theirs: "theirs\n".to_string().into(),
                choice: ConflictChoice::Ours,
                resolved: false,
            }),
            ConflictSegment::Text("post\n".to_string().into()),
        ]
    }

    fn find_conflict_ix_by_choice(
        segments: &[ConflictSegment],
        choice: ConflictChoice,
    ) -> Option<usize> {
        segments
            .iter()
            .filter_map(|seg| match seg {
                ConflictSegment::Block(block) => Some(block),
                _ => None,
            })
            .enumerate()
            .find_map(|(ix, block)| (block.resolved && block.choice == choice).then_some(ix))
    }

    fn apply_pick_sequence(
        segments: &mut Vec<ConflictSegment>,
        region_indices: &mut Vec<usize>,
        picks: &[ConflictChoice],
    ) {
        let mut current_ix = 0usize;
        for (ix, choice) in picks.iter().copied().enumerate() {
            if ix == 0 {
                let target = segments.iter_mut().find_map(|seg| match seg {
                    ConflictSegment::Block(block) => Some(block),
                    _ => None,
                });
                if let Some(block) = target {
                    block.choice = choice;
                    block.resolved = true;
                } else {
                    panic!("expected conflict block");
                }
                continue;
            }
            let inserted_ix = super::append_choice_after_conflict_block(
                segments,
                region_indices,
                current_ix,
                choice,
            );
            assert_eq!(inserted_ix, Some(current_ix.saturating_add(1)));
            current_ix = inserted_ix.unwrap_or(current_ix);
        }
    }

    let original = initial_segments();
    let cases: Vec<(Vec<ConflictChoice>, Vec<ConflictChoice>)> = vec![
        // Full three-pick flows in different select/deselect orders.
        (
            vec![
                ConflictChoice::Base,
                ConflictChoice::Ours,
                ConflictChoice::Theirs,
            ],
            vec![
                ConflictChoice::Base,
                ConflictChoice::Ours,
                ConflictChoice::Theirs,
            ],
        ),
        (
            vec![
                ConflictChoice::Base,
                ConflictChoice::Ours,
                ConflictChoice::Theirs,
            ],
            vec![
                ConflictChoice::Theirs,
                ConflictChoice::Ours,
                ConflictChoice::Base,
            ],
        ),
        (
            vec![
                ConflictChoice::Theirs,
                ConflictChoice::Base,
                ConflictChoice::Ours,
            ],
            vec![
                ConflictChoice::Base,
                ConflictChoice::Theirs,
                ConflictChoice::Ours,
            ],
        ),
        (
            vec![
                ConflictChoice::Ours,
                ConflictChoice::Theirs,
                ConflictChoice::Base,
            ],
            vec![
                ConflictChoice::Base,
                ConflictChoice::Ours,
                ConflictChoice::Theirs,
            ],
        ),
        // Repeated two-pick cycle case.
        (
            vec![ConflictChoice::Ours, ConflictChoice::Theirs],
            vec![ConflictChoice::Theirs, ConflictChoice::Ours],
        ),
    ];

    for (picks, deselects) in cases {
        // Run each case twice to cover repeated select/deselect cycles.
        for _ in 0..2 {
            let mut segments = original.clone();
            let mut region_indices = vec![0];

            apply_pick_sequence(&mut segments, &mut region_indices, &picks);

            for deselect_choice in deselects.iter().copied() {
                let Some(conflict_ix) = find_conflict_ix_by_choice(&segments, deselect_choice)
                else {
                    panic!(
                        "expected to find selected conflict for {:?}",
                        deselect_choice
                    );
                };
                assert!(
                    super::reset_conflict_block_selection(
                        &mut segments,
                        &mut region_indices,
                        conflict_ix
                    ),
                    "expected deselect to succeed for {:?}",
                    deselect_choice
                );
            }

            assert_eq!(segments, original);
            assert_eq!(region_indices, vec![0]);
            assert_eq!(
                conflict_resolver::generate_resolved_text(&segments),
                conflict_resolver::generate_resolved_text(&original)
            );
        }
    }
}

#[test]
fn conflict_choice_hints_override_identical_text_to_selected_source() {
    fn shared(s: &str) -> gpui::SharedString {
        s.to_string().into()
    }

    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: Some("same\n".to_string().into()),
        ours: "same\n".to_string().into(),
        theirs: "same\n".to_string().into(),
        choice: ConflictChoice::Ours,
        resolved: true,
    })];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let output_lines = conflict_resolver::split_output_lines_for_outline(&output);
    let sources = SourceLines {
        a: &[shared("same")],
        b: &[shared("same")],
        c: &[shared("same")],
    };

    let mut meta = conflict_resolver::compute_resolved_line_provenance(&output_lines, &sources);
    // Raw text matching alone picks A because A has higher matching priority.
    assert_eq!(meta[0].source, ResolvedLineSource::A);

    apply_conflict_choice_provenance_hints(
        &mut meta,
        &segments,
        &output,
        ConflictResolverViewMode::ThreeWay,
    );

    assert_eq!(meta[0].source, ResolvedLineSource::B);
    assert_eq!(meta[0].input_line, Some(1));
}

#[test]
fn empty_base_conflict_hint_overrides_false_a_badge() {
    fn shared(s: &str) -> gpui::SharedString {
        s.to_string().into()
    }

    let segments = vec![
        ConflictSegment::Text("dup\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some(String::new().into()),
            ours: "dup\n".to_string().into(),
            theirs: "other\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
        }),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let output_lines = conflict_resolver::split_output_lines_for_outline(&output);

    let a = vec![shared("dup")];
    let b = vec![shared("dup"), shared("dup")];
    let c = vec![shared("dup"), shared("other")];
    let sources = SourceLines {
        a: &a,
        b: &b,
        c: &c,
    };

    let mut meta = conflict_resolver::compute_resolved_line_provenance(&output_lines, &sources);
    // Raw content matching can pick A because "dup" exists in A.
    assert_eq!(meta[1].source, ResolvedLineSource::A);

    apply_three_way_empty_base_provenance_hints(&mut meta, &segments, &output);

    assert_eq!(meta[1].source, ResolvedLineSource::B);
    assert_eq!(meta[1].input_line, Some(2));
    assert_eq!(
        conflict_resolver::build_resolved_output_line_sources_index(
            &meta,
            &output_lines,
            ConflictResolverViewMode::ThreeWay
        )
        .contains(&conflict_resolver::SourceLineKey::new(
            ConflictResolverViewMode::ThreeWay,
            ResolvedLineSource::B,
            2,
            "dup"
        )),
        true
    );
}

#[test]
fn resolved_output_syntax_state_uses_prepared_document_for_multiline_comment() {
    let theme = AppTheme::gitcomet_dark();
    let output = "/* open comment\nstill comment */ let x = 1;";
    let output_model = TextModel::from(output);
    let output_snapshot = output_model.snapshot();
    let line_starts = output_snapshot.shared_line_starts();
    let second_line_start = line_starts[1];

    let syntax_state = build_resolved_output_syntax_state_for_snapshot(
        theme,
        &output_snapshot,
        Some(rows::DiffSyntaxLanguage::Rust),
        None,
        None,
    );

    let document = syntax_state.prepared_document.expect(
        "resolved output should keep a prepared document when full-document syntax is available",
    );
    // The state now returns a lazy provider instead of materialized highlights.
    // Call the provider for the second line's byte range to verify multiline comment
    // highlighting works correctly through the provider path.
    let provider = syntax_state
        .highlight_provider
        .expect("resolved output should return a highlight provider when prepared document exists");
    let mut result = provider.resolve(second_line_start..output.len());
    if result.pending {
        let started = std::time::Instant::now();
        while rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        result = provider.resolve(second_line_start..output.len());
    }

    let highlights = result.highlights;
    assert!(
        highlights.iter().any(|(range, style)| {
            range.start <= second_line_start
                && range.end > second_line_start
                && style.color == Some(theme.colors.text_muted.into())
        }),
        "second line should inherit comment highlighting from the multiline document parse"
    );
}

#[test]
fn resolved_output_syntax_state_requests_background_prepare_for_large_documents() {
    let theme = AppTheme::gitcomet_dark();
    let output = "let value = Some(42);\n".repeat(4_001);
    let output_model = TextModel::from(output.clone());
    let output_snapshot = output_model.snapshot();

    let syntax_state = build_resolved_output_syntax_state_for_snapshot_with_budget(
        theme,
        &output_snapshot,
        Some(rows::DiffSyntaxLanguage::Rust),
        None,
        None,
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::ZERO,
        },
    );

    assert!(
        syntax_state.needs_background_prepare,
        "large resolved output should stay eligible for document syntax and continue in the background when the foreground budget times out"
    );
    assert!(
        syntax_state.prepared_document.is_none(),
        "timed out foreground parses should not claim a prepared document"
    );
    assert!(
        syntax_state.highlight_provider.is_none(),
        "provider should only be installed once a prepared document exists"
    );
    assert!(
        syntax_state.highlights.is_empty(),
        "pending document syntax should paint plain text instead of materializing a full fallback highlight vector"
    );
}

#[test]
fn resolved_output_highlight_provider_binding_key_tracks_theme_language_and_document() {
    let theme = AppTheme::gitcomet_dark();
    let output_a = TextModel::from("fn alpha() -> usize { 1 }\n");
    let state_a = build_resolved_output_syntax_state_for_snapshot(
        theme,
        &output_a.snapshot(),
        Some(rows::DiffSyntaxLanguage::Rust),
        None,
        None,
    );
    let document_a = state_a
        .prepared_document
        .expect("small Rust output should produce a prepared document");

    let key_a = resolved_output_highlight_provider_binding_key(
        1,
        rows::DiffSyntaxLanguage::Rust,
        document_a,
    );
    let key_theme_changed = resolved_output_highlight_provider_binding_key(
        2,
        rows::DiffSyntaxLanguage::Rust,
        document_a,
    );
    let key_language_changed = resolved_output_highlight_provider_binding_key(
        1,
        rows::DiffSyntaxLanguage::Html,
        document_a,
    );

    let output_b = TextModel::from("fn beta() -> usize { 2 }\n");
    let state_b = build_resolved_output_syntax_state_for_snapshot(
        theme,
        &output_b.snapshot(),
        Some(rows::DiffSyntaxLanguage::Rust),
        None,
        None,
    );
    let document_b = state_b
        .prepared_document
        .expect("different Rust output should produce a prepared document");
    let key_document_changed = resolved_output_highlight_provider_binding_key(
        1,
        rows::DiffSyntaxLanguage::Rust,
        document_b,
    );

    assert_ne!(key_a, key_theme_changed);
    assert_ne!(key_a, key_language_changed);
    assert_ne!(key_a, key_document_changed);
}

#[test]
fn pane_content_width_for_layout_omits_hidden_handles_when_panels_collapsed() {
    let total_w = gpui::px(1000.0);
    let expanded =
        pane_content_width_for_layout(total_w, gpui::px(280.0), gpui::px(420.0), false, false);
    let both_collapsed =
        pane_content_width_for_layout(total_w, gpui::px(34.0), gpui::px(34.0), true, true);

    assert_eq!(expanded, gpui::px(284.0));
    assert_eq!(both_collapsed, gpui::px(932.0));
}

#[test]
fn pane_content_width_for_layout_clamps_at_zero_for_tight_space() {
    let total_w = gpui::px(200.0);
    let width =
        pane_content_width_for_layout(total_w, gpui::px(140.0), gpui::px(80.0), false, false);

    assert_eq!(width, gpui::px(0.0));
}
