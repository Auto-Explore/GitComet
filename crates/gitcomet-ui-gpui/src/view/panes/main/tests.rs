use super::{
    ClearDiffSelectionAction, FocusedMergetoolOutput, RenderableConflictFile,
    ResolvedOutputConflictMarker, ResolvedOutputSourceRevision, ResolvedOutputUnresolvedSpans,
    VersionedCachedDiffStyledText, apply_conflict_choice_provenance_hints,
    apply_focused_mergetool_output, apply_resolved_output_unresolved_highlights,
    apply_three_way_empty_base_provenance_hints, build_focused_mergetool_save_payload,
    build_line_starts, build_resolved_output_conflict_markers,
    build_resolved_output_conflict_markers_from_block_ranges, clear_diff_selection_action,
    coalesce_resolved_output_edit_deltas, conflict_file_is_binary,
    conflict_marker_nav_entries_from_markers, conflict_resolver_output_context_line,
    conflict_strategy_needs_full_side_payloads, dirty_byte_range_to_line_range,
    first_output_marker_line_for_conflict, focused_mergetool_save_exit_code,
    historical_browse_content, output_line_range_for_conflict_block_in_text,
    pane_content_width_for_layout, parse_conflict_canvas_rows_env,
    remap_resolved_output_conflict_block_ranges_for_delta, renderable_conflict_file,
    resolved_outline_delta_between_texts, resolved_outline_delta_for_snapshot_transition,
    resolved_output_active_conflict_background, resolved_output_active_unresolved_highlight_style,
    resolved_output_conflict_block_ranges_in_text, resolved_output_live_highlight_provider,
    resolved_output_live_provider_binding_key, resolved_output_live_syntax_mask,
    resolved_output_marker_for_line, resolved_output_markers_for_text,
    resolved_output_placeholder_protected_ranges, resolved_output_snapshot_is_modified,
    resolved_output_unresolved_byte_ranges, resolved_output_unresolved_highlight_style,
    split_target_conflict_block_into_subchunks, versioned_cached_diff_styled_text_is_current,
    versioned_query_cached_diff_styled_text_is_current, worktree_output_requires_protection,
};
use crate::kit::text_model::TextModel;
use crate::theme::AppTheme;
use crate::view::conflict_resolver::{
    self, ConflictBlock, ConflictChoice, ConflictResolverViewMode, ConflictSegment,
    ResolvedLineSource, SourceLines,
};
use crate::view::rows;
use crate::view::{ConflictResolverUiState, GitCometViewMode};
use gitcomet_core::domain::{CommitId, DiffTarget, FileSource, RepoSpec};
use gitcomet_state::model::{ConflictFile, Loadable, RepoId, RepoState};
use palette::IntoColor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Block ownership for output text that still reads back exactly as the
/// segments render, which is what these marker tests build.
fn block_map_for(segments: &[ConflictSegment]) -> conflict_resolver::ResolvedOutputBlockMap {
    conflict_resolver::ResolvedOutputBlockMap::from_segments(segments)
}

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

#[test]
fn specialized_conflict_strategies_require_full_side_payloads() {
    use gitcomet_core::conflict_session::ConflictResolverStrategy;

    for strategy in [
        ConflictResolverStrategy::BinarySidePick,
        ConflictResolverStrategy::TwoWayKeepDelete,
        ConflictResolverStrategy::DecisionOnly,
    ] {
        assert!(conflict_strategy_needs_full_side_payloads(Some(strategy)));
    }
    assert!(!conflict_strategy_needs_full_side_payloads(Some(
        ConflictResolverStrategy::FullTextResolver
    )));
    assert!(!conflict_strategy_needs_full_side_payloads(None));
}

#[test]
fn ordinary_git_markers_do_not_protect_output_when_every_line_comes_from_a_stage() {
    let current = "before\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> topic\nafter\n";
    let projection = "before\n<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nafter\n";

    assert!(!worktree_output_requires_protection(
        Some(current),
        Some(projection),
        None,
        Some("before\nours\nafter\n"),
        Some("before\ntheirs\nafter\n"),
    ));
}

/// Two correct three-way merges can put their conflict boundaries in different
/// places. Here git conflicts over `B`, while our plan anchors on `mid` and
/// leaves `B` as one-sided context — so the documents interleave the same lines
/// in different orders and neither reconstructs to the other's sides. Requiring
/// such an equality is what protected every real merge and left the resolver
/// inert.
#[test]
fn a_merge_whose_boundaries_disagree_with_ours_stays_interactive() {
    let base = "head\nB\nmid\nD\n";
    let ours = "head\nB1\nmid\nD\n";
    let theirs = "head\nB2\nmid2\nD2\n";
    let current = "head\n<<<<<<< HEAD\nB1\nmid\n=======\nB2\nmid2\n>>>>>>> topic\nD2\n";
    let projection =
        "head\n<<<<<<< ours\nB1\n||||||| base\nB\n=======\nB2\n>>>>>>> theirs\nmid2\nD2\n";

    assert!(!worktree_output_requires_protection(
        Some(current),
        Some(projection),
        Some(base),
        Some(ours),
        Some(theirs),
    ));

    // A line that belongs to no stage is a hand resolution, wherever the
    // boundaries fall, and still holds the worktree bytes.
    let hand_edited = "head\n<<<<<<< HEAD\nB1\nmid\n=======\nB2\nmid2\n>>>>>>> topic\nD2 edited\n";
    assert!(worktree_output_requires_protection(
        Some(hand_edited),
        Some(projection),
        Some(base),
        Some(ours),
        Some(theirs),
    ));
}

/// Marker *labels* are git's, not ours, and must never count as content.
#[test]
fn marker_label_lines_are_not_weighed_against_the_stages() {
    let current = "<<<<<<< HEAD\nours\n||||||| 1a2b3c4:path/to/file.rs\nbase\n=======\ntheirs\n>>>>>>> a-branch-name\n";

    assert!(!worktree_output_requires_protection(
        Some(current),
        Some("<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n"),
        Some("base\n"),
        Some("ours\n"),
        Some("theirs\n"),
    ));
}

#[test]
fn manually_edited_worktree_output_is_protected_from_stage_projection() {
    let projection = concat!(
        "before\n",
        "<<<<<<< ours\n",
        "ours one\nours two\n",
        "=======\n",
        "theirs one\ntheirs two\n",
        ">>>>>>> theirs\n",
        "after\n",
    );
    let partially_resolved = concat!(
        "before\n",
        "manual one\n",
        "<<<<<<< HEAD\n",
        "ours two\n",
        "=======\n",
        "theirs two\n",
        ">>>>>>> topic\n",
        "after\n",
    );

    assert!(worktree_output_requires_protection(
        Some(partially_resolved),
        Some(projection),
        None,
        Some("before\nours one\nours two\nafter\n"),
        Some("before\ntheirs one\ntheirs two\nafter\n"),
    ));
    assert!(worktree_output_requires_protection(
        Some("before\nmanually merged\nafter\n"),
        Some(projection),
        None,
        Some("before\nours one\nours two\nafter\n"),
        Some("before\ntheirs one\ntheirs two\nafter\n"),
    ));
}

#[test]
fn mixed_line_ending_worktree_output_is_protected_from_stage_projection() {
    // Reconstructing this document's sides reproduces the stages byte for byte,
    // so only the mixed terminators distinguish it from the projection.
    let current = "a\r\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> topic\n";
    let projection = "a\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n";

    assert!(worktree_output_requires_protection(
        Some(current),
        Some(projection),
        None,
        Some("a\r\nx\n"),
        Some("a\r\ny\n"),
    ));
}

#[test]
fn uniform_crlf_worktree_output_stays_interactive() {
    let current = "a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> topic\r\n";
    let projection = "a\r\n<<<<<<< ours\r\nx\r\n=======\r\ny\r\n>>>>>>> theirs\r\n";

    assert!(!worktree_output_requires_protection(
        Some(current),
        Some(projection),
        None,
        Some("a\r\nx\r\n"),
        Some("a\r\ny\r\n"),
    ));
}

#[test]
fn identical_current_and_projection_remain_interactive_without_stage_payloads() {
    let markers = "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\n";

    assert!(!worktree_output_requires_protection(
        Some(markers),
        Some(markers),
        None,
        None,
        None,
    ));
}

#[test]
fn focused_mergetool_output_writes_exact_binary_bytes_and_creates_parent() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = dir.path().join("nested/result.bin");
    let bytes = b"\0ours\xffresult";

    apply_focused_mergetool_output(&output, FocusedMergetoolOutput::Write(bytes))
        .expect("write focused mergetool output");

    assert_eq!(std::fs::read(output).expect("read output"), bytes);
}

#[test]
fn focused_mergetool_delete_accepts_existing_and_missing_outputs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let output = dir.path().join("result.txt");
    std::fs::write(&output, "merged").expect("seed output");

    apply_focused_mergetool_output(&output, FocusedMergetoolOutput::Delete)
        .expect("delete focused mergetool output");
    assert!(!output.exists());

    apply_focused_mergetool_output(&output, FocusedMergetoolOutput::Delete)
        .expect("missing output is already deleted");
}

#[test]
fn focused_mergetool_output_reports_filesystem_failures() {
    let dir = tempfile::tempdir().expect("temp dir");

    let err = apply_focused_mergetool_output(
        dir.path(),
        FocusedMergetoolOutput::Write(b"cannot replace a directory"),
    )
    .expect_err("directory target must fail");

    assert_ne!(err.kind(), std::io::ErrorKind::NotFound);
}

fn focused_mergetool_marker_labels() -> gitcomet_core::conflict_output::ConflictMarkerLabels<'static>
{
    gitcomet_core::conflict_output::ConflictMarkerLabels {
        local: "LOCAL",
        remote: "REMOTE",
        base: "BASE",
    }
}

fn apply_mapped_output_test_edit(
    map: &mut conflict_resolver::ResolvedOutputBlockMap,
    output: &mut String,
    range: std::ops::Range<usize>,
    replacement: &str,
) {
    let inserted = range.start..range.start + replacement.len();
    assert!(map.apply_edit_delta(range.clone(), inserted));
    output.replace_range(range, replacement);
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

fn repo_browsing_commit(sha: &str, content_preview: bool) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.file_browser.source = FileSource::Commit(CommitId(sha.into()));
    repo.diff_state.content_preview = content_preview;
    repo
}

fn commit_content_target(sha: &str) -> DiffTarget {
    DiffTarget::Commit {
        commit_id: CommitId(sha.into()),
        path: Some(PathBuf::from("src/main.rs")),
    }
}

#[test]
fn historical_browse_content_matches_the_browsed_commit() {
    let repo = repo_browsing_commit("aaaa", true);
    assert!(historical_browse_content(
        &repo,
        Some(&commit_content_target("aaaa"))
    ));
}

#[test]
fn historical_browse_content_ignores_another_commits_content() {
    let repo = repo_browsing_commit("aaaa", true);
    assert!(!historical_browse_content(
        &repo,
        Some(&commit_content_target("bbbb"))
    ));
}

#[test]
fn historical_browse_content_ignores_plain_diffs_and_live_state() {
    // Content preview off: a patch diff of the browsed commit is not the
    // browsed file's content.
    let no_preview = repo_browsing_commit("aaaa", false);
    assert!(!historical_browse_content(
        &no_preview,
        Some(&commit_content_target("aaaa"))
    ));

    let previewing = repo_browsing_commit("aaaa", true);
    assert!(!historical_browse_content(&previewing, None));
    assert!(!historical_browse_content(
        &previewing,
        Some(&DiffTarget::WorkingTree {
            path: PathBuf::from("src/main.rs"),
            area: gitcomet_core::domain::DiffArea::Unstaged,
        })
    ));

    // No browse point at all.
    let mut live = repo_browsing_commit("aaaa", true);
    live.file_browser.source = FileSource::WorkingDirectory;
    assert!(!historical_browse_content(
        &live,
        Some(&commit_content_target("aaaa"))
    ));
}

fn text_conflict_file(path: &Path, current: &str) -> ConflictFile {
    ConflictFile {
        path: path.to_path_buf().into(),
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
        path: path.to_path_buf().into(),
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
fn current_only_non_utf8_payload_is_detected_as_binary() {
    let path = PathBuf::from("asset.bin");
    let file = ConflictFile {
        path: path.into(),
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: Some(Arc::from(&b"\xff\xfe"[..])),
        base: None,
        ours: None,
        theirs: None,
        current: None,
    };

    assert!(conflict_file_is_binary(&file));
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
        whitespace_only: false,
    })];
    let block_map = conflict_resolver::ResolvedOutputBlockMap::from_segments(&segments);

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0],
        &block_map,
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("middle\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-2\n".to_string().into(),
            theirs: "theirs-2\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("bottom\n".to_string().into()),
    ];
    let mut block_map = conflict_resolver::ResolvedOutputBlockMap::from_segments(&segments);
    assert!(block_map.apply_edit_delta(4..11, 4..13));

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0, 1],
        &block_map,
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
fn focused_mergetool_save_payload_preserves_edited_context() {
    let segments = vec![
        ConflictSegment::Text("top\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-1\n".into(),
            theirs: "theirs-1\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("middle\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours-2\n".into(),
            theirs: "theirs-2\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("bottom\n".into()),
    ];
    let mut output = conflict_resolver::generate_resolved_text(&segments);
    let mut block_map = conflict_resolver::ResolvedOutputBlockMap::from_segments(&segments);

    apply_mapped_output_test_edit(&mut block_map, &mut output, 0..4, "TOP EDIT\n");
    let first = block_map.ranges()[0].clone();
    apply_mapped_output_test_edit(&mut block_map, &mut output, first, "manual-1\n");
    let middle = output.find("middle\n").expect("middle context");
    apply_mapped_output_test_edit(
        &mut block_map,
        &mut output,
        middle..middle + "middle\n".len(),
        "MIDDLE EDIT\n",
    );
    let bottom = output.find("bottom\n").expect("bottom context");
    apply_mapped_output_test_edit(
        &mut block_map,
        &mut output,
        bottom..bottom + "bottom\n".len(),
        "BOTTOM EDIT\n",
    );

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0, 1],
        &block_map,
        Some(&output),
        focused_mergetool_marker_labels(),
    );
    assert_eq!(
        payload.output,
        concat!(
            "TOP EDIT\n",
            "manual-1\n",
            "MIDDLE EDIT\n",
            "<<<<<<< LOCAL\n",
            "ours-2\n",
            "=======\n",
            "theirs-2\n",
            ">>>>>>> REMOTE\n",
            "BOTTOM EDIT\n",
        ),
    );
}

#[test]
fn focused_mergetool_save_payload_marks_manual_output_as_resolved() {
    let segments = vec![ConflictSegment::Block(ConflictBlock {
        base: None,
        ours: "ours\n".to_string().into(),
        theirs: "theirs\n".to_string().into(),
        choice: ConflictChoice::Ours,
        resolved: false,
        whitespace_only: false,
    })];
    let mut block_map = conflict_resolver::ResolvedOutputBlockMap::from_segments(&segments);
    assert!(block_map.apply_edit_delta(0..5, 0..7));

    let payload = build_focused_mergetool_save_payload(
        &segments,
        &[0],
        &block_map,
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
fn resolved_output_source_revision_tracks_edits_and_document_replacement() {
    let mut model = TextModel::from("alpha");
    let initial = ResolvedOutputSourceRevision::from_snapshot(&model.snapshot());

    model.replace_range(5..5, " beta");
    let edited = ResolvedOutputSourceRevision::from_snapshot(&model.snapshot());
    assert_eq!(edited.model_id, initial.model_id);
    assert!(edited.revision > initial.revision);

    model.set_text("replacement");
    let replaced = ResolvedOutputSourceRevision::from_snapshot(&model.snapshot());
    assert_ne!(replaced.model_id, edited.model_id);
}

#[test]
fn resolved_output_modified_state_tracks_saved_snapshot_and_undo() {
    let mut model = TextModel::from("saved output");
    let saved = model.snapshot();
    assert!(!resolved_output_snapshot_is_modified(None, &saved));
    assert!(!resolved_output_snapshot_is_modified(Some(&saved), &saved));

    model.replace_range(6..12, "result");
    assert!(resolved_output_snapshot_is_modified(
        Some(&saved),
        &model.snapshot(),
    ));

    model = saved.clone().into();
    assert!(!resolved_output_snapshot_is_modified(
        Some(&saved),
        &model.snapshot(),
    ));
}

#[test]
fn resolved_outline_delta_for_snapshot_transition_defers_after_multiple_revisions() {
    let mut model = TextModel::from("abcdef");
    let old_snapshot = model.snapshot();
    let _first = model.replace_range(1..2, "B");
    let latest = model.replace_range(4..5, "E");
    let new_snapshot = model.snapshot();

    let delta = resolved_outline_delta_for_snapshot_transition(
        &old_snapshot,
        &new_snapshot,
        Some((4..5, latest)),
    );

    assert_eq!(delta, None);
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "y\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
        }),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let ranges = resolved_output_conflict_block_ranges_in_text(&segments, output.as_str())
        .expect("block ranges");
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "z\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
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
            whitespace_only: false,
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\n".to_string().into(),
            theirs: "y\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("mid\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "b\nc\n".to_string().into(),
            theirs: "z\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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

/// A block whose sides run over several lines still collapses to one
/// placeholder row, and that row carries the whole conflict's `?` gutter and
/// bracket.
#[test]
fn build_resolved_output_conflict_markers_cover_a_multi_line_block() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a1\na2\na3\n".to_string().into(),
            theirs: "x1\n".to_string().into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("bottom\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(output, "top\n<Merge Conflict>\nbottom\n");
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

    assert_eq!(
        markers[1],
        Some(ResolvedOutputConflictMarker {
            conflict_ix: 0,
            range_start: 1,
            range_end: 2,
            is_start: true,
            is_end: true,
            unresolved: true,
        }),
        "the placeholder row is the whole block"
    );
    assert_eq!(markers[0], None);
    assert_eq!(
        markers[2], None,
        "the text after the block is not part of it"
    );
}

/// A conflict that is the file's last line owns exactly that line. The output
/// keeps a trailing empty row after the final newline (see
/// `resolved_output_outline_line_count`); that row belongs to no conflict and
/// must not inherit the block's `?` gutter.
#[test]
fn build_resolved_output_conflict_markers_stop_at_a_file_final_block() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\n".to_string().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    assert_eq!(
        line_count, 3,
        "the trailing newline keeps an empty last row"
    );
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
    assert_eq!(
        markers[2], None,
        "the empty row after the final newline is not part of the conflict"
    );
}

/// A block that ends the file with no trailing newline still owns that line —
/// no newline accounts for it, so its range must not collapse to zero width.
#[test]
fn build_resolved_output_conflict_markers_cover_a_file_final_block_without_newline() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a".to_string().into(),
            theirs: "x".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
        }),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
}

#[test]
fn remap_resolved_output_conflict_block_ranges_expands_edited_block() {
    let old_ranges = vec![1..3, 5..6];
    let new_ranges =
        remap_resolved_output_conflict_block_ranges_for_delta(&old_ranges, 2..3, 2..4, 7);

    assert_eq!(new_ranges, vec![1..4, 6..7]);
}

#[test]
fn remapped_resolved_output_conflict_markers_cover_inserted_rows() {
    let segments = vec![
        ConflictSegment::Text("top\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "a\nb\n".to_string().into(),
            theirs: "x\n".to_string().into(),
            choice: ConflictChoice::Ours,
            resolved: true,
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let old_ranges = resolved_output_conflict_block_ranges_in_text(&segments, output.as_str())
        .expect("block ranges");
    let new_ranges =
        remap_resolved_output_conflict_block_ranges_for_delta(&old_ranges, 2..3, 2..4, 5);
    let markers = build_resolved_output_conflict_markers_from_block_ranges(
        &segments,
        new_ranges.as_slice(),
        5,
    );

    assert!(markers[1].is_some());
    assert!(markers[2].is_some(), "inserted row should keep its marker");
    assert!(markers[3].is_some());
    assert_eq!(markers[4], None);
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".to_string().into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let cursor_offset = 0usize;
    let clicked_offset = "top\nours-1\n".len();
    let clicked_line =
        conflict_resolver_output_context_line(&output, cursor_offset, Some(clicked_offset));
    let marker = resolved_output_marker_for_line(
        &segments,
        &output,
        clicked_line,
        &block_map_for(&segments),
    )
    .expect("marker");
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
            whitespace_only: false,
        })];

    let output = conflict_resolver::generate_resolved_text(&segments);
    let line_count = conflict_resolver::split_output_lines_for_outline(&output).len();
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );

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
    let markers = build_resolved_output_conflict_markers(
        &segments,
        output.as_str(),
        line_count,
        &block_map_for(&segments),
    );
    let starts = markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(
        starts, 1,
        "an unsplit unresolved block should have one placeholder marker"
    );
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

    let before_markers = resolved_output_markers_for_text(
        &segments,
        output_before.as_str(),
        &block_map_for(&segments),
    );
    let before_starts = before_markers
        .iter()
        .flatten()
        .filter(|m| m.conflict_ix == 0 && m.is_start)
        .count();
    assert_eq!(
        before_starts, 1,
        "the unsplit block should begin as one unresolved placeholder"
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
        output_before
            .matches(conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER)
            .count(),
        1,
    );
    assert_eq!(
        output_after
            .matches(conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER)
            .count(),
        2,
        "each split unresolved block should receive its own placeholder row"
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

    let after_markers = resolved_output_markers_for_text(
        &segments,
        output_after.as_str(),
        &block_map_for(&segments),
    );
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
            whitespace_only: false,
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
            whitespace_only: false,
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
            whitespace_only: false,
        }),
        ConflictSegment::Text("middle\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
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
            whitespace_only: false,
        }),
        ConflictSegment::Block(ConflictBlock {
            base: Some("base\n".to_string().into()),
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
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
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
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
    assert_eq!(
        conflict_resolver::generate_resolved_text(&segments),
        "pre\n<Merge Conflict>\npost\n"
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
                choice: ConflictChoice::empty(),
                resolved: false,
                whitespace_only: false,
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
        whitespace_only: false,
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
            whitespace_only: false,
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
    assert!(
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
        ))
    );
}

#[test]
fn unresolved_output_ranges_cover_placeholders_and_selected_unresolved_rows() {
    let segments = vec![
        ConflictSegment::Text("head\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("middle\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "selected\n".into(),
            theirs: "alternate\n".into(),
            choice: ConflictChoice::Ours,
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let spans =
        resolved_output_unresolved_byte_ranges(&segments, &output, &block_map_for(&segments), None);
    let highlighted_text: Vec<&str> = spans
        .all
        .iter()
        .map(|range| &output[range.clone()])
        .collect();

    assert_eq!(highlighted_text, ["<Merge Conflict>", "selected"]);
    assert!(
        spans.active.is_empty(),
        "no conflict is selected, so no row is the active one"
    );
}

/// The active half is what the yellow wash is painted from, so it must name the
/// selected conflict's rows and nobody else's — with two conflicts open, the one
/// the resolver is parked on is the only one that qualifies.
#[test]
fn unresolved_output_ranges_single_out_the_active_conflict() {
    let segments = vec![
        ConflictSegment::Text("head\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".into(),
            theirs: "theirs\n".into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("middle\n".into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "second\n".into(),
            theirs: "alternate\n".into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("tail\n".into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    let block_map = block_map_for(&segments);

    let second_active =
        resolved_output_unresolved_byte_ranges(&segments, &output, &block_map, Some(1));
    assert_eq!(second_active.all.len(), 2, "both conflicts are still open");
    assert_eq!(
        second_active.active.as_ref(),
        std::slice::from_ref(&second_active.all[1]),
        "only the second conflict's row is active"
    );

    let first_active =
        resolved_output_unresolved_byte_ranges(&segments, &output, &block_map, Some(0));
    assert_eq!(
        first_active.active.as_ref(),
        std::slice::from_ref(&first_active.all[0])
    );

    // Selecting a conflict does not change *which* rows are unresolved, but the
    // provider must still be rebound — otherwise the wash stays on the row the
    // previous selection put it on.
    assert_ne!(
        resolved_output_live_provider_binding_key(1, 1, &first_active),
        resolved_output_live_provider_binding_key(1, 1, &second_active),
    );
}

#[test]
fn pane_content_width_for_layout_ignores_overlaid_resize_handles() {
    let total_w = gpui::px(1000.0);
    let expanded =
        pane_content_width_for_layout(total_w, gpui::px(280.0), gpui::px(420.0), false, false);
    let both_collapsed =
        pane_content_width_for_layout(total_w, gpui::px(34.0), gpui::px(34.0), true, true);

    assert_eq!(expanded, gpui::px(300.0));
    assert_eq!(both_collapsed, gpui::px(932.0));
}

#[test]
fn pane_content_width_for_layout_clamps_at_zero_for_tight_space() {
    let total_w = gpui::px(200.0);
    let width =
        pane_content_width_for_layout(total_w, gpui::px(140.0), gpui::px(80.0), false, false);

    assert_eq!(width, gpui::px(0.0));
}

/// The strict segment walk only reports block ranges while the buffer still
/// reads back exactly as the segments render, so one keystroke anywhere used to
/// strip every conflict of its color, its bracket and its chunk menu. The block
/// map tracks that ownership through edits and must keep the markers standing.
#[test]
fn conflict_markers_survive_a_manual_edit_outside_the_blocks() {
    let segments = vec![
        ConflictSegment::Text("pre\n".to_string().into()),
        ConflictSegment::Block(ConflictBlock {
            base: None,
            ours: "ours\n".to_string().into(),
            theirs: "theirs\n".to_string().into(),
            choice: ConflictChoice::empty(),
            resolved: false,
            whitespace_only: false,
        }),
        ConflictSegment::Text("post\n".to_string().into()),
    ];
    let output = conflict_resolver::generate_resolved_text(&segments);
    assert_eq!(output, "pre\n<Merge Conflict>\npost\n");

    let mut block_map = block_map_for(&segments);
    let edited = "pre-edited\n<Merge Conflict>\npost\n";
    assert!(block_map.apply_edit_delta(3..3, 3.."-edited".len() + 3));
    assert!(
        resolved_output_conflict_block_ranges_in_text(&segments, edited).is_none(),
        "the edited buffer no longer matches the segments verbatim"
    );

    let line_count = conflict_resolver::resolved_output_outline_line_count(edited);
    let markers = build_resolved_output_conflict_markers(&segments, edited, line_count, &block_map);
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
    assert!(markers[0].is_none() && markers[2].is_none());

    let spans = resolved_output_unresolved_byte_ranges(&segments, edited, &block_map, Some(0));
    let highlighted: Vec<&str> = spans
        .all
        .iter()
        .map(|range| &edited[range.clone()])
        .collect();
    assert_eq!(highlighted, ["<Merge Conflict>"]);
    assert_eq!(
        spans.active.as_ref(),
        spans.all.as_ref(),
        "the one open conflict is the selected one, edit or no edit"
    );
}

#[test]
fn placeholder_protected_ranges_cover_whole_placeholder_lines() {
    let output = "head\n<Merge Conflict>\n<Merge Conflict (Whitespace only)>\r\ntail";
    let ranges = resolved_output_placeholder_protected_ranges(output);
    let protected: Vec<&str> = ranges.iter().map(|range| &output[range.clone()]).collect();

    assert_eq!(
        protected,
        [
            "<Merge Conflict>\n",
            "<Merge Conflict (Whitespace only)>\r\n"
        ]
    );
}

#[test]
fn placeholder_protected_ranges_are_empty_without_a_placeholder() {
    let output = "head\nresolved\ntail\n";

    assert!(resolved_output_placeholder_protected_ranges(output).is_empty());
}

#[test]
fn live_resolved_output_masks_placeholders_and_keeps_syntax_after_them() {
    // HTML is the worst case for the old behaviour: handed `<Merge Conflict>`
    // verbatim, the grammar reads it as an opening element and swallows the
    // rest of the document, so the `<span>` below lost its highlighting.
    let theme = AppTheme::gitcomet_dark();
    let output = "<div>clean</div>\n<Merge Conflict>\n<span>tail</span>\n";
    let model = TextModel::from(output);
    let snapshot = model.snapshot();

    let protected = resolved_output_placeholder_protected_ranges(output);
    let mask = resolved_output_live_syntax_mask(protected.as_ref(), output);

    let placeholder_start = output.find("<Merge Conflict>").expect("placeholder");
    let placeholder_range = placeholder_start
        ..placeholder_start + conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER.len();
    assert_eq!(
        mask.as_ref(),
        std::slice::from_ref(&placeholder_range),
        "the mask should cover the placeholder text but not its newline"
    );

    let document =
        rows::LiveSyntaxDocument::new(rows::DiffSyntaxLanguage::Html, snapshot.rope(), mask, None)
            .expect("html output should build a live syntax document");
    let provider = resolved_output_live_highlight_provider(
        theme,
        document.snapshot(theme),
        ResolvedOutputUnresolvedSpans {
            all: Arc::from([placeholder_range.clone()]),
            active: Arc::default(),
        },
    );

    let result = provider.resolve(0..output.len());
    assert!(
        !result.pending,
        "the live provider is always exact for its text and must never report pending"
    );

    let placeholder_highlights: Vec<_> = result
        .highlights
        .iter()
        .filter(|(range, _)| {
            range.start < placeholder_range.end && range.end > placeholder_range.start
        })
        .collect();
    assert_eq!(placeholder_highlights.len(), 1);
    assert_eq!(placeholder_highlights[0].0, placeholder_range);
    assert_eq!(
        placeholder_highlights[0].1.color,
        Some(theme.colors.status.danger.foreground.into_color())
    );
    assert_eq!(
        placeholder_highlights[0].1.background_color, None,
        "an unselected conflict is called out in red alone"
    );

    let tail_start = output.find("<span>").expect("tail element");
    assert!(
        result
            .highlights
            .iter()
            .any(|(range, _)| range.start >= tail_start),
        "the element after an unresolved conflict must still be highlighted: {:?}",
        result.highlights
    );
}

#[test]
fn resolved_output_without_a_language_still_marks_unresolved_rows() {
    let theme = AppTheme::gitcomet_dark();
    let output = "head\n<Merge Conflict>\ntail\n";
    let placeholder_start = output.find("<Merge Conflict>").expect("placeholder");
    let placeholder_range = placeholder_start
        ..placeholder_start + conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER.len();

    let highlights = apply_resolved_output_unresolved_highlights(
        Vec::new(),
        &ResolvedOutputUnresolvedSpans {
            all: Arc::from([placeholder_range.clone()]),
            active: Arc::default(),
        },
        0..output.len(),
        resolved_output_unresolved_highlight_style(theme),
        resolved_output_active_unresolved_highlight_style(theme),
    );

    assert_eq!(highlights.len(), 1);
    assert_eq!(highlights[0].0, placeholder_range);
    assert_eq!(
        highlights[0].1.color,
        Some(theme.colors.status.danger.foreground.into_color())
    );
}

/// The point of the wash: with several conflicts open, only the one being
/// resolved is painted, and it keeps the danger text that marks it unresolved.
#[test]
fn the_active_conflicts_row_is_washed_yellow_and_the_others_are_not() {
    let theme = AppTheme::gitcomet_dark();
    let output = "head\n<Merge Conflict>\nmiddle\n<Merge Conflict>\ntail\n";
    let first = output.find("<Merge Conflict>").expect("first placeholder");
    let first_range = first..first + conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER.len();
    let second = output[first_range.end..]
        .find("<Merge Conflict>")
        .expect("second placeholder")
        + first_range.end;
    let second_range =
        second..second + conflict_resolver::UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER.len();

    let highlights = apply_resolved_output_unresolved_highlights(
        Vec::new(),
        &ResolvedOutputUnresolvedSpans {
            all: Arc::from([first_range.clone(), second_range.clone()]),
            active: Arc::from([second_range.clone()]),
        },
        0..output.len(),
        resolved_output_unresolved_highlight_style(theme),
        resolved_output_active_unresolved_highlight_style(theme),
    );

    let style_for = |range: &std::ops::Range<usize>| {
        highlights
            .iter()
            .find(|(highlighted, _)| highlighted == range)
            .map(|(_, style)| *style)
            .expect("every unresolved row is styled")
    };
    assert_eq!(style_for(&first_range).background_color, None);
    assert_eq!(
        style_for(&second_range).background_color,
        Some(resolved_output_active_conflict_background(theme).into_color())
    );
    assert_eq!(
        style_for(&second_range).color,
        Some(theme.colors.status.danger.foreground.into_color()),
        "the wash marks the selection; the row is still an unresolved one"
    );
}

#[test]
fn coalescing_edit_deltas_covers_every_edit_in_the_batch() {
    // One delta passes through unchanged.
    assert_eq!(
        coalesce_resolved_output_edit_deltas(&[(3..3, 3..4)]),
        Some((3..3, 3..4))
    );
    assert_eq!(
        coalesce_resolved_output_edit_deltas(&[(3..4, 3..3)]),
        Some((3..4, 3..3))
    );
    assert_eq!(coalesce_resolved_output_edit_deltas(&[]), None);

    // A run of single-character inserts at one caret collapses exactly: the
    // replaced span stays empty while the inserted span grows. This is the
    // dominant typing case, so getting it wrong would widen every reparse.
    assert_eq!(
        coalesce_resolved_output_edit_deltas(&[(3..3, 3..4), (4..4, 4..5), (5..5, 5..6)]),
        Some((3..3, 3..6))
    );

    // Two edits far apart widen to their union rather than being dropped.
    // Reparsing more than strictly necessary is sound; missing an edit is not.
    let (replaced, inserted) =
        coalesce_resolved_output_edit_deltas(&[(2..4, 2..2), (10..10, 10..13)])
            .expect("a non-empty batch always coalesces");
    assert!(
        replaced.start <= 2 && inserted.start <= 2,
        "the union must start at or before the earliest edit: {replaced:?} {inserted:?}"
    );
    assert!(
        inserted.end >= 13,
        "the union must reach past the latest edit: {inserted:?}"
    );
    assert_eq!(
        inserted.len() as isize - replaced.len() as isize,
        1,
        "the coalesced span must carry the batch's net length change (-2 then +3)"
    );
}

#[test]
fn the_live_provider_binding_key_is_stable_for_unchanged_inputs() {
    // Installing a provider notifies the input, which re-enters the
    // `cx.observe` that installed it. If the key were freshly minted each time,
    // that re-entry would rebind, notify again, and spin forever — the observe
    // loop would never settle and the pane would hang. The key must therefore
    // be a function of what the provider closes over, not a counter.
    let spans = ResolvedOutputUnresolvedSpans {
        all: Arc::from([5..21usize; 1]),
        active: Arc::default(),
    };

    let key = resolved_output_live_provider_binding_key(7, 3, &spans);
    assert_eq!(
        key,
        resolved_output_live_provider_binding_key(7, 3, &spans),
        "identical inputs must produce an identical key, or the observe cycle never settles"
    );

    assert_ne!(
        key,
        resolved_output_live_provider_binding_key(8, 3, &spans),
        "a new document version must rebind so interpolation is reset"
    );
    assert_ne!(
        key,
        resolved_output_live_provider_binding_key(7, 4, &spans),
        "the syntax palette is baked into the snapshot, so a theme change must rebind. \
         A theme epoch is used rather than sampled colours because two dark themes can \
         agree on any few colours you sample and still differ on the palette."
    );
    assert_ne!(
        key,
        resolved_output_live_provider_binding_key(
            7,
            3,
            &ResolvedOutputUnresolvedSpans {
                all: Arc::from([5..21usize, 40..56]),
                active: Arc::default(),
            }
        ),
        "the unresolved overlay is baked into the closure, so it must rebind"
    );
    assert_ne!(
        key,
        resolved_output_live_provider_binding_key(
            7,
            3,
            &ResolvedOutputUnresolvedSpans {
                all: Arc::clone(&spans.all),
                active: Arc::clone(&spans.all),
            }
        ),
        "selecting the conflict moves the wash onto its row, so it must rebind too"
    );
}

/// The resolved-output scanners read through the rope.
///
/// `ResolvedOutputSource` is implemented for both `str` and `Rope` so the same
/// analysis serves a materialized string and the live buffer. These pin the two
/// halves that matter: the answers must be identical, and the rope path must not
/// flatten the document to produce them.
#[cfg(test)]
mod rope_backed_scanner_tests {
    use super::super::helpers::resolved_output_unresolved_rows;
    use super::*;
    use crate::view::conflict_resolver::ResolvedOutputSource;

    fn fixture() -> (Vec<ConflictSegment>, String) {
        let mut segments = vec![ConflictSegment::Text("head\n".into())];
        for ix in 0..40 {
            segments.push(ConflictSegment::Block(ConflictBlock {
                base: None,
                ours: format!("ours {ix}\n").into(),
                theirs: format!("theirs {ix}\n").into(),
                choice: if ix % 3 == 0 {
                    ConflictChoice::Ours
                } else {
                    ConflictChoice::empty()
                },
                resolved: ix % 3 == 0,
                whitespace_only: false,
            }));
            segments.push(ConflictSegment::Text(
                format!("context {ix}\nmore {ix}\n").into(),
            ));
        }
        let output = conflict_resolver::generate_resolved_text(&segments);
        (segments, output)
    }

    /// The rope source must answer for non-ASCII text, not abort on it.
    ///
    /// `byte_at` is called on row ends (trimming `\r`) and `starts_with_at` on
    /// text segments while checking whether the buffer still matches the
    /// segments — both land inside a multi-byte character as soon as the user
    /// types one, and both used to slice a chunk as `&str` at that offset.
    #[test]
    fn the_rope_source_reads_non_ascii_offsets_without_panicking() {
        let text = "// caf\u{e9}\r\nlet s = \"\u{1f642}\";\n";
        let rope = crate::kit::rope::Rope::from_str(text);

        // Every offset, including interior ones, must answer as the raw byte.
        for offset in 0..text.len() {
            assert_eq!(
                ResolvedOutputSource::byte_at(&rope, offset),
                Some(text.as_bytes()[offset]),
                "byte_at({offset}) on {text:?}"
            );
        }
        assert_eq!(ResolvedOutputSource::byte_at(&rope, text.len()), None);

        // The `\r` trim in `resolved_output_unresolved_rows` reads the byte
        // before a row end; here that byte is the tail of `é`.
        let row_end = text.find('\n').expect("fixture has a row terminator");
        assert_eq!(
            ResolvedOutputSource::byte_at(&rope, row_end - 1),
            Some(b'\r')
        );

        // A needle that ends inside a character is a mismatch, not a panic.
        assert!(!ResolvedOutputSource::starts_with_at(
            &rope,
            0,
            "// caf\u{ea}"
        ));
        assert!(ResolvedOutputSource::starts_with_at(
            &rope,
            0,
            "// caf\u{e9}"
        ));
        // Starting inside a character likewise reports mismatch.
        assert!(!ResolvedOutputSource::starts_with_at(&rope, 7, "\u{e9}"));
        assert!(ResolvedOutputSource::starts_with_at(&rope, 6, "\u{e9}"));
        // And the str impl agrees on the answers it can give.
        assert!(ResolvedOutputSource::starts_with_at(
            text,
            0,
            "// caf\u{e9}"
        ));
        assert!(!ResolvedOutputSource::starts_with_at(
            text,
            0,
            "// caf\u{ea}"
        ));
    }

    /// Resolving a block must invalidate the unresolved-row cache even when the
    /// output text does not move.
    ///
    /// Picking the side already displayed — or resolving a whitespace-only block
    /// — rewrites the segments while leaving the buffer byte-identical, so its
    /// revision never bumps. Keyed on the revision alone, the cache would keep
    /// serving the pre-pick rows and the yellow wash would stay painted on a
    /// conflict the user just resolved.
    #[test]
    fn the_unresolved_row_key_changes_when_a_block_resolves_without_editing_the_text() {
        use super::super::helpers::ResolvedOutputKey;
        use crate::kit::text_model::TextModel;

        let (segments, output) = fixture();
        let snapshot = TextModel::from_large_text(&output).snapshot();

        let map = block_map_for(&segments);
        let before = ResolvedOutputKey::new(&snapshot, &segments, &map);

        // Resolve one still-open block. The output text is untouched.
        let mut resolved = segments.clone();
        let flipped = resolved
            .iter_mut()
            .find_map(|segment| match segment {
                ConflictSegment::Block(block) if !block.resolved => Some(block),
                _ => None,
            })
            .expect("fixture has an unresolved block");
        flipped.resolved = true;
        flipped.choice = ConflictChoice::Ours;

        let after = ResolvedOutputKey::new(&snapshot, &resolved, &map);

        assert_eq!(
            before.revision, after.revision,
            "this test is only meaningful while the buffer revision stays put"
        );
        assert_ne!(
            before, after,
            "the cache key must notice a block that changed resolution state"
        );

        // The rows also fall back to the block map for block geometry, so the
        // map is part of what they depend on. A map rebuilt or reset without the
        // text or any resolution moving would otherwise serve rows placed by the
        // old geometry.
        let empty_map = conflict_resolver::ResolvedOutputBlockMap::default();
        assert_ne!(
            ResolvedOutputKey::new(&snapshot, &segments, &map),
            ResolvedOutputKey::new(&snapshot, &segments, &empty_map),
            "the cache key must notice the block map changing under it"
        );
    }

    #[test]
    fn rope_and_str_scanners_agree() {
        let (segments, output) = fixture();
        let block_map = block_map_for(&segments);
        let rope = crate::kit::rope::Rope::from_str(&output);

        assert_eq!(
            resolved_output_placeholder_protected_ranges(output.as_str()).as_ref(),
            resolved_output_placeholder_protected_ranges(&rope).as_ref(),
            "protected ranges"
        );

        let protected = resolved_output_placeholder_protected_ranges(&rope);
        assert!(!protected.is_empty(), "fixture should have open conflicts");
        assert_eq!(
            resolved_output_live_syntax_mask(protected.as_ref(), output.as_str()).as_ref(),
            resolved_output_live_syntax_mask(protected.as_ref(), &rope).as_ref(),
            "syntax mask"
        );

        assert_eq!(
            resolved_output_markers_for_text(&segments, output.as_str(), &block_map),
            resolved_output_markers_for_text(&segments, &rope, &block_map),
            "conflict markers"
        );
    }

    /// The point of the exercise: scanning must not flatten the buffer.
    #[test]
    fn scanning_through_the_rope_does_not_materialize() {
        let (segments, output) = fixture();
        let block_map = block_map_for(&segments);

        let mut model = TextModel::from_large_text(&output);
        // Edit so the materialization cache is cold and cannot be a leftover.
        model.replace_range(0..0, "// edited\n");
        let snapshot = model.snapshot();
        assert!(!snapshot.is_materialized());

        let rope = snapshot.rope();
        let _ = resolved_output_unresolved_rows(&segments, &rope, &block_map);
        let protected = resolved_output_placeholder_protected_ranges(&rope);
        let _ = resolved_output_live_syntax_mask(protected.as_ref(), &rope);
        let _ = resolved_output_markers_for_text(&segments, &rope, &block_map);

        assert!(
            !snapshot.is_materialized(),
            "the scanners must read through the rope, not flatten it"
        );
        assert!(
            !snapshot.is_line_index_built(),
            "the scanners must seek rows in the rope, not build a document-sized \
             line-start index"
        );
    }
}

/// The heuristic arm — the one large buffers without a live tree land on.
#[cfg(test)]
mod heuristic_window_tests {
    use super::super::helpers::resolved_output_heuristic_highlights_for_range;
    use super::*;
    use crate::kit::rope::Rope;

    fn fixture() -> String {
        (0..400)
            .map(|ix| {
                format!("fn value_{ix}() -> &'static str {{ /* note {ix} */ \"text {ix}\" }}\n")
            })
            .collect()
    }

    /// Tokenizing a window must give exactly what tokenizing the document gives
    /// for those rows — the property that makes windowing this arm sound rather
    /// than approximate.
    #[test]
    fn a_window_matches_the_whole_document_over_the_same_rows() {
        let theme = AppTheme::gitcomet_dark();
        let text = fixture();
        let rope = Rope::from_str(&text);
        let language = rows::DiffSyntaxLanguage::Rust;

        let whole =
            resolved_output_heuristic_highlights_for_range(theme, &rope, language, 0..text.len());
        assert!(!whole.is_empty(), "fixture should produce highlights");

        // A window in the middle, snapped to row boundaries the same way the
        // function does internally.
        let start = rope.line_range(150).start;
        let end = rope.line_range(180).end;
        let window =
            resolved_output_heuristic_highlights_for_range(theme, &rope, language, start..end);

        let expected: Vec<_> = whole
            .iter()
            .filter(|(range, _)| range.start >= rope.line_range(150).start && range.end <= end)
            .cloned()
            .collect();
        assert!(!expected.is_empty(), "the window should contain highlights");
        for hit in &expected {
            assert!(
                window.contains(hit),
                "window is missing {hit:?} that the whole-document pass produced"
            );
        }
        // And it must not reach far outside the window.
        for (range, _) in &window {
            assert!(
                range.start >= rope.line_range(150).start && range.end <= end,
                "window produced {range:?} outside {start}..{end}"
            );
        }
    }

    #[test]
    fn tokenizing_a_window_does_not_materialize_the_document() {
        let theme = AppTheme::gitcomet_dark();
        let mut model = TextModel::from_large_text(&fixture());
        model.replace_range(0..0, "// edited\n");
        let snapshot = model.snapshot();
        assert!(!snapshot.is_materialized());

        let rope = snapshot.rope();
        let start = rope.line_range(100).start;
        let end = rope.line_range(120).end;
        let highlights = resolved_output_heuristic_highlights_for_range(
            theme,
            &rope,
            rows::DiffSyntaxLanguage::Rust,
            start..end,
        );

        assert!(!highlights.is_empty());
        assert!(
            !snapshot.is_materialized(),
            "the heuristic arm must read rows through the rope, not flatten the buffer"
        );
    }

    #[test]
    fn an_empty_or_out_of_bounds_window_is_empty() {
        let theme = AppTheme::gitcomet_dark();
        let rope = Rope::from_str(&fixture());
        let language = rows::DiffSyntaxLanguage::Rust;
        assert!(
            resolved_output_heuristic_highlights_for_range(theme, &rope, language, 5..5).is_empty()
        );
        assert!(
            resolved_output_heuristic_highlights_for_range(
                theme,
                &rope,
                language,
                rope.len()..rope.len() + 99
            )
            .is_empty()
        );
    }
}

/// The `str` and `Rope` implementations must answer identically, including for
/// ranges no caller should have produced.
///
/// Both are used against the same buffers in the same frame — `&str` from the
/// block-map rebuild, `&Rope` from the syntax and unresolved-row paths — so a
/// disagreement means one caller judges the output valid and another does not.
/// `str::count_newlines_in` used to answer 0 for any out-of-range or mid-
/// character span, which reads as "no newlines here" and lands conflict markers
/// on the wrong rows.
#[test]
fn the_str_and_rope_sources_agree_on_out_of_range_and_mid_character_input() {
    use crate::kit::rope::Rope;
    use crate::view::conflict_resolver::ResolvedOutputSource;

    for text in [
        "a\u{e9}bc\nd\u{1f642}e\nfg",
        "plain\nascii\nrows\n",
        "\u{1f642}\n\u{1f642}",
        "",
    ] {
        let rope = Rope::from_str(text);
        let past_end = text.len() + 5;

        for start in 0..=past_end {
            for end in 0..=past_end {
                let range = start..end;
                assert_eq!(
                    ResolvedOutputSource::count_newlines_in(text, range.clone()),
                    ResolvedOutputSource::count_newlines_in(&rope, range.clone()),
                    "count_newlines_in({range:?}) on {text:?}"
                );
            }
            assert_eq!(
                ResolvedOutputSource::byte_at(text, start),
                ResolvedOutputSource::byte_at(&rope, start),
                "byte_at({start}) on {text:?}"
            );
            for needle in ["\n", "a", "\u{e9}", "\u{1f642}", "bc"] {
                assert_eq!(
                    ResolvedOutputSource::starts_with_at(text, start, needle),
                    ResolvedOutputSource::starts_with_at(&rope, start, needle),
                    "starts_with_at({start}, {needle:?}) on {text:?}"
                );
            }
        }

        assert_eq!(
            ResolvedOutputSource::row_count(text),
            ResolvedOutputSource::row_count(&rope),
            "row_count on {text:?}"
        );
    }
}
