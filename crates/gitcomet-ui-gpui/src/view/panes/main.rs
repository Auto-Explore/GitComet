use super::super::path_display;
use super::super::perf::{self, ViewPerfSpan};
use super::super::*;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI32, Ordering};

mod actions_impl;
mod core_impl;
pub(in crate::view) mod diff_cache;
mod diff_search;
mod diff_text;
mod helpers;
mod preview;

pub(in crate::view) use helpers::*;

const CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS: u64 = 140;
const CONFLICT_RESOLVED_OUTLINE_AUTO_SYNTAX_MAX_LINES: usize = 4_000;
const CONFLICT_RESOLVED_OUTPUT_ROW_HEIGHT_PX: f32 = 20.0;
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

pub(in crate::view) fn pane_content_width_for_layout(
    total_w: Pixels,
    sidebar_w: Pixels,
    details_w: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> Pixels {
    let handles_w = (if sidebar_collapsed {
        px(0.0)
    } else {
        px(PANE_RESIZE_HANDLE_PX)
    }) + (if details_collapsed {
        px(0.0)
    } else {
        px(PANE_RESIZE_HANDLE_PX)
    });
    (total_w - sidebar_w - details_w - handles_w).max(px(0.0))
}

impl Render for MainPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        debug_assert!(matches!(
            self.view_mode,
            GitCometViewMode::Normal | GitCometViewMode::FocusedMergetool
        ));
        self.last_window_size = window.viewport_size();
        self.history_view
            .update(cx, |v, _| v.set_last_window_size(self.last_window_size));
        self.sync_root_layout_snapshot(cx);

        let show_diff = self
            .active_repo()
            .and_then(|r| r.diff_state.diff_target.as_ref())
            .is_some();
        if show_diff {
            div().size_full().child(self.diff_view(cx))
        } else {
            div().size_full().child(self.history_view.clone())
        }
    }
}

#[cfg(test)]
mod tests;
