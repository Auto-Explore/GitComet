use super::super::caches::{
    BranchSidebarFingerprint, branch_sidebar_cache_lookup,
    branch_sidebar_cache_lookup_by_cached_source, branch_sidebar_cache_lookup_by_source,
    branch_sidebar_cache_store,
};
use super::super::*;
use rustc_hash::FxHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub(in super::super) struct SidebarPaneView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    branches_scroll: UniformListScrollHandle,
    branch_sidebar_cache: Option<BranchSidebarCache>,
    path_display_cache: std::cell::RefCell<path_display::PathDisplayCache>,
    sidebar_collapsed_items_by_repo: BTreeMap<std::path::PathBuf, BTreeSet<String>>,
    root_view: WeakEntity<GitCometView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: SidebarNotifyFingerprint,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SidebarNotifyFingerprint {
    active_repo_id: Option<RepoId>,
    repo_fingerprint: Option<BranchSidebarFingerprint>,
    open_repo_workdirs_count: usize,
    open_repo_workdirs_hash: u64,
    active_workspace_badges_count: usize,
    active_workspace_badges_hash: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SidebarLazyLoadPlan {
    worktrees: bool,
    submodules: bool,
    subtrees: bool,
    stashes: bool,
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
        Self {
            active_repo_id,
            repo_fingerprint,
            open_repo_workdirs_count,
            open_repo_workdirs_hash,
            active_workspace_badges_count,
            active_workspace_badges_hash,
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

            this.notify_fingerprint = next_fingerprint;
            this.state = next;

            if should_notify {
                cx.notify();
            }
        });

        Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            branches_scroll: UniformListScrollHandle::default(),
            branch_sidebar_cache: None,
            path_display_cache: std::cell::RefCell::new(path_display::PathDisplayCache::default()),
            sidebar_collapsed_items_by_repo,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
        }
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
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
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in super::super) fn active_workspace_paths_by_branch(
        &self,
    ) -> HashMap<String, std::path::PathBuf> {
        self.active_repo()
            .map(|repo| {
                crate::view::rows::active_workspace_paths_by_branch(
                    repo,
                    self.state.repos.as_slice(),
                )
            })
            .unwrap_or_default()
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

        self.branch_sidebar_cache = None;
        self.schedule_ui_settings_persist(cx);
        cx.notify();
    }

    pub(in super::super) fn branch_sidebar_rows_cached(
        &mut self,
    ) -> Option<Rc<[BranchSidebarRow]>> {
        let Some(repo_id) = self.active_repo_id() else {
            self.branch_sidebar_cache = None;
            return None;
        };
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;

        let empty = BTreeSet::new();
        let collapsed_items = self
            .sidebar_collapsed_items_by_repo
            .get(&repo.spec.workdir)
            .unwrap_or(&empty);
        let lazy_loads = pending_sidebar_lazy_loads(repo, collapsed_items);

        if lazy_loads.worktrees {
            self.store.dispatch(Msg::LoadWorktrees { repo_id });
        }
        if lazy_loads.submodules {
            self.store.dispatch(Msg::LoadSubmodules { repo_id });
        }
        if lazy_loads.subtrees {
            self.store.dispatch(Msg::LoadSubtrees { repo_id });
        }
        if lazy_loads.stashes {
            self.store.dispatch(Msg::LoadStashes { repo_id });
        }

        let fingerprint = BranchSidebarFingerprint::from_repo(repo);

        if let Some(rows) =
            branch_sidebar_cache_lookup(&mut self.branch_sidebar_cache, repo_id, fingerprint)
        {
            return Some(rows);
        }

        if let Some(rows) = branch_sidebar_cache_lookup_by_cached_source(
            &mut self.branch_sidebar_cache,
            repo,
            fingerprint,
        ) {
            return Some(rows);
        }

        let (source_fingerprint, source_parts) = {
            let cached_source_parts = self
                .branch_sidebar_cache
                .as_ref()
                .filter(|cached| cached.repo_id == repo_id)
                .map(|cached| &cached.source_parts);
            branch_sidebar::branch_sidebar_source_fingerprint(repo, cached_source_parts)
        };

        if let Some(rows) = branch_sidebar_cache_lookup_by_source(
            &mut self.branch_sidebar_cache,
            repo_id,
            fingerprint,
            source_fingerprint,
            &source_parts,
        ) {
            return Some(rows);
        }

        let rows: Rc<[BranchSidebarRow]> =
            { branch_sidebar::branch_sidebar_rows(repo, collapsed_items).into() };

        branch_sidebar_cache_store(
            &mut self.branch_sidebar_cache,
            repo_id,
            fingerprint,
            source_fingerprint,
            source_parts,
            Rc::clone(&rows),
        );
        Some(rows)
    }

    pub(in super::super) fn sidebar(&mut self, cx: &mut gpui::Context<Self>) -> gpui::Div {
        const SIDEBAR_TOP_INSET_PX: f32 = 2.0;

        let theme = self.theme;
        let Some(rows) = self.branch_sidebar_rows_cached() else {
            return div()
                .flex()
                .flex_col()
                .h_full()
                .min_h(px(0.0))
                .child(components::empty_state(
                    theme,
                    "Branches",
                    "No repository selected.",
                ));
        };

        let row_count = rows.len();
        let list = uniform_list(
            "branch_sidebar",
            row_count,
            cx.processor(Self::render_branch_sidebar_rows),
        )
        .h_full()
        .min_h(px(0.0))
        .track_scroll(self.branches_scroll.clone());
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
}

impl Render for SidebarPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        self.sidebar(cx)
    }
}

fn pending_sidebar_lazy_loads(
    repo: &RepoState,
    collapsed_items: &BTreeSet<String>,
) -> SidebarLazyLoadPlan {
    SidebarLazyLoadPlan {
        // Worktree data also drives local branch workspace badges, so load it even when the
        // Worktrees section itself is collapsed.
        worktrees: matches!(repo.worktrees, Loadable::NotLoaded),
        submodules: !branch_sidebar::is_collapsed(
            collapsed_items,
            branch_sidebar::submodules_section_storage_key(),
        ) && matches!(repo.submodules, Loadable::NotLoaded),
        subtrees: !branch_sidebar::is_collapsed(
            collapsed_items,
            branch_sidebar::subtrees_section_storage_key(),
        ) && matches!(repo.subtrees, Loadable::NotLoaded),
        stashes: !branch_sidebar::is_collapsed(
            collapsed_items,
            branch_sidebar::stash_section_storage_key(),
        ) && matches!(repo.stashes, Loadable::NotLoaded),
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
    fn pending_sidebar_lazy_loads_preloads_worktrees_for_workspace_badges() {
        let repo = RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );

        let expanded = pending_sidebar_lazy_loads(&repo, &BTreeSet::new());
        assert_eq!(
            expanded,
            SidebarLazyLoadPlan {
                worktrees: true,
                ..SidebarLazyLoadPlan::default()
            }
        );

        let expanded = BTreeSet::from([
            branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::worktrees_section_storage_key(),
            )
            .expect("worktrees should support explicit expansion"),
            branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::submodules_section_storage_key(),
            )
            .expect("submodules should support explicit expansion"),
            branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::subtrees_section_storage_key(),
            )
            .expect("subtrees should support explicit expansion"),
            branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::stash_section_storage_key(),
            )
            .expect("stash should support explicit expansion"),
        ]);
        let expanded = pending_sidebar_lazy_loads(&repo, &expanded);
        assert_eq!(
            expanded,
            SidebarLazyLoadPlan {
                worktrees: true,
                submodules: true,
                subtrees: true,
                stashes: true,
            }
        );
    }

    #[test]
    fn pending_sidebar_lazy_loads_handles_mixed_repo_state() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
        repo.submodules = Loadable::Loading;
        repo.subtrees = Loadable::NotLoaded;
        repo.stashes = Loadable::NotLoaded;

        let collapsed = BTreeSet::from([
            branch_sidebar::submodules_section_storage_key().to_string(),
            branch_sidebar::subtrees_section_storage_key().to_string(),
            branch_sidebar::expanded_default_section_storage_key(
                branch_sidebar::stash_section_storage_key(),
            )
            .expect("stash should support explicit expansion"),
        ]);
        let plan = pending_sidebar_lazy_loads(&repo, &collapsed);
        assert_eq!(
            plan,
            SidebarLazyLoadPlan {
                worktrees: false,
                submodules: false,
                subtrees: false,
                stashes: true,
            }
        );
    }

    #[test]
    fn sidebar_notify_fingerprint_tracks_open_repo_workdirs() {
        let mut state = AppState::default();
        state.active_repo = Some(RepoId(1));
        state.repos.push(repo_state(RepoId(1), "/tmp/repo"));

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
        let mut state_a = AppState::default();
        state_a.active_repo = Some(RepoId(1));
        state_a.repos.push(repo_state(RepoId(1), "/tmp/repo"));
        state_a.repos.push(repo_state(RepoId(2), "/tmp/repo-wt"));

        let mut state_b = AppState::default();
        state_b.active_repo = Some(RepoId(1));
        state_b.repos.push(repo_state(RepoId(2), "/tmp/repo-wt"));
        state_b.repos.push(repo_state(RepoId(1), "/tmp/repo"));

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
}
