use super::super::branch_sidebar::BranchSection;
use super::super::caches::BranchSidebarFingerprint;
use super::super::file_icons;
use super::super::sidebar_presentation::{
    SidebarPresentation, SidebarPresentationCache, SidebarRequestFingerprint,
};
use super::super::*;
use gitcomet_core::domain::{FileEntry, FileEntryKind, LogScope};
use gitcomet_state::model::{Loadable, SidebarMode};
use gitcomet_state::msg::Msg;
use rustc_hash::FxHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::kit::TextInput;
use crate::kit::TextInputOptions;

type FileBrowserRowsCache =
    std::cell::RefCell<Option<((RepoId, u64), Rc<[FileBrowserVisibleRow]>)>>;

#[derive(Clone, Debug)]
struct FileBrowserVisibleRow {
    entry_index: usize,
    depth: usize,
    is_directory: bool,
    is_expanded: bool,
}

pub(in super::super) struct SidebarPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    branches_scroll: UniformListScrollHandle,
    file_browser_scroll: UniformListScrollHandle,
    file_browser_search_input: Entity<TextInput>,
    _search_input_subscription: gpui::Subscription,
    sidebar_presentation_cache: SidebarPresentationCache,
    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    sidebar_collapsed_items_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
    root_view: WeakEntity<GitCometView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: SidebarNotifyFingerprint,
    sidebar_request_fingerprint: SidebarRequestFingerprint,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,
    selected_branch: Option<SelectedBranch>,
    file_browser_rows_cache: FileBrowserRowsCache,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SidebarNotifyFingerprint {
    active_repo_id: Option<RepoId>,
    repo_fingerprint: Option<BranchSidebarFingerprint>,
    open_repo_workdirs_count: usize,
    open_repo_workdirs_hash: u64,
    active_workspace_badges_count: usize,
    active_workspace_badges_hash: u64,
    file_browser_rev: u64,
}

impl SidebarNotifyFingerprint {
    fn from_state(state: &AppState) -> Self {
        let active_repo_id = state.active_repo;
        let repo_fingerprint = active_repo_id
            .and_then(|repo_id| state.repos.iter().find(|r| r.id == repo_id))
            .map(BranchSidebarFingerprint::from_repo);
        let (open_repo_workdirs_count, open_repo_workdirs_hash) =
            open_repo_workdirs_fingerprint(state);
        let (active_workspace_badges_count, active_workspace_badges_hash) =
            active_workspace_badges_fingerprint(state);
        let file_browser_rev = active_repo_id
            .and_then(|repo_id| state.repos.iter().find(|r| r.id == repo_id))
            .map(|r| r.file_browser.file_browser_rev)
            .unwrap_or(0);
        Self {
            active_repo_id,
            repo_fingerprint,
            open_repo_workdirs_count,
            open_repo_workdirs_hash,
            active_workspace_badges_count,
            active_workspace_badges_hash,
            file_browser_rev,
        }
    }
}

impl SidebarPaneView {
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        sidebar_collapsed_items_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = SidebarNotifyFingerprint::from_state(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = SidebarNotifyFingerprint::from_state(&next);
            let should_notify = next_fingerprint != this.notify_fingerprint;
            let repo_changed =
                this.notify_fingerprint.active_repo_id != next_fingerprint.active_repo_id;

            this.notify_fingerprint = next_fingerprint;
            this.state = next;
            this.dispatch_sidebar_data_request_if_needed(cx);

            // Reflect the newly-active repo's stored search query in the input.
            // Guarded by repo change so it never fights per-keystroke edits.
            if repo_changed {
                this.sync_search_input_with_state(cx);
            }

            if should_notify {
                cx.notify();
            }
        });

        let file_browser_search_input = cx.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Search files...".into(),
                    multiline: false,
                    read_only: false,
                    chromeless: true,
                    soft_wrap: false,
                },
                cx,
            )
        });
        let store_for_search = Arc::clone(&store);
        let search_input_subscription =
            cx.observe(&file_browser_search_input, move |this, input, cx| {
                // The TextInput entity owns its text (uncontrolled). We only read
                // the typed value and mirror it into app state for filtering — we
                // never write back into the input on a keystroke, which would reset
                // the cursor and flicker between the old and new value.
                let text = input.read(cx).text().to_string();
                if let Some(repo) = this.active_repo()
                    && repo.file_browser.search_query != text {
                        let repo_id = repo.id;
                        store_for_search.dispatch(Msg::SetFileBrowserSearch {
                            repo_id,
                            query: text,
                        });
                    }
                cx.notify();
            });

        let mut this = Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            branches_scroll: UniformListScrollHandle::default(),
            file_browser_scroll: UniformListScrollHandle::default(),
            file_browser_search_input,
            _search_input_subscription: search_input_subscription,
            sidebar_presentation_cache: SidebarPresentationCache::default(),
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            sidebar_collapsed_items_by_repo,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            sidebar_request_fingerprint: SidebarRequestFingerprint::default(),
            active_context_menu_invoker: None,
            selected_branch: None,
            file_browser_rows_cache: std::cell::RefCell::new(None),
        };
        this.dispatch_sidebar_data_request_if_needed(cx);
        // Reflect any already-active repo's stored search query on first mount.
        this.sync_search_input_with_state(cx);
        this
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    /// Push the active repo's stored search query into the input. Call this only
    /// on active-repo change — calling it per keystroke creates a feedback loop
    /// with the input observer and flickers the typed text.
    fn sync_search_input_with_state(&mut self, cx: &mut gpui::Context<Self>) {
        let query = self
            .active_repo()
            .map(|r| r.file_browser.search_query.clone())
            .unwrap_or_default();
        let input_text = self
            .file_browser_search_input
            .read_with(cx, |i: &TextInput, _cx| i.text().to_string());
        if input_text != query {
            self.file_browser_search_input
                .update(cx, |input: &mut TextInput, cx| {
                    input.set_text(query, cx);
                });
        }
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    pub(in super::super) fn set_selected_branch(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some(SelectedBranch {
            repo_id,
            section,
            name: name.to_string(),
        });
        if self.selected_branch.as_ref() == next.as_ref() {
            return;
        }
        self.selected_branch = next;
        cx.notify();
    }

    pub(in super::super) fn selected_branch(&self) -> Option<&SelectedBranch> {
        self.selected_branch.as_ref()
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn open_repo_for_workdir(
        &self,
        workdir: &std::path::Path,
    ) -> Option<&RepoState> {
        self.state.repos.iter().find(|r| r.spec.workdir == workdir)
    }

    pub(in super::super) fn cached_path_display(&self, path: &std::path::Path) -> SharedString {
        let mut cache = self.path_display_cache.borrow_mut();
        path_display::cached_path_display(&mut cache, path)
    }

    pub(in super::super) fn saved_sidebar_collapsed_items(
        &self,
    ) -> BTreeMap<std::path::PathBuf, BTreeSet<String>> {
        self.sidebar_collapsed_items_by_repo
            .iter()
            .filter(|&(_repo, items)| !items.is_empty())
            .map(|(repo, items)| (repo.clone(), items.clone()))
            .collect()
    }

    fn schedule_ui_settings_persist(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.schedule_ui_settings_persist(cx);
        });
    }

    pub(in super::super) fn toggle_active_repo_collapse_key(
        &mut self,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };

        let repo_path = repo.spec.workdir.clone();
        let repo_id = repo.id;
        let should_load_submodules_on_expand = collapse_key.as_ref().trim()
            == branch_sidebar::submodules_section_storage_key()
            && matches!(repo.submodules, Loadable::NotLoaded | Loadable::Error(_));
        let collapse_key = collapse_key.as_ref().trim();
        if collapse_key.is_empty() {
            return;
        }

        let items = self
            .sidebar_collapsed_items_by_repo
            .entry(repo_path.clone())
            .or_default();
        branch_sidebar::toggle_collapse_state(items, collapse_key);
        if items.is_empty() {
            self.sidebar_collapsed_items_by_repo.remove(&repo_path);
        }
        let expanded_now = self.sidebar_collapsed_items_by_repo.get(&repo_path).map_or(
            !branch_sidebar::is_collapsed(&BTreeSet::new(), collapse_key),
            |items| !branch_sidebar::is_collapsed(items, collapse_key),
        );

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.schedule_ui_settings_persist(cx);
        if should_load_submodules_on_expand && expanded_now {
            self.store.dispatch(Msg::LoadSubmodules { repo_id });
        }
        self.dispatch_sidebar_data_request_if_needed(cx);
        cx.notify();
    }

    fn dispatch_sidebar_data_request_if_needed(&mut self, cx: &mut gpui::Context<Self>) {
        let next = sidebar_presentation::sidebar_request_fingerprint(
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
        );
        if next == self.sidebar_request_fingerprint {
            return;
        }
        self.sidebar_request_fingerprint = next;

        let Some((repo_id, request)) = sidebar_presentation::active_sidebar_data_request(
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
        ) else {
            return;
        };

        let store = Arc::clone(&self.store);
        cx.defer(move |_cx| store.dispatch(Msg::EnsureSidebarData { repo_id, request }));
    }

    pub(in super::super) fn branch_sidebar_presentation_cached(
        &mut self,
    ) -> Option<SidebarPresentation> {
        sidebar_presentation::build_sidebar_presentation(
            &mut self.sidebar_presentation_cache,
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
        )
    }

    pub(in super::super) fn sidebar(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;

        let tab_bar = self.render_tab_bar(theme, cx);
        let mode = self.state.sidebar_mode;
        let content = match mode {
            SidebarMode::Branches => self.render_branches_content(theme, cx),
            SidebarMode::Files => self.render_file_browser_content(theme, cx),
        };

        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .child(tab_bar)
            .child(content)
    }

    fn render_tab_bar(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let border_color = theme.colors.border;
        let bg = theme.colors.surface_bg;
        let mode = self.state.sidebar_mode;

        let store_branches = Arc::clone(&self.store);
        let store_files = Arc::clone(&self.store);

        let branches_tab = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0))
            .py(px(2.0))
            .h(px(28.0))
            .when(mode == SidebarMode::Branches, |d| {
                d.bg(theme.colors.active_section)
                    .text_color(theme.colors.text)
            })
            .when(mode != SidebarMode::Branches, |d| {
                d.bg(gpui::transparent_black())
                    .text_color(theme.colors.text_muted)
            })
            .hover(|d| {
                if mode != SidebarMode::Branches {
                    d.bg(theme.colors.hover)
                } else {
                    d
                }
            })
            .text_size(px(12.0))
            .child("Branches")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _e, _window, _cx| {
                    store_branches.dispatch(Msg::SetSidebarMode {
                        mode: SidebarMode::Branches,
                    });
                }),
            );

        let files_tab = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0))
            .py(px(2.0))
            .h(px(28.0))
            .when(mode == SidebarMode::Files, |d| {
                d.bg(theme.colors.active_section)
                    .text_color(theme.colors.text)
            })
            .when(mode != SidebarMode::Files, |d| {
                d.bg(gpui::transparent_black())
                    .text_color(theme.colors.text_muted)
            })
            .hover(|d| {
                if mode != SidebarMode::Files {
                    d.bg(theme.colors.hover)
                } else {
                    d
                }
            })
            .text_size(px(12.0))
            .child("Files")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _e, _window, _cx| {
                    store_files.dispatch(Msg::SetSidebarMode {
                        mode: SidebarMode::Files,
                    });
                }),
            );

        div()
            .flex()
            .flex_row()
            .w_full()
            .h(px(28.0))
            .border_b_1()
            .border_color(border_color)
            .bg(bg)
            .child(branches_tab)
            .child(files_tab)
    }

    fn render_branches_content(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        const SIDEBAR_TOP_INSET_PX: f32 = 2.0;

        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return div()
                .flex()
                .flex_col()
                .h_full()
                .min_h(px(0.0))
                .child(components::empty_state(
                    theme,
                    "Branches",
                    "No repository selected.",
                ))
                .into_any();
        };

        let row_count = presentation.rows.len();
        let list = uniform_list(
            "branch_sidebar",
            row_count,
            cx.processor(Self::render_branch_sidebar_rows),
        )
        .h_full()
        .min_h(px(0.0))
        .track_scroll(&self.branches_scroll);
        let list = restrict_scroll_to_vertical_axis(list);
        let scrollbar_gutter = components::Scrollbar::visible_gutter(
            self.branches_scroll.clone(),
            components::ScrollbarAxis::Vertical,
        );
        let list = div()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(SIDEBAR_TOP_INSET_PX))
            .pl(px(2.0))
            .pr(px(2.0) + scrollbar_gutter)
            .child(list);
        let panel_body: AnyElement = div()
            .id("branch_sidebar_scroll_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(list.into_any_element())
            .child(
                components::Scrollbar::new(
                    "branch_sidebar_scrollbar",
                    self.branches_scroll.clone(),
                )
                .render(theme),
            )
            .into_any_element();

        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .child(panel_body)
            .into_any()
    }

    fn render_file_browser_content(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let search_bar = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(4.0))
            .py(px(2.0))
            .border_b_1()
            .border_color(theme.colors.border)
            .child(self.file_browser_search_input.clone());

        let source_text =
            self.active_repo()
                .map_or("".to_string(), |repo| match &repo.file_browser.source {
                    gitcomet_core::domain::FileSource::WorkingDirectory => "HEAD".to_string(),
                    gitcomet_core::domain::FileSource::Commit(commit_id) => {
                        let hex = commit_id.as_ref();
                        if hex.len() > 7 {
                            format!("commit {}", &hex[..7])
                        } else {
                            format!("commit {hex}")
                        }
                    }
                    gitcomet_core::domain::FileSource::Branch(name) => {
                        format!("branch {name}")
                    }
                });

        let source_label = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(8.0))
            .py(px(2.0))
            .text_size(px(10.0))
            .text_color(theme.colors.text_muted)
            .child(source_text);

        let visible_rows = self.file_browser_visible_rows();

        let body: AnyElement = if visible_rows.is_empty() {
            let repo = self.active_repo();
            let message = match repo {
                None => "No repository selected.",
                Some(r) => match &r.file_browser.entries {
                    Loadable::NotLoaded => "Loading files...",
                    Loadable::Loading => "Loading files...",
                    Loadable::Ready(entries) if entries.is_empty() => "Empty repository.",
                    Loadable::Ready(_) => "No files visible.",
                    Loadable::Error(_) => "Error loading files.",
                },
            };
            components::empty_state(theme, "Files", message).into_any_element()
        } else {
            let row_count = visible_rows.len();
            let list = uniform_list(
                "file_browser",
                row_count,
                cx.processor(Self::render_file_browser_rows),
            )
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_browser_scroll);
            let list = restrict_scroll_to_vertical_axis(list);
            let scrollbar_gutter = components::Scrollbar::visible_gutter(
                self.file_browser_scroll.clone(),
                components::ScrollbarAxis::Vertical,
            );
            let list = div()
                .flex_1()
                .min_h(px(0.0))
                .pt(px(2.0))
                .pl(px(2.0))
                .pr(px(2.0) + scrollbar_gutter)
                .child(list);
            div()
                .id("file_browser_scroll_container")
                .relative()
                .flex()
                .flex_col()
                .flex_1()
                .h_full()
                .child(list.into_any_element())
                .child(
                    components::Scrollbar::new(
                        "file_browser_scrollbar",
                        self.file_browser_scroll.clone(),
                    )
                    .render(theme),
                )
                .into_any_element()
        };

        let browsing_commit = self
            .active_repo()
            .is_some_and(|r| r.browsing_commit().is_some());
        let purple = crate::theme::historical_outline(theme.is_dark);
        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .when(browsing_commit, |d| d.border_2().border_color(purple))
            .child(search_bar)
            .child(source_label)
            .child(body)
            .into_any()
    }

    fn file_browser_visible_rows(&self) -> Vec<FileBrowserVisibleRow> {
        let Some(repo) = self.active_repo() else {
            return Vec::new();
        };

        // Key on the repo id too: file_browser_rev is a per-repo counter, so two
        // repos can share a value and collide otherwise (stale rows for the wrong
        // tree after switching repos).
        let cache_key = (repo.id, repo.file_browser.file_browser_rev);
        let mut cache = self.file_browser_rows_cache.borrow_mut();
        if let Some((cached_key, cached_rows)) = cache.as_ref()
            && *cached_key == cache_key {
                return cached_rows.to_vec();
            }

        let rows = self.compute_file_browser_visible_rows(repo);
        *cache = Some((cache_key, Rc::from(rows.clone())));
        rows
    }

    fn compute_file_browser_visible_rows(&self, repo: &RepoState) -> Vec<FileBrowserVisibleRow> {
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            return Vec::new();
        };

        let has_search = !repo.file_browser.search_query.is_empty();
        let search_query = repo.file_browser.search_query.to_lowercase();

        if has_search {
            let mut matching_entry_indices: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            let mut ancestor_paths: std::collections::HashSet<Arc<PathBuf>> =
                std::collections::HashSet::new();

            for (i, entry) in entries.iter().enumerate() {
                let path_str = entry.path.to_string_lossy().to_lowercase();
                if path_str.contains(&search_query) {
                    matching_entry_indices.insert(i);
                    let mut parent = entry.path.parent();
                    while let Some(p) = parent {
                        if !p.as_os_str().is_empty() {
                            ancestor_paths.insert(Arc::new(p.to_path_buf()));
                        }
                        parent = p.parent();
                    }
                }
            }

            entries
                .iter()
                .enumerate()
                .filter(|(i, entry)| {
                    matching_entry_indices.contains(i) || ancestor_paths.contains(&entry.path)
                })
                .map(|(i, entry)| {
                    let is_expanded = match entry.kind {
                        FileEntryKind::Directory => true,
                        FileEntryKind::File => false,
                    };
                    FileBrowserVisibleRow {
                        entry_index: i,
                        depth: entry.depth,
                        is_directory: entry.kind == FileEntryKind::Directory,
                        is_expanded,
                    }
                })
                .collect()
        } else {
            let visible_mask = self.file_browser_visible_mask(entries);

            entries
                .iter()
                .enumerate()
                .filter(|(i, _)| visible_mask.contains(i))
                .map(|(i, entry)| {
                    let is_expanded = entry.kind == FileEntryKind::Directory
                        && repo.file_browser.expanded_dirs.contains(&entry.path);
                    FileBrowserVisibleRow {
                        entry_index: i,
                        depth: entry.depth,
                        is_directory: entry.kind == FileEntryKind::Directory,
                        is_expanded,
                    }
                })
                .collect()
        }
    }

    fn file_browser_visible_mask(&self, entries: &[FileEntry]) -> std::collections::HashSet<usize> {
        let Some(repo) = self.active_repo() else {
            return std::collections::HashSet::new();
        };
        let expanded = &repo.file_browser.expanded_dirs;

        let mut visible = std::collections::HashSet::new();
        let mut skip_until_sibling: Option<(usize, usize)> = None;

        for (i, entry) in entries.iter().enumerate() {
            if let Some((skip_depth, sibling_end)) = skip_until_sibling {
                if i < sibling_end && entry.depth > skip_depth {
                    continue;
                }
                skip_until_sibling = None;
            }

            visible.insert(i);

            if entry.kind == FileEntryKind::Directory && !expanded.contains(&entry.path) {
                let skip_depth = entry.depth;
                let sibling_end = entries[i + 1..]
                    .iter()
                    .position(|e| e.depth <= skip_depth)
                    .map(|pos| i + 1 + pos)
                    .unwrap_or(entries.len());
                skip_until_sibling = Some((skip_depth, sibling_end));
            }
        }

        visible
    }

    pub(in super::super) fn render_file_browser_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        const INDENT_STEP_PX: f32 = 12.0;
        const CHEVRON_SLOT_PX: f32 = 14.0;
        const ICON_SLOT_PX: f32 = 16.0;
        const ROW_HEIGHT_PX: f32 = 22.0;

        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let theme = this.theme;
        let icon_muted = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.6 } else { 0.5 },
        );
        // Zed renders file/folder icons in a neutral, muted tone rather than a
        // bright accent — match that so the tree reads the same way.
        let icon_color = theme.colors.text_muted;
        let text_color = theme.colors.text;
        let store = Arc::clone(&this.store);

        let visible_rows = this.file_browser_visible_rows();
        let repo = this.active_repo();
        let entries = repo
            .and_then(|r| match &r.file_browser.entries {
                Loadable::Ready(e) => Some(e.as_slice()),
                _ => None,
            })
            .unwrap_or(&[]);

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_icon(path, color, scaled_px(size_px))
        };

        let svg_chevron =
            |expanded: bool| svg_icon(file_icons::chevron_icon(expanded), icon_muted, 10.0);

        let chevron_slot = |is_directory: bool, is_expanded: bool| {
            div()
                .w(scaled_px(CHEVRON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when(is_directory, |d| d.child(svg_chevron(is_expanded)))
        };

        let file_or_folder_icon_path = |entry: &FileEntry, expanded: bool| -> &'static str {
            if entry.kind == FileEntryKind::Directory {
                file_icons::folder_icon(expanded)
            } else {
                file_icons::file_icon_for_path(&entry.path)
            }
        };

        let icon_slot = |path: &'static str| {
            div()
                .w(scaled_px(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, icon_color, 12.0))
        };

        range
            .filter_map(|ix| {
                visible_rows
                    .get(ix)
                    .and_then(|row| entries.get(row.entry_index).map(|e| (ix, row, e)))
            })
            .map(|(ix, row, entry)| {
                let left_pad = scaled_px(INDENT_STEP_PX * row.depth as f32);
                let store = Arc::clone(&store);

                let mut row_div = div()
                    .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
                    .flex()
                    .flex_row()
                    .items_center()
                    .h(scaled_px(ROW_HEIGHT_PX))
                    .w_full()
                    .pl(left_pad)
                    .cursor_pointer()
                    .rounded(px(theme.radii.row))
                    .hover(|d| d.bg(theme.colors.hover));

                if row.is_directory {
                    let path = (*entry.path).clone();
                    row_div = row_div.on_click(cx.listener(
                        move |_this, _e: &gpui::ClickEvent, _window, _cx| {
                            store.dispatch(Msg::ToggleFileBrowserDir {
                                repo_id,
                                path: path.clone(),
                            });
                        },
                    ));
                } else {
                    let path = (*entry.path).clone();
                    let menu_path = path.clone();
                    let source = repo
                        .map(|r| r.file_browser.source.clone())
                        .unwrap_or(gitcomet_core::domain::FileSource::WorkingDirectory);
                    let menu_invoker = SharedString::from(format!("file_browser_file_{ix}"));
                    row_div = row_div
                        .on_click(
                            cx.listener(move |_this, _e: &gpui::ClickEvent, _window, _cx| {
                                store.dispatch(Msg::OpenFileContent {
                                    repo_id,
                                    source: source.clone(),
                                    path: path.clone(),
                                });
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(menu_invoker.clone(), cx);
                                this.open_popover_at(
                                    PopoverKind::FileBrowserFileMenu {
                                        repo_id,
                                        path: menu_path.clone(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        );
                }

                row_div
                    .child(chevron_slot(row.is_directory, row.is_expanded))
                    .child(icon_slot(file_or_folder_icon_path(entry, row.is_expanded)))
                    .child(
                        div()
                            .flex_1()
                            .text_color(text_color)
                            .text_size(scaled_px(11.5))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .ml(scaled_px(4.0))
                            .child(entry.name.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
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

    pub(in super::super) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                cx.notify();
            });
        });
    }

    pub(in super::super) fn reveal_branch_commit_in_history(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        branch_name: &str,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        let branch_name = branch_name.to_string();
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.main_pane.update(cx, |pane, cx| {
                    pane.reveal_history_branch_commit(
                        repo_id,
                        section,
                        &branch_name,
                        commit_id,
                        fallback_scope,
                        cx,
                    );
                });
            });
        });
    }
}

impl Render for SidebarPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.sidebar(cx)
    }
}

fn open_repo_workdirs_fingerprint(state: &AppState) -> (usize, u64) {
    let mut workdirs = state
        .repos
        .iter()
        .map(|repo| repo.spec.workdir.as_path())
        .collect::<Vec<_>>();
    workdirs.sort_unstable_by(|left, right| left.as_os_str().cmp(right.as_os_str()));

    let mut hasher = FxHasher::default();
    workdirs.len().hash(&mut hasher);
    for workdir in workdirs {
        workdir.hash(&mut hasher);
    }

    (state.repos.len(), hasher.finish())
}

fn active_workspace_badges_fingerprint(state: &AppState) -> (usize, u64) {
    let Some(active_repo_id) = state.active_repo else {
        return (0, 0);
    };
    let Some(active_repo) = state.repos.iter().find(|repo| repo.id == active_repo_id) else {
        return (0, 0);
    };

    let mut badges =
        crate::view::rows::active_workspace_paths_by_branch(active_repo, state.repos.as_slice())
            .into_iter()
            .collect::<Vec<_>>();
    badges.sort_unstable_by(|(left_branch, left_path), (right_branch, right_path)| {
        left_branch
            .cmp(right_branch)
            .then_with(|| left_path.as_os_str().cmp(right_path.as_os_str()))
    });

    let mut hasher = FxHasher::default();
    badges.len().hash(&mut hasher);
    for (branch, path) in &badges {
        branch.hash(&mut hasher);
        path.hash(&mut hasher);
    }

    (badges.len(), hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_state(id: RepoId, path: &str) -> RepoState {
        RepoState::new_opening(
            id,
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(path),
            },
        )
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_open_repo_workdirs() {
        let mut state = AppState {
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos.push(repo_state(RepoId(2), "/tmp/repo-wt"));

        assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_live_workspace_badge_branch_changes() {
        let mut active = repo_state(RepoId(1), "/tmp/repo");
        active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));

        let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
        worktree_repo.head_branch = Loadable::Ready("feature/old".to_string());
        let mut state = AppState {
            repos: vec![active, worktree_repo],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[1].head_branch = Loadable::Ready("feature/new".to_string());
        state.repos[1].head_branch_rev = 1;

        assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_workspace_badge_removal_when_tab_closes() {
        let mut active = repo_state(RepoId(1), "/tmp/repo");
        active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));

        let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
        worktree_repo.head_branch = Loadable::Ready("feature".to_string());
        let mut state = AppState {
            repos: vec![active, worktree_repo],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos.pop();

        assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_workspace_badge_removal_when_worktree_detaches() {
        let mut active = repo_state(RepoId(1), "/tmp/repo");
        active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));

        let mut worktree_repo = repo_state(RepoId(2), "/tmp/repo-feature");
        worktree_repo.head_branch = Loadable::Ready("feature".to_string());
        let mut state = AppState {
            repos: vec![active, worktree_repo],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[1].head_branch = Loadable::Ready("HEAD".to_string());
        state.repos[1].head_branch_rev = 1;
        state.repos[1].detached_head_commit = Some(CommitId("deadbeef".into()));

        assert_ne!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_ignores_repo_tab_order() {
        let state_a = AppState {
            repos: vec![
                repo_state(RepoId(1), "/tmp/repo"),
                repo_state(RepoId(2), "/tmp/repo-wt"),
            ],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let state_b = AppState {
            repos: vec![
                repo_state(RepoId(2), "/tmp/repo-wt"),
                repo_state(RepoId(1), "/tmp/repo"),
            ],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        assert_eq!(
            SidebarNotifyFingerprint::from_state(&state_a),
            SidebarNotifyFingerprint::from_state(&state_b)
        );
    }

    #[test]
    fn toggling_default_closed_sections_persists_expanded_overrides() {
        let mut collapsed_items = BTreeSet::new();

        branch_sidebar::toggle_collapse_state(
            &mut collapsed_items,
            branch_sidebar::worktrees_section_storage_key(),
        );

        assert!(
            !branch_sidebar::is_collapsed(
                &collapsed_items,
                branch_sidebar::worktrees_section_storage_key(),
            ),
            "opening a default-closed section should persist an expanded override"
        );
        assert_eq!(
            collapsed_items,
            BTreeSet::from([branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::worktrees_section_storage_key(),
            )
            .expect("worktrees should support explicit expansion")])
        );

        branch_sidebar::toggle_collapse_state(
            &mut collapsed_items,
            branch_sidebar::worktrees_section_storage_key(),
        );

        assert!(
            branch_sidebar::is_collapsed(
                &collapsed_items,
                branch_sidebar::worktrees_section_storage_key(),
            ),
            "closing a default-closed section should drop the override"
        );
        assert!(collapsed_items.is_empty());
    }

    #[test]
    fn sidebar_notify_fingerprint_ignores_inactive_repo_changes() {
        let active = repo_state(RepoId(1), "/tmp/active");
        let inactive = repo_state(RepoId(2), "/tmp/inactive");
        let mut state = AppState {
            repos: vec![active, inactive],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[1].head_branch_rev = 1;
        state.repos[1].branches_rev = 1;
        state.repos[1].remote_branches_rev = 1;
        state.repos[1].worktrees_rev = 1;
        state.repos[1].submodules_rev = 1;
        state.repos[1].stashes_rev = 1;
        state.repos[1].branch_sidebar_rev = 1;

        assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_ignores_unrelated_open_repo_branch_changes() {
        let mut active = repo_state(RepoId(1), "/tmp/active");
        active.worktrees = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Worktree {
            path: PathBuf::from("/tmp/active-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));
        let related = repo_state(RepoId(2), "/tmp/active-feature");
        let unrelated = repo_state(RepoId(3), "/tmp/unrelated");
        let mut state = AppState {
            repos: vec![active, related, unrelated],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[2].head_branch = Loadable::Ready("other".to_string());
        state.repos[2].head_branch_rev = 1;

        assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_active_repo_branch_sidebar_changes() {
        let mut state = AppState {
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[0].head_branch_rev = 1;
        let after_head = SidebarNotifyFingerprint::from_state(&state);
        assert_ne!(after_head, initial);

        state.repos[0].branches_rev = 1;
        let after_branches = SidebarNotifyFingerprint::from_state(&state);
        assert_ne!(after_branches, after_head);

        state.repos[0].branch_sidebar_rev = 42;
        assert_ne!(SidebarNotifyFingerprint::from_state(&state), after_branches);
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_file_browser_rev() {
        let mut state = AppState {
            repos: vec![repo_state(RepoId(1), "/tmp/repo")],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        state.repos[0].file_browser.file_browser_rev = 1;
        let after_bump = SidebarNotifyFingerprint::from_state(&state);
        assert_ne!(after_bump, initial);

        state.repos[0].file_browser.file_browser_rev = 99;
        assert_ne!(SidebarNotifyFingerprint::from_state(&state), after_bump);
    }

    #[test]
    fn sidebar_notify_fingerprint_ignores_inactive_file_browser_rev() {
        let mut state = AppState {
            repos: vec![
                repo_state(RepoId(1), "/tmp/active"),
                repo_state(RepoId(2), "/tmp/inactive"),
            ],
            active_repo: Some(RepoId(1)),
            ..AppState::default()
        };

        let initial = SidebarNotifyFingerprint::from_state(&state);

        // Only change the INACTIVE repo's file_browser_rev
        state.repos[1].file_browser.file_browser_rev = 42;
        assert_eq!(SidebarNotifyFingerprint::from_state(&state), initial);
    }
}
