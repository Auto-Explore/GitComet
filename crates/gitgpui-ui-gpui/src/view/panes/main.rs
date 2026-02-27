use super::super::path_display;
use super::super::*;
use std::hash::{Hash, Hasher};

mod diff_cache;
mod diff_search;
mod diff_text;
mod preview;

pub(in super::super) struct MainPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) date_time_format: DateTimeFormat,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitGpuiView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: u64,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,

    pub(in super::super) last_window_size: Size<Pixels>,

    pub(in super::super) show_whitespace: bool,
    pub(in super::super) diff_view: DiffViewMode,
    pub(in super::super) svg_diff_view_mode: SvgDiffViewMode,
    pub(in super::super) diff_word_wrap: bool,
    pub(in super::super) diff_split_ratio: f32,
    pub(in super::super) diff_split_resize: Option<DiffSplitResizeState>,
    pub(in super::super) diff_split_last_synced_y: Pixels,
    pub(in super::super) diff_horizontal_min_width: Pixels,
    pub(in super::super) diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) diff_cache_rev: u64,
    pub(in super::super) diff_cache_target: Option<DiffTarget>,
    pub(in super::super) diff_cache: Vec<AnnotatedDiffLine>,
    pub(in super::super) diff_file_for_src_ix: Vec<Option<Arc<str>>>,
    pub(in super::super) diff_language_for_src_ix: Vec<Option<rows::DiffSyntaxLanguage>>,
    pub(in super::super) diff_click_kinds: Vec<DiffClickKind>,
    pub(in super::super) diff_header_display_cache: HashMap<usize, SharedString>,
    pub(in super::super) diff_split_cache: Vec<PatchSplitRow>,
    pub(in super::super) diff_split_cache_len: usize,
    pub(in super::super) diff_panel_focus_handle: FocusHandle,
    pub(in super::super) diff_autoscroll_pending: bool,
    pub(in super::super) diff_raw_input: Entity<zed::TextInput>,
    pub(in super::super) diff_visible_indices: Vec<usize>,
    pub(in super::super) diff_visible_cache_len: usize,
    pub(in super::super) diff_visible_view: DiffViewMode,
    pub(in super::super) diff_visible_is_file_view: bool,
    pub(in super::super) diff_scrollbar_markers_cache: Vec<zed::ScrollbarMarker>,
    pub(in super::super) diff_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) diff_word_highlights_seq: u64,
    pub(in super::super) diff_word_highlights_inflight: Option<u64>,
    pub(in super::super) diff_file_stats: Vec<Option<(usize, usize)>>,
    pub(in super::super) diff_text_segments_cache: Vec<Option<CachedDiffStyledText>>,
    pub(in super::super) diff_selection_anchor: Option<usize>,
    pub(in super::super) diff_selection_range: Option<(usize, usize)>,
    pub(in super::super) diff_text_selecting: bool,
    pub(in super::super) diff_text_anchor: Option<DiffTextPos>,
    pub(in super::super) diff_text_head: Option<DiffTextPos>,
    diff_text_autoscroll_seq: u64,
    diff_text_autoscroll_target: Option<DiffTextAutoscrollTarget>,
    diff_text_last_mouse_pos: Point<Pixels>,
    pub(in super::super) diff_suppress_clicks_remaining: u8,
    pub(in super::super) diff_text_hitboxes: HashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    pub(in super::super) diff_text_layout_cache_epoch: u64,
    pub(in super::super) diff_text_layout_cache: HashMap<u64, DiffTextLayoutCacheEntry>,
    pub(in super::super) diff_hunk_picker_search_input: Option<Entity<zed::TextInput>>,
    pub(in super::super) diff_search_active: bool,
    pub(in super::super) diff_search_query: SharedString,
    pub(in super::super) diff_search_matches: Vec<usize>,
    pub(in super::super) diff_search_match_ix: Option<usize>,
    pub(in super::super) diff_search_input: Entity<zed::TextInput>,
    _diff_search_subscription: gpui::Subscription,

    pub(in super::super) file_diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) file_diff_cache_rev: u64,
    pub(in super::super) file_diff_cache_target: Option<DiffTarget>,
    pub(in super::super) file_diff_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) file_diff_cache_language: Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) file_diff_cache_rows: Vec<FileDiffRow>,
    pub(in super::super) file_diff_inline_cache: Vec<AnnotatedDiffLine>,
    pub(in super::super) file_diff_inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_split_word_highlights_old: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_split_word_highlights_new: Vec<Option<Vec<Range<usize>>>>,
    pub(in super::super) file_diff_cache_seq: u64,
    pub(in super::super) file_diff_cache_inflight: Option<u64>,

    pub(in super::super) file_image_diff_cache_repo_id: Option<RepoId>,
    pub(in super::super) file_image_diff_cache_rev: u64,
    pub(in super::super) file_image_diff_cache_target: Option<DiffTarget>,
    pub(in super::super) file_image_diff_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) file_image_diff_cache_old: Option<Arc<gpui::Image>>,
    pub(in super::super) file_image_diff_cache_new: Option<Arc<gpui::Image>>,

    pub(in super::super) worktree_preview_path: Option<std::path::PathBuf>,
    pub(in super::super) worktree_preview: Loadable<Arc<Vec<String>>>,
    pub(in super::super) worktree_preview_segments_cache_path: Option<std::path::PathBuf>,
    pub(in super::super) worktree_preview_syntax_language: Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) worktree_preview_segments_cache: HashMap<usize, CachedDiffStyledText>,
    pub(in super::super) diff_preview_is_new_file: bool,
    pub(in super::super) diff_preview_new_file_lines: Arc<Vec<String>>,

    pub(in super::super) conflict_resolver_input: Entity<zed::TextInput>,
    _conflict_resolver_input_subscription: gpui::Subscription,
    pub(in super::super) conflict_resolver: ConflictResolverUiState,
    pub(in super::super) conflict_resolver_vsplit_ratio: f32,
    pub(in super::super) conflict_resolver_vsplit_resize: Option<ConflictVSplitResizeState>,
    pub(in super::super) conflict_three_way_col_ratios: [f32; 2],
    pub(in super::super) conflict_three_way_col_widths: [Pixels; 3],
    pub(in super::super) conflict_hsplit_resize: Option<ConflictHSplitResizeState>,
    pub(in super::super) conflict_diff_split_ratio: f32,
    pub(in super::super) conflict_diff_split_resize: Option<ConflictDiffSplitResizeState>,
    pub(in super::super) conflict_diff_split_col_widths: [Pixels; 2],
    pub(in super::super) conflict_diff_segments_cache_split:
        HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
    pub(in super::super) conflict_diff_segments_cache_inline: HashMap<usize, CachedDiffStyledText>,
    pub(in super::super) conflict_three_way_segments_cache:
        HashMap<(usize, ThreeWayColumn), CachedDiffStyledText>,
    pub(in super::super) conflict_resolved_preview_path: Option<std::path::PathBuf>,
    pub(in super::super) conflict_resolved_preview_source_hash: Option<u64>,
    pub(in super::super) conflict_resolved_preview_syntax_language:
        Option<rows::DiffSyntaxLanguage>,
    pub(in super::super) conflict_resolved_preview_lines: Vec<String>,
    pub(in super::super) conflict_resolved_preview_segments_cache:
        HashMap<usize, CachedDiffStyledText>,

    pub(in super::super) history_view: Entity<super::HistoryView>,
    pub(in super::super) diff_scroll: UniformListScrollHandle,
    pub(in super::super) diff_split_right_scroll: UniformListScrollHandle,
    pub(in super::super) conflict_resolver_diff_scroll: UniformListScrollHandle,
    pub(in super::super) conflict_resolved_preview_scroll: UniformListScrollHandle,
    pub(in super::super) worktree_preview_scroll: UniformListScrollHandle,

    path_display_cache: std::cell::RefCell<HashMap<std::path::PathBuf, SharedString>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffTextAutoscrollTarget {
    DiffLeftOrInline,
    DiffSplitRight,
    WorktreePreview,
    ConflictResolvedPreview,
}

impl MainPaneView {
    fn notify_fingerprint_for(state: &AppState) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.diff_state_rev.hash(&mut hasher);
            repo.conflict_rev.hash(&mut hasher);

            // Only include status changes when viewing a working tree diff.
            let status_rev = if matches!(repo.diff_target, Some(DiffTarget::WorkingTree { .. })) {
                repo.status_rev
            } else {
                0
            };
            status_rev.hash(&mut hasher);
        }

        hasher.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        root_view: WeakEntity<GitGpuiView>,
        tooltip_host: WeakEntity<TooltipHost>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.apply_state_snapshot(next, cx);
            cx.notify();
        });

        let diff_raw_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });

        let conflict_resolver_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "Resolve file contents…".into(),
                    multiline: true,
                    read_only: false,
                    chromeless: true,
                    soft_wrap: true,
                },
                window,
                cx,
            )
        });

        let conflict_resolver_subscription =
            cx.observe(&conflict_resolver_input, |this, input, cx| {
                let output_text = input.read(cx).text();
                let mut output_hasher = std::collections::hash_map::DefaultHasher::new();
                output_text.hash(&mut output_hasher);
                let output_hash = output_hasher.finish();

                let path = this.conflict_resolver.path.clone();
                let needs_update = this.conflict_resolved_preview_path != path
                    || this.conflict_resolved_preview_source_hash != Some(output_hash);
                if !needs_update {
                    return;
                }

                this.conflict_resolved_preview_path = path.clone();
                this.conflict_resolved_preview_source_hash = Some(output_hash);
                this.conflict_resolved_preview_syntax_language = path.as_ref().and_then(|p| {
                    rows::diff_syntax_language_for_path(p.to_string_lossy().as_ref())
                });
                this.conflict_resolved_preview_lines =
                    output_text.split('\n').map(|s| s.to_string()).collect();
                if this.conflict_resolved_preview_lines.is_empty() {
                    this.conflict_resolved_preview_lines.push(String::new());
                }
                this.conflict_resolved_preview_segments_cache.clear();
                cx.notify();
            });

        let diff_search_input = cx.new(|cx| {
            zed::TextInput::new(
                zed::TextInputOptions {
                    placeholder: "Search diff".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: false,
                    soft_wrap: false,
                },
                window,
                cx,
            )
        });
        let diff_search_subscription = cx.observe(&diff_search_input, |this, input, cx| {
            let next: SharedString = input.read(cx).text().to_string().into();
            if this.diff_search_query != next {
                this.diff_search_query = next;
                this.diff_text_segments_cache.clear();
                this.worktree_preview_segments_cache_path = None;
                this.worktree_preview_segments_cache.clear();
                this.conflict_diff_segments_cache_split.clear();
                this.conflict_diff_segments_cache_inline.clear();
                this.diff_search_recompute_matches();
                cx.notify();
            }
        });

        let diff_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);

        let last_window_size = window.window_bounds().get_bounds().size;
        let history_view = cx.new(|cx| {
            super::HistoryView::new(
                Arc::clone(&store),
                ui_model.clone(),
                theme,
                date_time_format,
                timezone,
                history_show_author,
                history_show_date,
                history_show_sha,
                root_view.clone(),
                tooltip_host.clone(),
                last_window_size,
                window,
                cx,
            )
        });

        let mut pane = Self {
            store,
            state,
            theme,
            date_time_format,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            last_window_size: size(px(0.0), px(0.0)),
            show_whitespace: false,
            diff_view: DiffViewMode::Split,
            svg_diff_view_mode: SvgDiffViewMode::Image,
            diff_word_wrap: false,
            diff_split_ratio: 0.5,
            diff_split_resize: None,
            diff_split_last_synced_y: px(0.0),
            diff_horizontal_min_width: px(0.0),
            diff_cache_repo_id: None,
            diff_cache_rev: 0,
            diff_cache_target: None,
            diff_cache: Vec::new(),
            diff_file_for_src_ix: Vec::new(),
            diff_language_for_src_ix: Vec::new(),
            diff_click_kinds: Vec::new(),
            diff_header_display_cache: HashMap::default(),
            diff_split_cache: Vec::new(),
            diff_split_cache_len: 0,
            diff_panel_focus_handle,
            diff_autoscroll_pending: false,
            diff_raw_input,
            diff_visible_indices: Vec::new(),
            diff_visible_cache_len: 0,
            diff_visible_view: DiffViewMode::Split,
            diff_visible_is_file_view: false,
            diff_scrollbar_markers_cache: Vec::new(),
            diff_word_highlights: Vec::new(),
            diff_word_highlights_seq: 0,
            diff_word_highlights_inflight: None,
            diff_file_stats: Vec::new(),
            diff_text_segments_cache: Vec::new(),
            diff_selection_anchor: None,
            diff_selection_range: None,
            diff_text_selecting: false,
            diff_text_anchor: None,
            diff_text_head: None,
            diff_text_autoscroll_seq: 0,
            diff_text_autoscroll_target: None,
            diff_text_last_mouse_pos: point(px(0.0), px(0.0)),
            diff_suppress_clicks_remaining: 0,
            diff_text_hitboxes: HashMap::default(),
            diff_text_layout_cache_epoch: 0,
            diff_text_layout_cache: HashMap::default(),
            diff_hunk_picker_search_input: None,
            diff_search_active: false,
            diff_search_query: "".into(),
            diff_search_matches: Vec::new(),
            diff_search_match_ix: None,
            diff_search_input,
            _diff_search_subscription: diff_search_subscription,
            file_diff_cache_repo_id: None,
            file_diff_cache_rev: 0,
            file_diff_cache_target: None,
            file_diff_cache_path: None,
            file_diff_cache_language: None,
            file_diff_cache_rows: Vec::new(),
            file_diff_inline_cache: Vec::new(),
            file_diff_inline_word_highlights: Vec::new(),
            file_diff_split_word_highlights_old: Vec::new(),
            file_diff_split_word_highlights_new: Vec::new(),
            file_diff_cache_seq: 0,
            file_diff_cache_inflight: None,
            file_image_diff_cache_repo_id: None,
            file_image_diff_cache_rev: 0,
            file_image_diff_cache_target: None,
            file_image_diff_cache_path: None,
            file_image_diff_cache_old: None,
            file_image_diff_cache_new: None,
            worktree_preview_path: None,
            worktree_preview: Loadable::NotLoaded,
            worktree_preview_segments_cache_path: None,
            worktree_preview_syntax_language: None,
            worktree_preview_segments_cache: HashMap::default(),
            diff_preview_is_new_file: false,
            diff_preview_new_file_lines: Arc::new(Vec::new()),
            conflict_resolver_input,
            _conflict_resolver_input_subscription: conflict_resolver_subscription,
            conflict_resolver: ConflictResolverUiState::default(),
            conflict_resolver_vsplit_ratio: 0.5,
            conflict_resolver_vsplit_resize: None,
            conflict_three_way_col_ratios: [1.0 / 3.0, 2.0 / 3.0],
            conflict_three_way_col_widths: [px(0.0); 3],
            conflict_hsplit_resize: None,
            conflict_diff_split_ratio: 0.5,
            conflict_diff_split_resize: None,
            conflict_diff_split_col_widths: [px(0.0); 2],
            conflict_diff_segments_cache_split: HashMap::default(),
            conflict_diff_segments_cache_inline: HashMap::default(),
            conflict_three_way_segments_cache: HashMap::default(),
            conflict_resolved_preview_path: None,
            conflict_resolved_preview_source_hash: None,
            conflict_resolved_preview_syntax_language: None,
            conflict_resolved_preview_lines: Vec::new(),
            conflict_resolved_preview_segments_cache: HashMap::default(),
            history_view,
            diff_scroll: UniformListScrollHandle::default(),
            diff_split_right_scroll: UniformListScrollHandle::default(),
            conflict_resolver_diff_scroll: UniformListScrollHandle::default(),
            conflict_resolved_preview_scroll: UniformListScrollHandle::default(),
            worktree_preview_scroll: UniformListScrollHandle::default(),
            path_display_cache: std::cell::RefCell::new(HashMap::default()),
        };

        pane.set_theme(theme, cx);
        pane.rebuild_diff_cache(cx);
        pane
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.diff_text_segments_cache.clear();
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_segments_cache.clear();
        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();
        self.conflict_resolved_preview_segments_cache.clear();
        self.diff_raw_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.diff_search_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.conflict_resolver_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        if let Some(input) = &self.diff_hunk_picker_search_input {
            input.update(cx, |input, cx| input.set_theme(theme, cx));
        }
        self.history_view
            .update(cx, |view, cx| view.set_theme(theme, cx));
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next.clone();
        self.history_view
            .update(cx, |view, cx| view.set_active_context_menu_invoker(next, cx));
        cx.notify();
    }

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.history_view
            .update(cx, |view, cx| view.set_date_time_format(next, cx));
        cx.notify();
    }

    pub(in super::super) fn set_timezone(
        &mut self,
        next: Timezone,
        cx: &mut gpui::Context<Self>,
    ) {
        self.history_view
            .update(cx, |view, cx| view.set_timezone(next, cx));
        cx.notify();
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn history_visible_column_preferences(
        &self,
        cx: &gpui::App,
    ) -> (bool, bool, bool) {
        self.history_view.read(cx).history_visible_column_preferences()
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    pub(in super::super) fn open_popover_at_cursor(
        &mut self,
        kind: PopoverKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, root.last_mouse_pos, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn clear_status_multi_selection(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.remove(&repo_id);
                cx.notify();
            });
        });
    }

    pub(in super::super) fn scroll_status_list_to_ix(
        &mut self,
        area: DiffArea,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.details_pane
                .update(cx, |pane: &mut DetailsPaneView, cx| {
                    match area {
                        DiffArea::Unstaged => pane
                            .unstaged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                        DiffArea::Staged => pane
                            .staged_scroll
                            .scroll_to_item_strict(ix, gpui::ScrollStrategy::Center),
                    }
                    cx.notify();
                });
        });
    }

    pub(in super::super) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    pub(in super::super) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }

    pub(super) fn apply_state_snapshot(
        &mut self,
        next: Arc<AppState>,
        cx: &mut gpui::Context<Self>,
    ) {
        let prev_active_repo_id = self.state.active_repo;
        let prev_diff_target = self
            .active_repo()
            .and_then(|r| r.diff_target.as_ref())
            .cloned();

        let next_repo_id = next.active_repo;
        let next_repo = next_repo_id.and_then(|id| next.repos.iter().find(|r| r.id == id));
        let next_diff_target = next_repo.and_then(|r| r.diff_target.as_ref()).cloned();
        let next_diff_rev = next_repo.map(|r| r.diff_rev).unwrap_or(0);

        if prev_diff_target != next_diff_target {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            self.diff_autoscroll_pending = next_diff_target.is_some();
        }

        self.state = next;

        self.sync_conflict_resolver(cx);

        if prev_active_repo_id != next_repo_id {
            self.history_view.update(cx, |view, _| {
                view.history_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            });
        }

        let should_rebuild_diff_cache = self.diff_cache_repo_id != next_repo_id
            || self.diff_cache_rev != next_diff_rev
            || self.diff_cache_target != next_diff_target;
        if should_rebuild_diff_cache {
            self.rebuild_diff_cache(cx);
        }

        // History caches are now managed by HistoryView.
    }

    pub(in super::super) fn cached_path_display(&self, path: &std::path::PathBuf) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in super::super) fn touch_diff_text_layout_cache(
        &mut self,
        key: u64,
        layout: Option<ShapedLine>,
    ) {
        let epoch = self.diff_text_layout_cache_epoch;
        match layout {
            Some(layout) => {
                self.diff_text_layout_cache.insert(
                    key,
                    DiffTextLayoutCacheEntry {
                        layout,
                        last_used_epoch: epoch,
                    },
                );
            }
            None => {
                if let Some(entry) = self.diff_text_layout_cache.get_mut(&key) {
                    entry.last_used_epoch = epoch;
                }
            }
        }

    }

    /// Prune the layout cache if it has grown past the high-water mark.
    /// Call once per render frame (after bumping the epoch), **not** from
    /// the per-row `touch_diff_text_layout_cache` hot path.
    pub(in super::super) fn prune_diff_text_layout_cache(&mut self) {
        if self.diff_text_layout_cache.len()
            <= DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES + DIFF_TEXT_LAYOUT_CACHE_PRUNE_OVERAGE
        {
            return;
        }

        let over_by = self
            .diff_text_layout_cache
            .len()
            .saturating_sub(DIFF_TEXT_LAYOUT_CACHE_MAX_ENTRIES);
        if over_by == 0 {
            return;
        }

        let mut by_age: Vec<(u64, u64)> = self
            .diff_text_layout_cache
            .iter()
            .map(|(k, v)| (*k, v.last_used_epoch))
            .collect();
        by_age.sort_by_key(|(_, last_used)| *last_used);

        for (key, _) in by_age.into_iter().take(over_by) {
            self.diff_text_layout_cache.remove(&key);
        }
    }

    pub(in super::super) fn diff_text_segments_cache_get(
        &self,
        key: usize,
    ) -> Option<&CachedDiffStyledText> {
        self.diff_text_segments_cache
            .get(key)
            .and_then(Option::as_ref)
    }

    pub(in super::super) fn diff_text_segments_cache_set(
        &mut self,
        key: usize,
        value: CachedDiffStyledText,
    ) -> &CachedDiffStyledText {
        if self.diff_text_segments_cache.len() <= key {
            self.diff_text_segments_cache.resize_with(key + 1, || None);
        }
        self.diff_text_segments_cache[key] = Some(value);
        self.diff_text_segments_cache[key]
            .as_ref()
            .expect("just set")
    }

    pub(in super::super) fn is_file_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_diff_cache_repo_id == Some(repo.id)
            && self.file_diff_cache_rev == repo.diff_file_rev
            && self.file_diff_cache_target == repo.diff_target
            && self.file_diff_cache_path.is_some()
    }

    pub(in super::super) fn is_file_image_diff_view_active(&self) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        self.file_image_diff_cache_repo_id == Some(repo.id)
            && self.file_image_diff_cache_rev == repo.diff_file_rev
            && self.file_image_diff_cache_target == repo.diff_target
            && self.file_image_diff_cache_path.is_some()
            && (self.file_image_diff_cache_old.is_some()
                || self.file_image_diff_cache_new.is_some())
    }

    pub(in super::super) fn consume_suppress_click_after_drag(&mut self) -> bool {
        if self.diff_suppress_clicks_remaining > 0 {
            self.diff_suppress_clicks_remaining =
                self.diff_suppress_clicks_remaining.saturating_sub(1);
            return true;
        }
        false
    }

    fn diff_src_ixs_for_visible_ix(&self, visible_ix: usize) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return Vec::new();
        }
        let Some(&mapped_ix) = self.diff_visible_indices.get(visible_ix) else {
            return Vec::new();
        };

        match self.diff_view {
            DiffViewMode::Inline => vec![mapped_ix],
            DiffViewMode::Split => {
                let Some(row) = self.diff_split_cache.get(mapped_ix) else {
                    return Vec::new();
                };
                match row {
                    PatchSplitRow::Raw { src_ix, .. } => vec![*src_ix],
                    PatchSplitRow::Aligned {
                        old_src_ix,
                        new_src_ix,
                        ..
                    } => {
                        let mut out = Vec::with_capacity(2);
                        if let Some(ix) = old_src_ix {
                            out.push(*ix);
                        }
                        if let Some(ix) = new_src_ix
                            && out.first().copied() != Some(*ix)
                        {
                            out.push(*ix);
                        }
                        out
                    }
                }
            }
        }
    }

    fn diff_enclosing_hunk_src_ix(&self, src_ix: usize) -> Option<usize> {
        enclosing_hunk_src_ix(&self.diff_cache, src_ix)
    }

    pub(in super::super) fn select_all_diff_text(&mut self) {
        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let end_visible_ix = count - 1;
            let end_text = self.diff_text_line_for_region(end_visible_ix, DiffTextRegion::Inline);

            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix: 0,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix: end_visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_text.len(),
            });
            return;
        }

        if self.diff_visible_indices.is_empty() {
            return;
        }

        let start_region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => self
                .diff_text_head
                .or(self.diff_text_anchor)
                .map(|p| p.region)
                .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight))
                .unwrap_or(DiffTextRegion::SplitLeft),
        };

        let end_visible_ix = self.diff_visible_indices.len() - 1;
        let end_region = start_region;
        let end_text = self.diff_text_line_for_region(end_visible_ix, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: 0,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: end_visible_ix,
            region: end_region,
            offset: end_text.len(),
        });
    }

    fn select_diff_text_rows_range(
        &mut self,
        start_visible_ix: usize,
        end_visible_ix: usize,
        region: DiffTextRegion,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            return;
        }

        let a = start_visible_ix.min(list_len - 1);
        let b = end_visible_ix.min(list_len - 1);
        let (a, b) = if a <= b { (a, b) } else { (b, a) };

        let region = match self.diff_view {
            DiffViewMode::Inline => DiffTextRegion::Inline,
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitRight => DiffTextRegion::SplitRight,
                _ => DiffTextRegion::SplitLeft,
            },
        };
        let start_region = region;
        let end_region = region;

        let end_text = self.diff_text_line_for_region(b, end_region);

        self.diff_text_selecting = false;
        self.diff_text_anchor = Some(DiffTextPos {
            visible_ix: a,
            region: start_region,
            offset: 0,
        });
        self.diff_text_head = Some(DiffTextPos {
            visible_ix: b,
            region: end_region,
            offset: end_text.len(),
        });

        // Double-click produces two click events; suppress both.
        self.diff_suppress_clicks_remaining = 2;
    }

    pub(in super::super) fn double_click_select_diff_text(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        kind: DiffClickKind,
    ) {
        if self.is_file_preview_active() {
            let Some(count) = self.worktree_preview_line_count() else {
                return;
            };
            if count == 0 {
                return;
            }
            let visible_ix = visible_ix.min(count - 1);
            let end_text = self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
            self.diff_text_selecting = false;
            self.diff_text_anchor = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            self.diff_text_head = Some(DiffTextPos {
                visible_ix,
                region: DiffTextRegion::Inline,
                offset: end_text.len(),
            });

            // Double-click produces two click events; suppress both.
            self.diff_suppress_clicks_remaining = 2;
            return;
        }

        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            return;
        }
        let visible_ix = visible_ix.min(list_len - 1);

        // File-diff view doesn't have file/hunk header blocks; treat as row selection.
        if self.is_file_diff_view_active() {
            self.select_diff_text_rows_range(visible_ix, visible_ix, region);
            return;
        }

        let end = match self.diff_view {
            DiffViewMode::Inline => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        let line = &self.diff_cache[src_ix];
                        matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk)
                            || (matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                                && line.text.starts_with("diff --git "))
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .diff_next_boundary_visible_ix(visible_ix, |src_ix| {
                        let line = &self.diff_cache[src_ix];
                        matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                            && line.text.starts_with("diff --git ")
                    })
                    .unwrap_or(list_len - 1),
            },
            DiffViewMode::Split => match kind {
                DiffClickKind::Line => visible_ix,
                DiffClickKind::HunkHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
                DiffClickKind::FileHeader => self
                    .split_next_boundary_visible_ix(visible_ix, |row| {
                        matches!(
                            row,
                            PatchSplitRow::Raw {
                                click_kind: DiffClickKind::FileHeader,
                                ..
                            }
                        )
                    })
                    .unwrap_or(list_len - 1),
            },
        };

        self.select_diff_text_rows_range(visible_ix, end, region);
    }

    fn split_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(&PatchSplitRow) -> bool,
    ) -> Option<usize> {
        let from_visible_ix =
            from_visible_ix.min(self.diff_visible_indices.len().saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..self.diff_visible_indices.len() {
            let row_ix = *self.diff_visible_indices.get(visible_ix)?;
            let row = self.diff_split_cache.get(row_ix)?;
            if is_boundary(row) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    fn diff_next_boundary_visible_ix(
        &self,
        from_visible_ix: usize,
        is_boundary: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        let from_visible_ix =
            from_visible_ix.min(self.diff_visible_indices.len().saturating_sub(1));
        for visible_ix in (from_visible_ix + 1)..self.diff_visible_indices.len() {
            let src_ix = *self.diff_visible_indices.get(visible_ix)?;
            if is_boundary(src_ix) {
                return Some(visible_ix.saturating_sub(1));
            }
        }
        None
    }

    pub(in super::super) fn sync_diff_split_vertical_scroll(&mut self) {
        let left_handle = self.diff_scroll.0.borrow().base_handle.clone();
        let right_handle = self.diff_split_right_scroll.0.borrow().base_handle.clone();
        let left_offset = left_handle.offset();
        let right_offset = right_handle.offset();

        if left_offset.y == right_offset.y {
            self.diff_split_last_synced_y = left_offset.y;
            return;
        }

        let last_synced_y = self.diff_split_last_synced_y;
        let left_changed = left_offset.y != last_synced_y;
        let right_changed = right_offset.y != last_synced_y;

        let master_y = match (left_changed, right_changed) {
            (true, false) => left_offset.y,
            (false, true) => right_offset.y,
            // If both changed (or neither changed), prefer the left scroll (the vertical scrollbar).
            _ => left_offset.y,
        };

        left_handle.set_offset(point(left_offset.x, master_y));
        right_handle.set_offset(point(right_offset.x, master_y));
        self.diff_split_last_synced_y = master_y;
    }

    pub(in super::super) fn main_pane_content_width(&self, cx: &mut gpui::Context<Self>) -> Pixels {
        let fallback_sidebar = px(280.0);
        let fallback_details = px(420.0);
        let (sidebar_w, details_w) = self
            .root_view
            .update(cx, |root, _cx| (root.sidebar_width, root.details_width))
            .unwrap_or((fallback_sidebar, fallback_details));

        let handles_w = px(PANE_RESIZE_HANDLE_PX) * 2.0;
        (self.last_window_size.width - sidebar_w - details_w - handles_w).max(px(0.0))
    }

}

impl MainPaneView {
    pub(in super::super) fn handle_patch_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        if self.is_file_diff_view_active() {
            self.handle_file_diff_row_click(clicked_visible_ix, shift);
            return;
        }
        match self.diff_view {
            DiffViewMode::Inline => self.handle_diff_row_click(clicked_visible_ix, kind, shift),
            DiffViewMode::Split => self.handle_split_row_click(clicked_visible_ix, kind, shift),
        }
    }

    fn handle_split_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::HunkHeader | DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .split_next_boundary_visible_ix(clicked_visible_ix, |row| {
                    matches!(
                        row,
                        PatchSplitRow::Raw {
                            click_kind: DiffClickKind::FileHeader,
                            ..
                        }
                    )
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    fn handle_diff_row_click(
        &mut self,
        clicked_visible_ix: usize,
        kind: DiffClickKind,
        shift: bool,
    ) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);

        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        let end = match kind {
            DiffClickKind::Line => clicked_visible_ix,
            DiffClickKind::HunkHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    let line = &self.diff_cache[src_ix];
                    matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk)
                        || (matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                            && line.text.starts_with("diff --git "))
                })
                .unwrap_or(list_len - 1),
            DiffClickKind::FileHeader => self
                .diff_next_boundary_visible_ix(clicked_visible_ix, |src_ix| {
                    let line = &self.diff_cache[src_ix];
                    matches!(line.kind, gitgpui_core::domain::DiffLineKind::Header)
                        && line.text.starts_with("diff --git ")
                })
                .unwrap_or(list_len - 1),
        };

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, end));
    }

    fn handle_file_diff_row_click(&mut self, clicked_visible_ix: usize, shift: bool) {
        let list_len = self.diff_visible_indices.len();
        if list_len == 0 {
            self.diff_selection_anchor = None;
            self.diff_selection_range = None;
            return;
        }

        let clicked_visible_ix = clicked_visible_ix.min(list_len - 1);
        if shift && let Some(anchor) = self.diff_selection_anchor {
            let a = anchor.min(clicked_visible_ix);
            let b = anchor.max(clicked_visible_ix);
            self.diff_selection_range = Some((a, b));
            return;
        }

        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
    }

    fn file_change_visible_indices(&self) -> Vec<usize> {
        if !self.is_file_diff_view_active() {
            return Vec::new();
        }
        match self.diff_view {
            DiffViewMode::Inline => diff_navigation::change_block_entries(
                self.diff_visible_indices.len(),
                |visible_ix| {
                    let Some(&inline_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return false;
                    };
                    self.file_diff_inline_cache.get(inline_ix).is_some_and(|l| {
                        matches!(
                            l.kind,
                            gitgpui_core::domain::DiffLineKind::Add
                                | gitgpui_core::domain::DiffLineKind::Remove
                        )
                    })
                },
            ),
            DiffViewMode::Split => diff_navigation::change_block_entries(
                self.diff_visible_indices.len(),
                |visible_ix| {
                    let Some(&row_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return false;
                    };
                    self.file_diff_cache_rows.get(row_ix).is_some_and(|row| {
                        !matches!(row.kind, gitgpui_core::file_diff::FileDiffRowKind::Context)
                    })
                },
            ),
        }
    }

    fn patch_hunk_entries(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (visible_ix, &ix) in self.diff_visible_indices.iter().enumerate() {
            match self.diff_view {
                DiffViewMode::Inline => {
                    let Some(line) = self.diff_cache.get(ix) else {
                        continue;
                    };
                    if matches!(line.kind, gitgpui_core::domain::DiffLineKind::Hunk) {
                        out.push((visible_ix, ix));
                    }
                }
                DiffViewMode::Split => {
                    let Some(row) = self.diff_split_cache.get(ix) else {
                        continue;
                    };
                    if let PatchSplitRow::Raw {
                        src_ix,
                        click_kind: DiffClickKind::HunkHeader,
                    } = row
                    {
                        out.push((visible_ix, *src_ix));
                    }
                }
            }
        }
        out
    }

    pub(in super::super) fn diff_nav_entries(&self) -> Vec<usize> {
        if self.is_file_diff_view_active() {
            return self.file_change_visible_indices();
        }
        self.patch_hunk_entries()
            .into_iter()
            .map(|(visible_ix, _)| visible_ix)
            .collect()
    }

    pub(in super::super) fn conflict_nav_entries(&self) -> Vec<usize> {
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                diff_navigation::conflict_nav_entries_for_three_way(
                    &self.conflict_resolver.three_way_conflict_ranges,
                )
            }
            ConflictResolverViewMode::TwoWayDiff => match self.conflict_resolver.diff_mode {
                ConflictDiffMode::Split => diff_navigation::conflict_nav_entries_for_split(
                    &self.conflict_resolver.diff_rows,
                ),
                ConflictDiffMode::Inline => diff_navigation::conflict_nav_entries_for_inline(
                    &self.conflict_resolver.inline_rows,
                ),
            },
        }
    }

    pub(in super::super) fn conflict_jump_prev(&mut self) {
        let entries = self.conflict_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                // In ThreeWay mode, entries are line indices directly from conflict ranges.
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                // Update active_conflict to the range index that starts at this target.
                if let Some(range_ix) = self
                    .conflict_resolver
                    .three_way_conflict_ranges
                    .iter()
                    .position(|r| r.start == target)
                {
                    self.conflict_resolver.active_conflict = range_ix;
                }
            }
            ConflictResolverViewMode::TwoWayDiff => self
                .conflict_resolver_diff_scroll
                .scroll_to_item_strict(target, gpui::ScrollStrategy::Center),
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    pub(in super::super) fn conflict_jump_next(&mut self) {
        let entries = self.conflict_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.conflict_resolver.nav_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                // In ThreeWay mode, entries are line indices directly from conflict ranges.
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(target, gpui::ScrollStrategy::Center);
                // Update active_conflict to the range index that starts at this target.
                if let Some(range_ix) = self
                    .conflict_resolver
                    .three_way_conflict_ranges
                    .iter()
                    .position(|r| r.start == target)
                {
                    self.conflict_resolver.active_conflict = range_ix;
                }
            }
            ConflictResolverViewMode::TwoWayDiff => self
                .conflict_resolver_diff_scroll
                .scroll_to_item_strict(target, gpui::ScrollStrategy::Center),
        }
        self.conflict_resolver.nav_anchor = Some(target);
    }

    pub(in super::super) fn scroll_diff_to_item(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item(target, strategy);
        }
    }

    pub(in super::super) fn scroll_diff_to_item_strict(
        &mut self,
        target: usize,
        strategy: gpui::ScrollStrategy,
    ) {
        self.diff_scroll.scroll_to_item_strict(target, strategy);
        if self.diff_view == DiffViewMode::Split {
            self.diff_split_right_scroll
                .scroll_to_item_strict(target, strategy);
        }
    }

    pub(in super::super) fn diff_jump_prev(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_prev_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in super::super) fn diff_jump_next(&mut self) {
        let entries = self.diff_nav_entries();
        if entries.is_empty() {
            return;
        }

        let current = self.diff_selection_anchor.unwrap_or(0);
        let Some(target) = diff_navigation::diff_nav_next_target(&entries, current) else {
            return;
        };

        self.scroll_diff_to_item_strict(target, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
    }

    pub(in super::super) fn maybe_autoscroll_diff_to_first_change(&mut self) {
        if !self.diff_autoscroll_pending {
            return;
        }
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_autoscroll_pending = false;
            return;
        }
        if self.diff_visible_indices.is_empty() {
            return;
        }

        let entries = self.diff_nav_entries();
        let target = entries.first().copied().unwrap_or(0);

        self.scroll_diff_to_item(target, gpui::ScrollStrategy::Top);
        self.diff_selection_anchor = Some(target);
        self.diff_selection_range = Some((target, target));
        self.diff_autoscroll_pending = false;
    }

    fn sync_conflict_resolver(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            self.conflict_resolver = ConflictResolverUiState::default();
            return;
        };

        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            self.conflict_resolver = ConflictResolverUiState::default();
            return;
        };

        let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_target.as_ref() else {
            self.conflict_resolver = ConflictResolverUiState::default();
            return;
        };
        if *area != DiffArea::Unstaged {
            self.conflict_resolver = ConflictResolverUiState::default();
            return;
        }

        let is_conflicted = match &repo.status {
            Loadable::Ready(status) => status.unstaged.iter().any(|e| {
                e.path == *path && e.kind == gitgpui_core::domain::FileStatusKind::Conflicted
            }),
            _ => false,
        };
        if !is_conflicted {
            self.conflict_resolver = ConflictResolverUiState::default();
            return;
        }

        let path = path.clone();

        let should_load = repo.conflict_file_path.as_ref() != Some(&path)
            && !matches!(repo.conflict_file, Loadable::Loading);
        if should_load {
            self.conflict_resolver = ConflictResolverUiState::default();
            let theme = self.theme;
            self.conflict_resolver_input.update(cx, |input, cx| {
                input.set_theme(theme, cx);
                input.set_text("", cx);
            });
            self.store.dispatch(Msg::LoadConflictFile { repo_id, path });
            return;
        }

        let Loadable::Ready(Some(file)) = &repo.conflict_file else {
            return;
        };
        if file.path != path {
            return;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        file.base.hash(&mut hasher);
        file.ours.hash(&mut hasher);
        file.theirs.hash(&mut hasher);
        file.current.hash(&mut hasher);
        let source_hash = hasher.finish();

        let needs_rebuild = self.conflict_resolver.repo_id != Some(repo_id)
            || self.conflict_resolver.path.as_ref() != Some(&path)
            || self.conflict_resolver.source_hash != Some(source_hash);

        if !needs_rebuild {
            return;
        }

        self.conflict_diff_segments_cache_split.clear();
        self.conflict_diff_segments_cache_inline.clear();

        let (marker_segments, resolved) = if let Some(cur) = file.current.as_deref() {
            let segments = conflict_resolver::parse_conflict_markers(cur);
            if conflict_resolver::conflict_count(&segments) > 0 {
                let resolved = conflict_resolver::generate_resolved_text(&segments);
                (segments, resolved)
            } else {
                (Vec::new(), cur.to_string())
            }
        } else if let Some(ours) = file.ours.as_deref() {
            (Vec::new(), ours.to_string())
        } else if let Some(theirs) = file.theirs.as_deref() {
            (Vec::new(), theirs.to_string())
        } else {
            (Vec::new(), String::new())
        };
        let ours_text = file.ours.as_deref().unwrap_or("");
        let theirs_text = file.theirs.as_deref().unwrap_or("");
        let base_text = file.base.as_deref().unwrap_or("");
        let diff_rows = gitgpui_core::file_diff::side_by_side_rows(ours_text, theirs_text);
        let inline_rows = conflict_resolver::build_inline_rows(&diff_rows);

        fn split_lines_shared(text: &str) -> Vec<SharedString> {
            if text.is_empty() {
                return Vec::new();
            }
            let mut out =
                Vec::with_capacity(text.as_bytes().iter().filter(|&&b| b == b'\n').count() + 1);
            out.extend(text.lines().map(|line| line.to_string().into()));
            out
        }

        let three_way_base_lines = split_lines_shared(base_text);
        let three_way_ours_lines = split_lines_shared(ours_text);
        let three_way_theirs_lines = split_lines_shared(theirs_text);
        let three_way_len = three_way_base_lines
            .len()
            .max(three_way_ours_lines.len())
            .max(three_way_theirs_lines.len());

        let three_way_conflict_ranges = {
            let mut ranges = Vec::new();
            let mut line_offset = 0usize;
            for seg in &marker_segments {
                match seg {
                    conflict_resolver::ConflictSegment::Text(text) => {
                        line_offset += text.lines().count();
                    }
                    conflict_resolver::ConflictSegment::Block(block) => {
                        let count = if block.ours.is_empty() {
                            0
                        } else {
                            block.ours.lines().count()
                        };
                        ranges.push(line_offset..line_offset + count);
                        line_offset += count;
                    }
                }
            }
            ranges
        };

        let view_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.view_mode
        } else if file.base.is_some() {
            ConflictResolverViewMode::ThreeWay
        } else {
            ConflictResolverViewMode::TwoWayDiff
        };

        let diff_mode = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.diff_mode
        } else {
            ConflictDiffMode::Split
        };
        let nav_anchor = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            self.conflict_resolver.nav_anchor
        } else {
            None
        };
        let active_conflict = if self.conflict_resolver.repo_id == Some(repo_id)
            && self.conflict_resolver.path.as_ref() == Some(&path)
        {
            let total = conflict_resolver::conflict_count(&marker_segments);
            if total == 0 {
                0
            } else {
                self.conflict_resolver.active_conflict.min(total - 1)
            }
        } else {
            0
        };

        let (three_way_word_highlights_base, three_way_word_highlights_ours, three_way_word_highlights_theirs) =
            conflict_resolver::compute_three_way_word_highlights(
                &three_way_base_lines,
                &three_way_ours_lines,
                &three_way_theirs_lines,
                &three_way_conflict_ranges,
            );
        let diff_word_highlights_split =
            conflict_resolver::compute_two_way_word_highlights(&diff_rows);

        self.conflict_three_way_segments_cache.clear();

        self.conflict_resolver = ConflictResolverUiState {
            repo_id: Some(repo_id),
            path: Some(path),
            source_hash: Some(source_hash),
            current: file.current.clone(),
            marker_segments,
            active_conflict,
            view_mode,
            diff_rows,
            inline_rows,
            three_way_base_lines,
            three_way_ours_lines,
            three_way_theirs_lines,
            three_way_len,
            three_way_conflict_ranges,
            three_way_word_highlights_base,
            three_way_word_highlights_ours,
            three_way_word_highlights_theirs,
            diff_word_highlights_split,
            diff_mode,
            nav_anchor,
            split_selected: std::collections::BTreeSet::new(),
            inline_selected: std::collections::BTreeSet::new(),
        };

        let line_ending = crate::kit::TextInput::detect_line_ending(&resolved);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_line_ending(line_ending);
            input.set_text(resolved, cx);
        });

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
    }

    pub(in super::super) fn conflict_resolver_set_mode(
        &mut self,
        mode: ConflictDiffMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.diff_mode == mode {
            return;
        }
        self.conflict_resolver.diff_mode = mode;
        self.conflict_resolver.nav_anchor = None;
        self.conflict_resolver.split_selected.clear();
        self.conflict_resolver.inline_selected.clear();
        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches();
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_set_view_mode(
        &mut self,
        view_mode: ConflictResolverViewMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver.view_mode == view_mode {
            return;
        }
        self.conflict_resolver.view_mode = view_mode;
        self.conflict_resolver.split_selected.clear();
        self.conflict_resolver.inline_selected.clear();
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_selection_is_empty(&self) -> bool {
        match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => self.conflict_resolver.split_selected.is_empty(),
            ConflictDiffMode::Inline => self.conflict_resolver.inline_selected.is_empty(),
        }
    }

    pub(in super::super) fn conflict_resolver_clear_selection(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.split_selected.clear();
        self.conflict_resolver.inline_selected.clear();
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_toggle_split_selected(
        &mut self,
        row_ix: usize,
        side: ConflictPickSide,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.nav_anchor = Some(row_ix);
        let key = (row_ix, side);
        if self.conflict_resolver.split_selected.contains(&key) {
            self.conflict_resolver.split_selected.remove(&key);
        } else {
            self.conflict_resolver.split_selected.insert(key);
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_toggle_inline_selected(
        &mut self,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.nav_anchor = Some(ix);
        if self.conflict_resolver.inline_selected.contains(&ix) {
            self.conflict_resolver.inline_selected.remove(&ix);
        } else {
            self.conflict_resolver.inline_selected.insert(ix);
        }
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_append_selection_to_output(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let lines = match self.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => conflict_resolver::collect_split_selection(
                &self.conflict_resolver.diff_rows,
                &self.conflict_resolver.split_selected,
            ),
            ConflictDiffMode::Inline => conflict_resolver::collect_inline_selection(
                &self.conflict_resolver.inline_rows,
                &self.conflict_resolver.inline_selected,
            ),
        };
        if lines.is_empty() {
            return;
        }

        let current = self
            .conflict_resolver_input
            .read_with(cx, |i, _| i.text().to_string());
        let next = conflict_resolver::append_lines_to_output(&current, &lines);
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(next, cx);
        });
    }

    pub(in super::super) fn conflict_resolver_set_output(
        &mut self,
        text: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        self.conflict_resolver_input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(text, cx);
        });
    }

    pub(in super::super) fn conflict_resolver_reset_output_from_markers(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(current) = self.conflict_resolver.current.as_deref() else {
            return;
        };
        let segments = conflict_resolver::parse_conflict_markers(current);
        if conflict_resolver::conflict_count(&segments) == 0 {
            return;
        }
        self.conflict_resolver.marker_segments = segments;
        self.conflict_resolver.active_conflict = 0;
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        self.conflict_resolver_set_output(resolved, cx);
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_conflict_count(&self) -> usize {
        conflict_resolver::conflict_count(&self.conflict_resolver.marker_segments)
    }

    fn conflict_resolver_active_block_mut(
        &mut self,
    ) -> Option<&mut conflict_resolver::ConflictBlock> {
        let target = self.conflict_resolver.active_conflict;
        let mut seen = 0usize;
        for seg in &mut self.conflict_resolver.marker_segments {
            let conflict_resolver::ConflictSegment::Block(block) = seg else {
                continue;
            };
            if seen == target {
                return Some(block);
            }
            seen += 1;
        }
        None
    }

    pub(in super::super) fn conflict_resolver_prev_conflict(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        if self.conflict_resolver.active_conflict == 0 {
            return;
        }
        self.conflict_resolver.active_conflict -= 1;
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_next_conflict(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let total = self.conflict_resolver_conflict_count();
        if total == 0 {
            return;
        }
        if self.conflict_resolver.active_conflict + 1 >= total {
            return;
        }
        self.conflict_resolver.active_conflict += 1;
        cx.notify();
    }

    pub(in super::super) fn conflict_resolver_pick_at(
        &mut self,
        range_ix: usize,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        self.conflict_resolver.active_conflict = range_ix;
        self.conflict_resolver_pick_active_conflict(choice, cx);
    }

    pub(in super::super) fn conflict_resolver_pick_active_conflict(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        let Some(block) = self.conflict_resolver_active_block_mut() else {
            return;
        };
        if matches!(choice, conflict_resolver::ConflictChoice::Base) && block.base.is_none() {
            return;
        }
        block.choice = choice;
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        self.conflict_resolver_set_output(resolved, cx);

        // Auto-advance to the next conflict (kdiff3-style).
        let total = self.conflict_resolver_conflict_count();
        if total > 0 && self.conflict_resolver.active_conflict + 1 < total {
            self.conflict_resolver.active_conflict += 1;
            // Scroll the 3-way view to the new active conflict's range.
            if let Some(range) = self
                .conflict_resolver
                .three_way_conflict_ranges
                .get(self.conflict_resolver.active_conflict)
                .cloned()
            {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(range.start, gpui::ScrollStrategy::Center);
            }
            cx.notify();
        }
    }

    pub(in super::super) fn conflict_resolver_pick_all_conflicts(
        &mut self,
        choice: conflict_resolver::ConflictChoice,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.conflict_resolver_conflict_count() == 0 {
            return;
        }
        for seg in &mut self.conflict_resolver.marker_segments {
            if let conflict_resolver::ConflictSegment::Block(block) = seg {
                if matches!(choice, conflict_resolver::ConflictChoice::Base)
                    && block.base.is_none()
                {
                    continue;
                }
                block.choice = choice;
            }
        }
        let resolved =
            conflict_resolver::generate_resolved_text(&self.conflict_resolver.marker_segments);
        self.conflict_resolver_set_output(resolved, cx);
    }
}

impl Render for MainPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.last_window_size = window.window_bounds().get_bounds().size;
        self.history_view
            .update(cx, |v, _| v.set_last_window_size(self.last_window_size));

        let show_diff = self
            .active_repo()
            .and_then(|r| r.diff_target.as_ref())
            .is_some();
        if show_diff {
            div().size_full().child(self.diff_view(cx))
        } else {
            div().size_full().child(self.history_view.clone())
        }
    }
}
