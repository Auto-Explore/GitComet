//! Extension views built per repository: the repository-view router
//! (History or one of the extensions' repository views in the main area,
//! chosen from tabs in the action bar), details tabs beside the details
//! pane's own content, sidebar tabs after Branches and Files, and sidebar
//! sections below the sidebar's own. A details or sidebar tab can belong to
//! one repository view: it is listed only with that view, and may open
//! whenever the view does.
//!
//! Each exists only when an extension registers that kind; otherwise its
//! area renders exactly as before. A view is built on first use for a
//! repository and kept until that repository closes; an unselected built-in
//! pane is not rendered but keeps its state.

use super::extension_panels::{RepoKey, repo_key};
use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_extension_api::{
    ContributionId, DetailsTabDescriptor, PanelArea, PanelContentPolicy, PanelTabDescriptor,
    RepositoryViewContext, RepositoryViewDescriptor, SidebarSectionDescriptor,
    SidebarTabDescriptor, ViewBuilder, ViewTarget,
};
use std::rc::Rc;

/// A contribution built per repository: its title, tab icon and builder.
pub(in crate::view) trait RoutedContribution {
    fn title(&self) -> SharedString;
    fn icon(&self) -> Option<SharedString>;
    fn builder(&self) -> ViewBuilder<RepositoryViewContext>;
    /// The repository view the contribution is listed with; `None` is every
    /// view.
    fn view(&self) -> Option<&ViewTarget> {
        None
    }
    /// Selected whenever its view is.
    fn opens_with_view(&self) -> bool {
        false
    }
}

macro_rules! routed_contribution {
    ($descriptor:ty, |$this:ident| $icon:expr) => {
        impl RoutedContribution for $descriptor {
            fn title(&self) -> SharedString {
                self.title.clone()
            }

            fn icon(&self) -> Option<SharedString> {
                let $this = self;
                $icon
            }

            fn builder(&self) -> ViewBuilder<RepositoryViewContext> {
                Rc::clone(&self.build)
            }
        }
    };
}

routed_contribution!(RepositoryViewDescriptor, |view| (!view.icon.is_empty())
    .then(|| view.icon.clone()));
routed_contribution!(SidebarSectionDescriptor, |_section| None);

macro_rules! view_scoped_contribution {
    ($descriptor:ty, |$this:ident| $icon:expr) => {
        impl RoutedContribution for $descriptor {
            fn title(&self) -> SharedString {
                self.title.clone()
            }

            fn icon(&self) -> Option<SharedString> {
                let $this = self;
                $icon
            }

            fn builder(&self) -> ViewBuilder<RepositoryViewContext> {
                Rc::clone(&self.build)
            }

            fn view(&self) -> Option<&ViewTarget> {
                self.view.as_ref()
            }

            fn opens_with_view(&self) -> bool {
                self.opens_with_view
            }
        }
    };
}

view_scoped_contribution!(PanelTabDescriptor, |tab| tab.icon.clone());

/// Built views of one contribution kind per repository, and which one each
/// repository shows (absent: the built-in content).
pub(in crate::view) struct ViewRouter<D> {
    views: Rc<[(ContributionId, D)]>,
    selected: FxHashMap<std::path::PathBuf, usize>,
    built: FxHashMap<(RepoKey, usize), gpui::AnyView>,
    /// Repository views' action-bar contexts, built with their views.
    action_bars: FxHashMap<(RepoKey, usize), gpui::AnyView>,
    /// What each repository showed before a view's own tab opened with it,
    /// to come back to when the view is left.
    returns: FxHashMap<std::path::PathBuf, Option<usize>>,
}

pub(in crate::view) type RepositoryViewRouter = ViewRouter<RepositoryViewDescriptor>;

impl<D: RoutedContribution + Clone> ViewRouter<D> {
    /// `None` when nothing of this kind is registered.
    fn new(views: &[(ContributionId, D)]) -> Option<Self> {
        if views.is_empty() {
            return None;
        }
        Some(Self {
            views: views.to_vec().into(),
            selected: FxHashMap::default(),
            built: FxHashMap::default(),
            action_bars: FxHashMap::default(),
            returns: FxHashMap::default(),
        })
    }

    /// Whether contribution `index` is listed while `active` is the view.
    fn listed(&self, index: usize, active: &ViewTarget) -> bool {
        self.views
            .get(index)
            .is_some_and(|(_, view)| view.view().is_none_or(|view| view == active))
    }

    /// The tabs listed while `active` is the view, with their positions.
    fn listed_tabs(&self, active: &ViewTarget) -> Vec<(usize, SharedString, Option<SharedString>)> {
        self.views
            .iter()
            .enumerate()
            .filter(|(index, _)| self.listed(*index, active))
            .map(|(index, (_, view))| (index, view.title(), view.icon()))
            .collect()
    }

    fn index_of(&self, id: &ContributionId) -> Option<usize> {
        self.views.iter().position(|(candidate, _)| candidate == id)
    }

    /// What `repo` should show once `active` is its view: `Some` when the
    /// selection changes. A tab that opens with the view is selected, keeping
    /// what to return to; a tab no longer listed gives way to that.
    fn follow(&mut self, repo: &RepoState, active: &ViewTarget) -> Option<Option<usize>> {
        let path = &repo.spec.workdir;
        let current = self.selected(repo);
        let opener = self
            .views
            .iter()
            .position(|(_, view)| view.opens_with_view() && view.view() == Some(active));
        if let Some(open) = opener {
            if current == Some(open) {
                return None;
            }
            // Leaving one view's tab for another's keeps the first return.
            let back = match current {
                Some(current) if !self.listed(current, active) => {
                    self.returns.get(path).copied().flatten()
                }
                current => current,
            };
            self.returns.insert(path.clone(), back);
            return Some(Some(open));
        }
        let current = current?;
        if self.listed(current, active) {
            return None;
        }
        let back = self
            .returns
            .remove(path)
            .flatten()
            .filter(|back| self.listed(*back, active));
        Some(back)
    }

    pub(in crate::view) fn len(&self) -> usize {
        self.views.len()
    }

    fn titles(&self) -> Vec<SharedString> {
        self.views.iter().map(|(_, view)| view.title()).collect()
    }

    /// The selected view for `repo`: `None` is the built-in content.
    pub(in crate::view) fn selected(&self, repo: &RepoState) -> Option<usize> {
        self.selected.get(&repo.spec.workdir).copied()
    }

    /// The view to show instead of the built-in content, if one is selected.
    pub(in crate::view) fn active_view(&self, repo: &RepoState) -> Option<gpui::AnyView> {
        let key = repo_key(repo);
        let index = *self.selected.get(&repo.spec.workdir)?;
        self.built.get(&(key, index)).cloned()
    }

    pub(in crate::view) fn built(&self, repo: &RepoState, index: usize) -> Option<gpui::AnyView> {
        self.built.get(&(repo_key(repo), index)).cloned()
    }

    /// The selected view's action-bar context, if it has one.
    fn active_action_bar(&self, repo: &RepoState) -> Option<gpui::AnyView> {
        let index = *self.selected.get(&repo.spec.workdir)?;
        self.action_bars.get(&(repo_key(repo), index)).cloned()
    }

    /// Forgets views and selections of repositories no longer open.
    pub(in crate::view) fn retain_open(&mut self, state: &AppState) {
        if self.selected.is_empty() && self.built.is_empty() && self.action_bars.is_empty() {
            return;
        }
        let open = |key: &RepoKey| {
            state
                .repos
                .iter()
                .any(|repo| repo.id == key.0 && repo.lifetime() == key.1)
        };
        self.selected
            .retain(|path, _| state.repos.iter().any(|repo| &repo.spec.workdir == path));
        self.returns
            .retain(|path, _| state.repos.iter().any(|repo| &repo.spec.workdir == path));
        self.built.retain(|(key, _), _| open(key));
        self.action_bars.retain(|(key, _), _| open(key));
    }
}

/// Sidebar sections and which of them are collapsed in this window.
pub(in crate::view) struct SidebarSections {
    router: ViewRouter<SidebarSectionDescriptor>,
    collapsed: FxHashSet<usize>,
}

/// Which router a selection belongs to.
#[derive(Clone, Copy)]
pub(in crate::view) enum RoutedArea {
    Main,
    Details,
    Sidebar,
}

/// A registered repository view as the action bar's tabs show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) struct ViewTab {
    /// The view's position among the registered views.
    pub(in crate::view) index: usize,
    pub(in crate::view) title: SharedString,
    pub(in crate::view) icon: Option<SharedString>,
    /// Listed in the More menu rather than as a tab.
    pub(in crate::view) under_more: bool,
}

/// The active repository's views for the action bar: History, then each
/// registered view, and which one is showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::view) struct ViewTabs {
    pub(in crate::view) views: Vec<ViewTab>,
    /// The selected view's index; `None` is History.
    pub(in crate::view) selected: Option<usize>,
}

/// The More tab's invoker, so the tab shows open while its menu does.
pub(in crate::view) const MORE_VIEWS_INVOKER: &str = "repository_view_more";

impl GitCometView {
    pub(in crate::view) fn extension_navigation_context(
        &self,
    ) -> Option<(
        RepositoryViewContext,
        Option<gitcomet_extension_api::ViewNavigation>,
    )> {
        let repo = self.active_repo()?;
        let router = self.repository_views.as_ref()?;
        let selected = router.selected(repo)?;
        let host = self.extension_window.as_ref()?.host();
        Some((
            RepositoryViewContext {
                window: host,
                repository: super::extension_host::repository_handle(
                    self.window_handle.window_id(),
                    repo,
                ),
            },
            router.views.get(selected)?.1.navigation.clone(),
        ))
    }

    pub(in crate::view) fn sync_extension_navigation(&self, cx: &mut gpui::Context<Self>) {
        let navigation = self.extension_navigation_context();
        let active_view = self
            .active_repo()
            .and_then(|repo| {
                self.repository_views.as_ref().and_then(|router| {
                    router.selected(repo).and_then(|index| {
                        router.views.get(index).map(|(id, _)| {
                            gitcomet_extension_api::ViewTarget::Extension(id.clone())
                        })
                    })
                })
            })
            .unwrap_or(gitcomet_extension_api::ViewTarget::History);
        self.bottom_status_bar
            .update(cx, |bar, cx| bar.set_active_view(active_view.clone(), cx));
        let enabled = !self.window_gated && navigation.is_none();
        let slot = navigation.as_ref().and_then(|_| {
            let repo = self.active_repo()?;
            self.repository_views.as_ref()?.active_action_bar(repo)
        });
        let tabs = self.repository_view_tabs();
        self.action_bar.update(cx, |bar, cx| {
            bar.set_active_view(active_view, cx);
            bar.set_extension_navigation(navigation, slot, cx);
            bar.set_view_tabs(tabs, cx);
        });
        crate::app::set_diff_fallback_enabled(self.window_handle.window_id(), enabled, cx);
    }

    pub(in crate::view) fn route_extension_navigation(
        &self,
        forward: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some((context, navigation)) = self.extension_navigation_context() else {
            return false;
        };
        if let Some(navigation) = navigation {
            let allowed = if forward {
                &navigation.can_forward
            } else {
                &navigation.can_back
            };
            if allowed(&context, cx) {
                let run = if forward {
                    navigation.forward
                } else {
                    navigation.back
                };
                cx.defer(move |cx| run(context, cx));
            }
        }
        true
    }

    /// The action bar's view tabs for the active repository: `None` when no
    /// view is registered or no repository is open, so the bar is as before.
    pub(in crate::view) fn repository_view_tabs(&self) -> Option<ViewTabs> {
        let router = self.repository_views.as_ref()?;
        let repo = self.active_repo()?;
        Some(ViewTabs {
            views: router
                .views
                .iter()
                .enumerate()
                .map(|(index, (_, view))| ViewTab {
                    index,
                    title: view.title(),
                    icon: view.icon(),
                    under_more: view.under_more,
                })
                .collect(),
            selected: router.selected(repo),
        })
    }

    /// The views listed under More, as a menu below `anchor`.
    pub(in crate::view) fn open_more_views_menu(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use gitcomet_extension_api::{HostedAction, HostedMenuItem};
        // A hosted menu survives the gate; the views behind it must not.
        if self.window_gated {
            return;
        }
        let Some(tabs) = self.repository_view_tabs() else {
            return;
        };
        let root = cx.weak_entity();
        let window_handle = window.window_handle();
        let items: Vec<HostedMenuItem> = tabs
            .views
            .into_iter()
            .filter(|view| view.under_more)
            .map(|view| {
                let root = root.clone();
                let index = view.index;
                let item = HostedMenuItem::action(HostedAction::new(view.title, move |cx| {
                    let root = root.clone();
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = root.update(cx, |root, cx| {
                            root.select_routed_view(RoutedArea::Main, Some(index), window, cx)
                        });
                    });
                }));
                match view.icon {
                    Some(icon) => item.with_icon(icon),
                    None => item,
                }
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let id = super::extension_host::next_dialog_id();
        self.popover_host
            .update(cx, |host, _| host.set_hosted_menu(id, items));
        self.open_popover_for_bounds(
            PopoverKind::Hosted { id, menu: true }.invoked_by(MORE_VIEWS_INVOKER.into()),
            anchor,
            window,
            cx,
        );
    }

    pub(in crate::view) fn repository_view_router(cx: &App) -> Option<RepositoryViewRouter> {
        ViewRouter::new(super::extension_host::registry(cx)?.repository_views())
    }

    pub(in crate::view) fn details_tab_router(
        cx: &App,
    ) -> Option<ViewRouter<DetailsTabDescriptor>> {
        ViewRouter::new(super::extension_host::registry(cx)?.details_tabs())
    }

    pub(in crate::view) fn sidebar_tab_router(
        cx: &App,
    ) -> Option<ViewRouter<SidebarTabDescriptor>> {
        ViewRouter::new(super::extension_host::registry(cx)?.sidebar_tabs())
    }

    pub(in crate::view) fn sidebar_section_router(cx: &App) -> Option<SidebarSections> {
        Some(SidebarSections {
            router: ViewRouter::new(super::extension_host::registry(cx)?.sidebar_sections())?,
            collapsed: FxHashSet::default(),
        })
    }

    pub(in crate::view) fn retain_open_repository_views(&mut self) {
        if let Some(router) = self.repository_views.as_mut() {
            router.retain_open(&self.state);
        }
        if let Some(router) = self.details_tabs.as_mut() {
            router.retain_open(&self.state);
        }
        if let Some(router) = self.sidebar_tabs.as_mut() {
            router.retain_open(&self.state);
        }
        if let Some(sections) = self.sidebar_sections.as_mut() {
            sections.router.retain_open(&self.state);
        }
    }

    /// Builds `build` for `repo` with a context naming this window.
    fn build_routed(
        &self,
        build: ViewBuilder<RepositoryViewContext>,
        repo: &RepoState,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<gpui::AnyView> {
        let host = self
            .extension_window
            .as_ref()
            .map(super::extension_host::ExtensionWindow::host)?;
        let context = RepositoryViewContext {
            window: host,
            repository: super::extension_host::repository_handle(
                self.window_handle.window_id(),
                repo,
            ),
        };
        super::perf::extension_dispatch();
        Some(build(context, window, cx))
    }

    fn panel_content_policy(&self, repo: &RepoState, area: PanelArea) -> PanelContentPolicy {
        self.repository_views
            .as_ref()
            .and_then(|router| {
                router
                    .selected(repo)
                    .and_then(|index| router.views.get(index))
            })
            .map_or(PanelContentPolicy::default(), |(_, view)| {
                view.panel_content(area)
            })
    }

    fn panel_router(&self, area: PanelArea) -> Option<&ViewRouter<PanelTabDescriptor>> {
        match area {
            PanelArea::Details => self.details_tabs.as_ref(),
            PanelArea::Sidebar => self.sidebar_tabs.as_ref(),
            _ => None,
        }
    }

    fn panel_has_host_tabs(&self, repo: &RepoState, area: PanelArea) -> bool {
        self.panel_content_policy(repo, area) != PanelContentPolicy::ExtensionsOnly
            || self.panel_router(area).is_none_or(|router| {
                router
                    .listed_tabs(&self.active_view_target(repo))
                    .is_empty()
            })
    }

    pub(in crate::view) fn active_view_target(&self, repo: &RepoState) -> ViewTarget {
        self.repository_views
            .as_ref()
            .and_then(|router| {
                let index = router.selected(repo)?;
                router.views.get(index).map(|(id, _)| id.clone())
            })
            .map(ViewTarget::Extension)
            .unwrap_or(ViewTarget::History)
    }

    /// `repo` changed views: its view's own details and sidebar tabs open,
    /// and tabs of the view it left give way.
    fn follow_view(&mut self, repo: &RepoState, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let active = self.active_view_target(repo);
        for area in [PanelArea::Details, PanelArea::Sidebar] {
            let replaces = !self.panel_has_host_tabs(repo, area);
            let router = match area {
                PanelArea::Details => self.details_tabs.as_mut(),
                PanelArea::Sidebar => self.sidebar_tabs.as_mut(),
                _ => continue,
            };
            let Some(router) = router else { continue };
            let next = router.follow(repo, &active).or_else(|| {
                (replaces && router.selected(repo).is_none()).then(|| {
                    router
                        .views
                        .iter()
                        .enumerate()
                        .find(|(index, _)| router.listed(*index, &active))
                        .map(|(index, _)| index)
                })
            });
            if let Some(index) = next {
                let routed = match area {
                    PanelArea::Details => RoutedArea::Details,
                    PanelArea::Sidebar => RoutedArea::Sidebar,
                    _ => continue,
                };
                self.select_routed_for_repo(repo.clone(), routed, index, window, cx);
            }
        }
    }

    /// The active repository's sidebar tabs, as its sidebar draws them.
    pub(in crate::view) fn sync_sidebar_tabs(&self, cx: &mut gpui::Context<Self>) {
        let tabs = self
            .sidebar_tabs
            .as_ref()
            .zip(self.active_repo())
            .map(|(router, repo)| {
                let active = self.active_view_target(repo);
                crate::view::panes::SidebarExtensionTabs {
                    tabs: router.listed_tabs(&active),
                    selected: router.selected(repo),
                    view: router.active_view(repo),
                    host_tabs: self.panel_has_host_tabs(repo, PanelArea::Sidebar),
                }
            });
        self.sidebar_pane
            .update(cx, |pane, cx| pane.set_extension_tabs(tabs, cx));
    }

    /// Panel tabs share repository routing and lifetime checks; revealing
    /// differs only by placement.
    pub(in crate::view) fn show_panel_tab(
        &mut self,
        repository: &gitcomet_extension_api::RepositoryHandle,
        area: PanelArea,
        id: &ContributionId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo) = self.repository_state(repository) else {
            return false;
        };
        let active = self.active_view_target(&repo);
        let Some(index) = self.panel_router(area).and_then(|router| {
            router
                .index_of(id)
                .filter(|index| router.listed(*index, &active))
        }) else {
            return false;
        };
        match area {
            PanelArea::Details => {
                self.select_routed_for_repo(repo, RoutedArea::Details, Some(index), window, cx);
                self.set_details_collapsed(false, cx);
            }
            PanelArea::Sidebar => {
                self.select_routed_for_repo(repo, RoutedArea::Sidebar, Some(index), window, cx);
                self.set_sidebar_collapsed(false, cx);
            }
            _ => return false,
        }
        true
    }

    fn repository_state(
        &self,
        repository: &gitcomet_extension_api::RepositoryHandle,
    ) -> Option<RepoState> {
        self.state
            .repos
            .iter()
            .find(|repo| {
                repo.id == repository.repo_id() && repo.lifetime() == repository.lifetime()
            })
            .cloned()
    }

    /// Shows the built-in content (`None`) or contribution `index` in
    /// `area` for the active repository, building the view the first time it
    /// is chosen.
    pub(in crate::view) fn select_routed_view(
        &mut self,
        area: RoutedArea,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo().cloned() else {
            return;
        };
        self.select_routed_for_repo(repo, area, index, window, cx);
    }

    pub(in crate::view) fn repository_view_index(&self, id: &ContributionId) -> Option<usize> {
        self.repository_views
            .as_ref()?
            .views
            .iter()
            .position(|(candidate, _)| candidate == id)
    }

    pub(in crate::view) fn select_repository_view(
        &mut self,
        repository: &gitcomet_extension_api::RepositoryHandle,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self
            .state
            .repos
            .iter()
            .find(|repo| {
                repo.id == repository.repo_id() && repo.lifetime() == repository.lifetime()
            })
            .cloned()
        else {
            return;
        };
        self.select_routed_for_repo(repo, RoutedArea::Main, index, window, cx);
    }

    fn select_routed_for_repo(
        &mut self,
        repo: RepoState,
        area: RoutedArea,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let key = repo_key(&repo);
        let (count, current, build, bar_build) = match area {
            RoutedArea::Main => {
                let Some(router) = self.repository_views.as_ref() else {
                    return;
                };
                let unbuilt = index
                    .filter(|index| router.built(&repo, *index).is_none())
                    .and_then(|index| router.views.get(index))
                    .map(|(_, view)| view);
                (
                    router.len(),
                    router.selected(&repo),
                    unbuilt.map(|view| view.builder()),
                    unbuilt.and_then(|view| view.action_bar.clone()),
                )
            }
            RoutedArea::Details => {
                let Some(router) = self.details_tabs.as_ref() else {
                    return;
                };
                let build = index
                    .filter(|index| router.built(&repo, *index).is_none())
                    .and_then(|index| router.views.get(index))
                    .map(|(_, view)| view.builder());
                (router.len(), router.selected(&repo), build, None)
            }
            RoutedArea::Sidebar => {
                let Some(router) = self.sidebar_tabs.as_ref() else {
                    return;
                };
                let build = index
                    .filter(|index| router.built(&repo, *index).is_none())
                    .and_then(|index| router.views.get(index))
                    .map(|(_, view)| view.builder());
                (router.len(), router.selected(&repo), build, None)
            }
        };
        let index = index.filter(|index| *index < count);
        if current == index {
            return;
        }
        // Built before the router is borrowed again: the builder may reach
        // the router through the host.
        let built = build.and_then(|build| self.build_routed(build, &repo, window, cx));
        let bar = built
            .as_ref()
            .and(bar_build)
            .and_then(|build| self.build_routed(build, &repo, window, cx));
        let (selected, views, action_bars) = match area {
            RoutedArea::Main => match self.repository_views.as_mut() {
                Some(router) => (
                    &mut router.selected,
                    &mut router.built,
                    Some(&mut router.action_bars),
                ),
                None => return,
            },
            RoutedArea::Details => match self.details_tabs.as_mut() {
                Some(router) => (&mut router.selected, &mut router.built, None),
                None => return,
            },
            RoutedArea::Sidebar => match self.sidebar_tabs.as_mut() {
                Some(router) => (&mut router.selected, &mut router.built, None),
                None => return,
            },
        };
        match index {
            Some(index) => {
                if let Some(view) = built {
                    views.insert((key, index), view);
                }
                if let (Some(bar), Some(action_bars)) = (bar, action_bars) {
                    action_bars.insert((key, index), bar);
                }
                if !views.contains_key(&(key, index)) {
                    return;
                }
                selected.insert(repo.spec.workdir.clone(), index);
            }
            None => {
                selected.remove(&repo.spec.workdir);
            }
        }
        if matches!(area, RoutedArea::Main) {
            self.follow_view(&repo, window, cx);
            // The old view may have hidden the field that held focus. Give
            // it to the newly selected view even before its first frame.
            let focus = self.repository_views.as_ref().and_then(|router| {
                let index = router.selected(&repo)?;
                let (_, descriptor) = router.views.get(index)?;
                let view = router.built(&repo, index)?;
                descriptor.focus.as_ref()?.as_ref()(&view, cx)
            });
            if let Some(focus) = focus {
                window.focus(&focus, cx);
            } else if index.is_some() {
                window.focus(&self.repository_view_focus, cx);
            } else {
                let focus = self.main_pane.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
            }
        }
        self.sync_sidebar_tabs(cx);
        self.sync_extension_navigation(cx);
        if let Some(extension) = &self.extension_window {
            extension.emit(gitcomet_extension_api::ShellEvent::ViewChanged, cx);
        }
        cx.notify();
    }

    /// The details area's strip of tabs: the details pane's own first, then
    /// each contribution with its icon.
    fn details_strip(
        &self,
        tabs: Vec<(usize, SharedString, Option<SharedString>)>,
        selected: Option<usize>,
        own: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let mut strip = components::navigation_tab_strip(theme.colors.surface.canvas, ui_scale)
            .id("details_tab_strip")
            .debug_selector(|| "details_tab_strip".to_string())
            .border_b_1()
            .border_color(theme.colors.stroke.subtle);
        let builtin = ("Details".into(), Some("icons/side_panel_right.svg".into()));
        let entries = own
            .then(|| ("details_tab_details".to_string(), builtin, None))
            .into_iter()
            .chain(tabs.into_iter().map(|(index, title, icon)| {
                (format!("details_tab_{index}"), (title, icon), Some(index))
            }));
        for (id, (title, icon), index) in entries {
            let mut tab = components::NavTab::new(id, title).selected(selected == index);
            if let Some(icon) = icon {
                tab = tab.icon(icon);
            }
            strip = strip.child(tab.render(theme, ui_scale).on_activate(
                false,
                controls::ControlActivation::ManagedFocus,
                cx.listener(move |this, _, window, cx| {
                    this.select_routed_view(RoutedArea::Details, index, window, cx);
                }),
            ));
        }
        strip
    }

    /// The main area for the active repository: History, the extension view
    /// selected from the action bar's tabs, or an open standalone document.
    pub(in crate::view) fn repository_main_content(&mut self) -> AnyElement {
        // A standalone document takes the main slot; the panes around it stay.
        if self.documents_active {
            return stable_cached_fill_view(self.documents.clone());
        }
        let history = || stable_cached_fill_view(self.main_pane.clone());
        let active = self
            .repository_views
            .as_ref()
            .zip(self.active_repo())
            .and_then(|(router, repo)| router.active_view(repo));
        match active {
            // The scope `diff_shortcut_target` searches for hosted panes.
            Some(view) => div()
                .size_full()
                .track_focus(&self.repository_view_focus)
                .child(view)
                .into_any_element(),
            None => history(),
        }
    }

    /// The details area with extension tabs, or `None` when no extension
    /// tab is listed in the active repository's view (the details pane then
    /// mounts exactly as before).
    pub(in crate::view) fn details_tab_content(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let router = self.details_tabs.as_ref()?;
        let details = || {
            div()
                .flex_1()
                .min_h(px(0.0))
                .child(stable_cached_fill_view(self.details_pane.clone()))
                .into_any_element()
        };
        let Some(repo) = self.active_repo() else {
            return Some(details());
        };
        let tabs = router.listed_tabs(&self.active_view_target(repo));
        if tabs.is_empty() {
            return None;
        }
        let selected = router.selected(repo);
        // A view whose tabs replace Details shows one of them; the tab that
        // opens with it is selected when it opens.
        let own = self.panel_has_host_tabs(repo, PanelArea::Details);
        let active = router.active_view(repo).or_else(|| {
            (!own)
                .then(|| {
                    tabs.iter()
                        .find_map(|(index, ..)| router.built(repo, *index))
                })
                .flatten()
        });
        let strip = self.details_strip(tabs, selected, own, cx);
        let body = match active {
            Some(view) => div().flex_1().min_h(px(0.0)).child(view).into_any_element(),
            None => details(),
        };
        Some(
            div()
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .flex_col()
                .child(strip)
                .child(body)
                .into_any_element(),
        )
    }

    /// Extension sections for the active repository below the sidebar's
    /// own, built on first render for each repository; `None` when no
    /// extension registers one.
    pub(in crate::view) fn sidebar_section_content(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let sections = self.sidebar_sections.as_ref()?;
        let repo = self.active_repo()?.clone();
        let missing: Vec<(usize, ViewBuilder<RepositoryViewContext>)> = sections
            .router
            .views
            .iter()
            .enumerate()
            .filter(|(index, _)| sections.router.built(&repo, *index).is_none())
            .map(|(index, (_, view))| (index, view.builder()))
            .collect();
        for (index, build) in missing {
            if let Some(view) = self.build_routed(build, &repo, window, cx)
                && let Some(sections) = self.sidebar_sections.as_mut()
            {
                sections.router.built.insert((repo_key(&repo), index), view);
            }
        }
        let sections = self.sidebar_sections.as_ref()?;
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let titles = sections.router.titles();
        let mut column = div()
            .id("sidebar_extension_sections")
            .debug_selector(|| "sidebar_extension_sections".to_string())
            .flex_none()
            .flex()
            .flex_col()
            .max_h(gpui::relative(0.5))
            .border_t_1()
            .border_color(theme.colors.stroke.subtle);
        for (index, title) in titles.into_iter().enumerate() {
            let collapsed = sections.collapsed.contains(&index);
            let header = div()
                .id(("sidebar_extension_section", index))
                .debug_selector(move || format!("sidebar_extension_section_{index}"))
                .flex()
                .items_center()
                .gap(ui_scale.px(4.0))
                .px(ui_scale.px(10.0))
                .py(ui_scale.px(4.0))
                .cursor_pointer()
                .text_size(theme.ui_text(11.0))
                .text_color(theme.colors.foreground.secondary)
                .child(svg_icon(
                    if collapsed {
                        "icons/chevron_right.svg"
                    } else {
                        "icons/chevron_down.svg"
                    },
                    theme.colors.foreground.secondary,
                    ui_scale.px(12.0),
                ))
                .child(title)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        if let Some(sections) = this.sidebar_sections.as_mut()
                            && !sections.collapsed.remove(&index)
                        {
                            sections.collapsed.insert(index);
                        }
                        cx.notify();
                    }),
                );
            column = column.child(header);
            if !collapsed && let Some(view) = sections.router.built(&repo, index) {
                column = column.child(div().flex_none().child(view));
            }
        }
        Some(column.into_any_element())
    }
}
