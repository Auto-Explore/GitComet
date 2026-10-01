//! Focused-mergetool mode: exit codes, conflict load strategy, and saving
//! the resolved output.

use super::*;
use crate::view::conflict_resolver::ConflictSegment;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view::panes::main) enum ClearDiffSelectionAction {
    ClearSelection,
    ExitFocusedMergetool,
}

pub(in crate::view::panes::main) fn clear_diff_selection_action(
    view_mode: GitCometViewMode,
) -> ClearDiffSelectionAction {
    match view_mode {
        GitCometViewMode::Normal | GitCometViewMode::FocusedDiff => {
            ClearDiffSelectionAction::ClearSelection
        }
        GitCometViewMode::FocusedMergetool => ClearDiffSelectionAction::ExitFocusedMergetool,
    }
}

pub(in crate::view::panes::main) fn focused_mergetool_save_exit_code(
    total_conflicts: usize,
    resolved_conflicts: usize,
) -> i32 {
    if total_conflicts == 0 || total_conflicts == resolved_conflicts {
        FOCUSED_MERGETOOL_EXIT_SUCCESS
    } else {
        FOCUSED_MERGETOOL_EXIT_CANCELED
    }
}

pub(in crate::view::panes::main) fn conflict_strategy_needs_full_side_payloads(
    strategy: Option<gitcomet_core::conflict_session::ConflictResolverStrategy>,
) -> bool {
    matches!(
        strategy,
        Some(
            gitcomet_core::conflict_session::ConflictResolverStrategy::BinarySidePick
                | gitcomet_core::conflict_session::ConflictResolverStrategy::TwoWayKeepDelete
                | gitcomet_core::conflict_session::ConflictResolverStrategy::DecisionOnly
        )
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view::panes::main) enum FocusedMergetoolOutput<'a> {
    Write(&'a [u8]),
    Delete,
}

/// Apply a focused-mergetool result to the repository-relative `path`. Resolved
/// here, not at the call sites, so none of them can write through a symlink.
pub(in crate::view::panes::main) fn apply_focused_mergetool_output(
    workdir: &std::path::Path,
    path: &std::path::Path,
    output: FocusedMergetoolOutput<'_>,
) -> std::io::Result<()> {
    let relative = gitcomet_core::path_utils::validated_repo_relative_path(path)?;
    let target = gitcomet_core::path_utils::symlink_free_write_target(workdir, &relative)?;
    match output {
        FocusedMergetoolOutput::Write(bytes) => {
            if let Some(parent) = target
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, bytes)
        }
        FocusedMergetoolOutput::Delete => match std::fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view::panes::main) struct FocusedMergetoolSavePayload {
    pub(in crate::view::panes::main) output: String,
    pub(in crate::view::panes::main) total_conflicts: usize,
    pub(in crate::view::panes::main) resolved_conflicts: usize,
}

pub(in crate::view::panes::main) fn build_focused_mergetool_save_payload(
    marker_segments: &[ConflictSegment],
    block_region_indices: &[usize],
    block_map: &conflict_resolver::ResolvedOutputBlockMap,
    materialized_output_text: Option<&str>,
    labels: gitcomet_core::conflict_output::ConflictMarkerLabels<'_>,
) -> FocusedMergetoolSavePayload {
    use gitcomet_core::conflict_output::{GenerateResolvedTextOptions, UnresolvedConflictMode};

    let render_preserve_markers = |segments: &[ConflictSegment]| {
        conflict_resolver::generate_resolved_text_with_options(
            segments,
            GenerateResolvedTextOptions {
                unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
                labels: Some(labels),
            },
        )
    };

    if let Some(output_text) = materialized_output_text {
        if let Some(updates) = conflict_resolver::derive_region_resolution_updates_from_output(
            marker_segments,
            block_region_indices,
            block_map,
            output_text,
        ) {
            let mut save_segments = marker_segments.to_vec();
            let ordered_resolutions: Vec<_> = updates
                .into_iter()
                .map(|(_, resolution)| resolution)
                .collect();
            conflict_resolver::apply_ordered_region_resolutions(
                &mut save_segments,
                &ordered_resolutions,
            );
            let mut output = output_text.to_string();
            let blocks: Vec<_> = marker_segments
                .iter()
                .filter_map(|segment| match segment {
                    ConflictSegment::Block(block) => Some(block),
                    ConflictSegment::Text(_) => None,
                })
                .collect();
            for ((block, range), resolution) in blocks
                .into_iter()
                .zip(block_map.ranges())
                .zip(&ordered_resolutions)
                .rev()
            {
                if matches!(
                    resolution,
                    gitcomet_core::conflict_session::ConflictRegionResolution::Unresolved
                ) {
                    let marker_text =
                        render_preserve_markers(&[ConflictSegment::Block(block.clone())]);
                    output.replace_range(range.clone(), &marker_text);
                }
            }
            return FocusedMergetoolSavePayload {
                output,
                total_conflicts: conflict_resolver::conflict_count(&save_segments),
                resolved_conflicts: conflict_resolver::resolved_conflict_count(&save_segments),
            };
        }

        let total_conflicts = conflict_resolver::conflict_count(marker_segments);
        return FocusedMergetoolSavePayload {
            output: output_text.to_string(),
            total_conflicts,
            resolved_conflicts: if conflict_resolver::text_contains_conflict_markers(output_text) {
                0
            } else {
                total_conflicts
            },
        };
    }

    FocusedMergetoolSavePayload {
        output: render_preserve_markers(marker_segments),
        total_conflicts: conflict_resolver::conflict_count(marker_segments),
        resolved_conflicts: conflict_resolver::resolved_conflict_count(marker_segments),
    }
}
