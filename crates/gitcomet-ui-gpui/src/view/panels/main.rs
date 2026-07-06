use super::*;

mod binary_conflict;
mod conflict_resolver_view;
mod decision_conflict;
mod diff;
mod diff_view;
mod diff_view_helpers;
mod history;
mod keep_delete_conflict;
mod status_nav;

pub(super) fn show_external_mergetool_actions(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn show_conflict_save_stage_action(view_mode: GitCometViewMode) -> bool {
    matches!(view_mode, GitCometViewMode::Normal)
}

pub(super) fn next_conflict_diff_split_ratio(
    state: ConflictDiffSplitResizeState,
    current_x: Pixels,
    column_widths: [Pixels; 2],
) -> Option<f32> {
    let main_width = column_widths[0] + column_widths[1] + px(PANE_RESIZE_HANDLE_PX);
    if main_width <= px(0.0) {
        return None;
    }

    let dx = current_x - state.start_x;
    let delta = dx / main_width;
    Some((state.start_ratio + delta).clamp(0.1, 0.9))
}
