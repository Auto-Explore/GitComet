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
    AppNotificationKind, AppState, AuthPromptKind, CloneOpState, CloneOpStatus, DiagnosticKind,
    Loadable, RepoId, RepoState, SubmoduleTrustPromptOperation,
};
use gitcomet_state::msg::{Msg, RepoExternalChange, StoreEvent};
use gitcomet_state::session;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, Bounds, ClickEvent, Corner, CursorStyle,
    Decorations, DispatchPhase, Element, ElementId, Entity, FocusHandle, FontWeight,
    GlobalElementId, InspectorElementId, IsZero, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ResizeEdge, ScrollHandle,
    ScrollWheelEvent, ShapedLine, SharedString, Size, Style, StyleRefinement, TextRun, Tiling,
    UniformListScrollHandle, WeakEntity, Window, WindowControlArea, actions, anchored, div, fill,
    point, px, relative, size, uniform_list,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
#[cfg(test)]
use std::collections::BTreeMap;
use std::hash::Hash;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::{Duration, Instant};

actions!(
    text_input_diff_navigation,
    [
        TextInputCommitSubmit,
        TextInputDiffPrevFile,
        TextInputDiffNextFile,
        TextInputDiffPrevSearchMatchOrChange,
        TextInputDiffNextSearchMatchOrChange,
        TextInputDiffPrevChange,
        TextInputDiffNextChange,
    ]
);

mod app_model;
mod branch_sidebar;
mod caches;
mod chrome;
pub(crate) mod clone_progress;
mod color;
pub(crate) mod components;
pub(crate) mod conflict_resolver;
mod date_time;
mod diff_navigation;
mod diff_preview;
mod diff_text_model;
mod diff_text_selection;
mod diff_utils;
mod fingerprint;
mod history_graph;
pub(crate) mod history_mode;
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
pub(super) mod platform_open;
mod poller;
mod repo_open;
pub(crate) mod rows;
mod settings_window;
mod sidebar_presentation;
mod splash;
mod state_apply;
#[cfg(test)]
pub(crate) mod test_support;
mod toast_host;
mod tooltip;
mod tooltip_host;
mod update_check;
mod word_diff;

use app_model::AppUiModel;
use branch_sidebar::{BranchSection, BranchSidebarRow};
use caches::{
    HistoryBaseCache, HistoryBaseCacheRequest, HistoryBaseRowVm, HistoryCache,
    HistoryCacheBuildRequest, HistoryDecorationCache, HistoryDecorationCacheRequest,
    HistoryDecorationRowVm, HistoryDisplayKey, HistoryStashIdsCache, HistoryTextVm,
    HistoryWorktreeSummaryCache,
};
use chrome::{TitleBarView, cursor_style_for_resize_edge, resize_edge};
use conflict_resolver::{ConflictPickSide, ConflictResolverViewMode};
#[cfg(test)]
use date_time::format_datetime;
#[cfg(test)]
use date_time::format_datetime_utc;
use date_time::{DateTimeFormat, Timezone, format_datetime_into};
use diff_preview::build_new_file_preview_from_diff;
use patch_split::build_patch_split_rows;
use poller::Poller;
use word_diff::capped_word_diff_ranges;

#[cfg(test)]
use diff_text_model::CachedDiffTextSegment;
use diff_text_model::{CachedDiffStyledText, SyntaxTokenKind};
use diff_text_selection::{DiffTextSelectionOverlay, DiffTextSelectionTracker};
use diff_utils::{
    build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard,
    compute_diff_file_for_src_ix, compute_diff_file_stats,
    context_menu_selection_range_from_diff_text, diff_content_text, image_format_for_path,
    parse_diff_git_header_path, parse_unified_hunk_header_for_display,
    scrollbar_markers_from_flags,
};
use mod_helpers::*;
pub use mod_helpers::{
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, InitialRepositoryLaunchMode, StartupCrashReport,
};
use panels::{ActionBarView, BottomStatusBarView, PopoverHost, RepoTabsBarView, action_bar_height};
use panes::{DetailsPaneInit, DetailsPaneView, HistoryView, MainPaneView, SidebarPaneView};
pub(crate) use settings_window::{SettingsWindowView, open_settings_window};
use toast_host::ToastHost;
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

const PANE_RESIZE_HANDLE_PX: f32 = 8.0;
const PANE_COLLAPSED_PX: f32 = 34.0;
const PANE_COLLAPSE_ANIM_MS: u64 = 120;
const SIDEBAR_MIN_PX: f32 = 200.0;
const DETAILS_MIN_PX: f32 = 280.0;
const MAIN_MIN_PX: f32 = 280.0;

const DIFF_SPLIT_COL_MIN_PX: f32 = 160.0;

const DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 4000;
const DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE: usize = 256;
const TOAST_FADE_IN_MS: u64 = 180;
const TOAST_FADE_OUT_MS: u64 = 220;
const TOAST_SLIDE_PX: f32 = 12.0;
pub(crate) const EDITIONS_URL: &str = "https://gitcomet.dev/#editions";

// Only use these wrappers for views that remain mounted while their parent is mounted.
// Parent-controlled mount/unmount boundaries, like collapsible panes, must rebuild their child.
fn stable_cached_view<V: Render>(view: Entity<V>, style: StyleRefinement) -> AnyView {
    let view = AnyView::from(view);
    // GPUI's cached mount path skips some test-only debug bounds and paint tracking.
    if cfg!(test) { view } else { view.cached(style) }
}

fn stable_cached_fill_view<V: Render>(view: Entity<V>) -> AnyView {
    stable_cached_view(view, StyleRefinement::default().size_full())
}

fn stable_cached_fixed_height_view<V: Render>(view: Entity<V>, height: Pixels) -> AnyView {
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
            if phase != DispatchPhase::Capture
                || !event.modifiers.secondary()
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

pub(in crate::view) fn pane_resize_handles_width(
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> Pixels {
    let visible_handles = u8::from(!sidebar_collapsed).saturating_add(u8::from(!details_collapsed));
    px(f32::from(visible_handles) * PANE_RESIZE_HANDLE_PX)
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
    sidebar_collapsed: bool,
    details_collapsed: bool,
) -> (Pixels, Pixels) {
    let handles_w = pane_resize_handles_width(sidebar_collapsed, details_collapsed);
    let main_min = px(MAIN_MIN_PX);
    let collapsed_w = px(PANE_COLLAPSED_PX);
    let available_w = total_w - main_min - handles_w;
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

impl GitCometView {
    pub(in crate::view) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_at(kind, anchor, window, cx)
        });
    }

    pub(in crate::view) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.popover_host.update(cx, |host, cx| {
            host.open_popover_for_bounds(kind, anchor_bounds, window, cx)
        });
    }

    pub(in crate::view) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();

        let sidebar_pane = self.sidebar_pane.clone();
        let main_pane = self.main_pane.clone();
        let details_pane = self.details_pane.clone();
        let action_bar = self.action_bar.clone();
        let bottom_status_bar = self.bottom_status_bar.clone();

        cx.defer(move |cx| {
            sidebar_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            main_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            details_pane.update(cx, |pane, cx| {
                pane.set_active_context_menu_invoker(next.clone(), cx);
            });
            action_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
            bottom_status_bar.update(cx, |bar, cx| {
                bar.set_active_context_menu_invoker(next.clone(), cx);
            });
        });
    }

    pub(in crate::view) fn register_pending_worktree_branch_removal(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        branch: String,
    ) {
        self.pending_worktree_branch_removals
            .insert((repo_id, path), branch);
    }

    fn take_pending_worktree_branch_removal(
        &mut self,
        repo_id: RepoId,
        path: &std::path::Path,
    ) -> Option<String> {
        self.pending_worktree_branch_removals
            .remove(&(repo_id, path.to_path_buf()))
    }

    #[cfg(test)]
    pub fn new(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        initial_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let config = match initial_path {
            Some(path) => GitCometViewConfig::normal_with_initial_repository(path, None),
            None => GitCometViewConfig::normal(None),
        };
        Self::new_with_config(store, events, config, window, cx)
    }

    pub fn new_with_config(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        config: GitCometViewConfig,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let GitCometViewConfig {
            mut initial_path,
            initial_repository_launch_mode,
            view_mode,
            focused_mergetool,
            focused_mergetool_exit_code,
            startup_crash_report,
        } = config;
        if initial_path.is_none() {
            initial_path = focused_mergetool.as_ref().map(|cfg| cfg.repo_path.clone());
        }
        let focused_mergetool_labels = focused_mergetool.as_ref().map(|cfg| cfg.labels.clone());
        let focused_mergetool_bootstrap = if view_mode == GitCometViewMode::FocusedMergetool {
            focused_mergetool
                .clone()
                .map(FocusedMergetoolBootstrap::from_view_config)
        } else {
            None
        };
        let store = Arc::new(store);

        let mut ui_session = session::load();
        let ui_scale = ui_scale::current_or_initialize_from_session(&ui_session, cx);
        let _font_preferences =
            crate::font_preferences::current_or_initialize_from_session(window, &ui_session, cx);
        if should_seed_initial_repository_from_session(
            view_mode,
            initial_path.as_deref(),
            initial_repository_launch_mode,
            !ui_session.open_repos.is_empty(),
        ) && let Some(path) = initial_path.as_ref()
        {
            if !ui_session.open_repos.iter().any(|p| p == path) {
                ui_session.open_repos.push(path.clone());
            }
            ui_session.active_repo = Some(path.clone());
        }

        let restored_sidebar_width = ui_session.sidebar_width;
        let restored_details_width = ui_session.details_width;
        let _ = crate::theme::ensure_user_themes_dir_exists();
        let theme_mode = ui_session
            .theme_mode
            .as_deref()
            .and_then(ThemeMode::from_key)
            .unwrap_or_default();
        let initial_theme = theme_mode.resolve_theme(window.appearance());
        let date_time_format = ui_session
            .date_time_format
            .as_deref()
            .and_then(DateTimeFormat::from_key)
            .unwrap_or(DateTimeFormat::YmdHm);
        let timezone = ui_session
            .timezone
            .as_deref()
            .and_then(Timezone::from_key)
            .unwrap_or_default();
        let show_timezone = ui_session.show_timezone.unwrap_or(true);
        let change_tracking_view = ui_session
            .change_tracking_view
            .as_deref()
            .and_then(ChangeTrackingView::from_key)
            .unwrap_or_default();
        let diff_scroll_sync = ui_session
            .diff_scroll_sync
            .as_deref()
            .and_then(DiffScrollSync::from_key)
            .unwrap_or_default();
        let restored_change_tracking_height = ui_session.change_tracking_height;
        let restored_untracked_height = ui_session.untracked_height;

        let history_show_graph = ui_session.history_show_graph.unwrap_or(true);
        let history_show_author = ui_session.history_show_author.unwrap_or(true);
        let history_show_date = ui_session.history_show_date.unwrap_or(true);
        let history_show_sha = ui_session.history_show_sha.unwrap_or(false);
        let history_show_tags = ui_session.history_show_tags.unwrap_or(true);
        let history_tag_fetch_mode = ui_session.history_tag_fetch_mode.unwrap_or_default();
        store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: history_show_tags,
            tag_fetch_mode: history_tag_fetch_mode,
        });
        let saved_open_repos = ui_session.open_repos.clone();
        let saved_active_repo = ui_session.active_repo.clone();
        let mut startup_repo_bootstrap_pending = false;
        let mut deferred_repo_bootstrap = None;

        // Only auto-restore/open on startup if the store hasn't already been preloaded.
        // This avoids re-opening repos (and changing RepoIds) when the UI is attached to an
        // already-initialized store (notably in `gpui::test` setup).
        let initial_store_state = store.snapshot();
        let store_preloaded = !initial_store_state.repos.is_empty();
        let git_runtime_available = initial_store_state.git_runtime.is_available();
        let should_auto_restore = !crate::startup_probe::disable_auto_restore()
            && view_mode != GitCometViewMode::FocusedMergetool
            && crate::ui_runtime::current().auto_restores_session()
            && !store_preloaded;

        if should_auto_restore {
            if !saved_open_repos.is_empty() {
                if git_runtime_available {
                    store.dispatch(Msg::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                    startup_repo_bootstrap_pending = true;
                } else {
                    deferred_repo_bootstrap = Some(DeferredRepoBootstrap::RestoreSession {
                        open_repos: saved_open_repos,
                        active_repo: saved_active_repo,
                    });
                }
            }
        } else if store_preloaded {
            if let Some(path) = initial_path.as_ref() {
                if git_runtime_available {
                    store.dispatch(Msg::OpenRepo(path.clone()));
                } else {
                    deferred_repo_bootstrap = Some(DeferredRepoBootstrap::OpenRepo(path.clone()));
                }
            }
        } else if let Some(path) = initial_path.as_ref() {
            if git_runtime_available {
                store.dispatch(Msg::OpenRepo(path.clone()));
                startup_repo_bootstrap_pending = true;
            } else {
                deferred_repo_bootstrap = Some(DeferredRepoBootstrap::OpenRepo(path.clone()));
            }
        }

        let initial_state = store.snapshot();
        if !initial_state.repos.is_empty() {
            startup_repo_bootstrap_pending = false;
        }
        let ui_model = cx.new(|_cx| AppUiModel::new(Arc::clone(&initial_state)));

        let ui_model_subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let should_quit = crate::startup_probe::observe_app_state(next.as_ref());
            let should_notify = this.apply_state_snapshot(next, cx);
            if should_notify {
                cx.notify();
            }
            if should_quit {
                cx.quit();
            }
        });

        let weak_view = cx.weak_entity();
        let poller = Poller::start(Arc::clone(&store), events, ui_model.downgrade(), window, cx);

        let title_bar = cx.new(|_cx| {
            TitleBarView::new(
                initial_theme,
                weak_view.clone(),
                titlebar_workspace_actions_enabled(view_mode, !initial_state.repos.is_empty()),
            )
        });
        let tooltip_host = cx.new(|_cx| TooltipHost::new(initial_theme));
        let toast_host = cx
            .new(|_cx| ToastHost::new(initial_theme, tooltip_host.downgrade(), weak_view.clone()));
        let repo_tabs_bar = cx.new(|cx| {
            RepoTabsBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let action_bar = cx.new(|cx| {
            ActionBarView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let bottom_status_bar = cx.new(|_cx| {
            BottomStatusBarView::new(initial_theme, weak_view.clone(), tooltip_host.downgrade())
        });

        let sidebar_pane = cx.new(|cx| {
            SidebarPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                ui_session.repo_sidebar_collapsed_items.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                cx,
            )
        });
        let main_pane = cx.new(|cx| {
            MainPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                date_time_format,
                timezone,
                show_timezone,
                diff_scroll_sync,
                history_show_graph,
                history_show_author,
                history_show_date,
                history_show_sha,
                history_show_tags,
                matches!(
                    history_tag_fetch_mode,
                    gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
                ),
                view_mode,
                focused_mergetool_labels,
                focused_mergetool_exit_code.clone(),
                weak_view.clone(),
                tooltip_host.downgrade(),
                window,
                cx,
            )
        });
        let details_pane = cx.new(|cx| {
            DetailsPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                DetailsPaneInit {
                    theme: initial_theme,
                    change_tracking_view,
                    change_tracking_height: restored_change_tracking_height,
                    untracked_height: restored_untracked_height,
                    ui_scale_percent: ui_scale.percent,
                    root_view: weak_view.clone(),
                    tooltip_host: tooltip_host.downgrade(),
                },
                window,
                cx,
            )
        });

        let popover_host = cx.new(|cx| {
            PopoverHost::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
                theme_mode.clone(),
                date_time_format,
                timezone,
                show_timezone,
                change_tracking_view,
                weak_view.clone(),
                main_pane.clone(),
                details_pane.clone(),
                window,
                cx,
            )
        });

        let activation_subscription = cx.observe_window_activation(window, |this, window, _cx| {
            if !window.is_window_active() {
                return;
            }
            let runtime = refresh_git_runtime();
            if runtime != this.state.git_runtime {
                this.store
                    .dispatch(Msg::SetGitRuntimeState(runtime.clone()));
            }
            if !runtime.is_available() {
                return;
            }
            if let Some(repo) = this.active_repo()
                && matches!(repo.open, Loadable::Ready(_))
            {
                this.store.dispatch(Msg::RepoExternallyChanged {
                    repo_id: repo.id,
                    change: RepoExternalChange::GitState,
                });
            }
        });

        let appearance_subscription = {
            let view = cx.weak_entity();
            let mut first = true;
            window.observe_window_appearance(move |window, app| {
                if first {
                    first = false;
                    return;
                }
                let _ = view.update(app, |this, cx| {
                    if !this.theme_mode.is_automatic() {
                        return;
                    }
                    let theme = this.theme_mode.resolve_theme(window.appearance());
                    this.set_theme(theme, cx);
                    cx.notify();
                });
            })
        };

        let open_repo_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "/path/to/repo".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let error_banner_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let auth_prompt_username_input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Username".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let auth_prompt_secret_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Password / passphrase / confirmation".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            );
            input.set_masked(true, cx);
            input
        });

        let scale = ui_scale::UiScale::from_percent(ui_scale.percent);
        let initial_sidebar_width_design =
            ui_scale::design_units_from_stored(restored_sidebar_width)
                .unwrap_or(280.0)
                .max(SIDEBAR_MIN_PX);
        let initial_details_width_design =
            ui_scale::design_units_from_stored(restored_details_width)
                .unwrap_or(420.0)
                .max(DETAILS_MIN_PX);
        let initial_sidebar_width = scale.px(initial_sidebar_width_design);
        let initial_details_width = scale.px(initial_details_width_design);

        let mut view = Self {
            state: Arc::clone(&initial_state),
            window_handle: window.window_handle(),
            _ui_model: ui_model,
            store,
            _poller: poller,
            _ui_model_subscription: ui_model_subscription,
            _activation_subscription: activation_subscription,
            _appearance_subscription: appearance_subscription,
            view_mode,
            theme_mode,
            theme: initial_theme,
            title_bar,
            sidebar_pane,
            main_pane,
            details_pane,
            repo_tabs_bar,
            action_bar,
            bottom_status_bar,
            tooltip_host,
            toast_host,
            popover_host,
            focused_mergetool_bootstrap,
            deferred_repo_bootstrap,
            startup_repo_bootstrap_pending,
            splash_backdrop_image: splash::load_splash_backdrop_image(),
            last_window_size: size(px(0.0), px(0.0)),
            ui_window_size_last_seen: size(px(0.0), px(0.0)),
            ui_settings_persist_seq: 0,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            diff_scroll_sync,
            ui_scale_percent: ui_scale.percent,
            open_repo_panel: false,
            open_repo_input,
            hover_resize_edge: None,
            sidebar_collapsed: false,
            details_collapsed: false,
            sidebar_width_design: initial_sidebar_width_design,
            details_width_design: initial_details_width_design,
            sidebar_width: initial_sidebar_width,
            details_width: initial_details_width,
            sidebar_render_width: initial_sidebar_width,
            details_render_width: initial_details_width,
            sidebar_width_anim_seq: 0,
            details_width_anim_seq: 0,
            sidebar_width_animating: false,
            details_width_animating: false,
            pane_resize: None,
            last_mouse_pos: point(px(0.0), px(0.0)),
            pending_pull_reconcile_prompt: None,
            pending_force_delete_branch_prompt: None,
            pending_force_remove_worktree_prompt: None,
            pending_submodule_trust_prompt: None,
            pending_worktree_branch_removals: HashMap::default(),
            startup_crash_report,
            #[cfg(target_os = "macos")]
            recent_repos_menu_fingerprint: ui_session.recent_repos.clone(),
            error_banner_input,
            auth_prompt_username_input,
            auth_prompt_secret_input,
            auth_prompt_key: None,
            active_context_menu_invoker: None,
        };

        view.set_theme(initial_theme, cx);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        view.maybe_auto_install_linux_desktop_integration(cx);

        view.drive_focused_mergetool_bootstrap();
        view.maybe_check_for_updates_on_startup(cx);

        crate::app::sync_gitcomet_window_state(
            cx,
            view.window_handle,
            cx.weak_entity(),
            view.view_mode,
            view.state
                .repos
                .iter()
                .map(|repo| repo.spec.workdir.clone())
                .collect(),
        );

        view
    }

    fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.title_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.sidebar_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.main_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.details_pane
            .update(cx, |pane, cx| pane.set_theme(theme, cx));
        self.repo_tabs_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.action_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.bottom_status_bar
            .update(cx, |bar, cx| bar.set_theme(theme, cx));
        self.tooltip_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.toast_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.popover_host
            .update(cx, |host, cx| host.set_theme(theme, cx));
        self.open_repo_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.error_banner_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.auth_prompt_username_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.auth_prompt_secret_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
    }

    fn notify_font_preferences_changed(&mut self, cx: &mut gpui::Context<Self>) {
        self.title_bar.update(cx, |_bar, cx| cx.notify());
        self.sidebar_pane.update(cx, |_pane, cx| cx.notify());
        self.main_pane
            .update(cx, |pane, cx| pane.invalidate_font_metrics(cx));
        self.details_pane.update(cx, |_pane, cx| cx.notify());
        self.repo_tabs_bar.update(cx, |_bar, cx| cx.notify());
        self.action_bar.update(cx, |_bar, cx| cx.notify());
        self.bottom_status_bar.update(cx, |_bar, cx| cx.notify());
        self.tooltip_host.update(cx, |_host, cx| cx.notify());
        self.toast_host.update(cx, |_host, cx| cx.notify());
        self.popover_host.update(cx, |_host, cx| cx.notify());
        self.open_repo_input.update(cx, |_input, cx| cx.notify());
        self.error_banner_input.update(cx, |_input, cx| cx.notify());
        self.auth_prompt_username_input
            .update(cx, |_input, cx| cx.notify());
        self.auth_prompt_secret_input
            .update(cx, |_input, cx| cx.notify());
        cx.notify();
    }

    fn ui_scale(&self) -> ui_scale::UiScale {
        ui_scale::UiScale::from_percent(self.ui_scale_percent)
    }

    fn sync_cached_pane_widths_from_design(&mut self) {
        let scale = self.ui_scale();
        self.sidebar_width = scale.px(self.sidebar_width_design);
        self.details_width = scale.px(self.details_width_design);
    }

    fn set_sidebar_width_from_pixels(&mut self, width: Pixels) {
        self.sidebar_width = width;
        self.sidebar_width_design = self.ui_scale().design_units_from_pixels(width);
    }

    fn set_details_width_from_pixels(&mut self, width: Pixels) {
        self.details_width = width;
        self.details_width_design = self.ui_scale().design_units_from_pixels(width);
    }

    fn scaled_px(&self, value: f32) -> Pixels {
        self.ui_scale().px(value)
    }

    fn pane_collapsed_width(&self) -> Pixels {
        self.scaled_px(PANE_COLLAPSED_PX)
    }

    fn main_min_width(&self) -> Pixels {
        self.scaled_px(MAIN_MIN_PX)
    }

    fn sidebar_min_width(&self) -> Pixels {
        self.scaled_px(SIDEBAR_MIN_PX)
    }

    fn details_min_width(&self) -> Pixels {
        self.scaled_px(DETAILS_MIN_PX)
    }

    fn pane_resize_handle_width(&self) -> Pixels {
        self.scaled_px(PANE_RESIZE_HANDLE_PX)
    }

    pub(crate) fn apply_ui_scale_percent(
        &mut self,
        percent: u32,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let percent = ui_scale::sanitize_percent(Some(percent));
        if self.ui_scale_percent == percent {
            return;
        }

        let previous_percent = self.ui_scale_percent;
        let scale = self.ui_scale();
        self.sidebar_width_design = scale.design_units_from_pixels(self.sidebar_width);
        self.details_width_design = scale.design_units_from_pixels(self.details_width);
        self.ui_scale_percent = percent;
        self.pane_resize = None;
        self.sidebar_width_anim_seq = self.sidebar_width_anim_seq.wrapping_add(1);
        self.details_width_anim_seq = self.details_width_anim_seq.wrapping_add(1);
        self.sidebar_width_animating = false;
        self.details_width_animating = false;

        ui_scale::apply_to_window(window, percent);
        crate::app::ensure_window_respects_min_size(
            window,
            crate::app::main_window_min_size_for_percent(percent),
        );

        self.last_window_size = window.viewport_size();
        self.ui_window_size_last_seen = self.last_window_size;
        self.sync_cached_pane_widths_from_design();

        let change_tracking_view = self.change_tracking_view;
        self.details_pane.update(cx, |pane, cx| {
            pane.apply_ui_scale_percent(previous_percent, percent, change_tracking_view, cx);
        });
        self.main_pane.update(cx, |pane, cx| {
            pane.apply_ui_scale_percent(previous_percent, percent, cx);
        });
        self.popover_host.update(cx, |_host, cx| {
            cx.notify();
        });

        self.clamp_pane_widths_to_window();
        self.notify_font_preferences_changed(cx);
        self.schedule_ui_settings_persist(cx);
    }

    fn set_theme_mode(
        &mut self,
        mode: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == mode {
            return;
        }

        self.theme_mode = mode.clone();
        self.set_theme(mode.resolve_theme(appearance), cx);
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        self.details_pane
            .update(cx, |pane, cx| pane.set_change_tracking_view(next, cx));
        self.popover_host
            .update(cx, |host, cx| host.sync_change_tracking_view(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_diff_scroll_sync(
        &mut self,
        next: DiffScrollSync,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_scroll_sync == next {
            return;
        }

        self.diff_scroll_sync = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_diff_scroll_sync(next, cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_column_preferences(show_graph, show_author, show_date, show_sha, cx);
        });
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn reset_history_column_widths(&mut self, cx: &mut gpui::Context<Self>) {
        self.main_pane
            .update(cx, |pane, cx| pane.reset_history_column_widths(cx));
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_history_tag_preferences(
        &mut self,
        show_tags: bool,
        tag_fetch_mode: gitcomet_state::model::GitLogTagFetchMode,
        cx: &mut gpui::Context<Self>,
    ) {
        let auto_fetch_tags_on_repo_activation = matches!(
            tag_fetch_mode,
            gitcomet_state::model::GitLogTagFetchMode::OnRepositoryActivation
        );
        self.main_pane.update(cx, |pane, cx| {
            pane.set_history_tag_preferences(show_tags, auto_fetch_tags_on_repo_activation, cx);
        });
        self.store.dispatch(Msg::SetGitLogSettings {
            show_history_tags: show_tags,
            tag_fetch_mode,
        });
        if show_tags
            && let Some(repo) = self.main_pane.read(cx).active_repo()
            && matches!(repo.tags, Loadable::NotLoaded | Loadable::Error(_))
        {
            self.store.dispatch(Msg::LoadTags { repo_id: repo.id });
        }
        self.schedule_ui_settings_persist(cx);
    }

    fn refresh_main_pane_after_panel_animation(&mut self, cx: &mut gpui::Context<Self>) {
        let main_pane = self.main_pane.clone();
        cx.defer(move |cx| {
            main_pane.update(cx, |pane, cx| {
                pane.sync_root_layout_snapshot(cx);
                cx.notify();
            });
        });
    }

    fn ease_out_cubic(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(3)
    }

    fn animate_sidebar_render_width_to(&mut self, target: Pixels, cx: &mut gpui::Context<Self>) {
        let start = self.sidebar_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.sidebar_width_anim_seq = self.sidebar_width_anim_seq.wrapping_add(1);
        let seq = self.sidebar_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.sidebar_render_width = target;
            self.sidebar_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.sidebar_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::ease_out_cubic(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.sidebar_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.sidebar_render_width != next_width {
                        this.sidebar_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.sidebar_render_width != px(target_f) {
                            this.sidebar_render_width = px(target_f);
                        }
                        this.sidebar_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    fn animate_details_render_width_to(&mut self, target: Pixels, cx: &mut gpui::Context<Self>) {
        let start = self.details_render_width;
        let start_f: f32 = start.into();
        let target_f: f32 = target.into();
        self.details_width_anim_seq = self.details_width_anim_seq.wrapping_add(1);
        let seq = self.details_width_anim_seq;
        if (start_f - target_f).abs() <= 0.5 {
            self.details_render_width = target;
            self.details_width_animating = false;
            return;
        }

        if !crate::ui_runtime::current().uses_pane_animations() {
            self.details_render_width = target;
            self.details_width_animating = false;
            self.refresh_main_pane_after_panel_animation(cx);
            cx.notify();
            return;
        }

        self.details_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                smol::Timer::after(Duration::from_millis(16)).await;

                let mut t =
                    started.elapsed().as_secs_f32() / duration.as_secs_f32().max(f32::EPSILON);
                if !t.is_finite() {
                    t = 1.0;
                }
                let t = t.clamp(0.0, 1.0);
                let eased = Self::ease_out_cubic(t);
                let mut done = t >= 1.0;

                let _ = view.update(cx, |this, cx| {
                    if this.details_width_anim_seq != seq {
                        done = true;
                        return;
                    }

                    let mut changed = false;
                    let next_width = px(start_f + (target_f - start_f) * eased);
                    if this.details_render_width != next_width {
                        this.details_render_width = next_width;
                        changed = true;
                    }
                    if t >= 1.0 {
                        if this.details_render_width != px(target_f) {
                            this.details_render_width = px(target_f);
                        }
                        this.details_width_animating = false;
                        this.refresh_main_pane_after_panel_animation(cx);
                        changed = true;
                    }
                    if changed {
                        cx.notify();
                    }
                });

                if done {
                    break;
                }
            },
        )
        .detach();
    }

    fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.sidebar_collapsed == collapsed {
            return;
        }

        self.sidebar_collapsed = collapsed;
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Sidebar,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.sidebar_width
        };
        self.animate_sidebar_render_width_to(target, cx);
        cx.notify();
    }

    fn set_details_collapsed(&mut self, collapsed: bool, cx: &mut gpui::Context<Self>) {
        if self.details_collapsed == collapsed {
            return;
        }

        self.details_collapsed = collapsed;
        if matches!(
            self.pane_resize,
            Some(PaneResizeState {
                handle: PaneResizeHandle::Details,
                ..
            })
        ) {
            self.pane_resize = None;
        }
        if !collapsed {
            self.clamp_pane_widths_to_window();
        }

        let target = if collapsed {
            self.pane_collapsed_width()
        } else {
            self.details_width
        };
        self.animate_details_render_width_to(target, cx);
        cx.notify();
    }

    fn pane_resize_handle(
        &self,
        theme: AppTheme,
        id: &'static str,
        handle: PaneResizeHandle,
        cx: &gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let collapsed = match handle {
            PaneResizeHandle::Sidebar => self.sidebar_collapsed,
            PaneResizeHandle::Details => self.details_collapsed,
        };
        if collapsed {
            return div().id(id).w(px(0.0)).h_full();
        }

        div()
            .id(id)
            .w(self.pane_resize_handle_width())
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(move |s| s.bg(with_alpha(theme.colors.hover, 0.65)))
            .active(move |s| s.bg(theme.colors.active))
            .child(div().w(px(1.0)).h_full().bg(theme.colors.border))
            .on_drag(handle, |_handle, _offset, _window, cx| {
                cx.new(|_cx| PaneResizeDragGhost)
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, e: &MouseDownEvent, _w, cx| {
                    cx.stop_propagation();
                    match handle {
                        PaneResizeHandle::Sidebar => {
                            this.sidebar_width_anim_seq =
                                this.sidebar_width_anim_seq.wrapping_add(1);
                            this.sidebar_width_animating = false;
                            this.sidebar_render_width = this.sidebar_width;
                        }
                        PaneResizeHandle::Details => {
                            this.details_width_anim_seq =
                                this.details_width_anim_seq.wrapping_add(1);
                            this.details_width_animating = false;
                            this.details_render_width = this.details_width;
                        }
                    }
                    this.pane_resize = Some(PaneResizeState::new(
                        handle,
                        e.position.x,
                        this.sidebar_width,
                        this.details_width,
                        this.last_window_size.width,
                        this.sidebar_collapsed,
                        this.details_collapsed,
                    ));
                    cx.notify();
                }),
            )
            .on_drag_move(cx.listener(
                move |this, e: &gpui::DragMoveEvent<PaneResizeHandle>, _w, cx| {
                    let Some(state) = this.pane_resize else {
                        return;
                    };
                    if state.handle != *e.drag(cx) {
                        return;
                    }

                    let total_w = this.last_window_size.width;
                    let next_width = next_pane_resize_drag_width(
                        &state,
                        e.event.position.x,
                        total_w,
                        this.sidebar_collapsed,
                        this.details_collapsed,
                    );
                    let mut changed = false;
                    match state.handle {
                        PaneResizeHandle::Sidebar => {
                            if this.sidebar_width != next_width {
                                this.set_sidebar_width_from_pixels(next_width);
                                changed = true;
                            }
                            if this.sidebar_render_width != next_width {
                                this.sidebar_render_width = next_width;
                                changed = true;
                            }
                        }
                        PaneResizeHandle::Details => {
                            if this.details_width != next_width {
                                this.set_details_width_from_pixels(next_width);
                                changed = true;
                            }
                            if this.details_render_width != next_width {
                                this.details_render_width = next_width;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.pane_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _e, _w, cx| {
                    if this.pane_resize.take().is_some() {
                        this.schedule_ui_settings_persist(cx);
                        cx.notify();
                    }
                }),
            )
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn drive_focused_mergetool_bootstrap(&mut self) {
        if !self.state.git_runtime.is_available() {
            return;
        }

        let Some(bootstrap) = self.focused_mergetool_bootstrap.as_ref() else {
            return;
        };
        let Some(action) = focused_mergetool_bootstrap_action(&self.state, bootstrap) else {
            return;
        };

        match action {
            FocusedMergetoolBootstrapAction::OpenRepo(path) => {
                self.store.dispatch(Msg::OpenRepo(path))
            }
            FocusedMergetoolBootstrapAction::SetActiveRepo(repo_id) => {
                self.store.dispatch(Msg::SetActiveRepo { repo_id });
            }
            FocusedMergetoolBootstrapAction::SelectConflictDiff { repo_id, path } => {
                self.store
                    .dispatch(Msg::SelectConflictDiff { repo_id, path });
            }
            FocusedMergetoolBootstrapAction::LoadConflictFile { repo_id, path } => {
                self.store.dispatch(Msg::LoadConflictFile {
                    repo_id,
                    path,
                    mode: gitcomet_state::model::ConflictFileLoadMode::CurrentOnly,
                });
            }
            FocusedMergetoolBootstrapAction::Complete => {
                self.focused_mergetool_bootstrap = None;
            }
        }
    }

    fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    #[cfg(test)]
    fn remote_rows(repo: &RepoState) -> Vec<RemoteRow> {
        let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();

        if let Loadable::Ready(remote_branches) = &repo.remote_branches {
            for branch in remote_branches.iter() {
                grouped
                    .entry(branch.remote.clone())
                    .or_default()
                    .push(branch.name.clone());
            }
        }

        if grouped.is_empty()
            && let Loadable::Ready(remotes) = &repo.remotes
        {
            for remote in remotes.iter() {
                grouped.entry(remote.name.clone()).or_default();
            }
        }

        let mut rows = Vec::new();
        for (remote, mut branches) in grouped {
            branches.sort_unstable();
            branches.dedup();
            rows.push(RemoteRow::Header(remote.clone()));
            for name in branches {
                rows.push(RemoteRow::Branch {
                    remote: remote.clone(),
                    name,
                });
            }
        }

        rows
    }

    fn show_error_banner(&mut self, repo_id: Option<RepoId>, message: String) {
        if message.trim().is_empty() {
            return;
        }

        if self
            .state
            .banner_error
            .as_ref()
            .is_some_and(|banner| banner.repo_id == repo_id && banner.message == message)
        {
            return;
        }

        self.store
            .dispatch(Msg::ShowBannerError { repo_id, message });
    }

    fn split_error_banner_message(err_text: &str) -> (Option<SharedString>, SharedString) {
        let lines: Vec<&str> = err_text.lines().collect();
        let Some(cmd_start) = lines.iter().position(|line| line.starts_with("    git ")) else {
            return (None, err_text.to_string().into());
        };

        let mut cmd_end = cmd_start;
        while cmd_end < lines.len() && lines[cmd_end].starts_with("    ") {
            cmd_end += 1;
        }

        let command = lines[cmd_start..cmd_end]
            .iter()
            .map(|line| line.strip_prefix("    ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");

        let mut body_lines: Vec<String> = Vec::with_capacity(lines.len());
        for line in &lines[..cmd_start] {
            body_lines.push((*line).to_string());
        }
        for line in &lines[cmd_end..] {
            body_lines.push(line.strip_prefix("    ").unwrap_or(line).to_string());
        }

        let mut collapsed: Vec<String> = Vec::with_capacity(body_lines.len());
        let mut prev_blank = false;
        for line in body_lines {
            let blank = line.trim().is_empty();
            if blank && prev_blank {
                continue;
            }
            collapsed.push(line);
            prev_blank = blank;
        }

        (Some(command.into()), collapsed.join("\n").into())
    }

    fn should_show_error_banner_overflow_hint(err_text: &str) -> bool {
        err_text.lines().count() > ERROR_BANNER_OVERFLOW_HINT_MIN_LINES
            || err_text.len() > ERROR_BANNER_OVERFLOW_HINT_MIN_CHARS
    }

    fn should_render_generic_error_banner(auth_prompt_active: bool) -> bool {
        !auth_prompt_active
    }

    fn auth_prompt_banner_colors(theme: AppTheme) -> (gpui::Rgba, gpui::Rgba) {
        (
            with_alpha(theme.colors.accent, 0.15),
            with_alpha(theme.colors.accent, 0.3),
        )
    }

    fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error) {
            self.show_error_banner(self.active_repo_id(), message);
            return;
        }
        self.toast_host
            .update(cx, |host, cx| host.push_toast(kind, message, cx));
    }

    #[cfg_attr(test, allow(dead_code))]
    fn push_toast_with_link(
        &mut self,
        kind: components::ToastKind,
        message: String,
        link_url: String,
        link_label: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error) {
            self.show_error_banner(self.active_repo_id(), message);
            return;
        }
        self.toast_host.update(cx, |host, cx| {
            host.push_toast_with_link(kind, message, link_url, link_label, cx)
        });
    }

    fn open_external_url(&mut self, url: &str) -> Result<(), std::io::Error> {
        platform_open::open_url(url)
    }

    fn defer_text_input_main_pane_action<F>(&self, cx: &mut gpui::Context<Self>, action: F)
    where
        F: FnOnce(&mut MainPaneView, &mut Window, &mut gpui::Context<MainPaneView>) -> bool
            + 'static,
    {
        let main_pane = self.main_pane.clone();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                main_pane.update(cx, |pane, cx| {
                    if action(pane, window, cx) {
                        cx.notify();
                    }
                });
            });
        });
    }

    fn defer_text_input_adjacent_diff_file_navigation(
        &self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) {
        self.defer_text_input_main_pane_action(cx, move |pane, window, cx| {
            let Some(repo_id) = pane.active_repo_id() else {
                return false;
            };
            pane.try_select_adjacent_diff_file_preserving_focus(repo_id, direction, window, cx)
        });
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn tooltip_text_for_test(&self, app: &App) -> Option<SharedString> {
        self.tooltip_host.read(app).tooltip_text_for_test()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn open_repo_panel_visible_for_test(&self) -> bool {
        self.open_repo_panel
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn show_timezone_for_test(&self) -> bool {
        self.show_timezone
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn change_tracking_view_for_test(&self) -> ChangeTrackingView {
        self.change_tracking_view
    }

    fn resume_after_git_runtime_recovery(&mut self) {
        if let Some(bootstrap) = self.deferred_repo_bootstrap.take() {
            match bootstrap {
                DeferredRepoBootstrap::RestoreSession {
                    open_repos,
                    active_repo,
                } => {
                    self.startup_repo_bootstrap_pending = true;
                    self.store.dispatch(Msg::RestoreSession {
                        open_repos,
                        active_repo,
                    });
                }
                DeferredRepoBootstrap::OpenRepo(path) => {
                    self.startup_repo_bootstrap_pending = true;
                    self.store.dispatch(Msg::OpenRepo(path));
                }
            }
            return;
        }

        if !self.state.repos.is_empty() {
            let repo_ids: Vec<_> = self.state.repos.iter().map(|repo| repo.id).collect();
            for repo_id in repo_ids {
                self.store.dispatch(Msg::ReloadRepo { repo_id });
            }
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(in crate::view) fn diff_scroll_sync_for_test(&self) -> DiffScrollSync {
        self.diff_scroll_sync
    }
}

impl Render for GitCometView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let font_preferences = crate::font_preferences::current(cx);
        debug_assert!(matches!(
            self.view_mode,
            GitCometViewMode::Normal | GitCometViewMode::FocusedMergetool
        ));
        self.last_window_size = window.viewport_size();
        self.clamp_pane_widths_to_window();
        if self.last_window_size != self.ui_window_size_last_seen {
            self.ui_window_size_last_seen = self.last_window_size;
            self.schedule_ui_settings_persist(cx);
        }

        if let Some(repo_id) = self.pending_pull_reconcile_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::PullReconcilePrompt { repo_id },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some((repo_id, name)) = self.pending_force_delete_branch_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::ForceDeleteBranchConfirm { repo_id, name },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some((repo_id, path, branch)) = self.pending_force_remove_worktree_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::ForceRemoveWorktreeConfirm {
                    repo_id,
                    path,
                    branch,
                },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        if let Some(prompt) = self.pending_submodule_trust_prompt.take()
            && self.active_repo_id() == Some(prompt.repo_id)
        {
            self.open_popover_at(
                PopoverKind::submodule(prompt.repo_id, SubmodulePopoverKind::TrustConfirm),
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        let decorations = window.window_decorations();
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (
                Some(tiling),
                chrome::client_side_decoration_inset(self.ui_scale_percent),
            ),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);

        let center_content = self.center_content(window, cx);
        let font_features = crate::font_preferences::current_font_features(cx);
        let show_custom_window_chrome =
            crate::linux_gui_env::LinuxGuiEnvironment::should_render_custom_window_chrome(
                decorations,
            );

        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .font(gpui::Font {
                family: crate::font_preferences::applied_ui_font_family(
                    &font_preferences.ui_font_family,
                )
                .into(),
                features: font_features,
                fallbacks: None,
                weight: gpui::FontWeight::default(),
                style: gpui::FontStyle::default(),
            })
            .text_color(theme.colors.text);

        if show_custom_window_chrome {
            body = body.child(stable_cached_fixed_height_view(
                self.title_bar.clone(),
                chrome::title_bar_height(self.ui_scale_percent),
            ));
        }

        body = body.child(center_content);

        if let Some(report) = self.startup_crash_report.clone()
            && self.view_mode == GitCometViewMode::Normal
        {
            let issue_url = report.issue_url.clone();
            let summary = report.summary.clone();

            let report_button =
                components::Button::new("startup_crash_report_open", "Report Issue")
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        match this.open_external_url(&issue_url) {
                            Ok(()) => {
                                this.push_toast(
                                    components::ToastKind::Success,
                                    "Opened crash report page in your browser.".to_string(),
                                    cx,
                                );
                                this.startup_crash_report = None;
                            }
                            Err(err) => {
                                this.push_toast(
                                    components::ToastKind::Error,
                                    format!("Failed to open browser: {err}"),
                                    cx,
                                );
                            }
                        }
                        cx.notify();
                    });

            let dismiss_button = components::Button::new("startup_crash_report_dismiss", "Dismiss")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.startup_crash_report = None;
                    cx.notify();
                });

            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .bg(with_alpha(theme.colors.warning, 0.13))
                    .border_1()
                    .border_color(with_alpha(theme.colors.warning, 0.30))
                    .rounded(px(theme.radii.panel))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child("GitComet recovered from program crash"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.colors.text_muted)
                                    .child(
                                        "Would you like to contribute by reporting issue to GitComet GitHub repository?",
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(format!("Summary: {summary}")),
                            )
                            .child(
                                div()
                                    .pt_1()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(report_button)
                                    .child(dismiss_button),
                            ),
                    ),
            );
        }

        if let Some(prompt) = self.state.auth_prompt.clone() {
            let prompt_key = format!("{:?}:{:?}", prompt.kind, prompt.operation);
            if self.auth_prompt_key.as_ref() != Some(&prompt_key) {
                self.auth_prompt_key = Some(prompt_key);
                self.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("", cx));
                self.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("", cx));
            }

            self.auth_prompt_username_input
                .update(cx, |input, cx| input.set_theme(theme, cx));
            let is_host_verification = prompt.kind == AuthPromptKind::HostVerification;
            self.auth_prompt_secret_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_masked(!is_host_verification, cx);
            });

            let requires_username = prompt.kind == AuthPromptKind::UsernamePassword;
            let title = match prompt.kind {
                AuthPromptKind::UsernamePassword => "Repository authentication required",
                AuthPromptKind::Passphrase => "Passphrase required",
                AuthPromptKind::HostVerification => "Host authenticity confirmation required",
            };
            let subtitle = match prompt.kind {
                AuthPromptKind::UsernamePassword => {
                    "Enter username and password, then confirm to retry."
                }
                AuthPromptKind::Passphrase => "Enter your key passphrase, then confirm to retry.",
                AuthPromptKind::HostVerification => {
                    "Enter `yes` to trust this host key, or paste the shown fingerprint."
                }
            };
            let secret_required_message = match prompt.kind {
                AuthPromptKind::UsernamePassword => "Password is required.",
                AuthPromptKind::Passphrase => "Passphrase is required.",
                AuthPromptKind::HostVerification => {
                    "Confirmation is required (`yes` or fingerprint)."
                }
            };

            let confirm_button = components::Button::new("auth_prompt_confirm", "Confirm")
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    let username = this
                        .auth_prompt_username_input
                        .read(cx)
                        .text()
                        .trim()
                        .to_string();
                    let secret = this.auth_prompt_secret_input.read(cx).text().to_string();

                    if requires_username && username.is_empty() {
                        this.push_toast(
                            components::ToastKind::Error,
                            "Username is required.".to_string(),
                            cx,
                        );
                        return;
                    }
                    if secret.trim().is_empty() {
                        this.push_toast(
                            components::ToastKind::Error,
                            secret_required_message.to_string(),
                            cx,
                        );
                        return;
                    }

                    this.store.dispatch(Msg::SubmitAuthPrompt {
                        username: requires_username.then_some(username),
                        secret,
                    });
                    cx.notify();
                });

            let cancel_button = components::Button::new("auth_prompt_cancel", "Cancel")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.store.dispatch(Msg::CancelAuthPrompt);
                    cx.notify();
                });

            let prompt_form = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().font_weight(FontWeight::BOLD).child(title))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.colors.text_muted)
                        .child(subtitle),
                )
                .when(requires_username, |this| {
                    this.child(self.auth_prompt_username_input.clone())
                })
                .child(self.auth_prompt_secret_input.clone())
                .when(is_host_verification, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("Use Cancel if you do not trust this host."),
                    )
                })
                .when(!prompt.reason.trim().is_empty(), |this| {
                    this.child(
                        div()
                            .id("auth_prompt_reason_scroll")
                            .max_h(px(96.0))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(prompt.reason.clone()),
                            ),
                    )
                })
                .child(
                    div()
                        .pt_1()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(confirm_button)
                        .child(cancel_button),
                );

            let (prompt_bg, prompt_border) = Self::auth_prompt_banner_colors(theme);
            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .bg(prompt_bg)
                    .border_1()
                    .border_color(prompt_border)
                    .rounded(px(theme.radii.panel))
                    .child(prompt_form),
            );
        } else {
            self.auth_prompt_key = None;
        }

        let banner_error =
            if Self::should_render_generic_error_banner(self.state.auth_prompt.is_some()) {
                self.state
                    .banner_error
                    .as_ref()
                    .map(|banner| banner.message.clone())
            } else {
                None
            };
        if let Some(err_text) = banner_error {
            let (error_command, display_error) =
                Self::split_error_banner_message(err_text.as_ref());
            let show_overflow_hint =
                Self::should_show_error_banner_overflow_hint(err_text.as_ref());
            self.error_banner_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text(display_error.clone(), cx);
                input.set_read_only(true, cx);
            });

            let dismiss = components::Button::new("repo_error_banner_close", "")
                .start_slot(svg_icon(
                    "icons/generic_close.svg",
                    theme.colors.text_muted,
                    px(12.0),
                ))
                .style(components::ButtonStyle::Transparent)
                .on_click(theme, cx, move |this, _e, _w, _cx| {
                    this.store.dispatch(Msg::DismissBannerError);
                });

            let command_block = error_command.as_ref().map(|command| {
                div()
                    .id("repo_error_banner_command")
                    .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                    .bg(with_alpha(
                        theme.colors.window_bg,
                        if theme.is_dark { 0.28 } else { 0.75 },
                    ))
                    .rounded(px(theme.radii.row))
                    .px_2()
                    .py_1()
                    .child(command.clone())
            });

            body = body.child(
                div()
                    .relative()
                    .px_2()
                    .py_1()
                    .pr(px(40.0))
                    .bg(with_alpha(theme.colors.danger, 0.15))
                    .border_1()
                    .border_color(with_alpha(theme.colors.danger, 0.3))
                    .rounded(px(theme.radii.panel))
                    .child(
                        div()
                            .id("repo_error_banner_scroll")
                            .max_h(px(140.0))
                            .overflow_y_scroll()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .when_some(command_block, |this, command_block| {
                                        this.child(command_block)
                                    })
                                    .child(self.error_banner_input.clone()),
                            ),
                    )
                    .when(show_overflow_hint, |this| {
                        this.child(
                            div()
                                .mt_1()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("Scroll for full output"),
                        )
                    })
                    .child(div().absolute().top(px(6.0)).right(px(6.0)).child(dismiss)),
            );
        }

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.text);
        root = root.relative();
        root = root.child(UiScaleScrollCapture { view: cx.entity() });
        root = root
            .on_action(cx.listener(|this, _: &TextInputCommitSubmit, window, cx| {
                let handled = this.details_pane.update(cx, |pane, cx| {
                    pane.handle_commit_submit_shortcut(window, cx)
                });
                if handled {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &TextInputDiffPrevFile, _window, cx| {
                this.defer_text_input_adjacent_diff_file_navigation(-1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &TextInputDiffNextFile, _window, cx| {
                this.defer_text_input_adjacent_diff_file_navigation(1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(
                |this, _: &TextInputDiffPrevSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_prev_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                },
            ))
            .on_action(cx.listener(
                |this, _: &TextInputDiffNextSearchMatchOrChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_next_search_match_or_diff_change(cx)
                    });
                    cx.stop_propagation();
                },
            ))
            .on_action(
                cx.listener(|this, _: &TextInputDiffPrevChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_prev_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            )
            .on_action(
                cx.listener(|this, _: &TextInputDiffNextChange, _window, cx| {
                    this.defer_text_input_main_pane_action(cx, |pane, _window, cx| {
                        pane.navigate_next_diff_change(cx)
                    });
                    cx.stop_propagation();
                }),
            );

        root = root.on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, window, cx| {
            this.last_mouse_pos = e.position;
            this.tooltip_host
                .update(cx, |tooltip, cx| tooltip.on_mouse_moved(e.position, cx));

            let Decorations::Client { tiling } = window.window_decorations() else {
                if this.hover_resize_edge.is_some() {
                    this.hover_resize_edge = None;
                    cx.notify();
                }
                return;
            };

            let size = window.viewport_size();
            let next = resize_edge(
                e.position,
                chrome::client_side_decoration_inset(this.ui_scale_percent),
                size,
                tiling,
            );
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));
        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = window.window_decorations() else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = resize_edge(
                        e.position,
                        chrome::client_side_decoration_inset(this.ui_scale_percent),
                        size,
                        tiling,
                    );
                    let Some(edge) = edge else {
                        return;
                    };

                    cx.stop_propagation();
                    window.start_window_resize(edge);
                }),
            );
        } else if self.hover_resize_edge.is_some() {
            self.hover_resize_edge = None;
        }

        root = root.child(chrome::window_frame(
            theme,
            decorations,
            body.into_any_element(),
            self.ui_scale_percent,
        ));

        root = root.child(stable_overlay_view(self.toast_host.clone()));

        root = root.child(stable_overlay_view(self.popover_host.clone()));

        root = root.child(stable_overlay_view(self.tooltip_host.clone()));

        if crate::startup_probe::is_enabled() {
            root = root.on_children_prepainted(|_children_bounds, window, _cx| {
                if crate::startup_probe::mark_first_paint() {
                    window.on_next_frame(|_window, cx| {
                        crate::startup_probe::mark_first_interactive();
                        if crate::startup_probe::should_exit_after_first_interactive() {
                            cx.quit();
                        }
                    });
                }
            });
        }

        root
    }
}

#[cfg(test)]
mod tests;
