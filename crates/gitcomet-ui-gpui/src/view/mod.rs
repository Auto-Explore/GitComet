use crate::theme::AppTheme;
use gitcomet_core::diff::{AnnotatedDiffLine, annotate_unified};
use gitcomet_core::domain::{
    Branch, Commit, CommitId, DiffArea, DiffTarget, FileStatus, FileStatusKind, RepoStatus, Tag,
    UpstreamDivergence,
};
use gitcomet_core::file_diff::FileDiffRow;
use gitcomet_core::services::{PullMode, RemoteUrlKind, ResetMode};
use gitcomet_state::model::{
    AppNotificationKind, AppState, AuthPromptKind, CloneOpState, CloneOpStatus, DiagnosticKind,
    Loadable, RepoId, RepoState,
};
use gitcomet_state::msg::{Msg, RepoExternalChange, StoreEvent};
use gitcomet_state::session;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Corner, CursorStyle, Decorations,
    Element, ElementId, Entity, FocusHandle, FontWeight, GlobalElementId, InspectorElementId,
    IsZero, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    Render, ResizeEdge, ScrollHandle, ShapedLine, SharedString, Size, Style, TextRun, Tiling,
    Timer, UniformListScrollHandle, WeakEntity, Window, WindowControlArea, anchored, div, fill,
    point, px, relative, size, uniform_list,
};
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;
use std::hash::Hash;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicI32;
use std::time::{Duration, Instant};

mod app_model;
mod branch_sidebar;
mod caches;
mod chrome;
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
mod icons;
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
mod state_apply;
mod toast_host;
mod tooltip;
mod tooltip_host;
mod update_check;
mod word_diff;

use app_model::AppUiModel;
use branch_sidebar::{BranchSection, BranchSidebarRow};
use caches::{
    BranchSidebarCache, HistoryCache, HistoryCacheRequest, HistoryCommitRowVm,
    HistoryStashIdsCache, HistoryWorktreeSummaryCache,
};
use chrome::{
    CLIENT_SIDE_DECORATION_INSET, TitleBarView, cursor_style_for_resize_edge, resize_edge,
};
use conflict_resolver::{ConflictPickSide, ConflictResolverViewMode};
#[cfg(test)]
use date_time::format_datetime_utc;
use date_time::{DateTimeFormat, Timezone, format_datetime};
use diff_preview::{build_deleted_file_preview_from_diff, build_new_file_preview_from_diff};
use patch_split::build_patch_split_rows;
use poller::Poller;
use word_diff::capped_word_diff_ranges;

use diff_text_model::{CachedDiffStyledText, CachedDiffTextSegment, SyntaxTokenKind};
use diff_text_selection::{DiffTextSelectionOverlay, DiffTextSelectionTracker};
use diff_utils::{
    build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard,
    compute_diff_file_for_src_ix, compute_diff_file_stats,
    context_menu_selection_range_from_diff_text, diff_content_text, image_format_for_path,
    parse_diff_git_header_path, parse_unified_hunk_header_for_display, rasterize_svg_preview_image,
    scrollbar_markers_from_flags,
};
use mod_helpers::*;
pub use mod_helpers::{
    FocusedMergetoolLabels, FocusedMergetoolViewConfig, GitCometView, GitCometViewConfig,
    GitCometViewMode, StartupCrashReport,
};
use panels::{ActionBarView, PopoverHost, RepoTabsBarView};
use panes::{DetailsPaneInit, DetailsPaneView, HistoryView, MainPaneView, SidebarPaneView};
pub(crate) use settings_window::open_settings_window;
use toast_host::ToastHost;
use tooltip_host::TooltipHost;

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

#[cfg(target_os = "windows")]
pub(crate) const UI_MONOSPACE_FONT_FAMILY: &str = "Consolas";
#[cfg(target_os = "macos")]
pub(crate) const UI_MONOSPACE_FONT_FAMILY: &str = "Menlo";
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub(crate) const UI_MONOSPACE_FONT_FAMILY: &str = "DejaVu Sans Mono";
#[cfg(not(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd"
)))]
pub(crate) const UI_MONOSPACE_FONT_FAMILY: &str = "monospace";

impl GitCometView {
    #[cfg(test)]
    pub(in crate::view) fn disable_poller_for_tests(&mut self) {
        self._poller = Poller::disabled();
    }

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
        });
    }

    #[cfg(test)]
    pub fn new(
        store: AppStore,
        events: smol::channel::Receiver<StoreEvent>,
        initial_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self::new_with_config(
            store,
            events,
            GitCometViewConfig::normal(initial_path, None),
            window,
            cx,
        )
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
        if view_mode == GitCometViewMode::Normal
            && let Some(path) = initial_path.as_ref()
        {
            if !ui_session.open_repos.iter().any(|p| p == path) {
                ui_session.open_repos.push(path.clone());
            }
            ui_session.active_repo = Some(path.clone());
        }

        let restored_sidebar_width = ui_session.sidebar_width;
        let restored_details_width = ui_session.details_width;
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
        let restored_change_tracking_height = ui_session.change_tracking_height;
        let restored_untracked_height = ui_session.untracked_height;

        let history_show_author = ui_session.history_show_author.unwrap_or(true);
        let history_show_date = ui_session.history_show_date.unwrap_or(true);
        let history_show_sha = ui_session.history_show_sha.unwrap_or(false);

        // Only auto-restore/open on startup if the store hasn't already been preloaded.
        // This avoids re-opening repos (and changing RepoIds) when the UI is attached to an
        // already-initialized store (notably in `gpui::test` setup).
        let store_preloaded = !store.snapshot().repos.is_empty();
        let should_auto_restore = if view_mode == GitCometViewMode::FocusedMergetool {
            false
        } else {
            #[cfg(test)]
            {
                false
            }
            #[cfg(not(test))]
            {
                !store_preloaded
            }
        };

        if should_auto_restore {
            if !ui_session.open_repos.is_empty() {
                store.dispatch(Msg::RestoreSession {
                    open_repos: ui_session.open_repos,
                    active_repo: ui_session.active_repo,
                });
            } else if let Ok(path) = std::env::current_dir() {
                store.dispatch(Msg::OpenRepo(path));
            }
        } else if store_preloaded {
            if let Some(path) = initial_path.as_ref() {
                store.dispatch(Msg::OpenRepo(path.clone()));
            }
        } else if let Some(path) = initial_path.as_ref() {
            store.dispatch(Msg::OpenRepo(path.clone()));
        }

        let initial_state = store.snapshot();
        let ui_model = cx.new(|_cx| AppUiModel::new(Arc::clone(&initial_state)));

        let ui_model_subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let should_notify = this.apply_state_snapshot(next, cx);
            if should_notify {
                cx.notify();
            }
        });

        let weak_view = cx.weak_entity();
        let poller = Poller::start(Arc::clone(&store), events, ui_model.downgrade(), window, cx);

        let title_bar = cx.new(|_cx| TitleBarView::new(initial_theme, weak_view.clone()));
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

        let sidebar_pane = cx.new(|cx| {
            SidebarPaneView::new(
                Arc::clone(&store),
                ui_model.clone(),
                initial_theme,
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
                history_show_author,
                history_show_date,
                history_show_sha,
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
                theme_mode,
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
                    if this.theme_mode != ThemeMode::Automatic {
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

        let initial_sidebar_width = restored_sidebar_width
            .map(|w| px(w as f32))
            .unwrap_or(px(280.0))
            .max(px(SIDEBAR_MIN_PX));
        let initial_details_width = restored_details_width
            .map(|w| px(w as f32))
            .unwrap_or(px(420.0))
            .max(px(DETAILS_MIN_PX));

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
            tooltip_host,
            toast_host,
            popover_host,
            focused_mergetool_bootstrap,
            last_window_size: size(px(0.0), px(0.0)),
            ui_window_size_last_seen: size(px(0.0), px(0.0)),
            ui_settings_persist_seq: 0,
            date_time_format,
            timezone,
            show_timezone,
            change_tracking_view,
            open_repo_panel: false,
            open_repo_input,
            hover_resize_edge: None,
            sidebar_collapsed: false,
            details_collapsed: false,
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

    fn set_theme_mode(
        &mut self,
        mode: ThemeMode,
        appearance: gpui::WindowAppearance,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.theme_mode == mode {
            return;
        }

        self.theme_mode = mode;
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

    fn visible_pane_resize_handles_width(&self) -> Pixels {
        let sidebar_handle = if self.sidebar_collapsed {
            px(0.0)
        } else {
            px(PANE_RESIZE_HANDLE_PX)
        };
        let details_handle = if self.details_collapsed {
            px(0.0)
        } else {
            px(PANE_RESIZE_HANDLE_PX)
        };
        sidebar_handle + details_handle
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

        self.sidebar_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                Timer::after(Duration::from_millis(16)).await;

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

        self.details_width_animating = true;
        let started = Instant::now();
        let duration = Duration::from_millis(PANE_COLLAPSE_ANIM_MS);

        cx.spawn(
            async move |view: WeakEntity<GitCometView>, cx: &mut gpui::AsyncApp| loop {
                Timer::after(Duration::from_millis(16)).await;

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
            px(PANE_COLLAPSED_PX)
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
            px(PANE_COLLAPSED_PX)
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
            .w(px(PANE_RESIZE_HANDLE_PX))
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
                    this.pane_resize = Some(PaneResizeState {
                        handle,
                        start_x: e.position.x,
                        start_sidebar: this.sidebar_width,
                        start_details: this.details_width,
                    });
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
                    let handles_w = this.visible_pane_resize_handles_width();
                    let main_min = px(MAIN_MIN_PX);
                    let sidebar_min = px(SIDEBAR_MIN_PX);
                    let details_min = px(DETAILS_MIN_PX);
                    let collapsed_w = px(PANE_COLLAPSED_PX);

                    let dx = e.event.position.x - state.start_x;
                    let mut changed = false;
                    match state.handle {
                        PaneResizeHandle::Sidebar => {
                            let details_w = if this.details_collapsed {
                                collapsed_w
                            } else {
                                state.start_details
                            };
                            let max_sidebar =
                                (total_w - details_w - main_min - handles_w).max(sidebar_min);
                            let next_sidebar =
                                (state.start_sidebar + dx).max(sidebar_min).min(max_sidebar);
                            if this.sidebar_width != next_sidebar {
                                this.sidebar_width = next_sidebar;
                                changed = true;
                            }
                            if this.sidebar_render_width != next_sidebar {
                                this.sidebar_render_width = next_sidebar;
                                changed = true;
                            }
                        }
                        PaneResizeHandle::Details => {
                            let sidebar_w = if this.sidebar_collapsed {
                                collapsed_w
                            } else {
                                state.start_sidebar
                            };
                            let max_details =
                                (total_w - sidebar_w - main_min - handles_w).max(details_min);
                            let next_details =
                                (state.start_details - dx).max(details_min).min(max_details);
                            if this.details_width != next_details {
                                this.details_width = next_details;
                                changed = true;
                            }
                            if this.details_render_width != next_details {
                                this.details_render_width = next_details;
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

    #[cfg(not(test))]
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

    #[cfg(test)]
    pub(crate) fn is_popover_open(&self, app: &App) -> bool {
        self.popover_host.read(app).is_open()
    }

    #[cfg(test)]
    pub(crate) fn tooltip_text_for_test(&self, app: &App) -> Option<SharedString> {
        self.tooltip_host.read(app).tooltip_text_for_test()
    }

    #[cfg(test)]
    pub(crate) fn show_timezone_for_test(&self) -> bool {
        self.show_timezone
    }

    #[cfg(test)]
    pub(in crate::view) fn change_tracking_view_for_test(&self) -> ChangeTrackingView {
        self.change_tracking_view
    }
}

impl Render for GitCometView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
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

        if let Some((repo_id, path)) = self.pending_force_remove_worktree_prompt.take()
            && self.active_repo_id() == Some(repo_id)
        {
            self.open_popover_at(
                PopoverKind::ForceRemoveWorktreeConfirm { repo_id, path },
                self.last_mouse_pos,
                window,
                cx,
            );
        }

        let decorations = window.window_decorations();
        let (tiling, client_inset) = match decorations {
            Decorations::Client { tiling } => (Some(tiling), CLIENT_SIDE_DECORATION_INSET),
            Decorations::Server => (None, px(0.0)),
        };
        window.set_client_inset(client_inset);

        let cursor = self
            .hover_resize_edge
            .map(cursor_style_for_resize_edge)
            .unwrap_or(CursorStyle::Arrow);

        let center_content = if renders_full_chrome(self.view_mode) {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(self.repo_tabs_bar.clone())
                .child(self.open_repo_panel(cx))
                .child(self.action_bar.clone())
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .child(
                            div()
                                .id("sidebar_pane")
                                .relative()
                                .w(self.sidebar_render_width)
                                .min_h(px(0.0))
                                .bg(theme.colors.surface_bg)
                                .when(self.sidebar_collapsed, |d| {
                                    d.border_r_1().border_color(theme.colors.border)
                                })
                                .when(!self.sidebar_collapsed, |d| {
                                    d.child(self.sidebar_pane.clone())
                                })
                                .child(
                                    div().absolute().bottom(px(6.0)).right(px(6.0)).child(
                                        components::Button::new("sidebar_toggle", "")
                                            .start_slot(svg_icon(
                                                if self.sidebar_collapsed {
                                                    "icons/arrow_right.svg"
                                                } else {
                                                    "icons/arrow_left.svg"
                                                },
                                                theme.colors.text_muted,
                                                px(12.0),
                                            ))
                                            .style(components::ButtonStyle::Transparent)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.set_sidebar_collapsed(
                                                    !this.sidebar_collapsed,
                                                    cx,
                                                );
                                            }),
                                    ),
                                ),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_sidebar",
                            PaneResizeHandle::Sidebar,
                            cx,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .min_h(px(0.0))
                                .child(self.main_pane.clone()),
                        )
                        .child(self.pane_resize_handle(
                            theme,
                            "pane_resize_details",
                            PaneResizeHandle::Details,
                            cx,
                        ))
                        .child(
                            div()
                                .id("details_pane")
                                .relative()
                                .w(self.details_render_width)
                                .min_h(px(0.0))
                                .flex()
                                .flex_col()
                                .when(self.details_collapsed, |d| {
                                    d.border_l_1().border_color(theme.colors.border)
                                })
                                .when(!self.details_collapsed, |d| {
                                    d.child(
                                        div()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .child(self.details_pane.clone()),
                                    )
                                })
                                .child(
                                    div().absolute().bottom(px(6.0)).left(px(6.0)).child(
                                        components::Button::new("details_toggle", "")
                                            .start_slot(svg_icon(
                                                if self.details_collapsed {
                                                    "icons/arrow_left.svg"
                                                } else {
                                                    "icons/arrow_right.svg"
                                                },
                                                theme.colors.text_muted,
                                                px(12.0),
                                            ))
                                            .style(components::ButtonStyle::Transparent)
                                            .on_click(theme, cx, |this, _e, _w, cx| {
                                                this.set_details_collapsed(
                                                    !this.details_collapsed,
                                                    cx,
                                                );
                                            }),
                                    ),
                                ),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(self.main_pane.clone()),
                )
                .into_any_element()
        };

        let mut body = div()
            .flex()
            .flex_col()
            .size_full()
            .text_color(theme.colors.text)
            .child(self.title_bar.clone())
            .child(center_content);

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
                    .font_family("monospace")
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
                    .child(div().absolute().top(px(6.0)).right(px(6.0)).child(dismiss)),
            );
        }

        let mut root = div()
            .size_full()
            .cursor(cursor)
            .text_color(theme.colors.text);
        root = root.relative();

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
            let next = resize_edge(e.position, CLIENT_SIDE_DECORATION_INSET, size, tiling);
            if next != this.hover_resize_edge {
                this.hover_resize_edge = next;
                cx.notify();
            }
        }));
        if tiling.is_some() {
            root = root.on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, e: &MouseDownEvent, window, cx| {
                    let Decorations::Client { tiling } = window.window_decorations() else {
                        return;
                    };

                    let size = window.viewport_size();
                    let edge = resize_edge(e.position, CLIENT_SIDE_DECORATION_INSET, size, tiling);
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

        root = root.child(window_frame(theme, decorations, body.into_any_element()));

        root = root.child(self.toast_host.clone());

        root = root.child(self.popover_host.clone());

        root = root.child(self.tooltip_host.clone());

        root
    }
}

#[cfg(test)]
mod tests;
