use crate::app::{
    CloseWindow, DecreaseUiScale, IncreaseUiScale, NewWindow, OpenRepository, ResetUiScale,
};
use crate::kit::{Scrollbar, ScrollbarAxis};
use crate::theme::AppTheme;
use crate::ui_scale;
use gitcomet_core::diff::AnnotatedDiffLine;
#[cfg(test)]
use gitcomet_core::diff::annotate_unified;
#[cfg(test)]
use gitcomet_core::domain::RepoStatus;
use gitcomet_core::domain::{
    Branch, Commit, CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, Tag,
    UpstreamDivergence,
};
use gitcomet_core::file_diff::FileDiffRow;
use gitcomet_core::process::refresh_git_runtime;
use gitcomet_core::services::{PullMode, RemoteUrlKind, ResetMode};
use gitcomet_state::model::{
    AppNotificationKind, AppState, AuthPromptKind, CloneOpState, CloneOpStatus, DefaultTagType,
    DiagnosticKind, Loadable, RepoId, RepoState, SubmoduleTrustPromptOperation,
};
use gitcomet_state::msg::{Msg, StoreEvent};
use gitcomet_state::session;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    Anchor, Animation, AnimationExt, AnyElement, AnyView, App, Bounds, ClickEvent, CursorStyle,
    Decorations, DispatchPhase, Element, ElementId, Entity, FocusHandle, FontWeight,
    GlobalElementId, InspectorElementId, IsZero, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ResizeEdge, ScrollHandle,
    ScrollWheelEvent, ShapedLine, SharedString, Size, Style, StyleRefinement, Styled, TextRun,
    Tiling, UniformListScrollHandle, WeakEntity, Window, WindowControlArea, actions, anchored, div,
    fill, point, px, relative, size, uniform_list,
};
use rustc_hash::{FxHashMap, FxHashSet};
#[cfg(test)]
use std::collections::BTreeMap;
use std::hash::Hash;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::{Duration, Instant};

const REPO_ACTIVATION_THROTTLE: Duration = Duration::from_secs(5);

/// How long after requesting an interactive move/resize grab a deactivation is
/// still attributed to that grab. Compositors hand over focus within a frame;
/// generous enough for a loaded system, short enough that a genuine alt-tab
/// right after a drag is not mistaken for the grab.
const WINDOW_GRAB_DEACTIVATE_GRACE: Duration = Duration::from_millis(1_500);

/// Upper bound on how long a drag may hold the grab before the re-activation is
/// no longer treated as its echo. Only a safety valve: arming already requires a
/// fresh grab plus a deactivation within [`WINDOW_GRAB_DEACTIVATE_GRACE`].
const WINDOW_GRAB_REACTIVATE_GRACE: Duration = Duration::from_secs(120);

actions!(
    text_input_diff_navigation,
    [
        DiffPrevFile,
        DiffNextFile,
        DiffPrevSearchMatchOrChange,
        DiffNextSearchMatchOrChange,
        TextInputCommitSubmit,
        TextInputDiffPrevFile,
        TextInputDiffNextFile,
        TextInputDiffPrevSearchMatchOrChange,
        TextInputDiffNextSearchMatchOrChange,
        TextInputDiffPrevChange,
        TextInputDiffNextChange,
        OpenActiveViewSearch,
        PopoverPromptDismiss,
        PopoverPromptTabNext,
        PopoverPromptTabPrev,
        TerminalCopy,
        TerminalPaste,
        TerminalSelectAll,
        ToggleCommandPalette,
        CommandPaletteDismiss,
        LocateFileInExplorer,
    ]
);

pub(crate) fn is_diff_shortcut_candidate(keystroke: &gpui::Keystroke) -> bool {
    let key = keystroke.key.as_str();
    let mods = keystroke.modifiers;
    let no_command_modifiers = !mods.control && !mods.alt && !mods.platform && !mods.function;

    (key == "escape" && no_command_modifiers)
        || (mods.secondary() && mods.number_of_modifiers() == 1 && key == "f")
        || (matches!(key, "f1" | "f2" | "f3" | "f4" | "f7") && no_command_modifiers)
        || (key == "space" && no_command_modifiers)
        || (mods.alt
            && !mods.control
            && !mods.platform
            && !mods.function
            && matches!(
                key,
                "e" | "i" | "s" | "w" | "up" | "down" | "left" | "right"
            ))
        || ((mods.control || mods.platform)
            && !mods.alt
            && !mods.function
            // Ctrl/Cmd+Shift+A is the app-level recent-repositories shortcut.
            && (key != "a" || !mods.shift)
            && matches!(
                key,
                "1" | "2" | "3" | "a" | "c" | "e" | "s" | "d" | "h" | "u"
            ))
        || (matches!(key, "a" | "b" | "c" | "d") && no_command_modifiers)
}

/// Whether this activation is the tail of a move/resize grab we started, and so
/// must not be treated as the user returning to the app. Always consumes the
/// marker; a drag that outlives [`WINDOW_GRAB_REACTIVATE_GRACE`] falls back to
/// refreshing, which is the conservative direction.
fn consume_window_grab_activation(suppressed_at: &mut Option<Instant>, now: Instant) -> bool {
    match suppressed_at.take() {
        Some(at) => now.saturating_duration_since(at) <= WINDOW_GRAB_REACTIVATE_GRACE,
        None => false,
    }
}

fn repo_activation_msg(
    state: &AppState,
    last_activation_dispatch: &mut FxHashMap<RepoId, Instant>,
    now: Instant,
) -> Option<Msg> {
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    if !matches!(repo.open, Loadable::Ready(_)) {
        return None;
    }
    if last_activation_dispatch
        .get(&repo_id)
        .is_some_and(|last| now.saturating_duration_since(*last) < REPO_ACTIVATION_THROTTLE)
    {
        return None;
    }
    last_activation_dispatch.insert(repo_id, now);
    Some(Msg::RepoActivated { repo_id })
}

mod app_model;
mod branch_sidebar;
mod caches;
mod chrome;
pub(crate) mod clone_progress;
mod color;
mod command_palette;
mod commit_message_hover;
mod commit_message_text;
pub(crate) mod components;
mod conflict_markers;
pub(crate) mod conflict_resolver;
mod date_time;
mod diff_navigation;
mod diff_preview;
mod diff_text_model;
mod diff_text_selection;
mod diff_utils;
mod external_drag;
mod file_diff_display;
mod file_icons;
mod fingerprint;
mod history_graph;
pub(crate) mod history_mode;
mod history_refs_hover;
mod icons;
#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
mod linux_desktop_integration;
mod markdown_preview;
mod mod_helpers;
mod open_source_licenses_data;
mod panels;
mod panes;
mod patch_split;
mod path_display;
mod perf;
mod permalink;
pub(super) mod platform_open;
mod poller;
mod reflog_panel;
mod repo_open;
pub(crate) mod rows;
mod settings_window;
pub(crate) mod shortcut_labels;
mod sidebar_presentation;
mod splash;
mod state_apply;
mod terminal_alacritty;
mod terminal_panel;
mod terminal_preferences;
#[cfg(test)]
pub(crate) mod test_support;
mod toast_host;
mod tooltip;
mod tooltip_host;
mod update_check;
mod user_survey;
mod word_diff;

use app_model::AppUiModel;
use branch_sidebar::{BranchSection, BranchSidebarRow};
use caches::{
    HistoryBaseCache, HistoryBaseCacheRequest, HistoryBaseRowVm, HistoryCache,
    HistoryCacheBuildRequest, HistoryDecorationCache, HistoryDecorationCacheRequest,
    HistoryDecorationRowVm, HistoryDisplayKey, HistoryRefListItem, HistoryRefListItemKind,
    HistoryStashIdsCache, HistoryTextVm, HistoryWorktreeSummaryCache,
};
use chrome::TitleBarView;
use conflict_resolver::{ConflictPickSide, ConflictResolverViewMode};
#[cfg(test)]
use date_time::format_datetime;
#[cfg(test)]
use date_time::format_datetime_utc;
use date_time::{DateTimeFormat, Timezone, format_datetime_into};
use diff_preview::build_new_file_preview_from_diff;
use patch_split::build_patch_split_rows;
use poller::Poller;
pub(in crate::view) use terminal_preferences::{
    ActionBarTerminalTarget, ExternalTerminalLaunchContext, ExternalTerminalMode,
    TerminalPreferences, parse_terminal_args_multiline, resolve_embedded_shell_program,
    resolve_external_terminal_launch_spec,
};
use word_diff::{capped_word_diff_ranges, capped_word_diff_ranges_for_file_diff_texts};

use commit_message_hover::{CommitMessageHoverHost, CommitMessageHoverState};
#[cfg(test)]
use diff_text_model::CachedDiffTextSegment;
use diff_text_model::{CachedDiffStyledText, SyntaxTokenKind};
use diff_text_selection::{
    ConflictRowSelectionTracker, DiffTextSelectionOverlay, DiffTextSelectionTracker,
};
use diff_utils::{
    build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply,
    compute_diff_file_for_src_ix, compute_diff_file_stats,
    context_menu_selection_range_from_diff_text, diff_content_text, image_format_for_path,
    parse_unified_hunk_header_for_display, scrollbar_markers_from_flags,
    scrollbar_markers_from_visible_ranges,
};
use file_diff_display::{
    LARGE_DIFF_TEXT_MIN_BYTES, append_diff_display_text_slice, append_file_diff_display_text_slice,
    file_diff_display_len, file_diff_display_text, should_truncate_file_diff_display,
};
use history_refs_hover::{HISTORY_REFS_HOVER_MENU_INVOKER_PREFIX, HistoryRefsHoverHost};
pub(crate) use mod_helpers::TerminalPanelResizeState;
use mod_helpers::*;
pub use mod_helpers::{
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, InitialRepositoryLaunchMode, StartupCrashReport,
};
use panels::{ActionBarView, BottomStatusBarView, PopoverHost, RepoTabsBarView, action_bar_height};
pub(crate) use panes::MainPaneView;
use panes::{
    CollapsedSidebarSection, DetailsPaneInit, DetailsPaneView, HistoryPrimarySelection,
    HistoryView, ReflogPaneInit, ReflogPaneView, SidebarPaneView, history_primary_selection,
};
pub(crate) use settings_window::{SettingsWindowView, open_settings_window};
use toast_host::ToastHost;
use tooltip::GitCometTooltipExt;
use tooltip_host::TooltipHost;

#[cfg(test)]
pub(crate) use chrome::window_frame;
use color::with_alpha;
use icons::{svg_icon, svg_spinner};

const HISTORY_COL_BRANCH_PX: f32 = 130.0;
const HISTORY_COL_GRAPH_PX: f32 = 80.0;
const HISTORY_COL_GRAPH_MAX_PX: f32 = 240.0;
const HISTORY_COL_AUTHOR_PX: f32 = 140.0;
const HISTORY_COL_DATE_PX: f32 = 160.0;
const HISTORY_COL_SHA_PX: f32 = 88.0;
const HISTORY_COL_HANDLE_PX: f32 = 8.0;

const HISTORY_COL_BRANCH_MIN_PX: f32 = 60.0;
const HISTORY_COL_BRANCH_MAX_PX: f32 = 320.0;
const HISTORY_COL_GRAPH_MIN_PX: f32 = 44.0;
const HISTORY_COL_AUTHOR_MIN_PX: f32 = 80.0;
const HISTORY_COL_AUTHOR_MAX_PX: f32 = 260.0;
const HISTORY_COL_DATE_MIN_PX: f32 = 110.0;
const HISTORY_COL_DATE_MAX_PX: f32 = 240.0;
const HISTORY_COL_SHA_MIN_PX: f32 = 60.0;
const HISTORY_COL_SHA_MAX_PX: f32 = 160.0;
const HISTORY_COL_MESSAGE_MIN_PX: f32 = 220.0;
const ERROR_BANNER_OVERFLOW_HINT_MIN_LINES: usize = 8;
const ERROR_BANNER_OVERFLOW_HINT_MIN_CHARS: usize = 240;

const HISTORY_GRAPH_COL_GAP_PX: f32 = 16.0;
const HISTORY_GRAPH_MARGIN_X_PX: f32 = 10.0;
/// Corner radius where a graph line turns between columns. Against a 16px column
/// pitch and a 14px half-row this leaves roughly a 10px straight horizontal run
/// per column crossed and 8px of straight vertical below the corner, so the turn
/// reads as a corner rather than as a diagonal.
const HISTORY_GRAPH_ELBOW_RADIUS_PX: f32 = 6.0;

/// Width of the lane-coloured wash at the right edge of the graph column. It
/// fades from transparent into the border on the message cell, tying a commit's
/// dot to its message.
const HISTORY_GRAPH_FADE_WIDTH_PX: f32 = 44.0;
/// Alpha the fade reaches where it meets the message border. Deliberately faint:
/// it runs behind the lane strokes on every row, so anything stronger reads as a
/// selection highlight.
const HISTORY_GRAPH_FADE_ALPHA: f32 = 0.10;
/// Below this much ref-column width the hover branch badge is dropped rather
/// than truncated to an unreadable stub.
const HISTORY_BRANCH_BADGE_MIN_W_PX: f32 = 34.0;
/// Alpha of the hover branch badge. Faint by design -- it is an on-demand hint
/// in a column that otherwise holds solid ref chips, and must not read as one.
const HISTORY_BRANCH_BADGE_ALPHA: f32 = 0.70;
/// Width of the lane-coloured border down the left edge of the message cell.
const HISTORY_MESSAGE_BORDER_W_PX: f32 = 3.0;
/// Vertical inset of that border, so consecutive rows read as separate borders
/// rather than as one continuous stripe down the list.
const HISTORY_MESSAGE_BORDER_INSET_Y_PX: f32 = 3.0;
/// Gap between that border and the message text.
const HISTORY_MESSAGE_BORDER_GAP_PX: f32 = 6.0;

/// Left offset of the message text inside its cell, in design px.
///
/// With the lane border shown the text clears the border by a fixed gap rather
/// than using the cell's own padding — the border would otherwise sit almost
/// against the text. Shared by the commit rows, which paint their text on a
/// canvas, and the two uncommitted-changes rows, which lay theirs out as
/// elements, so the three cannot drift apart.
const fn history_message_text_left_px(show_graph_color_marker: bool) -> f32 {
    if show_graph_color_marker {
        HISTORY_MESSAGE_BORDER_W_PX + HISTORY_MESSAGE_BORDER_GAP_PX
    } else {
        HISTORY_COL_HANDLE_PX / 2.0
    }
}

const PANE_RESIZE_HANDLE_PX: f32 = 8.0;
const PANE_COLLAPSED_PX: f32 = 34.0;
const PANE_COLLAPSE_ANIM_MS: u64 = 120;
/// Fade-in/out duration for the collapsed-sidebar section popover.
const COLLAPSED_POPOVER_FADE_MS: u64 = 110;
const SIDEBAR_MIN_PX: f32 = 200.0;
const DETAILS_MIN_PX: f32 = 280.0;
const MAIN_MIN_PX: f32 = 280.0;

const DIFF_SPLIT_COL_MIN_PX: f32 = 160.0;

const DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 4000;
const DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE: usize = 256;
const TOAST_FADE_IN_MS: u64 = 180;
const TOAST_FADE_OUT_MS: u64 = 220;
const TOAST_SLIDE_PX: f32 = 12.0;
const TERMINAL_PANEL_DEFAULT_HEIGHT_PX: f32 = 220.0;
const TERMINAL_PANEL_RESIZE_HANDLE_PX: f32 = 6.0;
pub(crate) const WEBSITE_URL: &str = "https://gitcomet.dev";
pub(crate) const EDITIONS_URL: &str = "https://gitcomet.dev/#editions";
pub(crate) const RELEASES_URL: &str = "https://github.com/Auto-Explore/GitComet/releases";
pub(crate) const DISCORD_URL: &str = "https://discord.com/invite/2ufDGP8RnA";

pub(in crate::view) fn restrict_scroll_to_vertical_axis<E: Styled>(mut element: E) -> E {
    element.style().restrict_scroll_to_axis = Some(true);
    element
}

// Only use these wrappers for views that remain mounted while their parent is mounted.
// Parent-controlled mount/unmount boundaries, like collapsible panes, must rebuild their child.
fn stable_cached_view<V: Render>(view: Entity<V>, style: StyleRefinement) -> AnyElement {
    let view = AnyView::from(view);
    // GPUI's cached mount path skips some test-only debug bounds and paint tracking.
    if cfg!(test) {
        view.into_any_element()
    } else {
        view.cached(style).into_any_element()
    }
}

fn stable_cached_fill_view<V: Render>(view: Entity<V>) -> AnyElement {
    stable_cached_view(view, StyleRefinement::default().size_full())
}

fn stable_cached_fixed_height_view<V: Render>(view: Entity<V>, height: Pixels) -> AnyElement {
    stable_cached_view(
        view,
        StyleRefinement::default().w_full().h(height).flex_none(),
    )
}

fn stable_overlay_view<V: Render>(view: Entity<V>) -> impl IntoElement {
    // Keep overlay hosts uncached. Their paint ranges are recorded after focused
    // TextInput views register platform input handlers, and Wayland text-input
    // replace_text_in_range can trigger a redraw while that handler is
    // temporarily unavailable. Reusing the cached overlay paint range then
    // replays a stale input-handler index and panics inside GPUI reuse_paint.
    div().absolute().top_0().left_0().size_full().child(view)
}

struct UiScaleScrollCapture {
    view: Entity<GitCometView>,
}

impl IntoElement for UiScaleScrollCapture {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for UiScaleScrollCapture {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = px(0.0).into();
        style.size.height = px(0.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !renders_full_chrome(self.view.read(cx).view_mode) {
            return;
        }

        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            let zoom_modifier = event.modifiers.secondary() || event.modifiers.control;
            if phase != DispatchPhase::Capture
                || !zoom_modifier
                || event.modifiers.alt
                || event.modifiers.function
            {
                return;
            }

            if !renders_full_chrome(view.read(cx).view_mode) {
                return;
            }

            let delta_y = event.delta.pixel_delta(window.line_height()).y;
            if delta_y.is_zero() {
                return;
            }

            let current = crate::ui_scale::current(cx).percent;
            let next = if delta_y > px(0.0) {
                crate::ui_scale::step_up(current)
            } else {
                crate::ui_scale::step_down(current)
            };

            cx.stop_propagation();
            if next == current {
                return;
            }

            cx.defer(move |cx| {
                crate::app::set_app_ui_scale_percent(cx, next);
            });
        });
    }
}

fn active_diff_target(state: &AppState) -> Option<(RepoId, DiffTarget)> {
    let repo_id = state.active_repo?;
    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    Some((repo_id, repo.diff_state.diff_target.clone()?))
}

fn active_merge_view_target(state: &AppState) -> Option<(RepoId, DiffTarget)> {
    let (repo_id, target) = active_diff_target(state)?;
    let DiffTarget::WorkingTree { path, area } = &target else {
        return None;
    };
    if *area != DiffArea::Unstaged {
        return None;
    }

    let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
    repo.status_entry_for_path(DiffArea::Unstaged, path)
        .filter(|entry| entry.kind == FileStatusKind::Conflicted && entry.conflict.is_some())?;
    Some((repo_id, target))
}

#[cfg(test)]
pub(in crate::view) fn pane_resize_drag_width_bounds(
    handle: PaneResizeHandle,
    start_sidebar: Pixels,
    start_details: Pixels,
    total_w: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> (Pixels, Pixels) {
    let (min_width, other_width, other_collapsed) = match handle {
        PaneResizeHandle::Sidebar => (px(SIDEBAR_MIN_PX), start_details, details_collapsed),
        PaneResizeHandle::Details => (px(DETAILS_MIN_PX), start_sidebar, sidebar_collapsed),
    };
    pane_resize_drag_width_bounds_for_other_pane(
        min_width,
        other_width,
        other_collapsed,
        total_w,
        sidebar_collapsed,
        details_collapsed,
    )
}

#[inline]
pub(in crate::view) fn pane_resize_drag_width_bounds_for_other_pane(
    min_width: Pixels,
    other_width: Pixels,
    other_collapsed: bool,
    total_w: Pixels,
    _sidebar_collapsed: bool,
    _details_collapsed: bool,
) -> (Pixels, Pixels) {
    let main_min = px(MAIN_MIN_PX);
    let collapsed_w = px(PANE_COLLAPSED_PX);
    // Both pane resize handles overlay their boundaries and consume no layout width.
    let available_w = total_w - main_min;
    let other_width = if other_collapsed {
        collapsed_w
    } else {
        other_width
    };
    let max_width = (available_w - other_width).max(min_width);
    (min_width, max_width)
}

pub(in crate::view) fn next_pane_resize_drag_width(
    state: &PaneResizeState,
    current_x: Pixels,
    total_w: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> Pixels {
    let dx = current_x - state.start_x;
    let (min_width, max_width) =
        state.drag_width_bounds(total_w, sidebar_collapsed, details_collapsed);
    (state.start_width + (dx * state.drag_delta_sign))
        .max(min_width)
        .min(max_width)
}

/// Pure helper: compute the next diff-split ratio for a single drag step.
///
/// Returns `None` when the available width is too narrow for two columns
/// (the caller should force 50/50 in that case).
pub(in crate::view) fn next_diff_split_drag_ratio(
    available: Pixels,
    min_col_w: Pixels,
    start_ratio: f32,
    dx: Pixels,
) -> Option<f32> {
    if available <= min_col_w * 2.0 {
        return None;
    }
    let max_left = available - min_col_w;
    let next_left = ((available * start_ratio) + dx)
        .max(min_col_w)
        .min(max_left);
    Some((next_left / available).clamp(0.0, 1.0))
}

/// Returns `(available, min_col_w)` for the diff-split layout given the main
/// pane's content width.  Bundles the handle-width and column-min constants so
/// callers do not need to reference them directly.
#[inline]
pub(in crate::view) fn diff_split_drag_params(main_pane_content_width: Pixels) -> (Pixels, Pixels) {
    let handle_w = px(PANE_RESIZE_HANDLE_PX);
    let min_col_w = px(DIFF_SPLIT_COL_MIN_PX);
    let available = (main_pane_content_width - handle_w).max(px(0.0));
    (available, min_col_w)
}

#[inline]
pub(in crate::view) fn diff_split_column_widths_from_available(
    available: Pixels,
    min_col_w: Pixels,
    ratio: f32,
) -> (Pixels, Pixels) {
    let left_w = if available <= min_col_w * 2.0 {
        available * 0.5
    } else {
        (available * ratio)
            .max(min_col_w)
            .min(available - min_col_w)
    };
    let right_w = available - left_w;
    (left_w, right_w)
}

#[inline]
pub(in crate::view) fn diff_split_column_widths(
    main_pane_content_width: Pixels,
    ratio: f32,
) -> (Pixels, Pixels) {
    let (available, min_col_w) = diff_split_drag_params(main_pane_content_width);
    diff_split_column_widths_from_available(available, min_col_w, ratio)
}

pub(crate) const UI_MONOSPACE_FONT_FAMILY: &str = crate::bundled_fonts::LILEX_FONT_FAMILY;

mod gitcomet_view;
mod gitcomet_view_render;

#[cfg(test)]
mod tests;
