use super::super::path_display;
use super::super::perf::{self, ViewPerfSpan};
use super::super::*;
use std::sync::atomic::{AtomicI32, Ordering};

mod actions_impl;
mod core_impl;
pub(in crate::view) mod diff_cache;
mod diff_search;
mod diff_text;
mod helpers;
mod preview;

#[cfg(feature = "benchmarks")]
#[allow(unused_imports)]
pub(in crate::view) use diff_search::{
    AsciiCaseInsensitiveNeedle, DiffSearchQueryReuse, diff_search_query_reuse,
};
pub(in crate::view) use helpers::*;

#[cfg(not(test))]
const CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS: u64 = 140;
const CONFLICT_RESOLVED_OUTPUT_ROW_HEIGHT_PX: f32 = 20.0;
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

#[inline]
pub(in crate::view) fn pane_non_main_width_for_layout(
    sidebar_w: Pixels,
    details_w: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> Pixels {
    sidebar_w + details_w + pane_resize_handles_width(sidebar_collapsed, details_collapsed)
}

#[inline]
pub(in crate::view) fn pane_content_width_for_layout_from_non_main_width(
    total_w: Pixels,
    non_main_w: Pixels,
) -> Pixels {
    (total_w - non_main_w).max(px(0.0))
}

pub(in crate::view) fn pane_content_width_for_layout(
    total_w: Pixels,
    sidebar_w: Pixels,
    details_w: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> Pixels {
    pane_content_width_for_layout_from_non_main_width(
        total_w,
        pane_non_main_width_for_layout(sidebar_w, details_w, sidebar_collapsed, details_collapsed),
    )
}

impl Render for MainPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        debug_assert!(matches!(
            self.view_mode,
            GitCometViewMode::Normal | GitCometViewMode::FocusedMergetool
        ));
        self.last_window_size = window.viewport_size();
        self.sync_root_layout_snapshot(cx);
        let history_content_width = self.main_pane_content_width(cx);
        self.history_view.update(cx, |v, _| {
            v.set_last_window_size(self.last_window_size);
            v.set_history_content_width(history_content_width);
        });

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
