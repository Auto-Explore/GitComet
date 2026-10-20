use super::super::path_display;
use super::super::perf::{self, ViewPerfSpan};
use super::super::*;
use std::sync::atomic::{AtomicI32, Ordering};

mod actions_impl;
mod conflict_actions;
mod core_impl;
pub(in crate::view) mod diff_cache;
mod diff_patch;
pub(in crate::view) mod diff_search;
mod diff_stage;
pub(in crate::view) mod diff_text;
mod file_disk;
pub(in crate::view) mod file_editor;
pub(in crate::view) mod helpers;
mod hosted_binding;
pub(in crate::view) mod input;
mod markdown_state;
mod media;
pub(in crate::view) use media::image_menu_request;
mod preview;
pub(in crate::view) mod services;
pub(in crate::view) use preview::{
    TextDecodeRequest, preflight_worktree_file_for_editing, read_worktree_file_for_editing,
    read_worktree_file_version_for_editing,
};
pub(in crate::view) mod submodule_summary;
mod surface;
mod text_format;
pub(in crate::view) use text_format::TextEncodingMenuState;

// The editor's free functions are exercised directly by the panel tests; the
// pane itself reaches them through `impl DiffRenderer`.
use crate::view::documents::disk::DiskIdentity;
pub(in crate::view) use core_impl::DiffRendererInit;
#[cfg(test)]
pub(in crate::view) use diff_text::take_opened_web_links_for_tests;
pub(in crate::view) use file_disk::{DiskCheckCause, DiskSurface, FileDiskNotice, FileDiskSeen};
#[cfg(test)]
pub(in crate::view) use file_editor::*;
pub(crate) use helpers::state::DiffRenderer;
use helpers::*;
use input::{DiffPaneInput, DiffSourceId};
pub(in crate::view) use markdown_state::*;
#[cfg(test)]
pub(in crate::view) use preview::{
    remote_markdown_image_row_visits_for_tests, reset_remote_markdown_image_row_visits_for_tests,
    take_link_followability_checks_for_tests,
};
use services::{DiffCommand, DiffCommands, DiffPopoverAnchor, DiffPreference, DiffServices};
#[cfg(test)]
pub(in crate::view) use surface::take_file_preview_active_checks_for_tests;
pub(in crate::view) use surface::{DiffBody, DiffSurfaceMemo};

#[cfg(not(test))]
const CONFLICT_RESOLVED_OUTLINE_DEBOUNCE_MS: u64 = 140;
const FOCUSED_MERGETOOL_EXIT_SUCCESS: i32 = 0;
const FOCUSED_MERGETOOL_EXIT_CANCELED: i32 = 1;
const FOCUSED_MERGETOOL_EXIT_ERROR: i32 = 2;

#[inline]
pub(in crate::view) fn pane_non_main_width_for_layout(
    sidebar_w: Pixels,
    details_w: Pixels,
    _sidebar_collapsed: bool,
    _details_collapsed: bool,
) -> Pixels {
    // Resize handles overlay pane boundaries and therefore consume no layout width.
    sidebar_w + details_w
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

impl Render for DiffRenderer {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.diff_surface_frame = self.diff_surface_frame.wrapping_add(1);
        self.last_window_size = window.viewport_size();
        self.sync_layout_snapshot(cx);
        if self.annotate_enabled && self.rendered_diff_target().is_some() {
            self.request_blame_for_current_target(false, cx);
        }
        let inner = if self.rendered_diff_target().is_some() {
            self.diff_view(window, cx).into_any_element()
        } else {
            div().into_any_element()
        };
        let search_action = std::mem::take(&mut self.diff_search_probe_render);
        crate::ui_probe::action_phase(search_action, "rendered", || {
            serde_json::json!({
                "window":format!("{:?}", window.window_handle().window_id()),
                "revision":self.diff_search_debounce_seq, "matches":self.diff_search_matches.len()
            })
        });
        div().size_full().relative().child(inner)
    }
}

impl Drop for DiffRenderer {
    fn drop(&mut self) {
        self.file_diff_cancellation.cancel();
        if let Some(cache) = self
            .hosted_decor
            .as_ref()
            .and_then(|decor| decor.file_cache.as_ref())
        {
            cache.cancel();
        }
        self.patch_cancellation.cancel();
        if let Some(token) = &self.diff_search_cancellation {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
