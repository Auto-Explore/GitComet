mod contributions;
mod header;

pub(in crate::view) struct SidebarHeaderChanged;

impl gpui::EventEmitter<SidebarHeaderChanged> for SidebarPaneView {}
use super::super::branch_sidebar::{BranchSection, BranchSidebarRow};
use super::super::caches::BranchSidebarFingerprint;
use super::super::file_icons;
use super::super::sidebar_presentation::{
    SidebarPresentation, SidebarPresentationCache, SidebarRequestFingerprint,
};
use super::super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_core::domain::{FileEntry, FileEntryKind, LogScope};
use gitcomet_state::model::{Loadable, SidebarDataRequest, SidebarMode};
use gitcomet_state::msg::Msg;
use rustc_hash::{FxHashSet, FxHasher};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crate::kit::TextInput;
use crate::kit::TextInputOptions;
use crate::view::components::InteractiveRowExt as _;
use gitcomet_core::text_search::{TextSearchMatcher, TextSearchOptions};
pub(in crate::view) mod explorer_operations;
pub(in crate::view) use explorer_operations::ExplorerAction;
mod file_browser_status;
use file_browser_status::FileBrowserStatusCache;
// File rows borrow the branch tree's row height: one rhythm for both lists.
use crate::view::rows::sidebar::{sidebar_list_row_height, sidebar_list_row_height_px};

type FileBrowserRowsCache = std::cell::RefCell<
    Option<(
        (RepoId, u64, TextSearchOptions, u64, bool),
        Rc<[FileBrowserVisibleRow]>,
    )>,
>;
type ExplorerCutCache = std::cell::RefCell<Option<(u64, Option<Rc<[PathBuf]>>)>>;

type FileSearchMatcherCache =
    std::cell::RefCell<Option<(String, TextSearchOptions, Rc<Vec<TextSearchMatcher>>)>>;

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
    NameEntry {
        depth: usize,
    },
    /// Header of the unsaved-edits section. Click toggles the section.
    FileSetHeader {
        count: usize,
    },
    /// A file with an unsaved editor buffer, shown by its full repo-relative
    /// path since it is out of its folder here.
    FileSetFile {
        path: Arc<PathBuf>,
        open: gitcomet_extension_api::HostedAction,
    },
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

/// How long a queued reveal may wait for the expanded rows it needs. Generous
/// enough for the store round trip, short enough that a request the user has
/// moved on from never fires.
const FILE_BROWSER_REVEAL_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// The checked-out local branch and its tip, but only once both pieces of
/// sidebar data agree. A detached `HEAD` and a branch list that has not landed
/// yet deliberately leave the locate action disabled.
fn active_local_branch_target(repo: &RepoState) -> Option<(&str, &CommitId)> {
    let Loadable::Ready(head) = &repo.head_branch else {
        return None;
    };
    if head == "HEAD" {
        return None;
    }
    let Loadable::Ready(branches) = &repo.branches else {
        return None;
    };
    branches
        .iter()
        .find(|branch| branch.name.as_str() == head.as_str())
        .map(|branch| (head.as_str(), &branch.target))
}

/// Expand the Local section and every possible slash-group on the path to a
/// branch. Including the full branch name matters when a ref is both a leaf and
/// a group (`feature` alongside `feature/one`); for an ordinary leaf its key is
/// simply absent and this is a no-op.
fn expand_local_branch_path(collapsed_items: &mut BTreeSet<String>, branch_name: &str) -> bool {
    let mut changed = false;
    let mut expand = |key: &str| {
        if branch_sidebar::is_collapsed(collapsed_items, key) {
            branch_sidebar::set_collapse_state(collapsed_items, key, false);
            changed = true;
        }
    };

    expand(branch_sidebar::local_section_storage_key());
    let mut group_path = String::new();
    for segment in branch_name.split('/') {
        if !group_path.is_empty() {
            group_path.push('/');
        }
        group_path.push_str(segment);
        expand(&branch_sidebar::local_group_storage_key(&group_path));
    }
    changed
}

/// Prefer the branch's row in the Local tree over a duplicate in the Pinned
/// section, which is emitted earlier in the presentation.
fn local_branch_home_row_index(rows: &[BranchSidebarRow], branch_name: &str) -> Option<usize> {
    rows.iter().rposition(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch {
                name,
                section: BranchSection::Local,
                ..
            } if name.as_ref() == branch_name
        )
    })
}

fn file_browser_row_label_color(theme: AppTheme, selected: bool) -> gpui::Rgba {
    if selected {
        selected_branch_label_color(theme)
    } else {
        theme.colors.foreground.primary
    }
}

/// A section of the sidebar that gets its own icon in the collapsed rail and,
/// when clicked, opens in a floating popover without expanding the sidebar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum CollapsedSidebarSection {
    Local,
    Remote,
    Worktrees,
    Submodules,
    Annex,
    Stashes,
    Files,
}

impl CollapsedSidebarSection {
    /// Rail order, top to bottom, matching the expanded sidebar.
    pub(in crate::view) const ALL: [Self; 7] = [
        Self::Local,
        Self::Remote,
        Self::Worktrees,
        Self::Submodules,
        Self::Annex,
        Self::Stashes,
        Self::Files,
    ];

    /// Whether the rail shows this section for `repo`. The git-annex section
    /// exists only where annex is in use, like its expanded counterpart.
    pub(in crate::view) fn is_available(self, repo: Option<&RepoState>) -> bool {
        match self {
            Self::Annex => repo.is_some_and(repo_uses_annex),
            _ => true,
        }
    }

    pub(in crate::view) fn icon_path(self) -> &'static str {
        match self {
            Self::Local => "icons/computer.svg",
            Self::Remote => "icons/cloud.svg",
            Self::Worktrees => "icons/git_worktree.svg",
            Self::Submodules => "icons/box.svg",
            Self::Annex => "icons/disk.svg",
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
            Self::Annex => "git-annex",
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
            Self::Annex => "collapsed_sidebar_icon_annex",
            Self::Stashes => "collapsed_sidebar_icon_stashes",
            Self::Files => "collapsed_sidebar_icon_files",
        }
    }

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
            Self::Annex => (
                format!("annex_section_menu_{}", repo_id.0),
                PopoverKind::annex(repo_id, AnnexPopoverKind::SectionMenu),
            ),
            Self::Stashes => (
                format!("stash_section_menu_{}", repo_id.0),
                PopoverKind::StashPrompt,
            ),
            Self::Files => (
                format!("explorer_settings_menu_{}", repo_id.0),
                PopoverKind::ExplorerSettingsMenu { repo_id },
            ),
        };
        Some((invoker.into(), kind))
    }

    fn storage_key(self) -> Option<&'static str> {
        match self {
            Self::Local => Some(branch_sidebar::local_section_storage_key()),
            Self::Remote => Some(branch_sidebar::remote_section_storage_key()),
            Self::Worktrees => Some(branch_sidebar::worktrees_section_storage_key()),
            Self::Submodules => Some(branch_sidebar::submodules_section_storage_key()),
            Self::Annex => Some(branch_sidebar::annex_section_storage_key()),
            Self::Stashes => Some(branch_sidebar::stash_section_storage_key()),
            Self::Files => None,
        }
    }
}

/// Extension tabs after Branches and Files for the active repository, as
/// the window routes them.
#[derive(Clone)]
pub(in crate::view) struct SidebarExtensionTabs {
    /// Each listed tab's position among the registered ones, and its title.
    pub(in crate::view) tabs: Vec<(usize, SharedString, Option<SharedString>)>,
    pub(in crate::view) selected: Option<usize>,
    pub(in crate::view) host_tabs: bool,
    /// The selected tab's view, once built.
    pub(in crate::view) view: Option<gpui::AnyView>,
}

impl SidebarExtensionTabs {
    /// The selected tab, when its view is there to show.
    fn showing(&self) -> Option<usize> {
        self.selected.filter(|_| self.view.is_some())
    }

    fn same(&self, other: &Self) -> bool {
        self.host_tabs == other.host_tabs
            && self.tabs == other.tabs
            && self.selected == other.selected
            && self.view.as_ref().map(gpui::AnyView::entity_id)
                == other.view.as_ref().map(gpui::AnyView::entity_id)
    }
}

pub(in super::super) struct SidebarPaneView {
    /// Tabs contributed after Branches and Files; `None` without any.
    extension_tabs: Option<SidebarExtensionTabs>,
    /// The on-demand file history tab; it has content while its pane has a target.
    file_history_pane: Entity<panes::file_history::FileHistoryPaneView>,
    pub(in crate::view) file_history_selected: bool,
    _file_history_subscription: gpui::Subscription,
    explorer_focus: gpui::FocusHandle,
    /// Input events can arrive before the store publishes the preceding click.
    explorer_pending_focus: Option<(RepoId, PathBuf)>,
    explorer_name_input: Entity<TextInput>,
    explorer_name_edit: Option<explorer_operations::NameEdit>,
    /// Cut paths mirrored from the clipboard, keyed on
    /// `clipboard::files_revision`. The row builder used to read the platform
    /// clipboard itself, which on Linux/X11 is a synchronous selection transfer
    /// -- once per prepaint, and gpui re-renders on every mouse move during a
    /// drag.
    explorer_cut_cache: ExplorerCutCache,
    explorer_drop_target: Option<PathBuf>,
    explorer_drop_region: std::cell::RefCell<Option<explorer_operations::DropRegion>>,
    explorer_drag_repo: Option<RepoId>,
    /// The row that claimed `explorer_drop_target`. Rows are not one-to-one
    /// with destinations, so only this identifies the claimant.
    explorer_drop_row: Option<PathBuf>,
    explorer_hover_task: Option<gpui::Task<()>>,
    explorer_scroll_task: Option<gpui::Task<()>>,
    explorer_empty_bounds: Rc<Cell<Option<gpui::Bounds<Pixels>>>>,
    contributions: Option<contributions::SidebarContributions>,
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    sidebar_focus: gpui::FocusHandle,
    _ui_model_subscription: gpui::Subscription,
    documents: crate::view::documents::buffers::DocumentOwner,
    _documents_subscription: gpui::Subscription,
    // Only one surface is mounted at a time. Park the other handle when
    // switching between the expanded tree and a rail popover.
    branches_scroll: UniformListScrollHandle,
    inactive_branches_scroll: UniformListScrollHandle,
    sticky_context: Option<sticky::StickyContext>,
    sidebar_selection_cache: Option<sticky::SelectionCache>,
    pending_sidebar_navigation: Option<sticky::NavigationTarget>,
    sidebar_scroll_animation: Option<sticky::ScrollAnimation>,
    expanded_branches_visible: bool,
    file_browser_scroll: UniformListScrollHandle,
    file_browser_search_input: Entity<TextInput>,
    _search_input_subscription: gpui::Subscription,
    /// Live filter for the branch sidebar (Local/Remote/pinned sections). The
    /// input entity owns the text; `branch_filter_query` mirrors it for the row
    /// builder, kept in sync by `_branch_filter_subscription`.
    branch_filter_input: Entity<TextInput>,
    pub(in super::super) branch_filter_query: String,
    _branch_filter_subscription: gpui::Subscription,
    branch_search_open: bool,
    pub(in crate::view) file_search_open: bool,
    branch_search_options: TextSearchOptions,
    file_matchers_cache: FileSearchMatcherCache,
    file_match_count_cache: std::cell::RefCell<Option<(Rc<[FileBrowserVisibleRow]>, usize)>>,
    section_locator_cache: std::cell::RefCell<Option<search::SectionLocatorCache>>,
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
    // A branch can appear in several pinned groups and as an individual pin.
    // Keep the clicked copy's stable row key; None selects the tree copy.
    selected_branch_pin_key: Option<SharedString>,
    file_search_options: TextSearchOptions,
    file_browser_rows_cache: FileBrowserRowsCache,
    file_browser_status_cache: std::cell::RefCell<FileBrowserStatusCache>,
    /// When set (and the sidebar is collapsed), this pane renders only the given
    /// section as popover content instead of the full sidebar. The root view
    /// syncs this to its `sidebar_collapsed_popover` before embedding the pane.
    pub(in crate::view) collapsed_popover_section: Option<CollapsedSidebarSection>,
    /// A file the explorer has been asked to scroll to, held until the store
    /// snapshot with its folders expanded arrives.
    pending_file_browser_reveal: Option<std::path::PathBuf>,
    /// When the request was made. Bounded so an unresolvable reveal expires
    /// instead of firing at some unrelated later moment.
    pending_file_browser_reveal_at: Option<std::time::Instant>,
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) render_count: usize,
    #[cfg(any(test, feature = "benchmarks"))]
    pub(in crate::view) rendered_rows: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SidebarNotifyFingerprint {
    sidebar_mode: SidebarMode,
    active_repo_id: Option<RepoId>,
    repository_mode: Option<gitcomet_core::domain::RepositoryMode>,
    repo_fingerprint: Option<BranchSidebarFingerprint>,
    open_repo_workdirs_count: usize,
    open_repo_workdirs_hash: u64,
    active_worktree_badges_count: usize,
    active_worktree_badges_hash: u64,
    file_browser_rev: u64,
    /// The file the main pane has open. The tree highlights it, so the sidebar
    /// has to repaint when it changes — nothing else in this fingerprint moves
    /// when the user opens a different file.
    diff_target_rev: u64,
    /// The focused refs. The sidebar draws them in the accent colour, gives
    /// every branch row an eye toggle and puts a banner above the branch list,
    /// and none of that is part of the cached row data — it is all computed at
    /// render time — so the pane has to be told to repaint when the set
    /// changes. Hashed because this fingerprint is `Copy`.
    history_focus_hash: u64,
    status_revs: (u64, u64),
    selected_commit: Option<CommitId>,
}

impl SidebarNotifyFingerprint {
    #[cfg(test)]
    fn from_state(state: &AppState) -> Self {
        Self::from_state_with_cache(state, &mut SidebarPresentationCache::default())
    }

    fn from_state_with_cache(state: &AppState, cache: &mut SidebarPresentationCache) -> Self {
        let active_repo_id = state.active_repo;
        let repo = active_repo_id.and_then(|repo_id| state.repos.iter().find(|r| r.id == repo_id));
        let repo_fingerprint = repo.map(BranchSidebarFingerprint::from_repo);
        let (open_repo_workdirs_count, open_repo_workdirs_hash) =
            open_repo_workdirs_fingerprint(state);
        let (active_worktree_badges_count, active_worktree_badges_hash) =
            cache.active_worktree_badges_fingerprint(state);
        let file_browser_rev = repo.map(|r| r.file_browser.file_browser_rev).unwrap_or(0);
        let diff_target_rev = repo.map(|r| r.diff_state.diff_target_rev).unwrap_or(0);
        let status_revs = repo
            .map(|r| (r.worktree_status_cache_rev(), r.staged_status_cache_rev()))
            .unwrap_or_default();
        let history_focus_hash = repo
            .map(|r| {
                let mut hasher = FxHasher::default();
                r.history_state.history_focus.hash(&mut hasher);
                hasher.finish()
            })
            .unwrap_or(0);
        Self {
            sidebar_mode: state.sidebar_mode,
            active_repo_id,
            repository_mode: repo.and_then(|repo| repo.repository_mode),
            repo_fingerprint,
            open_repo_workdirs_count,
            open_repo_workdirs_hash,
            active_worktree_badges_count,
            active_worktree_badges_hash,
            file_browser_rev,
            diff_target_rev,
            history_focus_hash,
            status_revs,
            selected_commit: repo.and_then(|repo| repo.history_state.selected_commit.clone()),
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
        file_history_pane: Entity<panes::file_history::FileHistoryPaneView>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let file_history_subscription = cx.observe(&file_history_pane, |_, _, cx| {
            cx.emit(SidebarHeaderChanged);
            cx.notify();
        });
        let state = Arc::clone(&ui_model.read(cx).state);
        let documents = ui_model.read(cx).documents.clone();
        let documents_subscription = cx.observe(documents.changes(), |_, _, cx| cx.notify());
        let mut sidebar_presentation_cache = SidebarPresentationCache::default();
        let initial_fingerprint = SidebarNotifyFingerprint::from_state_with_cache(
            &state,
            &mut sidebar_presentation_cache,
        );
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = SidebarNotifyFingerprint::from_state_with_cache(
                &next,
                &mut this.sidebar_presentation_cache,
            );
            let should_notify = next_fingerprint != this.notify_fingerprint;
            let repo_changed =
                this.notify_fingerprint.active_repo_id != next_fingerprint.active_repo_id;

            this.notify_fingerprint = next_fingerprint;
            if this.state.sidebar_mode != next.sidebar_mode {
                this.sidebar_scroll_animation = None;
                this.pending_sidebar_navigation = None;
            }
            this.sync_shared_sidebar_preferences(&next, cx);
            this.state = next;
            if this
                .explorer_pending_focus
                .as_ref()
                .is_some_and(|(id, path)| {
                    this.active_repo_id() != Some(*id)
                        || this.active_repo().is_some_and(|repo| {
                            repo.file_browser.selection.focused.as_ref() == Some(path)
                        })
                })
            {
                this.explorer_pending_focus = None;
            }
            if selected_remote_branch_is_missing(&this.state, this.selected_branch.as_ref()) {
                this.selected_branch = None;
                this.selected_branch_pin_key = None;
            }
            this.dispatch_sidebar_data_request_if_needed(cx);

            // Reflect the newly-active repo's stored search query in the input.
            // Guarded by repo change so it never fights per-keystroke edits.
            if repo_changed {
                this.sticky_context = None;
                this.pending_sidebar_navigation = None;
                this.sidebar_scroll_animation = None;
                this.branches_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                this.inactive_branches_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
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
                    leading_icon: Some("icons/zoom.svg"),
                    chromeless: true,
                    multiline: true,
                    ..Default::default()
                },
                cx,
            )
        });
        let store_for_search = Arc::clone(&store);
        let search_input_subscription = cx.subscribe(
            &file_browser_search_input,
            move |this, input, _: &crate::kit::TextInputChanged, cx| {
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
            },
        );

        let branch_filter_input = cx.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Search sidebar...".into(),
                    leading_icon: Some("icons/zoom.svg"),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            )
        });
        let branch_filter_subscription = cx.subscribe(
            &branch_filter_input,
            move |this, input, _: &crate::kit::TextInputChanged, cx| {
                // The input owns its text (uncontrolled); mirror it into the
                // local query used by the row builder, never writing back.
                let text = input.read(cx).text().to_string();
                if this.branch_filter_query != text {
                    this.sticky_context = None;
                    this.pending_sidebar_navigation = None;
                    this.sidebar_scroll_animation = None;
                    this.branch_filter_query = text;
                    this.branches_scroll
                        .scroll_to_item(0, gpui::ScrollStrategy::Top);
                    this.sync_popover_branch_filter(cx);
                    cx.notify();
                }
            },
        );

        let mut this = Self {
            explorer_focus: cx.focus_handle(),
            explorer_pending_focus: None,
            explorer_name_input: cx.new(|cx| {
                TextInput::new_inert(
                    TextInputOptions {
                        placeholder: "Name".into(),
                        chromeless: true,
                        ..Default::default()
                    },
                    cx,
                )
            }),
            explorer_name_edit: None,
            explorer_cut_cache: std::cell::RefCell::new(None),
            explorer_drop_target: None,
            explorer_drop_region: Default::default(),
            explorer_drag_repo: None,
            explorer_drop_row: None,
            explorer_hover_task: None,
            explorer_scroll_task: None,
            explorer_empty_bounds: Rc::new(Cell::new(None)),
            contributions: contributions::SidebarContributions::new(cx),
            store,
            state,
            theme,
            sidebar_focus: cx.focus_handle(),
            _ui_model_subscription: subscription,
            documents,
            _documents_subscription: documents_subscription,
            branches_scroll: UniformListScrollHandle::default(),
            inactive_branches_scroll: UniformListScrollHandle::default(),
            sticky_context: None,
            sidebar_selection_cache: None,
            pending_sidebar_navigation: None,
            sidebar_scroll_animation: None,
            expanded_branches_visible: false,
            file_browser_scroll: UniformListScrollHandle::default(),
            file_browser_search_input,
            _search_input_subscription: search_input_subscription,
            branch_filter_input,
            branch_filter_query: String::new(),
            _branch_filter_subscription: branch_filter_subscription,
            branch_search_open: false,
            file_search_open: false,
            branch_search_options: TextSearchOptions::default(),
            file_matchers_cache: Default::default(),
            file_match_count_cache: Default::default(),
            section_locator_cache: Default::default(),
            sidebar_presentation_cache,
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            sidebar_collapsed_items_by_repo,
            sidebar_pinned_branches_by_repo,
            extension_tabs: None,
            file_history_pane,
            file_history_selected: false,
            _file_history_subscription: file_history_subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            sidebar_request_fingerprint: SidebarRequestFingerprint::default(),
            active_context_menu_invoker: None,
            selected_branch: None,
            selected_branch_pin_key: None,
            file_search_options: TextSearchOptions::default(),
            file_browser_rows_cache: std::cell::RefCell::new(None),
            file_browser_status_cache: Default::default(),
            collapsed_popover_section: None,
            pending_file_browser_reveal: None,
            pending_file_browser_reveal_at: None,
            #[cfg(any(test, feature = "benchmarks"))]
            render_count: 0,
            #[cfg(any(test, feature = "benchmarks"))]
            rendered_rows: 0,
        };
        let initial_state = Arc::clone(&this.state);
        this.state = Arc::new(AppState::default());
        this.sync_shared_sidebar_preferences(&initial_state, cx);
        this.state = initial_state;
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
    /// Notifies on change: the expanded pane is mounted as a cached view, which
    /// only re-renders when it is marked dirty, so a silent field change could
    /// leave a stale frame on screen.
    pub(in super::super) fn set_collapsed_popover_section(
        &mut self,
        section: Option<CollapsedSidebarSection>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.collapsed_popover_section == section {
            return;
        }
        if self.collapsed_popover_section.is_some() != section.is_some() {
            std::mem::swap(
                &mut self.branches_scroll,
                &mut self.inactive_branches_scroll,
            );
        }
        self.sidebar_scroll_animation = None;
        self.collapsed_popover_section = section;
        self.sticky_context = None;
        if section.is_some() {
            self.branches_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }
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
        if !query.trim().is_empty() {
            self.file_search_open = true;
        }
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
        target: BranchMenuTarget,
        pin_key: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some(SelectedBranch { repo_id, target });
        let released_click = self.clear_sidebar_click_target();
        if self.selected_branch.as_ref() == next.as_ref()
            && self.selected_branch_pin_key == pin_key
            && !released_click
        {
            return;
        }
        self.selected_branch = next;
        self.selected_branch_pin_key = pin_key;
        cx.notify();
    }

    /// Apply the single-click outcome shared by a rendered branch row and any
    /// action that promises to behave like one.
    pub(in super::super) fn select_branch_and_reveal_tip(
        &mut self,
        repo_id: RepoId,
        target: BranchMenuTarget,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        pin_key: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_selected_branch(repo_id, target.clone(), pin_key, cx);
        self.reveal_branch_commit_in_history(repo_id, target, commit_id, fallback_scope, cx);
        cx.notify();
    }

    pub(in super::super) fn selected_branch(&self) -> Option<&SelectedBranch> {
        self.selected_branch.as_ref()
    }

    pub(in super::super) fn selected_branch_for_row(
        &self,
        pin_key: Option<&SharedString>,
    ) -> Option<&SelectedBranch> {
        self.selected_branch()
            .filter(|_| self.selected_branch_pin_key.as_ref() == pin_key)
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
    pub(in crate::view) fn list_scroll_for_test(&self) -> gpui::ScrollHandle {
        if self.collapsed_popover_section == Some(CollapsedSidebarSection::Files) {
            self.file_browser_scroll.0.borrow().base_handle.clone()
        } else {
            self.branches_scroll.0.borrow().base_handle.clone()
        }
    }

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

    fn sync_shared_sidebar_preferences(&mut self, next: &AppState, cx: &mut gpui::Context<Self>) {
        let mut changed = false;
        for repo in &next.repos {
            let Some(snapshot) = &repo.shared_preferences else {
                continue;
            };
            if self
                .state
                .repos
                .iter()
                .find(|old| old.id == repo.id)
                .and_then(|old| old.shared_preferences.as_ref())
                .is_some_and(|old| old.revision == snapshot.revision && old.key == snapshot.key)
            {
                continue;
            }
            let path = repo.spec.workdir.clone();
            self.sidebar_pinned_branches_by_repo
                .insert(path.clone(), snapshot.preferences.pinned_items.clone());
            let items = self
                .sidebar_collapsed_items_by_repo
                .entry(path)
                .or_default();
            items.retain(|key| branch_sidebar::is_top_level_collapse_key(key));
            items.extend(snapshot.preferences.collapsed_items.iter().cloned());
            changed = true;
        }
        if changed {
            self.sidebar_presentation_cache = SidebarPresentationCache::default();
            self.sync_popover_pinned_branches(cx);
            self.sync_popover_collapsed_items(cx);
        }
    }

    fn publish_sidebar_collapse_changes(&self, repo_id: RepoId, before: &BTreeSet<String>) {
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };
        let after = self
            .sidebar_collapsed_items_by_repo
            .get(&repo.spec.workdir)
            .cloned()
            .unwrap_or_default();
        let persistent = |key: &&String| !branch_sidebar::is_top_level_collapse_key(key);
        let added = after
            .difference(before)
            .filter(persistent)
            .cloned()
            .collect::<BTreeSet<_>>();
        let removed = before
            .difference(&after)
            .filter(persistent)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !added.is_empty() || !removed.is_empty() {
            self.store.dispatch(Msg::UpdateRepositoryPreference {
                repo_id,
                update: gitcomet_state::model::RepositoryPreferenceUpdate::CollapseItems {
                    added,
                    removed,
                },
            });
        }
    }

    #[cfg(test)]
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
        self.toggle_sidebar_pin(
            repo_id,
            branch_sidebar::branch_pin_storage_key(section, name),
            cx,
        );
    }

    pub(in crate::view) fn toggle_sidebar_pin(
        &mut self,
        repo_id: RepoId,
        key: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let repo_path = repo.spec.workdir.clone();

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
        let pinned = self
            .sidebar_pinned_branches_by_repo
            .get(&repo_path)
            .is_some_and(|items| items.contains(&key));
        self.store.dispatch(Msg::UpdateRepositoryPreference {
            repo_id,
            update: gitcomet_state::model::RepositoryPreferenceUpdate::Pin { key, pinned },
        });
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
        let filter = self.branch_filter_query.clone();
        self.unpin_branches_matching(repo_id, section, &filter, cx);
    }

    pub(in crate::view) fn unpin_branches_matching(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        filter: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return;
        };
        let repo_path = repo.spec.workdir.clone();
        let Some(items) = self.sidebar_pinned_branches_by_repo.get_mut(&repo_path) else {
            return;
        };

        let before = items.clone();
        for row in branch_sidebar::matching_pinned_roots(
            repo,
            items,
            &crate::view::sidebar_search::SidebarSearch::new(filter, self.branch_search_options),
        ) {
            if let Some((candidate, key)) = branch_sidebar::pinned_row_storage_key(&row)
                && candidate == section
            {
                items.remove(&key);
            }
        }
        if *items == before {
            return;
        }
        let removed = before.difference(items).cloned().collect();
        if items.is_empty() {
            self.sidebar_pinned_branches_by_repo.remove(&repo_path);
        }

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.store.dispatch(Msg::UpdateRepositoryPreference {
            repo_id,
            update: gitcomet_state::model::RepositoryPreferenceUpdate::RemovePins(removed),
        });
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
        let options = self.branch_search_options;
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.popover_host.update(cx, |host, cx| {
                    host.set_branch_search(query, options, cx);
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

    pub(in super::super) fn toggle_active_repo_collapse_key(
        &mut self,
        collapse_key: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        self.apply_active_repo_collapse_key(collapse_key, None, cx);
    }

    /// Drive a collapse key to an explicit state instead of flipping it.
    ///
    /// Branch groups render force-expanded while a branch filter is live,
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

        let before = self
            .sidebar_collapsed_items_by_repo
            .get(&repo_path)
            .cloned()
            .unwrap_or_default();
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
        // Explicit expansion changes retain the offset; only data refreshes
        // restore a content anchor. GPUI still clamps a shortened list.
        self.sticky_context = None;
        self.pending_sidebar_navigation = None;
        self.sidebar_scroll_animation = None;
        self.branches_scroll.0.borrow_mut().deferred_scroll_to_item = None;
        self.publish_sidebar_collapse_changes(repo_id, &before);
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
        if path.is_empty() && (section != BranchSection::Remote || remote.is_none()) {
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
        let repo_id = repo.id;
        let before = self
            .sidebar_collapsed_items_by_repo
            .get(&repo_path)
            .cloned()
            .unwrap_or_default();
        let items = self
            .sidebar_collapsed_items_by_repo
            .entry(repo_path.clone())
            .or_default();
        if path.is_empty() {
            let key = branch_sidebar::remote_header_storage_key(remote.as_deref().unwrap());
            branch_sidebar::set_collapse_state(items, &key, collapsed);
        }
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
        self.sticky_context = None;
        self.pending_sidebar_navigation = None;
        self.sidebar_scroll_animation = None;
        self.branches_scroll.0.borrow_mut().deferred_scroll_to_item = None;
        self.publish_sidebar_collapse_changes(repo_id, &before);
        self.sync_popover_collapsed_items(cx);
        self.dispatch_sidebar_data_request_if_needed(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_expanded_branches_visible(
        &mut self,
        visible: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.expanded_branches_visible = visible;
        if !visible {
            self.sidebar_scroll_animation = None;
            self.pending_sidebar_navigation = None;
        }
        self.dispatch_sidebar_data_request_if_needed(cx);
    }

    fn dispatch_sidebar_data_request_if_needed(&mut self, cx: &mut gpui::Context<Self>) {
        let next = sidebar_presentation::sidebar_request_fingerprint(
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
            self.expanded_branches_visible && self.state.sidebar_mode == SidebarMode::Branches,
        );
        if next == self.sidebar_request_fingerprint {
            return;
        }
        self.sidebar_request_fingerprint = next;

        let Some((repo_id, request)) = sidebar_presentation::active_sidebar_data_request(
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
            self.expanded_branches_visible && self.state.sidebar_mode == SidebarMode::Branches,
        ) else {
            return;
        };

        let store = Arc::clone(&self.store);
        cx.defer(move |_cx| store.dispatch(Msg::EnsureSidebarData { repo_id, request }));
    }

    pub(in super::super) fn branch_sidebar_presentation_cached(
        &mut self,
    ) -> Option<SidebarPresentation> {
        let presentation = sidebar_presentation::build_sidebar_presentation_scoped(
            &mut self.sidebar_presentation_cache,
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
            &self.sidebar_pinned_branches_by_repo,
            &self.branch_filter_query,
            self.branch_search_options,
            self.collapsed_popover_section
                .and_then(|section| section.storage_key()),
        )?;
        let presentation = if self.collapsed_popover_section.is_none() {
            match &mut self.contributions {
                Some(contributions) => contributions.project(presentation),
                None => presentation,
            }
        } else {
            presentation
        };
        self.update_sticky_context(&presentation);
        Some(presentation)
    }

    /// Shows Branches or Files (`None`) or contributed tab `index`.
    /// Deferred: a tab is clicked inside this pane's update, and the root
    /// updates this pane again as the selection changes.
    fn select_routed_tab(
        &self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root = self.root_view.clone();
        window.defer(cx, move |window, cx| {
            let _ = root.update(cx, |root, cx| {
                root.select_routed_view(
                    crate::view::repository_views::RoutedArea::Sidebar,
                    index,
                    window,
                    cx,
                )
            });
        });
    }

    /// The window's routing of contributed tabs for the active repository.
    pub(in crate::view) fn set_extension_tabs(
        &mut self,
        tabs: Option<SidebarExtensionTabs>,
        cx: &mut gpui::Context<Self>,
    ) {
        let same = match (&self.extension_tabs, &tabs) {
            (Some(current), Some(next)) => current.same(next),
            (None, None) => true,
            _ => false,
        };
        if !same {
            let was_showing = self
                .extension_tabs
                .as_ref()
                .is_some_and(|tabs| tabs.showing().is_some());
            if !was_showing && tabs.as_ref().is_some_and(|tabs| tabs.showing().is_some()) {
                self.file_history_selected = false;
            }
            self.extension_tabs = tabs;
            cx.emit(SidebarHeaderChanged);
            cx.notify();
        }
    }

    /// Whether the history tab is listed: it belongs to one repository.
    fn file_history_visible(&self, cx: &gpui::App) -> bool {
        self.file_history_pane
            .read(cx)
            .target
            .as_ref()
            .is_some_and(|(repo_id, _)| Some(*repo_id) == self.active_repo_id())
    }

    fn file_history_shown(&self, cx: &gpui::App) -> bool {
        self.file_history_selected && self.file_history_visible(cx)
    }

    /// Selects the history tab and leaves a contributed tab.
    pub(in crate::view) fn show_file_history(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_history_selected = true;
        if self
            .extension_tabs
            .as_ref()
            .is_some_and(|tabs| tabs.showing().is_some())
        {
            self.select_routed_tab(None, window, cx);
        }
        cx.emit(SidebarHeaderChanged);
        cx.notify();
    }

    pub(in super::super) fn sidebar(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        self.sync_contributed_rows(cx);
        let theme = self.theme;

        self.apply_pending_file_browser_reveal(cx);
        let tab_bar = self.render_tab_bar(theme, cx);
        let mode = self.state.sidebar_mode;
        let contributed = self
            .extension_tabs
            .as_ref()
            .filter(|tabs| tabs.showing().is_some())
            .and_then(|tabs| tabs.view.clone());
        let history_shown = self.file_history_shown(cx);
        let content = match (contributed, mode) {
            _ if history_shown => div()
                .flex_1()
                .min_h(px(0.0))
                .debug_selector(|| "sidebar_file_history_content".to_string())
                .child(self.file_history_pane.clone())
                .into_any_element(),
            (Some(view), _) => div()
                .flex_1()
                .min_h(px(0.0))
                .debug_selector(|| "sidebar_extension_tab_content".to_string())
                .child(view)
                .into_any_element(),
            (None, SidebarMode::Branches) => self.render_branches_content(theme, cx),
            (None, SidebarMode::Files) => self.render_file_browser_content(theme, cx),
        };

        // `size_full`, not just `h_full`: mounted as a cached view this is laid
        // out as a root against the pane's bounds, where an auto width would
        // shrink to the content instead of stretching like a flex child does.
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .on_key_down(cx.listener(Self::on_sidebar_key_down))
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
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = ui_scale::scaler(ui_scale_percent);

        // Every section has the same header menu the expanded sidebar hangs off
        // its section row or tab strip (add a worktree, stash, Files settings,
        // ...). The rail popover shows only the section's rows, so without this
        // button — and the right-click on the panel behind it — those actions
        // would be out of reach while the sidebar is collapsed.
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
            .pr(scaled_px(6.0))
            .pt(scaled_px(8.0))
            .pb(scaled_px(6.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(12.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child(section.title()),
            )
            .child(self.render_search_toggle(
                section == CollapsedSidebarSection::Files,
                "collapsed_popover_filter_toggle",
                cx,
            ))
            .child(self.render_section_locator(section, cx))
            .when_some(section_menu.clone(), |header, (invoker, kind)| {
                header.child(
                    components::Button::new("collapsed_popover_section_menu", "")
                        .borderless()
                        .style(components::ButtonStyle::Subtle)
                        .open(section_menu_active)
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
                            this.open_popover_at(
                                kind.clone().invoked_by(invoker.clone()),
                                e.position(),
                                window,
                                cx,
                            );
                        })
                        .w(components::control_height(ui_scale))
                        .h(components::control_height(ui_scale))
                        .gitcomet_tooltip(theme, "More actions".into())
                        .debug_selector(|| "collapsed_popover_section_menu".to_string()),
                )
            });

        self.apply_pending_file_browser_reveal(cx);
        let content = if section == CollapsedSidebarSection::Files {
            self.render_file_browser_content(theme, cx)
        } else {
            self.render_branches_content(theme, cx)
        };
        let _ = window;
        div()
            .id("collapsed_sidebar_popover_content")
            .debug_selector(|| "collapsed_sidebar_popover_content".to_string())
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .text_color(theme.colors.foreground.primary)
            .track_focus(&self.sidebar_focus)
            .on_key_down(cx.listener(Self::on_sidebar_key_down))
            .child(title)
            .child(div().flex_none().h(px(1.0)).bg(theme.colors.stroke.subtle))
            .child(content)
            .into_any_element()
    }

    #[cfg(test)]
    fn build_collapsed_popover_presentation(
        &mut self,
        section: CollapsedSidebarSection,
    ) -> Option<SidebarPresentation> {
        sidebar_presentation::build_sidebar_presentation_scoped(
            &mut self.sidebar_presentation_cache,
            self.state.as_ref(),
            &self.sidebar_collapsed_items_by_repo,
            &self.sidebar_pinned_branches_by_repo,
            &self.branch_filter_query,
            self.branch_search_options,
            section.storage_key(),
        )
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
                if repo.file_browser.needs_load() {
                    let source = repo.file_browser.source.clone();
                    self.store
                        .dispatch(Msg::LoadFileBrowser { repo_id, source });
                }
            }
            // Large-file support loads with the repository.
            CollapsedSidebarSection::Local
            | CollapsedSidebarSection::Remote
            | CollapsedSidebarSection::Annex => {}
        }
    }

    /// Scroll to the checked-out local branch, opening every group that hides
    /// it, and then select its tip exactly as a single click on its row would.
    pub(in super::super) fn locate_active_local_branch(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, repo_path, branch_name, commit_id)) =
            self.active_repo().and_then(|repo| {
                let (branch_name, commit_id) = active_local_branch_target(repo)?;
                Some((
                    repo.id,
                    repo.spec.workdir.clone(),
                    branch_name.to_string(),
                    commit_id.clone(),
                ))
            })
        else {
            return;
        };

        if !self.branch_filter_query.is_empty() {
            self.clear_branch_filter(cx);
        }

        let before = self
            .sidebar_collapsed_items_by_repo
            .get(&repo_path)
            .cloned()
            .unwrap_or_default();
        let collapse_changed = {
            let collapsed_items = self
                .sidebar_collapsed_items_by_repo
                .entry(repo_path.clone())
                .or_default();
            expand_local_branch_path(collapsed_items, &branch_name)
        };
        if self
            .sidebar_collapsed_items_by_repo
            .get(&repo_path)
            .is_some_and(BTreeSet::is_empty)
        {
            self.sidebar_collapsed_items_by_repo.remove(&repo_path);
        }

        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        if collapse_changed {
            self.publish_sidebar_collapse_changes(repo_id, &before);
            self.sync_popover_collapsed_items(cx);
            self.dispatch_sidebar_data_request_if_needed(cx);
        }

        let row_ix = self
            .branch_sidebar_presentation_cached()
            .and_then(|presentation| local_branch_home_row_index(&presentation.rows, &branch_name));
        self.select_branch_and_reveal_tip(
            repo_id,
            BranchMenuTarget::local(branch_name),
            commit_id,
            Some(LogScope::FullReachable),
            None,
            cx,
        );
        if row_ix.is_some() {
            self.pending_sidebar_navigation = self
                .selected_branch
                .as_ref()
                .map(|selected| sticky::NavigationTarget::Branch(selected.target.clone()));
        }
        cx.notify();
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

        self.file_history_selected = false;
        let files_visible = self
            .collapsed_popover_section
            .map_or(self.state.sidebar_mode == SidebarMode::Files, |section| {
                section == CollapsedSidebarSection::Files
            });
        if !files_visible {
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
        if self.collapsed_popover_section.is_none() && self.state.sidebar_mode != SidebarMode::Files
        {
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
        // Rows for a browse point that has moved on are about to be replaced.
        if repo.file_browser.stale {
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

    /// The accent banner above the branch list, shown only while the history is
    /// focused.
    ///
    /// The sidebar is where focus is switched on, one ref at a time, so it is
    /// also where the way out belongs: the banner says how much of the
    /// repository is currently in view and clears the whole set in one click.
    fn render_focus_banner(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::Div> {
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = ui_scale::scaler(ui_scale_percent);
        let repo = self.active_repo()?;
        let repo_id = repo.id;
        if repo.history_state.history_focus.is_empty() {
            return None;
        }
        // Both sides count branch refs, local and remote: remote branches can
        // be focused too, so leaving them out of the total would let the count
        // read "3/2".
        let count = crate::view::history_focus::history_focus_count(repo);
        let accent = theme.colors.accent;
        let store = self.store.clone();

        Some(
            div().px(scaled_px(8.0)).pb(scaled_px(6.0)).child(
                div()
                    .id("branch_sidebar_focus_banner")
                    .debug_selector(|| "branch_sidebar_focus_banner".to_string())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(scaled_px(6.0))
                    .px(scaled_px(8.0))
                    .py(scaled_px(4.0))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(accent.foreground)
                    .bg(accent.subtle_background)
                    .text_size(theme.ui_text(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(accent.foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .debug_selector(|| "branch_sidebar_focus_banner_count".to_string())
                            .child(format!("Focusing {}/{}", count.focused, count.total)),
                    )
                    .child(
                        components::Button::new("branch_sidebar_focus_banner_stop", "Stop focus")
                            .borderless()
                            .style(components::ButtonStyle::Subtle)
                            .on_click(theme, cx, move |_this, _e, _w, _cx| {
                                store.dispatch(Msg::SetHistoryFocus {
                                    repo_id,
                                    focus: gitcomet_core::domain::HistoryFocusSet::default(),
                                });
                            }),
                    ),
            ),
        )
    }

    fn render_branch_filter_bar(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.render_sidebar_search(false, theme, cx)
    }

    pub(in crate::view) fn set_extension_branch_search(
        &mut self,
        query: String,
        options: TextSearchOptions,
        cx: &mut gpui::Context<Self>,
    ) {
        self.branch_search_options = options;
        self.branch_filter_query = query.clone();
        self.branch_filter_input
            .update(cx, |input, cx| input.set_text(query, cx));
        self.branch_search_open = !self.branch_filter_query.is_empty();
        self.sidebar_presentation_cache = SidebarPresentationCache::default();
        self.sticky_context = None;
        self.sync_popover_branch_filter(cx);
        cx.notify();
    }

    fn clear_branch_filter(&mut self, cx: &mut gpui::Context<Self>) {
        self.branch_filter_input.update(cx, |input, cx| {
            input.set_text("", cx);
        });
        self.sticky_context = None;
        self.pending_sidebar_navigation = None;
        self.sidebar_scroll_animation = None;
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
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = ui_scale::scaler(ui_scale_percent);

        let filter_bar = self
            .branch_search_open
            .then(|| self.render_branch_filter_bar(theme, cx));
        let focus_banner = self.render_focus_banner(theme, cx);
        let Some(presentation) = self.branch_sidebar_presentation_cached() else {
            return div()
                .flex()
                .flex_col()
                .h_full()
                .min_h(px(0.0))
                .children(filter_bar)
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
        .track_scroll(&self.branches_scroll)
        .when(self.collapsed_popover_section.is_none(), |list| {
            list.with_decoration(sticky::StickyRows { view: cx.entity() })
        });
        let list = restrict_scroll_to_vertical_axis(list);
        let view = cx.entity();
        let row_height = sidebar_list_row_height(theme, ui_scale_percent);
        let list = div()
            .relative()
            .h_full()
            .min_h(px(0.0))
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        // Match the list's device-pixel rounding before it
                        // measures and renders the first row of this frame.
                        let row_height = window.pixel_snap(row_height);
                        view.update(cx, |this, cx| {
                            this.prepare_sidebar_scroll(bounds, row_height, window, cx)
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(list);
        // Rows use the full pane width; the scrollbar overlays them (its track
        // is transparent, only the thumb paints while scrolling/hovering).
        let list = div()
            .flex_1()
            .min_h(px(0.0))
            .pt(scaled_px(SIDEBAR_TOP_INSET_PX))
            .child(list);
        let panel_body: AnyElement = div()
            .id("branch_sidebar_scroll_container")
            .debug_selector(|| "branch_sidebar_scroll_container".to_string())
            .on_scroll_wheel(cx.listener(|this, _: &gpui::ScrollWheelEvent, _, cx| {
                if this.sidebar_scroll_animation.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_down_all({
                let view = cx.entity();
                move |event, phase, hitbox, _, cx| {
                    if phase == gpui::DispatchPhase::Capture
                        && hitbox.bounds.contains(&event.position)
                    {
                        view.update(cx, |this, _| {
                            this.sidebar_scroll_animation = None;
                        });
                    }
                }
            })
            .min_h(px(0.0))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .when_some(self.sidebar_click_target_bounds(), |panel, bounds| {
                panel.on_mouse_move_all({
                    let view = cx.entity();
                    move |event, phase, _, _, cx| {
                        if phase == gpui::DispatchPhase::Capture
                            && !bounds.contains(&event.position)
                        {
                            view.update(cx, |this, cx| {
                                if this.clear_sidebar_click_target() {
                                    cx.notify();
                                }
                            });
                        }
                    }
                })
            })
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
            .children(filter_bar)
            .children(focus_banner)
            .child(
                div()
                    .id("sidebar_branches_body")
                    .debug_selector(|| "sidebar_branches_body".to_string())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(panel_body),
            )
            .into_any()
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
        });
    }

    pub(in super::super) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.diff_renderer.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                cx.notify();
            });
        });
    }

    pub(in crate::view) fn explain_working_tree_required(&self, cx: &mut gpui::Context<Self>) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.push_toast(
                components::ToastKind::Warning,
                gitcomet_core::domain::WORKING_TREE_REQUIRED_MESSAGE.to_owned(),
                cx,
            );
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
        target: BranchMenuTarget,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.main_pane.update(cx, |pane, cx| {
                    pane.reveal_history_branch_commit(
                        repo_id,
                        target,
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
        if self
            .explorer_name_edit
            .as_ref()
            .is_some_and(|edit| Some(edit.repo_id) != self.active_repo_id())
        {
            self.explorer_name_edit = None;
        }
        if self.explorer_name_edit.is_some() {
            let height = ui_scale::UiScale::current(cx).px(sidebar_list_row_height_px(self.theme));
            self.explorer_name_input.update(cx, |input, cx| {
                input.set_line_height(Some(height - px(4.0)), cx)
            });
            if self
                .explorer_name_edit
                .as_ref()
                .is_some_and(|edit| edit.reveal)
                && let Some(index) = self
                    .file_browser_visible_rows(cx)
                    .iter()
                    .position(|row| matches!(row, FileBrowserVisibleRow::NameEntry { .. }))
            {
                // The rail popover shows the same list, so one scroll serves both.
                self.file_browser_scroll
                    .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
                if let Some(edit) = &mut self.explorer_name_edit {
                    edit.reveal = false;
                }
            }
        }
        // A drop elsewhere (or cancellation) removes the preview and refreshes
        // the window. Clear transient targeting while that frame is rendered.
        if !cx.has_active_drag()
            || self
                .explorer_drag_repo
                .is_some_and(|id| self.active_repo_id() != Some(id))
        {
            self.explorer_drag_repo = None;
            self.explorer_drop_row = None;
            self.explorer_drop_target = None;
            self.explorer_hover_task = None;
            self.explorer_scroll_task = None;
        }
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.render_count += 1;
            self.rendered_rows = 0;
        }
        match self.collapsed_popover_section {
            Some(section) => self.render_collapsed_popover(section, window, cx),
            None => self.sidebar(cx).into_any_element(),
        }
    }
}

fn repo_uses_annex(repo: &RepoState) -> bool {
    matches!(&repo.large_file_support, Loadable::Ready(support) if support.annex.in_use())
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

fn file_search_matchers(query: &str, options: TextSearchOptions) -> Vec<TextSearchMatcher> {
    file_search_query_lines(query)
        .map(|line| TextSearchMatcher::new(line, options))
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

fn file_search_matches(matchers: &[TextSearchMatcher], haystack: &str) -> bool {
    !matchers
        .iter()
        .any(|matcher| matcher.regex_error().is_some())
        && (matchers.is_empty() || matchers.iter().any(|matcher| matcher.is_match(haystack)))
}

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
    open: gitcomet_extension_api::HostedAction,
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
    let ui_scale_percent = crate::ui_scale::current(cx).percent;
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
    let icon_px = scaled_px(12.0);
    // The full repo-relative path, not just the file name: two `mod.rs` under
    // different folders are indistinguishable here, and this row is the only
    // place they appear side by side.
    let label = path.display().to_string();

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
        .gap(scaled_px(4.0))
        .interactive_row(
            row_style,
            components::InteractiveRowState::default().selected(is_open, open_row_bg),
        )
        // Straight into the editor, not the read-only view: every row in this
        // section has unsaved text, and the read-only view would show the file
        // on disk instead of what the user was in the middle of writing.
        .on_activate(
            false,
            controls::ControlActivation::Composite,
            cx.listener(move |this, _e: &gpui::ClickEvent, _window, cx| {
                this.show_repository_canvas(cx);
                open.invoke(cx);
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
                components::TruncatedText::new(label, theme.ui_text(14.0))
                    // Path elision keeps the file name, which identifies the
                    // row, and drops the folders in the middle.
                    .profile(components::TextTruncationProfile::Path)
                    .text_color(file_browser_row_label_color(theme, is_open))
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
                            root.diff_renderer.update(cx, |pane, cx| {
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
    matchers: &[TextSearchMatcher],
    name: &str,
) -> Vec<std::ops::Range<usize>> {
    const MAX_NAME_HIGHLIGHTS: usize = 16;
    if matchers
        .iter()
        .any(|matcher| matcher.regex_error().is_some())
    {
        return Vec::new();
    }
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
mod file_search_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod long_list_tests;

mod search;
mod sticky;

#[cfg(test)]
mod sticky_tests;

#[cfg(feature = "benchmarks")]
mod sticky_benchmark;
#[cfg(feature = "benchmarks")]
pub use sticky_benchmark::SidebarStickyFrameFixture;

#[cfg(test)]
mod explorer_drag_tests;

#[cfg(test)]
mod explorer_settings_tests;

mod file_browser_view;
#[cfg(test)]
mod repository_preferences_tests;
