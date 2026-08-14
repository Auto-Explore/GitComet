use super::super::branch_sidebar::{BranchSection, BranchSidebarRow};
use super::super::caches::BranchSidebarFingerprint;
use super::super::file_icons;
use super::super::sidebar_presentation::{
    SidebarPresentation, SidebarPresentationCache, SidebarRequestFingerprint,
};
use super::super::*;
use gitcomet_core::domain::{FileEntry, FileEntryKind, LogScope};
use gitcomet_state::model::{Loadable, SidebarDataRequest, SidebarMode};
use gitcomet_state::msg::Msg;
use rustc_hash::FxHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::kit::TextInput;
use crate::kit::TextInputOptions;
use crate::view::components::InteractiveRowExt as _;
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};

type FileBrowserRowsCache = std::cell::RefCell<
    Option<(
        (RepoId, u64, DiffSearchOptions, u64, bool),
        Rc<[FileBrowserVisibleRow]>,
    )>,
>;

/// One row of the file explorer list.
///
/// Most rows are tree entries, but the list is prefixed by a pinned section for
/// files the editor is holding unsaved buffers for — those files are the ones
/// the user is in the middle of something with, and hunting for them in a tree
/// they may have collapsed is the opposite of what that moment needs.
///
/// The pinned rows are a *separate* section rather than tree entries hoisted to
/// the top: a tree row carries a depth and a parent, and a file lifted out of
/// its folder has neither.
#[derive(Clone, Debug)]
enum FileBrowserVisibleRow {
    /// Header of the unsaved-edits section. Click toggles the section.
    UnsavedHeader { count: usize },
    /// A file with an unsaved editor buffer, shown by its full repo-relative
    /// path since it is out of its folder here.
    UnsavedFile { path: Arc<PathBuf> },
    Entry {
        entry_index: usize,
        depth: usize,
        is_directory: bool,
        is_expanded: bool,
    },
}

impl FileBrowserVisibleRow {
    /// The tree entry this row points at, or `None` for the pinned section.
    ///
    /// Every index-space walk over the row list has to go through this: the
    /// pinned rows share the list with the tree but not its index space, and a
    /// position computed as if they did lands on the wrong file.
    fn entry_index(&self) -> Option<usize> {
        match self {
            Self::Entry { entry_index, .. } => Some(*entry_index),
            _ => None,
        }
    }
}

/// Storage key for the unsaved-edits section's collapsed state, in the same map
/// the branch tree's sections use.
const FILE_BROWSER_UNSAVED_SECTION_KEY: &str = "file_browser:unsaved_edits";

const FILE_BROWSER_ROW_HEIGHT_PX: f32 = 22.0;
/// How long a queued reveal may wait for the expanded rows it needs. Generous
/// enough for the store round trip, short enough that a request the user has
/// moved on from never fires.
const FILE_BROWSER_REVEAL_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// A section of the sidebar that gets its own icon in the collapsed rail and,
/// when clicked, opens in a floating popover without expanding the sidebar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CollapsedSidebarSection {
    Local,
    Remote,
    Worktrees,
    Submodules,
    Stashes,
    Files,
}

impl CollapsedSidebarSection {
    /// Rail order, top to bottom.
    pub(in crate::view) const ALL: [Self; 6] = [
        Self::Local,
        Self::Remote,
        Self::Worktrees,
        Self::Submodules,
        Self::Stashes,
        Self::Files,
    ];

    pub(in crate::view) fn icon_path(self) -> &'static str {
        match self {
            Self::Local => "icons/computer.svg",
            Self::Remote => "icons/cloud.svg",
            Self::Worktrees => "icons/git_worktree.svg",
            Self::Submodules => "icons/box.svg",
            Self::Stashes => super::super::icons::STASH_ICON_PATH,
            Self::Files => "icons/file.svg",
        }
    }

    pub(in crate::view) fn title(self) -> &'static str {
        match self {
            Self::Local => "Local Branches",
            Self::Remote => "Remote Branches",
            Self::Worktrees => "Worktrees",
            Self::Submodules => "Submodules",
            Self::Stashes => "Stashes",
            Self::Files => "Files",
        }
    }

    pub(in crate::view) fn element_id(self) -> &'static str {
        match self {
            Self::Local => "collapsed_sidebar_icon_local",
            Self::Remote => "collapsed_sidebar_icon_remote",
            Self::Worktrees => "collapsed_sidebar_icon_worktrees",
            Self::Submodules => "collapsed_sidebar_icon_submodules",
            Self::Stashes => "collapsed_sidebar_icon_stashes",
            Self::Files => "collapsed_sidebar_icon_files",
        }
    }

    /// The branch list this section shows, if any. Branch sections are the ones
    /// that support the popover filter (and can spill results into each other).
    fn branch_section(self) -> Option<BranchSection> {
        match self {
            Self::Local => Some(BranchSection::Local),
            Self::Remote => Some(BranchSection::Remote),
            _ => None,
        }
    }

    /// The other branch section, whose matches the filter also surfaces.
    fn counterpart(self) -> Option<Self> {
        match self {
            Self::Local => Some(Self::Remote),
            Self::Remote => Some(Self::Local),
            _ => None,
        }
    }

    /// The section-level menu the expanded sidebar hangs off this section's
    /// header row, paired with the invoker id that header uses so both routes
    /// light up the same "menu is open" highlight. Files has no section menu:
    /// its actions live on the individual file rows.
    pub(in crate::view) fn section_menu(
        self,
        repo_id: RepoId,
    ) -> Option<(SharedString, PopoverKind)> {
        let (invoker, kind): (String, PopoverKind) = match self {
            Self::Local => (
                format!("branch_section_menu_{}_local", repo_id.0),
                PopoverKind::BranchSectionMenu {
                    repo_id,
                    section: BranchSection::Local,
                },
            ),
            Self::Remote => (
                format!("branch_section_menu_{}_remote", repo_id.0),
                PopoverKind::BranchSectionMenu {
                    repo_id,
                    section: BranchSection::Remote,
                },
            ),
            Self::Worktrees => (
                format!("worktrees_section_menu_{}", repo_id.0),
                PopoverKind::worktree(repo_id, WorktreePopoverKind::SectionMenu),
            ),
            Self::Submodules => (
                format!("submodules_section_menu_{}", repo_id.0),
                PopoverKind::submodule(repo_id, SubmodulePopoverKind::SectionMenu),
            ),
            Self::Stashes => (
                format!("stash_section_menu_{}", repo_id.0),
                PopoverKind::StashPrompt,
            ),
            Self::Files => return None,
        };
        Some((invoker.into(), kind))
    }

    fn storage_key(self) -> Option<&'static str> {
        match self {
            Self::Local => Some(branch_sidebar::local_section_storage_key()),
            Self::Remote => Some(branch_sidebar::remote_section_storage_key()),
            Self::Worktrees => Some(branch_sidebar::worktrees_section_storage_key()),
            Self::Submodules => Some(branch_sidebar::submodules_section_storage_key()),
            Self::Stashes => Some(branch_sidebar::stash_section_storage_key()),
            Self::Files => None,
        }
    }
}

pub(in super::super) struct SidebarPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    branches_scroll: UniformListScrollHandle,
    file_browser_scroll: UniformListScrollHandle,
    pub(in super::super) collapsed_popover_scroll: gpui::ScrollHandle,
    file_browser_search_input: Entity<TextInput>,
    _search_input_subscription: gpui::Subscription,
    /// Live filter for the branch sidebar (Local/Remote/pinned sections). The
    /// input entity owns the text; `branch_filter_query` mirrors it for the row
    /// builder, kept in sync by `_branch_filter_subscription`.
    branch_filter_input: Entity<TextInput>,
    pub(in super::super) branch_filter_query: String,
    _branch_filter_subscription: gpui::Subscription,
    /// The collapsed-rail branch popovers keep their filter behind a header
    /// toggle. Separate from `branch_filter_input` so a popover filter never
    /// leaks into the expanded sidebar's filter (and vice versa).
    collapsed_popover_filter_open: bool,
    collapsed_popover_filter_input: Entity<TextInput>,
    pub(in super::super) collapsed_popover_filter_query: String,
    _collapsed_popover_filter_subscription: gpui::Subscription,
    sidebar_presentation_cache: SidebarPresentationCache,
    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    sidebar_collapsed_items_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
    sidebar_pinned_branches_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
    root_view: WeakEntity<GitCometView>,
    pub(in crate::view) tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: SidebarNotifyFingerprint,
    sidebar_request_fingerprint: SidebarRequestFingerprint,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,
    selected_branch: Option<SelectedBranch>,
    file_search_options: DiffSearchOptions,
    file_browser_rows_cache: FileBrowserRowsCache,
    /// Set transiently while rendering a collapsed-sidebar section popover so the
    /// shared branch-row renderer draws the section-scoped rows instead of the
    /// full cached presentation. `None` during normal (expanded) rendering.
    pub(in super::super) collapsed_popover_presentation: Option<SidebarPresentation>,
    /// When set (and the sidebar is collapsed), this pane renders only the given
    /// section as popover content instead of the full sidebar. The root view
    /// syncs this to its `sidebar_collapsed_popover` before embedding the pane.
    collapsed_popover_section: Option<CollapsedSidebarSection>,
    /// A file the explorer has been asked to scroll to, held until the store
    /// snapshot with its folders expanded arrives.
    pending_file_browser_reveal: Option<std::path::PathBuf>,
    /// When the request was made. Bounded so an unresolvable reveal expires
    /// instead of firing at some unrelated later moment.
    pending_file_browser_reveal_at: Option<std::time::Instant>,
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
    /// The file the main pane has open. The tree highlights it, so the sidebar
    /// has to repaint when it changes — nothing else in this fingerprint moves
    /// when the user opens a different file.
    diff_target_rev: u64,
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
        let diff_target_rev = active_repo_id
            .and_then(|repo_id| state.repos.iter().find(|r| r.id == repo_id))
            .map(|r| r.diff_state.diff_target_rev)
            .unwrap_or(0);
        Self {
            active_repo_id,
            repo_fingerprint,
            open_repo_workdirs_count,
            open_repo_workdirs_hash,
            active_workspace_badges_count,
            active_workspace_badges_hash,
            file_browser_rev,
            diff_target_rev,
        }
    }
}

impl SidebarPaneView {
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        sidebar_collapsed_items_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
        sidebar_pinned_branches_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
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
                    chromeless: true,
                    multiline: true,
                    ..Default::default()
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
                    && repo.file_browser.search_query != text
                {
                    let repo_id = repo.id;
                    store_for_search.dispatch(Msg::SetFileBrowserSearch {
                        repo_id,
                        query: text,
                    });
                }
                cx.notify();
            });

        let branch_filter_input = cx.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Filter branches...".into(),
                    leading_icon: Some("icons/git_branch.svg"),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            )
        });
        let branch_filter_subscription =
            cx.observe(&branch_filter_input, move |this, input, cx| {
                // The input owns its text (uncontrolled); mirror it into the
                // local query used by the row builder, never writing back.
                let text = input.read(cx).text().to_string();
                if this.branch_filter_query != text {
                    this.branch_filter_query = text;
                    this.branches_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    this.sync_popover_branch_filter(cx);
                    cx.notify();
                }
            });

        let collapsed_popover_filter_input = cx.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Filter branches...".into(),
                    leading_icon: Some("icons/zoom.svg"),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            )
        });
        let collapsed_popover_filter_subscription =
            cx.observe(&collapsed_popover_filter_input, move |this, input, cx| {
                // Uncontrolled, like the sidebar filter: mirror the text into the
                // query the popover presentation builder reads, never writing back.
                let text = input.read(cx).text().to_string();
                if this.collapsed_popover_filter_query != text {
                    this.collapsed_popover_filter_query = text;
                    this.collapsed_popover_scroll
                        .set_offset(gpui::point(px(0.0), px(0.0)));
                    cx.notify();
                }
            });

        let mut this = Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            branches_scroll: UniformListScrollHandle::default(),
            file_browser_scroll: UniformListScrollHandle::default(),
            collapsed_popover_scroll: gpui::ScrollHandle::new(),
            file_browser_search_input,
            _search_input_subscription: search_input_subscription,
            branch_filter_input,
            branch_filter_query: String::new(),
            _branch_filter_subscription: branch_filter_subscription,
            collapsed_popover_filter_open: false,
            collapsed_popover_filter_input,
            collapsed_popover_filter_query: String::new(),
            _collapsed_popover_filter_subscription: collapsed_popover_filter_subscription,
            sidebar_presentation_cache: SidebarPresentationCache::default(),
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            sidebar_collapsed_items_by_repo,
            sidebar_pinned_branches_by_repo,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            sidebar_request_fingerprint: SidebarRequestFingerprint::default(),
            active_context_menu_invoker: None,
            selected_branch: None,
            file_search_options: DiffSearchOptions::default(),
            file_browser_rows_cache: std::cell::RefCell::new(None),
            collapsed_popover_presentation: None,
            collapsed_popover_section: None,
            pending_file_browser_reveal: None,
            pending_file_browser_reveal_at: None,
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

    /// Sync the section this pane should render as collapsed-rail popover content.
    /// No `cx.notify()`: the root re-renders (and re-embeds this pane) whenever the
    /// value changes, so an extra notify would only cause a redundant paint.
    pub(in super::super) fn set_collapsed_popover_section(
        &mut self,
        section: Option<CollapsedSidebarSection>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.collapsed_popover_section == section {
            return;
        }
        self.collapsed_popover_section = section;
        self.reset_collapsed_popover_filter(cx);
    }

    fn toggle_file_search_option(
        &mut self,
        toggle: impl FnOnce(&mut DiffSearchOptions),
        cx: &mut gpui::Context<Self>,
    ) {
        toggle(&mut self.file_search_options);
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

    #[cfg(test)]
    pub(in crate::view) fn collapsed_items_for_test(&self) -> BTreeSet<String> {
        self.sidebar_collapsed_items_by_repo
            .values()
            .flat_map(|items| items.iter().cloned())
            .collect()
    }

    #[cfg(test)]
    pub(in crate::view) fn pinned_branches_for_test(&self) -> BTreeSet<String> {
        self.sidebar_pinned_branches_by_repo
            .values()
            .flat_map(|items| items.iter().cloned())
            .collect()
    }

    /// Set the pane's mirror of the filter text directly. The real path writes
    /// it from the filter input's subscription, which a headless test has no
    /// way to drive.
    #[cfg(test)]
    pub(in crate::view) fn set_branch_filter_query_for_test(&mut self, query: &str) {
        self.branch_filter_query = query.to_string();
        self.sidebar_presentation_cache = SidebarPresentationCache::default();
    }

    /// Seed the collapse set for the active repo. The real path restores it from
    /// the saved session, which the test harness constructs without.
    #[cfg(test)]
    pub(in crate::view) fn set_collapsed_keys_for_test(&mut self, keys: &[&str]) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_path = repo.spec.workdir.clone();
        self.sidebar_collapsed_items_by_repo.insert(
            repo_path,
            keys.iter().map(|key| (*key).to_string()).collect(),
        );
        self.sidebar_presentation_cache = SidebarPresentationCache::default();
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

    pub(in super::super) fn saved_sidebar_pinned_branches(
        &self,
    ) -> BTreeMap<std::path::PathBuf, BTreeSet<String>> {
        self.sidebar_pinned_branches_by_repo
            .iter()
            .filter(|&(_repo, items)| !items.is_empty())
            .map(|(repo, items)| (repo.clone(), items.clone()))
            .collect()
    }

    pub(in super::super) fn toggle_pinned_branch(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let repo_path = repo.spec.workdir.clone();
        let key = branch_sidebar::branch_pin_storage_key(section, name);

        let items = self
            .sidebar_pinned_branches_by_repo
            .entry(repo_path.clone())
            .or_default();
        if !items.insert(key.clone()) {
            items.remove(&key);
        }
        if items.is_empty() {
            self.sidebar_pinned_branches_by_repo.remove(&repo_path);
        }

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.schedule_ui_settings_persist(cx);
        self.sync_popover_pinned_branches(cx);
        cx.notify();
    }

    /// Drop the pins the section is actually showing, leaving the other
    /// section's pins alone.
    ///
    /// Scoped to the rendered rows, not every key in the section: the menu
    /// labels itself with `PopoverHost::pinned_branch_count`, which skips a
    /// pin the branch filter excludes and one whose branch is gone. Retaining
    /// by section alone would delete pins the user cannot see under a count
    /// that never mentioned them.
    pub(in super::super) fn unpin_all_branches(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let repo_path = repo.spec.workdir.clone();
        let filter = self.branch_filter_query.as_str();
        let Some(items) = self.sidebar_pinned_branches_by_repo.get_mut(&repo_path) else {
            return;
        };

        let before = items.len();
        items.retain(|key| !branch_sidebar::pinned_branch_renders(repo, key, section, filter));
        if items.len() == before {
            return;
        }
        if items.is_empty() {
            self.sidebar_pinned_branches_by_repo.remove(&repo_path);
        }

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.schedule_ui_settings_persist(cx);
        self.sync_popover_pinned_branches(cx);
        cx.notify();
    }

    /// Mirror the branch filter into the popover host.
    ///
    /// The branch tree is filtered before the group tree is built, so a group
    /// row shows only its matching members. Menus acting on "the branches in
    /// this group" have to see the same filter, or they act on branches that
    /// are not on screen.
    fn sync_popover_branch_filter(&self, cx: &mut gpui::Context<Self>) {
        let query = self.branch_filter_query.clone();
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.popover_host.update(cx, |host, cx| {
                    host.set_branch_filter_query(query, cx);
                });
            });
        });
    }

    /// Mirror the collapse set into the popover host so the branch group menu
    /// can label its Expand/Collapse entry. Deferred for the same reason as
    /// [`Self::sync_popover_pinned_branches`].
    fn sync_popover_collapsed_items(&self, cx: &mut gpui::Context<Self>) {
        let collapsed = self.sidebar_collapsed_items_by_repo.clone();
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.popover_host.update(cx, |host, cx| {
                    host.set_collapsed_items(collapsed, cx);
                });
            });
        });
    }

    /// Mirror the pinned set into the popover host so the branch context menu
    /// can label its pin entry. Deferred because this runs inside the sidebar
    /// pane's own update, and the toggle itself may have been dispatched from
    /// the popover host (which is then mid-update too).
    fn sync_popover_pinned_branches(&self, cx: &mut gpui::Context<Self>) {
        let pinned = self.sidebar_pinned_branches_by_repo.clone();
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.popover_host.update(cx, |host, cx| {
                    host.set_pinned_branches(pinned, cx);
                });
            });
        });
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
        self.apply_active_repo_collapse_key(collapse_key, None, cx);
    }

    /// Drive a collapse key to an explicit state instead of flipping it.
    ///
    /// The pinned sections render force-expanded while a branch filter is live,
    /// no matter what the stored key says, so a menu labelling itself from the
    /// rendered state has to send the state it means — a flip would move the
    /// key the opposite way from the label the user clicked.
    pub(in super::super) fn set_active_repo_collapse_key(
        &mut self,
        collapse_key: SharedString,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_active_repo_collapse_key(collapse_key, Some(collapsed), cx);
    }

    /// Shared body of the two above: `None` flips the key, `Some` drives it.
    fn apply_active_repo_collapse_key(
        &mut self,
        collapse_key: SharedString,
        target: Option<bool>,
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
        match target {
            Some(collapsed) => branch_sidebar::set_collapse_state(items, collapse_key, collapsed),
            None => branch_sidebar::toggle_collapse_state(items, collapse_key),
        }
        if items.is_empty() {
            self.sidebar_collapsed_items_by_repo.remove(&repo_path);
        }
        let expanded_now = self.sidebar_collapsed_items_by_repo.get(&repo_path).map_or(
            !branch_sidebar::is_collapsed(&BTreeSet::new(), collapse_key),
            |items| !branch_sidebar::is_collapsed(items, collapse_key),
        );

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.schedule_ui_settings_persist(cx);
        self.sync_popover_collapsed_items(cx);
        if should_load_submodules_on_expand && expanded_now {
            self.store.dispatch(Msg::LoadSubmodules { repo_id });
        }
        self.dispatch_sidebar_data_request_if_needed(cx);
        cx.notify();
    }

    /// Collapse or expand a branch group together with every group beneath it.
    ///
    /// Branch collapse state is view-owned rather than a store message, so this
    /// is the recursive sibling of [`Self::toggle_active_repo_collapse_key`]
    /// rather than a `Msg`.
    pub(in super::super) fn set_branch_group_collapsed_recursive(
        &mut self,
        section: BranchSection,
        remote: Option<String>,
        path: String,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let path = path.trim();
        if path.is_empty() {
            return;
        }

        let group_paths = match section {
            BranchSection::Local => {
                let names = match &repo.branches {
                    Loadable::Ready(branches) => branches,
                    _ => return,
                };
                branch_sidebar::group_paths_at_or_below(
                    path,
                    names.iter().map(|branch| branch.name.as_str()),
                )
            }
            BranchSection::Remote => {
                let Some(remote) = remote.as_deref() else {
                    return;
                };
                let names = match &repo.remote_branches {
                    Loadable::Ready(branches) => branches,
                    _ => return,
                };
                branch_sidebar::group_paths_at_or_below(
                    path,
                    names
                        .iter()
                        .filter(|candidate| candidate.remote == remote)
                        .map(|branch| branch.name.as_str()),
                )
            }
        };

        let repo_path = repo.spec.workdir.clone();
        let items = self
            .sidebar_collapsed_items_by_repo
            .entry(repo_path.clone())
            .or_default();
        for group_path in &group_paths {
            let key = match section {
                BranchSection::Local => branch_sidebar::local_group_storage_key(group_path),
                BranchSection::Remote => branch_sidebar::remote_group_storage_key(
                    remote.as_deref().unwrap_or_default(),
                    group_path,
                ),
            };
            branch_sidebar::set_collapse_state(items, &key, collapsed);
        }
        if items.is_empty() {
            self.sidebar_collapsed_items_by_repo.remove(&repo_path);
        }

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.schedule_ui_settings_persist(cx);
        self.sync_popover_collapsed_items(cx);
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
            &self.sidebar_pinned_branches_by_repo,
            &self.branch_filter_query,
        )
    }

    pub(in super::super) fn sidebar(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;

        self.apply_pending_file_browser_reveal(cx);
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

    /// Render a single sidebar section as popover content, shown next to the
    /// collapsed rail without expanding the sidebar. Files reuses the file
    /// browser; branch sections render a scoped slice of the branch list.
    pub(in super::super) fn render_collapsed_popover(
        &mut self,
        section: CollapsedSidebarSection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);

        // Only the branch sections carry a filter; Files has its own always-visible
        // search bar, and the remaining sections have nothing to narrow.
        let filterable = section.branch_section().is_some();
        let filter_open = filterable && self.collapsed_popover_filter_open;

        // Every section but Files has the same header menu the expanded sidebar
        // hangs off its section row (add a worktree, stash, submodule, ...). The
        // rail popover shows only the section's rows, so without this button —
        // and the right-click on the panel behind it — those actions would be
        // out of reach while the sidebar is collapsed.
        let section_menu = self
            .active_repo_id()
            .and_then(|repo_id| section.section_menu(repo_id));
        let section_menu_active = section_menu
            .as_ref()
            .is_some_and(|(invoker, _)| self.active_context_menu_invoker.as_ref() == Some(invoker));

        let title = div()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .pl(scaled_px(10.0))
            // Keep the vertical metrics of the plain title so the sections
            // without a toggle (Files especially) lay out exactly as before.
            .pr(scaled_px(6.0))
            .pt(scaled_px(8.0))
            .pb(scaled_px(6.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(scaled_px(12.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child(section.title()),
            )
            .when(filterable, |header| {
                header.child(
                    components::Button::new("collapsed_popover_filter_toggle", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .selected(filter_open)
                        .selected_bg(with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.34 } else { 0.24 },
                        ))
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/zoom.svg",
                            if filter_open {
                                theme.colors.foreground.primary
                            } else {
                                theme.colors.foreground.secondary
                            },
                            scaled_px(13.0),
                        ))
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            this.toggle_collapsed_popover_filter(window, cx);
                        })
                        .w(scaled_px(22.0))
                        .h(scaled_px(22.0))
                        .gitcomet_tooltip(
                            theme,
                            if filter_open {
                                "Hide filter".into()
                            } else {
                                "Filter branches".into()
                            },
                        )
                        .debug_selector(|| "collapsed_popover_filter_toggle".to_string()),
                )
            })
            .when_some(section_menu.clone(), |header, (invoker, kind)| {
                header.child(
                    components::Button::new("collapsed_popover_section_menu", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .selected(section_menu_active)
                        .selected_bg(with_alpha(
                            theme.colors.accent.foreground,
                            if theme.is_dark { 0.34 } else { 0.24 },
                        ))
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/more_vertical.svg",
                            if section_menu_active {
                                theme.colors.foreground.primary
                            } else {
                                theme.colors.foreground.secondary
                            },
                            scaled_px(15.0),
                        ))
                        .on_click(theme, cx, move |this, e, window, cx| {
                            this.activate_context_menu_invoker(invoker.clone(), cx);
                            this.open_popover_at(kind.clone(), e.position(), window, cx);
                        })
                        .w(scaled_px(22.0))
                        .h(scaled_px(22.0))
                        .gitcomet_tooltip(theme, "More actions".into())
                        .debug_selector(|| "collapsed_popover_section_menu".to_string()),
                )
            });

        let filter_bar = filter_open.then(|| self.render_collapsed_popover_filter_bar(theme, cx));

        let divider = div()
            .flex_none()
            .h(px(1.0))
            .w_full()
            .bg(theme.colors.stroke.subtle);

        let is_files = matches!(section, CollapsedSidebarSection::Files);
        let collapsed_popover_scroll = self.collapsed_popover_scroll.clone();

        let surface = div()
            .id("collapsed_sidebar_popover_content")
            .debug_selector(|| "collapsed_sidebar_popover_content".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            // The rows are rendered by this entity, so scrolling must live here;
            // an overflow container in the parent entity cannot measure them.
            .overflow_y_scroll()
            .track_scroll(&collapsed_popover_scroll)
            // Establish the base text color for the popover subtree. Rows that rely
            // on the ambient color (e.g. worktree path labels) would otherwise fall
            // back to the default text style across the panel/entity boundary and
            // render near-black.
            .text_color(theme.colors.foreground.primary)
            .child(title)
            .child(divider)
            .children(filter_bar);

        let content = if is_files {
            self.render_collapsed_popover_file_section(theme, window, cx)
        } else {
            self.render_collapsed_popover_branch_section(section, window, cx)
        };
        let scrollbar = components::Scrollbar::new(
            "collapsed_sidebar_popover_scrollbar",
            collapsed_popover_scroll,
        );
        #[cfg(test)]
        let scrollbar = scrollbar.debug_selector("collapsed_sidebar_popover_scrollbar");

        // Keep the scrollbar outside the moving surface. If it is a child of the
        // surface, GPUI applies the content scroll offset to the track itself.
        div()
            .relative()
            .flex()
            .flex_col()
            .min_h(px(0.0))
            .text_color(theme.colors.foreground.primary)
            .child(surface.child(content))
            .child(scrollbar.render(theme))
            .into_any()
    }

    /// The filter field revealed by the popover header's magnifier. It sits below
    /// the header, above every branch row, and filters both branch sections.
    fn render_collapsed_popover_filter_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let has_query = !self.collapsed_popover_filter_query.trim().is_empty();
        div()
            .flex_none()
            .px(scaled_px(8.0))
            .pt(scaled_px(8.0))
            .pb(scaled_px(2.0))
            .debug_selector(|| "collapsed_popover_filter_bar".to_string())
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_h(scaled_px(28.0))
                    .pl(scaled_px(8.0))
                    .pr(scaled_px(2.0))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.panel)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .py(scaled_px(4.0))
                            .child(self.collapsed_popover_filter_input.clone()),
                    )
                    .when(has_query, |row| {
                        row.child(
                            components::Button::new("collapsed_popover_filter_clear", "")
                                .borderless()
                                .style(components::ButtonStyle::Subtle)
                                .start_slot(crate::view::icons::svg_icon(
                                    "icons/generic_close.svg",
                                    theme.colors.foreground.secondary,
                                    scaled_px(12.0),
                                ))
                                .on_click(theme, cx, |this, _e, _w, cx| {
                                    this.clear_collapsed_popover_filter(cx);
                                })
                                .w(scaled_px(24.0))
                                .h(scaled_px(24.0))
                                .gitcomet_tooltip(theme, "Clear filter".into())
                                .debug_selector(|| "collapsed_popover_filter_clear".to_string()),
                        )
                    }),
            )
    }

    /// Show/hide the popover filter. Opening focuses the field so the user can
    /// type straight away; closing drops the query so the rows come back.
    fn toggle_collapsed_popover_filter(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.collapsed_popover_filter_open = !self.collapsed_popover_filter_open;
        if self.collapsed_popover_filter_open {
            let focus_handle = self.collapsed_popover_filter_input.read(cx).focus_handle();
            window.focus(&focus_handle, cx);
        } else {
            self.clear_collapsed_popover_filter(cx);
        }
        cx.notify();
    }

    fn clear_collapsed_popover_filter(&mut self, cx: &mut gpui::Context<Self>) {
        if self.collapsed_popover_filter_query.is_empty() {
            return;
        }
        self.collapsed_popover_filter_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });
        self.collapsed_popover_filter_query.clear();
        cx.notify();
    }

    /// Reset the popover filter when the rail opens a different section (or the
    /// popover closes), so a stale query never greets the next section.
    pub(in super::super) fn reset_collapsed_popover_filter(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        self.clear_collapsed_popover_filter(cx);
        if self.collapsed_popover_filter_open {
            self.collapsed_popover_filter_open = false;
            cx.notify();
        }
    }

    fn render_collapsed_popover_file_section(
        &mut self,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let search_bar = self.render_file_browser_search_bar(theme, cx);
        let visible_rows = self.file_browser_visible_rows(cx);
        let body: AnyElement = if visible_rows.is_empty() {
            let message = match self.active_repo() {
                None => "No repository selected.",
                Some(repo) => match &repo.file_browser.entries {
                    Loadable::NotLoaded | Loadable::Loading => "Loading files...",
                    Loadable::Ready(entries) if entries.is_empty() => "Empty repository.",
                    Loadable::Ready(_) => "No files visible.",
                    Loadable::Error(_) => "Error loading files.",
                },
            };
            components::empty_state(theme, "Files", message).into_any_element()
        } else {
            let rows = Self::render_file_browser_rows(self, 0..visible_rows.len(), window, cx);
            // Match the branch-section popovers: intrinsic eager rows, with the
            // enclosing popover panel owning the min/max bounds and scrolling.
            div()
                .debug_selector(|| "collapsed_file_browser_rows".to_string())
                .flex()
                .flex_col()
                .pt(px(2.0))
                .pb(px(6.0))
                .pl(px(components::ROW_HIGHLIGHT_INSET_PX))
                .pr(px(components::ROW_HIGHLIGHT_INSET_PX))
                .children(rows)
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .child(search_bar)
            .child(body)
            .into_any_element()
    }

    fn render_collapsed_popover_branch_section(
        &mut self,
        section: CollapsedSidebarSection,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let Some(presentation) = self.build_collapsed_popover_presentation(section) else {
            return components::empty_state(theme, section.title(), "No repository selected.")
                .into_any_element();
        };
        let row_count = presentation.rows.len();
        if row_count == 0 {
            let filtering = self.collapsed_popover_filter_open
                && !self.collapsed_popover_filter_query.trim().is_empty();
            let message = if filtering {
                "No matching branches."
            } else {
                "Nothing here yet."
            };
            return components::empty_state(theme, section.title(), message).into_any_element();
        }

        // Render the scoped rows eagerly (a single section is bounded) so the
        // shared row renderer can reuse the transient presentation override.
        self.collapsed_popover_presentation = Some(presentation);
        let rows = Self::render_branch_sidebar_rows(self, 0..row_count, window, cx);
        self.collapsed_popover_presentation = None;

        // Intrinsic height: the enclosing popover panel sizes to content and owns
        // the scroll, so this just stacks the rows.
        div()
            .flex()
            .flex_col()
            .pt(px(2.0))
            // A little breathing room below the last row (content-sized popovers
            // otherwise sit the last item flush against the bottom border).
            .pb(px(6.0))
            .pl(px(components::ROW_HIGHLIGHT_INSET_PX))
            .pr(px(components::ROW_HIGHLIGHT_INSET_PX))
            .children(rows)
            .into_any_element()
    }

    fn build_collapsed_popover_presentation(
        &mut self,
        section: CollapsedSidebarSection,
    ) -> Option<SidebarPresentation> {
        // Workspace badges are collapse-independent; reuse the cached ones.
        let base = self.branch_sidebar_presentation_cached()?;
        let repo = self.active_repo()?;
        let mut collapsed = self
            .sidebar_collapsed_items_by_repo
            .get(&repo.spec.workdir)
            .cloned()
            .unwrap_or_default();
        // Force-expand the target section so its content is present regardless of
        // the persisted collapse state (which we never mutate here).
        if let Some(key) = section.storage_key()
            && branch_sidebar::is_collapsed(&collapsed, key)
        {
            branch_sidebar::toggle_collapse_state(&mut collapsed, key);
        }
        // Each branch popover surfaces its matching Pinned section, so keep that
        // one expanded regardless of the persisted collapse state.
        let pinned_section = match section {
            CollapsedSidebarSection::Local => Some(BranchSection::Local),
            CollapsedSidebarSection::Remote => Some(BranchSection::Remote),
            _ => None,
        };
        if let Some(pinned_section) = pinned_section {
            let pinned_key = branch_sidebar::pinned_section_storage_key(pinned_section);
            if branch_sidebar::is_collapsed(&collapsed, pinned_key) {
                branch_sidebar::toggle_collapse_state(&mut collapsed, pinned_key);
            }
        }
        let pinned = self
            .sidebar_pinned_branches_by_repo
            .get(&repo.spec.workdir)
            .cloned()
            .unwrap_or_default();
        let query = if self.collapsed_popover_filter_open {
            self.collapsed_popover_filter_query.trim()
        } else {
            ""
        };
        // While filtering, ignore every persisted collapse state: a match hidden
        // inside a collapsed `feat/` group would make the filter look broken.
        let collapsed = if query.is_empty() {
            collapsed
        } else {
            BTreeSet::new()
        };
        let full = branch_sidebar::branch_sidebar_rows(repo, &collapsed, &pinned, query);
        let scoped = if query.is_empty() {
            section_content_rows(&full, section)
        } else {
            filter_result_rows(&full, section)
        };
        Some(SidebarPresentation {
            rows: scoped.into(),
            workspace_badges: base.workspace_badges,
        })
    }

    /// Kick off any lazy data load a section needs before it can render in the
    /// collapsed-rail popover. Worktrees load eagerly, but stashes, submodules,
    /// and the file browser are only fetched when their section is opened.
    pub(in super::super) fn ensure_collapsed_section_data(
        &mut self,
        section: CollapsedSidebarSection,
        _cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        match section {
            CollapsedSidebarSection::Submodules => {
                if matches!(repo.submodules, Loadable::NotLoaded | Loadable::Error(_)) {
                    self.store.dispatch(Msg::LoadSubmodules { repo_id });
                }
            }
            CollapsedSidebarSection::Stashes => {
                self.store.dispatch(Msg::EnsureSidebarData {
                    repo_id,
                    request: SidebarDataRequest {
                        worktrees: true,
                        submodules: false,
                        stashes: true,
                    },
                });
            }
            CollapsedSidebarSection::Worktrees => {
                self.store.dispatch(Msg::EnsureSidebarData {
                    repo_id,
                    request: SidebarDataRequest {
                        worktrees: true,
                        submodules: false,
                        stashes: false,
                    },
                });
            }
            CollapsedSidebarSection::Files => {
                if matches!(
                    repo.file_browser.entries,
                    Loadable::NotLoaded | Loadable::Error(_)
                ) {
                    let source = repo.file_browser.source.clone();
                    self.store
                        .dispatch(Msg::LoadFileBrowser { repo_id, source });
                }
            }
            CollapsedSidebarSection::Local | CollapsedSidebarSection::Remote => {}
        }
    }

    fn render_tab_bar(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let mode = self.state.sidebar_mode;
        // The Files tab's list is the thing pinned to a commit, so the header
        // above it takes the browse tint only while that list is on screen.
        let browsing_files = mode == SidebarMode::Files
            && self
                .active_repo()
                .is_some_and(|r| r.browsing_commit().is_some());
        let bg = if browsing_files {
            crate::theme::historical_header_bg(theme, theme.colors.surface.chrome)
        } else {
            theme.colors.surface.chrome
        };

        let store_branches = Arc::clone(&self.store);
        let store_files = Arc::clone(&self.store);
        // `theme.colors.interaction.hover_background` is nearly identical to the sidebar chrome bg,
        // so use the standard text-tinted overlay that reads on hover.
        let tab_hover_bg = theme.hover_overlay();
        // Lifts the active chip, which already carries `interaction.selected_background` and so
        // cannot show the plain overlay the inactive one uses.
        let tab_active_hover_bg = crate::theme::mix_colors(
            theme.colors.interaction.selected_background,
            theme.colors.foreground.primary,
            if theme.is_dark { 0.08 } else { 0.05 },
        );
        // Same value as `ButtonStyle::Subtle`'s hover border, so the chips match
        // the locate button sharing their strip.
        let tab_hover_border = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.45 } else { 0.32 },
        );

        // The ids are load-bearing, not just for tests: gpui only allocates the
        // element state that makes a `.hover()` repaint the view for *stateful*
        // elements. On a bare `div()` the hover style is computed but nothing
        // ever asks for a new frame, so it never reaches the screen.
        let branches_tab = div()
            .id("sidebar_tab_branches")
            .debug_selector(|| "sidebar_tab_branches".to_string())
            .flex()
            .flex_row()
            .items_center()
            .px(scaled_px(8.0))
            .h(scaled_px(22.0))
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(mode == SidebarMode::Branches, |d| {
                d.bg(theme.colors.interaction.selected_background)
                    .text_color(theme.colors.foreground.primary)
            })
            .when(mode != SidebarMode::Branches, |d| {
                d.bg(gpui::transparent_black())
                    .text_color(theme.colors.foreground.secondary)
            })
            .hover(move |d| {
                let d = d.border_color(tab_hover_border);
                if mode != SidebarMode::Branches {
                    d.bg(tab_hover_bg)
                } else {
                    d.bg(tab_active_hover_bg)
                }
            })
            .cursor(CursorStyle::PointingHand)
            .text_size(scaled_px(12.0))
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
            .id("sidebar_tab_files")
            .debug_selector(|| "sidebar_tab_files".to_string())
            .flex()
            .flex_row()
            .items_center()
            .px(scaled_px(8.0))
            .h(scaled_px(22.0))
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(mode == SidebarMode::Files, |d| {
                // Carry the tint onto the active chip too, so it does not read
                // as a neutral hole punched in a tinted bar.
                d.bg(if browsing_files {
                    crate::theme::historical_header_bg(
                        theme,
                        theme.colors.interaction.selected_background,
                    )
                } else {
                    theme.colors.interaction.selected_background
                })
                .text_color(theme.colors.foreground.primary)
            })
            .when(mode != SidebarMode::Files, |d| {
                d.bg(gpui::transparent_black())
                    .text_color(theme.colors.foreground.secondary)
            })
            .hover(move |d| {
                let d = d.border_color(tab_hover_border);
                if mode != SidebarMode::Files {
                    d.bg(tab_hover_bg)
                } else {
                    d.bg(tab_active_hover_bg)
                }
            })
            .cursor(CursorStyle::PointingHand)
            .text_size(scaled_px(12.0))
            .child("Files")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _e, _window, _cx| {
                    store_files.dispatch(Msg::SetSidebarMode {
                        mode: SidebarMode::Files,
                    });
                }),
            );

        // Shown for the whole time the tree it scrolls is on screen — hiding it
        // when nothing is open made the strip's contents flicker as files were
        // opened and closed. It greys out instead.
        let show_locate = mode == SidebarMode::Files;
        let can_locate = self
            .active_repo()
            .and_then(|repo| repo.open_file_path())
            .is_some();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(scaled_px(2.0))
            .w_full()
            .h(scaled_px(28.0))
            .px(scaled_px(4.0))
            .bg(bg)
            .child(branches_tab)
            .child(files_tab)
            .when(show_locate, |strip| {
                strip.child(
                    components::Button::new("sidebar_locate_open_file", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .disabled(!can_locate)
                        .start_slot(crate::view::icons::svg_icon(
                            "icons/locate.svg",
                            if can_locate {
                                theme.colors.foreground.secondary
                            } else {
                                with_alpha(theme.colors.foreground.secondary, 0.45)
                            },
                            scaled_px(13.0),
                        ))
                        .on_click(theme, cx, |this, _e, _window, cx| {
                            this.locate_open_file(cx);
                        })
                        // Pushed to the far edge so it reads as an action on the
                        // strip rather than a third tab.
                        .ml_auto()
                        .w(scaled_px(22.0))
                        .h(scaled_px(22.0))
                        .gitcomet_tooltip(
                            theme,
                            if can_locate {
                                format!(
                                    "Show the open file in the explorer ({})",
                                    crate::view::shortcut_labels::secondary_shortcut("Shift+L")
                                )
                                .into()
                            } else {
                                SharedString::from("No file is open")
                            },
                        )
                        .debug_selector(|| "sidebar_locate_open_file".to_string()),
                )
            })
    }

    /// Scroll the file explorer to the file the main pane has open, expanding
    /// the folders on the way to it.
    ///
    /// Switches to the Files tab first when the sidebar is showing Branches —
    /// the action is reachable from the menu, the palette and a shortcut, where
    /// the user cannot be assumed to be looking at the tree already.
    pub(in super::super) fn locate_open_file(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let Some(path) = self
            .active_repo()
            .and_then(|repo| repo.open_file_path())
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };

        if self.state.sidebar_mode != SidebarMode::Files {
            self.store.dispatch(Msg::SetSidebarMode {
                mode: SidebarMode::Files,
            });
        }
        self.store.dispatch(Msg::RevealFileBrowserPath {
            repo_id,
            path: path.clone(),
        });
        // The reducer clears the stored query, but the filter input keeps its
        // text — and its `cx.observe` subscription re-dispatches that text on
        // the next notify (the caret blink is enough), refiltering the tree and
        // leaving the reveal permanently unresolved. Clear the input too, so the
        // two agree before that can happen.
        self.file_browser_search_input
            .update(cx, |input: &mut TextInput, cx| {
                if !input.text().is_empty() {
                    input.set_text("", cx);
                }
            });
        // The reducer runs on the store's worker thread, so the expanded set and
        // the row list this scroll indexes into only exist a frame later.
        self.pending_file_browser_reveal = Some(path);
        self.pending_file_browser_reveal_at = Some(std::time::Instant::now());
        cx.notify();
    }

    /// Consume a queued reveal once the store snapshot carrying it has arrived.
    ///
    /// Runs from render rather than a timer: the row list is derived from the
    /// snapshot, so the first frame that can compute the right index is exactly
    /// the first frame that has it.
    fn apply_pending_file_browser_reveal(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(path) = self.pending_file_browser_reveal.clone() else {
            return;
        };
        // A reveal is an answer to something the user just did. If it has not
        // resolved shortly after — they went back to Branches, or typed into the
        // filter again — it is stale, and firing it later would scroll the tree
        // out from under them unasked.
        //
        // Expired on the clock rather than on a frame count: this runs only from
        // `sidebar()`, so collapsing the sidebar stops the frames entirely and a
        // counter would freeze mid-life and fire whenever it was next expanded.
        if self
            .pending_file_browser_reveal_at
            .is_some_and(|at| at.elapsed() > FILE_BROWSER_REVEAL_MAX_WAIT)
        {
            self.pending_file_browser_reveal = None;
            self.pending_file_browser_reveal_at = None;
            return;
        }
        if self.state.sidebar_mode != SidebarMode::Files {
            return;
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        // Still filtered, or the entries have not landed: wait for the frame
        // that has them rather than scrolling to a row that will move.
        if !repo.file_browser.search_query.is_empty() {
            return;
        }
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            return;
        };
        let Some(entry_index) = entries
            .iter()
            .position(|entry| entry.path.as_path() == path.as_path())
        else {
            // The file is not in this tree at all (browsing a commit that never
            // had it). Drop the request rather than retrying every frame.
            self.pending_file_browser_reveal = None;
            self.pending_file_browser_reveal_at = None;
            return;
        };
        let rows = self.file_browser_visible_rows(cx);
        // Matched on the tree entry, not on any row that mentions the path: the
        // pinned section can be showing this very file, and scrolling to *that*
        // row would leave the tree exactly where it was.
        let Some(row_ix) = rows
            .iter()
            .position(|row| row.entry_index() == Some(entry_index))
        else {
            return;
        };
        self.pending_file_browser_reveal = None;
        self.pending_file_browser_reveal_at = None;
        self.file_browser_scroll
            .scroll_to_item(row_ix, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    /// A slim always-visible filter field pinned above the branch tree. It
    /// narrows the Local/Remote (and pinned) sections live; a query force-expands
    /// those sections so matches are always visible.
    fn render_branch_filter_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let has_query = !self.branch_filter_query.trim().is_empty();
        div()
            .px(scaled_px(8.0))
            .pt(scaled_px(8.0))
            .pb(scaled_px(6.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_h(scaled_px(28.0))
                    .pl(scaled_px(8.0))
                    .pr(scaled_px(2.0))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.raised)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .py(scaled_px(4.0))
                            .child(self.branch_filter_input.clone()),
                    )
                    .when(has_query, |row| {
                        row.child(
                            components::Button::new("branch_filter_clear", "")
                                .borderless()
                                .style(components::ButtonStyle::Subtle)
                                .start_slot(crate::view::icons::svg_icon(
                                    "icons/generic_close.svg",
                                    theme.colors.foreground.secondary,
                                    scaled_px(12.0),
                                ))
                                .on_click(theme, cx, |this, _e, _w, cx| {
                                    this.clear_branch_filter(cx);
                                })
                                .w(scaled_px(24.0))
                                .h(scaled_px(24.0))
                                .gitcomet_tooltip(theme, "Clear filter".into())
                                .debug_selector(|| "branch_filter_clear".to_string()),
                        )
                    }),
            )
    }

    fn clear_branch_filter(&mut self, cx: &mut gpui::Context<Self>) {
        self.branch_filter_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });
        self.branch_filter_query.clear();
        self.sync_popover_branch_filter(cx);
        cx.notify();
    }

    fn render_branches_content(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        const SIDEBAR_TOP_INSET_PX: f32 = 2.0;

        let filter_bar = self.render_branch_filter_bar(theme, cx);
        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return div()
                .flex()
                .flex_col()
                .h_full()
                .min_h(px(0.0))
                .child(filter_bar)
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
        // Rows use the full pane width; the scrollbar overlays them (its track
        // is transparent, only the thumb paints while scrolling/hovering).
        let list = div()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(SIDEBAR_TOP_INSET_PX))
            .pl(px(components::ROW_HIGHLIGHT_INSET_PX))
            .pr(px(components::ROW_HIGHLIGHT_INSET_PX))
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
                .auto_hide()
                .render(theme),
            )
            .into_any_element();

        div()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            .child(filter_bar)
            .child(panel_body)
            .into_any()
    }

    fn render_file_browser_search_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);
        let search_options = self.file_search_options;
        let search_query = self
            .active_repo()
            .map(|repo| repo.file_browser.search_query.clone())
            .unwrap_or_default();
        let search_error = file_search_matchers(&search_query, search_options)
            .iter()
            .any(|matcher| matcher.regex_error().is_some());
        let option_selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.34 } else { 0.24 },
        );
        div()
            .px(scaled_px(8.0))
            .pt(scaled_px(8.0))
            .pb(scaled_px(6.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .min_h(scaled_px(28.0))
                    .pl(scaled_px(8.0))
                    .pr(scaled_px(2.0))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(if search_error {
                        theme.colors.status.danger.foreground
                    } else {
                        theme.colors.stroke.default
                    })
                    .bg(theme.colors.surface.raised)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .py(scaled_px(4.0))
                            .child(self.file_browser_search_input.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(scaled_px(28.0))
                            .flex()
                            .items_center()
                            .child(
                                components::Button::new("file_search_match_case", "Aa")
                                    .borderless()
                                    .style(components::ButtonStyle::Subtle)
                                    .selected(search_options.match_case)
                                    .selected_bg(option_selected_bg)
                                    .on_click(theme, cx, |this, _e, _w, cx| {
                                        this.toggle_file_search_option(
                                            |options| options.match_case = !options.match_case,
                                            cx,
                                        );
                                    })
                                    .w(scaled_px(24.0))
                                    .h(scaled_px(24.0))
                                    .gitcomet_tooltip(theme, "Match case".into())
                                    .debug_selector(|| "file_search_match_case".to_string()),
                            )
                            .child(
                                components::Button::new("file_search_whole_word", "W")
                                    .borderless()
                                    .style(components::ButtonStyle::Subtle)
                                    .selected(search_options.whole_word)
                                    .selected_bg(option_selected_bg)
                                    .on_click(theme, cx, |this, _e, _w, cx| {
                                        this.toggle_file_search_option(
                                            |options| options.whole_word = !options.whole_word,
                                            cx,
                                        );
                                    })
                                    .w(scaled_px(24.0))
                                    .h(scaled_px(24.0))
                                    .gitcomet_tooltip(theme, "Match whole word".into())
                                    .debug_selector(|| "file_search_whole_word".to_string()),
                            )
                            .child(
                                components::Button::new("file_search_regex", ".*")
                                    .borderless()
                                    .style(components::ButtonStyle::Subtle)
                                    .selected(search_options.regex)
                                    .selected_bg(option_selected_bg)
                                    .on_click(theme, cx, |this, _e, _w, cx| {
                                        this.toggle_file_search_option(
                                            |options| options.regex = !options.regex,
                                            cx,
                                        );
                                    })
                                    .w(scaled_px(24.0))
                                    .h(scaled_px(24.0))
                                    .gitcomet_tooltip(theme, "Use regular expression".into())
                                    .debug_selector(|| "file_search_regex".to_string()),
                            ),
                    ),
            )
    }

    fn render_file_browser_content(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let search_bar = self.render_file_browser_search_bar(theme, cx);

        let visible_rows = self.file_browser_visible_rows(cx);

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
            // Same overlay-scrollbar treatment as the branches list above.
            let list = div()
                .flex_1()
                .min_h(px(0.0))
                .pt(px(2.0))
                .pl(px(components::ROW_HIGHLIGHT_INSET_PX))
                .pr(px(components::ROW_HIGHLIGHT_INSET_PX))
                .child(list);
            div()
                .id("file_browser_scroll_container")
                .debug_selector(|| "file_browser_scroll_container".to_string())
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
                    .auto_hide()
                    .render(theme),
                )
                .into_any_element()
        };

        let browsing_commit = self
            .active_repo()
            .is_some_and(|r| r.browsing_commit().is_some());
        div()
            .relative()
            .flex()
            .flex_col()
            .h_full()
            .min_h(px(0.0))
            // A wash rather than a frame: browse mode should read as a change of
            // surface, not as a box drawn around the list.
            .when(browsing_commit, |d| {
                // Over `surface.chrome`: that is what the pane behind this paints, so
                // browse mode shifts the hue without also stepping the lightness.
                d.bg(crate::theme::historical_surface_bg(
                    theme,
                    theme.colors.surface.chrome,
                ))
            })
            .child(search_bar)
            .child(body)
            .into_any()
    }

    fn file_browser_visible_rows(&self, cx: &gpui::App) -> Vec<FileBrowserVisibleRow> {
        let Some(repo) = self.active_repo() else {
            return Vec::new();
        };

        // Key on the repo id too: file_browser_rev is a per-repo counter, so two
        // repos can share a value and collide otherwise (stale rows for the wrong
        // tree after switching repos).
        //
        // The unsaved revision has to be in here as well: those buffers live in
        // the main pane, so nothing the store owns — `file_browser_rev`
        // included — moves when one goes dirty, and the cache would keep serving
        // rows without the pinned section.
        let cache_key = (
            repo.id,
            repo.file_browser.file_browser_rev,
            self.file_search_options,
            self.unsaved_file_edits_rev(cx),
            self.unsaved_section_is_collapsed(),
        );
        let mut cache = self.file_browser_rows_cache.borrow_mut();
        if let Some((cached_key, cached_rows)) = cache.as_ref()
            && *cached_key == cache_key
        {
            return cached_rows.to_vec();
        }

        let rows = self.compute_file_browser_visible_rows(repo, self.unsaved_file_edit_paths(cx));
        *cache = Some((cache_key, Rc::from(rows.clone())));
        rows
    }

    /// Paths in the active repo the editor is holding unsaved buffers for.
    ///
    /// Read through the root view because the buffers belong to the main pane,
    /// which is a sibling entity — the same hop the sidebar's click handlers
    /// already make, done here at render time.
    fn unsaved_file_edit_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        let Some(repo_id) = self.active_repo_id() else {
            return Vec::new();
        };
        let Some(root) = self.root_view.upgrade() else {
            return Vec::new();
        };
        root.read(cx)
            .main_pane
            .read(cx)
            .unsaved_file_edit_paths(repo_id)
    }

    fn unsaved_file_edits_rev(&self, cx: &gpui::App) -> u64 {
        self.root_view
            .upgrade()
            .map(|root| root.read(cx).main_pane.read(cx).unsaved_file_edits_rev)
            .unwrap_or(0)
    }

    /// Collapsed state rides in the same per-repo map the branch tree's sections
    /// use, so it persists across sessions the way those do.
    fn unsaved_section_is_collapsed(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            self.sidebar_collapsed_items_by_repo
                .get(&repo.spec.workdir)
                .is_some_and(|items| {
                    branch_sidebar::is_collapsed(items, FILE_BROWSER_UNSAVED_SECTION_KEY)
                })
        })
    }

    fn compute_file_browser_visible_rows(
        &self,
        repo: &RepoState,
        unsaved: Vec<PathBuf>,
    ) -> Vec<FileBrowserVisibleRow> {
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            // Still worth showing the pinned section: the buffers exist whether
            // or not the tree behind them has loaded.
            return self.unsaved_file_edit_rows(unsaved);
        };

        let matchers =
            file_search_matchers(&repo.file_browser.search_query, self.file_search_options);
        let has_search = !matchers.is_empty();

        let mut tree_rows: Vec<FileBrowserVisibleRow> = if has_search {
            let mut matching_entry_indices: std::collections::HashSet<usize> =
                std::collections::HashSet::new();
            let mut ancestor_paths: std::collections::HashSet<Arc<PathBuf>> =
                std::collections::HashSet::new();

            for (i, entry) in entries.iter().enumerate() {
                let path_str = entry.path.to_string_lossy();
                if file_search_matches(&matchers, path_str.as_ref()) {
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
                    FileBrowserVisibleRow::Entry {
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
                    FileBrowserVisibleRow::Entry {
                        entry_index: i,
                        depth: entry.depth,
                        is_directory: entry.kind == FileEntryKind::Directory,
                        is_expanded,
                    }
                })
                .collect()
        };

        let mut rows = self.unsaved_file_edit_rows(unsaved);
        rows.append(&mut tree_rows);
        rows
    }

    /// The pinned section's rows: a header, then one row per unsaved file.
    ///
    /// Empty when nothing is unsaved, so the section costs no vertical space in
    /// the common case, and header-only when it is collapsed.
    fn unsaved_file_edit_rows(&self, unsaved: Vec<PathBuf>) -> Vec<FileBrowserVisibleRow> {
        if unsaved.is_empty() {
            return Vec::new();
        }
        let mut rows = vec![FileBrowserVisibleRow::UnsavedHeader {
            count: unsaved.len(),
        }];
        if !self.unsaved_section_is_collapsed() {
            rows.extend(
                unsaved
                    .into_iter()
                    .map(|path| FileBrowserVisibleRow::UnsavedFile {
                        path: Arc::new(path),
                    }),
            );
        }
        rows
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
        const INDENT_STEP_PX: f32 = 8.0;
        const CHEVRON_SLOT_PX: f32 = 12.0;
        const ICON_SLOT_PX: f32 = 16.0;

        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = |value: f32| ui_scale::design_px_from_percent(value, ui_scale_percent);

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let theme = this.theme;
        let icon_muted = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.6 } else { 0.5 },
        );
        // Zed renders file/folder icons in a neutral, muted tone rather than a
        // bright accent — match that so the tree reads the same way.
        let icon_color = theme.colors.foreground.secondary;
        let text_color = theme.colors.foreground.primary;
        let row_surface = if matches!(
            this.collapsed_popover_section,
            Some(CollapsedSidebarSection::Files)
        ) {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.chrome
        };
        let row_style = components::InteractiveRowStyle::new(theme, row_surface);
        let store = Arc::clone(&this.store);
        let search_matchers = this
            .active_repo()
            .map(|repo| {
                file_search_matchers(&repo.file_browser.search_query, this.file_search_options)
            })
            .unwrap_or_default();

        let visible_rows = this.file_browser_visible_rows(cx);
        let repo = this.active_repo();
        // The file the main pane is showing, so the tree can mark it. Read
        // whatever the target names — a diff of a file is still "this file is
        // open", not only the read-only content view.
        let open_path = repo.and_then(|repo| repo.open_file_path().map(|p| p.to_path_buf()));
        // The same wash a selected branch row wears, so both trees in the
        // sidebar mark "this is the one you are looking at" identically.
        let open_row_bg = selected_branch_row_bg(theme);
        let entries = repo
            .and_then(|r| match &r.file_browser.entries {
                Loadable::Ready(e) => Some(e.as_slice()),
                _ => None,
            })
            .unwrap_or(&[]);
        // A filtered tree renders every directory expanded and never reads
        // `expanded_dirs`, so the reducer refuses the toggle. Drop the row's
        // chevron and its click with it — a control that cannot move is worse
        // than none, and the folder menu greys its Expand/Collapse entries for
        // exactly the same reason. Asked of the query rather than the matchers
        // so this tracks what the reducer will actually honour.
        let expansion_frozen =
            repo.is_some_and(|repo| file_browser_search_is_active(&repo.file_browser.search_query));

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
            let tint = file_icons::file_icon_color(path, theme.is_dark).unwrap_or(icon_color);
            div()
                .w(scaled_px(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, tint, 12.0))
        };

        let icon_slot_tinted = |path: &'static str, tint: gpui::Rgba| {
            div()
                .w(scaled_px(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, tint, 12.0))
        };

        // Looked up per row, so a set rather than the ordered vec the pinned
        // section is built from.
        let unsaved_paths: std::collections::HashSet<PathBuf> =
            this.unsaved_file_edit_paths(cx).into_iter().collect();

        let unsaved_collapsed = this.unsaved_section_is_collapsed();

        range
            .filter_map(|ix| {
                let row = visible_rows.get(ix)?;
                let (entry_index, depth, is_directory, is_expanded) = match row {
                    // The pinned section shares the list with the tree but not
                    // its shape, so both rows are built here and return early.
                    FileBrowserVisibleRow::UnsavedHeader { count } => {
                        return Some(
                            div()
                                .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
                                .debug_selector(|| "file_browser_unsaved_header".to_string())
                                .flex()
                                .flex_row()
                                .items_center()
                                .h(scaled_px(FILE_BROWSER_ROW_HEIGHT_PX))
                                .w_full()
                                .pl(scaled_px(6.0))
                                .pr_2()
                                .gap(scaled_px(4.0))
                                .interactive_row(
                                    row_style,
                                    components::InteractiveRowState::default(),
                                )
                                .on_click(cx.listener(
                                    move |this, _e: &gpui::ClickEvent, _window, cx| {
                                        this.toggle_active_repo_collapse_key(
                                            SharedString::from(FILE_BROWSER_UNSAVED_SECTION_KEY),
                                            cx,
                                        );
                                    },
                                ))
                                .child(chevron_slot(true, !unsaved_collapsed))
                                .child(icon_slot_tinted(
                                    "icons/pencil.svg",
                                    theme.colors.status.warning.foreground,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_xs()
                                        .text_color(theme.colors.foreground.secondary)
                                        .child(format!("Unsaved edits ({count})")),
                                )
                                .into_any_element(),
                        );
                    }
                    FileBrowserVisibleRow::UnsavedFile { path } => {
                        return Some(unsaved_file_row(
                            UnsavedFileRowCtx {
                                theme,
                                row_style,
                                open_row_bg,
                                repo_id,
                                ix,
                                is_open: open_path.as_deref() == Some(path.as_path()),
                            },
                            Arc::clone(path),
                            scaled_px(6.0 + INDENT_STEP_PX),
                            scaled_px(FILE_BROWSER_ROW_HEIGHT_PX),
                            scaled_px(ICON_SLOT_PX),
                            Arc::clone(&store),
                            cx,
                        ));
                    }
                    FileBrowserVisibleRow::Entry {
                        entry_index,
                        depth,
                        is_directory,
                        is_expanded,
                    } => (*entry_index, *depth, *is_directory, *is_expanded),
                };
                let entry = entries.get(entry_index)?;
                let element = {
                    let left_pad = scaled_px(6.0 + INDENT_STEP_PX * depth as f32);
                    let store = Arc::clone(&store);
                    // Files and folders get separate invoker names so the two
                    // menus can never light up each other's row.
                    let menu_invoker = SharedString::from(if is_directory {
                        format!("file_browser_folder_{ix}")
                    } else {
                        format!("file_browser_file_{ix}")
                    });
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&menu_invoker);
                    let is_open_file = !is_directory
                        && open_path
                            .as_ref()
                            .is_some_and(|open| open.as_path() == entry.path.as_path());
                    // The pen marks the file wherever it sits in the tree, so a user
                    // who navigated to it rather than to the pinned section still
                    // sees that it is holding unsaved text.
                    let has_unsaved_edits =
                        !is_directory && unsaved_paths.contains(entry.path.as_ref());
                    let row_state = components::InteractiveRowState::default()
                        .selected(is_open_file, open_row_bg)
                        .open(context_menu_active);

                    let mut row_div = div()
                        .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
                        .debug_selector(move || format!("file_browser_row_{ix}"))
                        .flex()
                        .flex_row()
                        .items_center()
                        .h(scaled_px(FILE_BROWSER_ROW_HEIGHT_PX))
                        .w_full()
                        .pl(left_pad)
                        .pr_2()
                        .gap(scaled_px(4.0))
                        .interactive_row(row_style, row_state);

                    if is_directory {
                        let path = (*entry.path).clone();
                        let menu_path = path.clone();
                        row_div = row_div
                            .when(!expansion_frozen, |row| {
                                row.on_click(cx.listener(
                                    move |_this, _e: &gpui::ClickEvent, _window, _cx| {
                                        store.dispatch(Msg::ToggleFileBrowserDir {
                                            repo_id,
                                            path: path.clone(),
                                        });
                                    },
                                ))
                            })
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, e: &gpui::MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(menu_invoker.clone(), cx);
                                    this.open_popover_at(
                                        PopoverKind::FileBrowserFolderMenu {
                                            repo_id,
                                            path: menu_path.clone(),
                                        },
                                        e.position,
                                        window,
                                        cx,
                                    );
                                }),
                            );
                    } else {
                        let path = (*entry.path).clone();
                        let menu_path = path.clone();
                        let source = repo
                            .map(|r| r.file_browser.source.clone())
                            .unwrap_or(gitcomet_core::domain::FileSource::WorkingDirectory);
                        row_div = row_div
                            .on_click(cx.listener(
                                move |_this, _e: &gpui::ClickEvent, _window, _cx| {
                                    // A file the editor is holding unsaved text for
                                    // opens straight back into the editor. Opening
                                    // the read-only view would show the text on
                                    // disk, which is not what the user left here.
                                    if has_unsaved_edits {
                                        store.dispatch(Msg::OpenFileEditor {
                                            repo_id,
                                            path: path.clone(),
                                        });
                                    } else {
                                        store.dispatch(Msg::OpenFileContent {
                                            repo_id,
                                            source: source.clone(),
                                            path: path.clone(),
                                        });
                                    }
                                },
                            ))
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
                        .child(chevron_slot(is_directory && !expansion_frozen, is_expanded))
                        .child(icon_slot(file_or_folder_icon_path(entry, is_expanded)))
                        .child({
                            let highlight_ranges =
                                file_search_highlight_ranges(&search_matchers, entry.name.as_ref());
                            let mut label = components::TruncatedText::new(entry.name.to_string())
                                .profile(components::TextTruncationProfile::End)
                                .text_color(text_color)
                                .text_sm();
                            if !highlight_ranges.is_empty() {
                                let style = gpui::HighlightStyle {
                                    color: Some(theme.colors.accent.foreground.into()),
                                    font_weight: Some(FontWeight::BOLD),
                                    ..gpui::HighlightStyle::default()
                                };
                                label = label.highlights(
                                    highlight_ranges.into_iter().map(|range| (range, style)),
                                );
                            }
                            div().flex_1().min_w(px(0.0)).child(label.render(cx))
                        })
                        .when(has_unsaved_edits, |row| {
                            row.child(div().flex_none().flex().items_center().child(svg_icon(
                                "icons/pencil.svg",
                                theme.colors.status.warning.foreground,
                                10.0,
                            )))
                        })
                        .into_any_element()
                };
                Some(element)
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

    /// Focus a worktree in the log: its uncommitted-changes row when it has
    /// changes, otherwise the commit its HEAD points at.
    ///
    /// HEAD comes from the worktree listing rather than the dirty scan, because
    /// the scan skips this tab's own worktree and omits clean ones entirely. The
    /// listing is loaded by the time a row in it can be clicked.
    pub(in super::super) fn reveal_worktree_in_history(
        &mut self,
        repo_id: RepoId,
        path: std::path::PathBuf,
        is_current: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let head = self.active_repo().and_then(|repo| match &repo.worktrees {
            Loadable::Ready(worktrees) => worktrees
                .iter()
                .find(|worktree| worktree.path == path)
                .and_then(|worktree| worktree.head.clone()),
            _ => None,
        });
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.main_pane.update(cx, |pane, cx| {
                    pane.reveal_history_worktree(repo_id, path, is_current, head, cx);
                });
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
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        match self.collapsed_popover_section {
            Some(section) => self.render_collapsed_popover(section, window, cx),
            None => self.sidebar(cx).into_any_element(),
        }
    }
}

/// True for any row that begins a top-level sidebar section, used to bound a
/// single section's content when scoping rows for a collapsed-sidebar popover.
fn is_section_header(row: &BranchSidebarRow) -> bool {
    matches!(
        row,
        BranchSidebarRow::PinnedHeader { .. }
            | BranchSidebarRow::SectionHeader { .. }
            | BranchSidebarRow::WorktreesHeader { .. }
            | BranchSidebarRow::SubmodulesHeader { .. }
            | BranchSidebarRow::StashHeader { .. }
    )
}

/// The rows of a single pinned section (header + pinned branches) for the given
/// branch section, used to surface pins at the top of the matching branch
/// popover in the collapsed sidebar.
fn pinned_section_rows(
    rows: &[BranchSidebarRow],
    branch_section: BranchSection,
) -> Vec<BranchSidebarRow> {
    let Some(start) = rows.iter().position(|r| {
        matches!(
            r,
            BranchSidebarRow::PinnedHeader { section, .. } if *section == branch_section
        )
    }) else {
        return Vec::new();
    };
    let end = rows[start + 1..]
        .iter()
        .position(is_section_header)
        .map(|pos| start + 1 + pos)
        .unwrap_or(rows.len());
    rows[start..end]
        .iter()
        .filter(|r| !matches!(r, BranchSidebarRow::SectionSpacer))
        .cloned()
        .collect()
}

fn matches_section_header(row: &BranchSidebarRow, section: CollapsedSidebarSection) -> bool {
    matches!(
        (row, section),
        (
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                ..
            },
            CollapsedSidebarSection::Local,
        ) | (
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                ..
            },
            CollapsedSidebarSection::Remote,
        ) | (
            BranchSidebarRow::WorktreesHeader { .. },
            CollapsedSidebarSection::Worktrees,
        ) | (
            BranchSidebarRow::SubmodulesHeader { .. },
            CollapsedSidebarSection::Submodules,
        ) | (
            BranchSidebarRow::StashHeader { .. },
            CollapsedSidebarSection::Stashes,
        )
    )
}

/// The content rows belonging to `section` (between its header and the next
/// section header), with the header and inter-section spacers dropped — the
/// popover supplies its own title.
fn section_content_rows(
    rows: &[BranchSidebarRow],
    section: CollapsedSidebarSection,
) -> Vec<BranchSidebarRow> {
    // Each branch popover additionally surfaces its matching Pinned section at
    // the top.
    let mut out = match section {
        CollapsedSidebarSection::Local => pinned_section_rows(rows, BranchSection::Local),
        CollapsedSidebarSection::Remote => pinned_section_rows(rows, BranchSection::Remote),
        _ => Vec::new(),
    };

    let Some(start) = rows.iter().position(|r| matches_section_header(r, section)) else {
        return out;
    };
    let end = rows[start + 1..]
        .iter()
        .position(is_section_header)
        .map(|pos| start + 1 + pos)
        .unwrap_or(rows.len());
    out.extend(
        rows[start + 1..end]
            .iter()
            .filter(|r| !matches!(r, BranchSidebarRow::SectionSpacer))
            .cloned(),
    );
    out
}

/// The rows a popover filter shows: the open section's matches first, followed
/// by the other branch section's matches. A branch you are looking for is worth
/// finding whichever list it lives in, so filtering Local also surfaces Remote
/// hits (and vice versa); each half gets a group label once both are present.
fn filter_result_rows(
    rows: &[BranchSidebarRow],
    section: CollapsedSidebarSection,
) -> Vec<BranchSidebarRow> {
    let primary = section_content_rows(rows, section);
    let Some(other) = section.counterpart() else {
        return primary;
    };
    let secondary = section_content_rows(rows, other);
    if secondary.is_empty() {
        return primary;
    }

    let mut out = Vec::with_capacity(primary.len() + secondary.len() + 3);
    if !primary.is_empty()
        && let Some(branch_section) = section.branch_section()
    {
        out.push(BranchSidebarRow::FilterGroupHeader {
            section: branch_section,
        });
        out.extend(primary);
        out.push(BranchSidebarRow::SectionSpacer);
    }
    if let Some(branch_section) = other.branch_section() {
        out.push(BranchSidebarRow::FilterGroupHeader {
            section: branch_section,
        });
    }
    out.extend(secondary);
    out
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

/// One matcher per non-empty query line: lines are OR-alternatives, so a
/// multiline query (via the newline button / Shift+Enter) filters by any of
/// several patterns at once.
fn file_search_matchers(query: &str, options: DiffSearchOptions) -> Vec<DiffSearchMatcher> {
    file_search_query_lines(query)
        .map(|line| DiffSearchMatcher::new(line, options))
        .collect()
}

fn file_search_query_lines(query: &str) -> impl Iterator<Item = &str> {
    query.lines().map(str::trim).filter(|line| !line.is_empty())
}

/// Whether `query` actually filters the file tree.
///
/// Not the same as "non-empty": the search input is multiline and stores what
/// was typed verbatim, so a lone space or newline is a query that yields no
/// matchers and leaves the tree unfiltered. Anything deciding behaviour on
/// "is the tree filtered right now" has to ask this rather than the raw string,
/// or it disagrees with what is on screen.
pub(in crate::view) fn file_browser_search_is_active(query: &str) -> bool {
    file_search_query_lines(query).next().is_some()
}

fn file_search_matches(matchers: &[DiffSearchMatcher], haystack: &str) -> bool {
    matchers.iter().any(|matcher| matcher.is_match(haystack))
}

/// Sorted, de-overlapped match ranges across all query lines, for label
/// highlighting in the results.
/// The row-invariant half of an unsaved-edits row, so the builder below stays
/// under a readable argument count.
struct UnsavedFileRowCtx {
    theme: AppTheme,
    row_style: components::InteractiveRowStyle,
    open_row_bg: gpui::Rgba,
    repo_id: RepoId,
    ix: usize,
    is_open: bool,
}

/// One row of the pinned unsaved-edits section: pen, path, discard.
///
/// Clicking the row opens the file, the way a tree row does — the section is a
/// shortcut to those files, not a separate kind of thing. The discard button is
/// on the row rather than behind a menu because getting rid of a stray buffer is
/// the whole reason the section is worth looking at, and it takes effect
/// immediately: it is only ever shown for a file that has something to discard,
/// and the alternative is one Ctrl+S away.
#[allow(clippy::too_many_arguments)]
fn unsaved_file_row(
    ctx: UnsavedFileRowCtx,
    path: Arc<PathBuf>,
    left_pad: Pixels,
    row_height: Pixels,
    icon_slot_px: Pixels,
    store: Arc<AppStore>,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    let UnsavedFileRowCtx {
        theme,
        row_style,
        open_row_bg,
        repo_id,
        ix,
        is_open,
    } = ctx;
    let icon_px = crate::ui_scale::design_px_from_percent(12.0, 100);
    // The full repo-relative path, not just the file name: two `mod.rs` under
    // different folders are indistinguishable here, and this row is the only
    // place they appear side by side.
    let label = path.display().to_string();
    let open_path = (*path).clone();

    div()
        .id(ElementId::Name(format!("file_browser_row_{ix}").into()))
        .debug_selector(move || format!("file_browser_unsaved_{ix}"))
        .flex()
        .flex_row()
        .items_center()
        .h(row_height)
        .w_full()
        .pl(left_pad)
        .pr_2()
        .gap(px(4.0))
        .interactive_row(
            row_style,
            components::InteractiveRowState::default().selected(is_open, open_row_bg),
        )
        // Straight into the editor, not the read-only view: every row in this
        // section has unsaved text, and the read-only view would show the file
        // on disk instead of what the user was in the middle of writing.
        .on_click(
            cx.listener(move |_this, _e: &gpui::ClickEvent, _window, _cx| {
                store.dispatch(Msg::OpenFileEditor {
                    repo_id,
                    path: open_path.clone(),
                });
            }),
        )
        .child(
            div()
                .w(icon_slot_px)
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(super::super::icons::svg_icon(
                    "icons/pencil.svg",
                    theme.colors.status.warning.foreground,
                    icon_px,
                )),
        )
        .child(
            div().flex_1().min_w(px(0.0)).child(
                components::TruncatedText::new(label)
                    // Path elision keeps the file name, which is what identifies
                    // the row, and drops the folders in the middle.
                    .profile(components::TextTruncationProfile::Path)
                    .text_color(theme.colors.foreground.primary)
                    .text_sm()
                    .render(cx),
            ),
        )
        .child(
            components::Button::new(SharedString::from(format!("unsaved_discard_{ix}")), "")
                .borderless()
                .style(components::ButtonStyle::Subtle)
                .start_slot(super::super::icons::svg_icon(
                    "icons/undo.svg",
                    theme.colors.foreground.secondary,
                    icon_px,
                ))
                .on_click(theme, cx, {
                    let path = Arc::clone(&path);
                    move |this, _e, _window, cx| {
                        let path = Arc::clone(&path);
                        let _ = this.root_view.update(cx, move |root, cx| {
                            root.main_pane.update(cx, |pane, cx| {
                                pane.discard_file_edits_for(repo_id, path.as_path(), cx);
                            });
                        });
                    }
                })
                .w(icon_slot_px)
                .h(icon_slot_px)
                .debug_selector(move || format!("file_browser_unsaved_discard_{ix}"))
                .gitcomet_tooltip(
                    theme,
                    "Throw away the unsaved changes and reload this file from disk".into(),
                ),
        )
        .into_any_element()
}

fn file_search_highlight_ranges(
    matchers: &[DiffSearchMatcher],
    name: &str,
) -> Vec<std::ops::Range<usize>> {
    const MAX_NAME_HIGHLIGHTS: usize = 16;
    let mut ranges = Vec::new();
    let mut buf = Vec::new();
    for matcher in matchers {
        matcher.find_ranges_into(name, &mut buf, MAX_NAME_HIGHLIGHTS);
        ranges.extend(buf.iter().cloned());
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<std::ops::Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

#[cfg(test)]
mod file_search_tests {
    use super::*;

    fn options(match_case: bool, whole_word: bool, regex: bool) -> DiffSearchOptions {
        DiffSearchOptions {
            match_case,
            whole_word,
            regex,
        }
    }

    #[test]
    fn default_search_is_case_insensitive_substring() {
        let matchers = file_search_matchers("Read", options(false, false, false));
        assert!(file_search_matches(&matchers, "src/reader.rs"));
        assert!(file_search_matches(&matchers, "README.md"));
        assert!(!file_search_matches(&matchers, "src/writer.rs"));
    }

    #[test]
    fn match_case_narrows_matches() {
        let matchers = file_search_matchers("READ", options(true, false, false));
        assert!(file_search_matches(&matchers, "README.md"));
        assert!(!file_search_matches(&matchers, "src/reader.rs"));
    }

    #[test]
    fn whole_word_requires_boundaries() {
        let matchers = file_search_matchers("read", options(false, true, false));
        assert!(file_search_matches(&matchers, "src/read.rs"));
        assert!(!file_search_matches(&matchers, "src/reader.rs"));
    }

    #[test]
    fn regex_mode_matches_patterns_and_reports_errors() {
        let matchers = file_search_matchers(r"re.d\.rs$", options(false, false, true));
        assert!(file_search_matches(&matchers, "src/read.rs"));
        assert!(!file_search_matches(&matchers, "src/read.rs.bak"));

        let broken = file_search_matchers("re(", options(false, false, true));
        assert!(broken[0].regex_error().is_some());
        assert!(!file_search_matches(&broken, "src/re(.rs"));
    }

    #[test]
    fn each_query_line_is_an_alternative() {
        let matchers = file_search_matchers("reader\nwriter\n\n", options(false, false, false));
        assert_eq!(matchers.len(), 2);
        assert!(file_search_matches(&matchers, "src/reader.rs"));
        assert!(file_search_matches(&matchers, "src/writer.rs"));
        assert!(!file_search_matches(&matchers, "src/printer.rs"));
    }

    #[test]
    fn highlight_ranges_are_sorted_and_merged() {
        let matchers = file_search_matchers("read\neader", options(false, false, false));
        let ranges = file_search_highlight_ranges(&matchers, "reader.rs");
        assert_eq!(ranges, vec![0..6]);

        let matchers = file_search_matchers("r", options(false, false, false));
        let ranges = file_search_highlight_ranges(&matchers, "reader.rs");
        assert_eq!(ranges, vec![0..1, 5..6, 7..8]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Three copies of "is the tree filtered" exist: this one, the reducer's
    /// `file_browser_is_filtered`, and the renderer's `file_search_matchers`.
    /// They have to agree or the reducer freezes toggles the tree still honours.
    #[test]
    fn file_browser_search_predicate_agrees_with_the_renderers_matchers() {
        let options = DiffSearchOptions::default();
        for query in [
            "", " ", "\n", "  \n \t ", "a", " a ", "a\nb", "\na", "#comment",
        ] {
            assert_eq!(
                file_browser_search_is_active(query),
                !file_search_matchers(query, options).is_empty(),
                "predicates disagree for {query:?}"
            );
        }
    }

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

    fn repo_with_local_and_remote_branches() -> RepoState {
        let mut repo = repo_state(RepoId(1), "/tmp/repo");
        repo.branches = Loadable::Ready(Arc::new(vec![
            gitcomet_core::domain::Branch {
                name: "feature/alpha".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: None,
                divergence: None,
            },
            gitcomet_core::domain::Branch {
                name: "main".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: None,
                divergence: None,
            },
        ]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![
            gitcomet_core::domain::RemoteBranch {
                remote: "origin".to_string(),
                name: "feature/beta".to_string(),
                target: CommitId("deadbeef".into()),
            },
            gitcomet_core::domain::RemoteBranch {
                remote: "origin".to_string(),
                name: "release".to_string(),
                target: CommitId("deadbeef".into()),
            },
        ]));
        repo
    }

    fn filter_rows(query: &str, section: CollapsedSidebarSection) -> Vec<BranchSidebarRow> {
        let repo = repo_with_local_and_remote_branches();
        let full =
            branch_sidebar::branch_sidebar_rows(&repo, &BTreeSet::new(), &BTreeSet::new(), query);
        filter_result_rows(&full, section)
    }

    fn branch_names(rows: &[BranchSidebarRow]) -> Vec<String> {
        rows.iter()
            .filter_map(|row| match row {
                BranchSidebarRow::Branch { name, .. } => Some(name.to_string()),
                _ => None,
            })
            .collect()
    }

    fn group_header_sections(rows: &[BranchSidebarRow]) -> Vec<BranchSection> {
        rows.iter()
            .filter_map(|row| match row {
                BranchSidebarRow::FilterGroupHeader { section } => Some(*section),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn collapsed_popover_filter_surfaces_the_other_branch_section() {
        let rows = filter_rows("feature", CollapsedSidebarSection::Local);

        assert_eq!(
            group_header_sections(&rows),
            vec![BranchSection::Local, BranchSection::Remote],
            "a cross-section match should label the open section then the other one"
        );
        assert_eq!(
            branch_names(&rows),
            vec![
                "feature/alpha".to_string(),
                "origin/feature/beta".to_string()
            ],
            "local matches lead, remote matches follow"
        );

        // Symmetric: the Remote popover surfaces local matches the same way.
        let rows = filter_rows("feature", CollapsedSidebarSection::Remote);
        assert_eq!(
            group_header_sections(&rows),
            vec![BranchSection::Remote, BranchSection::Local]
        );
        assert_eq!(
            branch_names(&rows),
            vec![
                "origin/feature/beta".to_string(),
                "feature/alpha".to_string()
            ]
        );
    }

    #[test]
    fn collapsed_popover_filter_stays_unlabelled_when_only_one_section_matches() {
        let rows = filter_rows("main", CollapsedSidebarSection::Local);

        assert!(
            group_header_sections(&rows).is_empty(),
            "a single-section result needs no group labels; the popover title says it"
        );
        assert_eq!(branch_names(&rows), vec!["main".to_string()]);
    }

    #[test]
    fn collapsed_popover_filter_shows_only_the_other_section_when_the_open_one_misses() {
        let rows = filter_rows("release", CollapsedSidebarSection::Local);

        assert_eq!(
            group_header_sections(&rows),
            vec![BranchSection::Remote],
            "with no local matches only the remote half is labelled"
        );
        assert_eq!(branch_names(&rows), vec!["origin/release".to_string()]);
    }
}
