//! Extension views built per repository: the repository-view router
//! (History or one of the extensions' repository views in the main area),
//! details tabs beside the details pane's own content, and sidebar sections
//! below the sidebar's own.
//!
//! Each exists only when an extension registers that kind; otherwise its
//! area renders exactly as before. A view is built on first use for a
//! repository and kept until that repository closes; an unselected built-in
//! pane is not rendered but keeps its state.

use super::extension_panels::{RepoKey, repo_key};
use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_extension_api::{
    ContributionId, DetailsTabDescriptor, RepositoryViewContext, RepositoryViewDescriptor,
    SidebarSectionDescriptor, ViewBuilder,
};
use std::rc::Rc;

/// A contribution built per repository: its title, tab icon and builder.
pub(in crate::view) trait RoutedContribution {
    fn title(&self) -> SharedString;
    fn icon(&self) -> Option<SharedString>;
    fn builder(&self) -> ViewBuilder<RepositoryViewContext>;
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
routed_contribution!(DetailsTabDescriptor, |tab| tab.icon.clone());
routed_contribution!(SidebarSectionDescriptor, |_section| None);

/// Built views of one contribution kind per repository, and which one each
/// repository shows (absent: the built-in content).
pub(in crate::view) struct ViewRouter<D> {
    views: Rc<[(ContributionId, D)]>,
    selected: FxHashMap<std::path::PathBuf, usize>,
    built: FxHashMap<(RepoKey, usize), gpui::AnyView>,
    /// Repository views' action-bar contexts, built with their views.
    action_bars: FxHashMap<(RepoKey, usize), gpui::AnyView>,
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
        })
    }

    pub(in crate::view) fn len(&self) -> usize {
        self.views.len()
    }

    fn titles(&self) -> Vec<SharedString> {
        self.views.iter().map(|(_, view)| view.title()).collect()
    }

    fn tabs(&self) -> Vec<(SharedString, Option<SharedString>)> {
        self.views
            .iter()
            .map(|(_, view)| (view.title(), view.icon()))
            .collect()
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
        self.built.retain(|(key, _), _| open(key));
        self.action_bars.retain(|(key, _), _| open(key));
    }
}

/// Sidebar sections and which of them are collapsed in this window.
pub(in crate::view) struct SidebarSections {
    router: ViewRouter<SidebarSectionDescriptor>,
    collapsed: FxHashSet<usize>,
}

/// Which router a strip or a selection belongs to.
#[derive(Clone, Copy)]
pub(in crate::view) enum RoutedArea {
    Main,
    Details,
}

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
            .update(cx, |bar, cx| bar.set_active_view(active_view, cx));
        let enabled = !self.window_gated && navigation.is_none();
        let slot = navigation.as_ref().and_then(|_| {
            let repo = self.active_repo()?;
            self.repository_views.as_ref()?.active_action_bar(repo)
        });
        self.action_bar.update(cx, |bar, cx| {
            bar.set_extension_navigation(navigation, slot, cx)
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

    pub(in crate::view) fn repository_view_router(cx: &App) -> Option<RepositoryViewRouter> {
        ViewRouter::new(super::extension_host::registry(cx)?.repository_views())
    }

    pub(in crate::view) fn details_tab_router(
        cx: &App,
    ) -> Option<ViewRouter<DetailsTabDescriptor>> {
        ViewRouter::new(super::extension_host::registry(cx)?.details_tabs())
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
        self.sync_extension_navigation(cx);
        if let Some(extension) = &self.extension_window {
            extension.emit(gitcomet_extension_api::ShellEvent::ViewChanged, cx);
        }
        cx.notify();
    }

    /// A strip of navigation tabs: the built-in content first, then each
    /// contribution with its icon.
    fn routed_strip(
        &self,
        area: RoutedArea,
        tabs: Vec<(SharedString, Option<SharedString>)>,
        selected: Option<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let (prefix, builtin_id, builtin, builtin_icon) = match area {
            RoutedArea::Main => (
                "repository_view",
                "repository_view_history",
                "History",
                "icons/history.svg",
            ),
            RoutedArea::Details => (
                "details_tab",
                "details_tab_details",
                "Details",
                "icons/side_panel_right.svg",
            ),
        };
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let strip_id = format!("{prefix}_strip");
        let selector = strip_id.clone();
        let builtin = (builtin.into(), Some(builtin_icon.into()));
        let mut strip = components::navigation_tab_strip(theme.colors.surface.canvas, ui_scale)
            .id(SharedString::from(strip_id))
            .debug_selector(move || selector.clone())
            .border_b_1()
            .border_color(theme.colors.stroke.subtle);
        let entries = std::iter::once((builtin_id.to_string(), builtin, None)).chain(
            tabs.into_iter()
                .enumerate()
                .map(|(index, tab)| (format!("{prefix}_{index}"), tab, Some(index))),
        );
        for (id, (title, icon), index) in entries {
            let mut tab = components::NavTab::new(id, title).selected(selected == index);
            if let Some(icon) = icon {
                tab = tab.icon(icon);
            }
            strip = strip.child(tab.render(theme, ui_scale).on_activate(
                false,
                controls::ControlActivation::ManagedFocus,
                cx.listener(move |this, _, window, cx| {
                    this.select_routed_view(area, index, window, cx);
                }),
            ));
        }
        strip
    }

    /// The main area for the active repository: History, or the selected
    /// extension view under a strip naming both, or an open standalone document.
    pub(in crate::view) fn repository_main_content(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // A standalone document takes the main slot; the panes around it stay.
        if self.documents_active {
            return stable_cached_fill_view(self.documents.clone());
        }
        let history = || stable_cached_fill_view(self.main_pane.clone());
        let (Some(router), Some(repo)) = (self.repository_views.as_ref(), self.active_repo())
        else {
            return history();
        };
        let selected = router.selected(repo);
        let active = router.active_view(repo);
        let tabs = router.tabs();
        let strip = self.routed_strip(RoutedArea::Main, tabs, selected, cx);
        let body = match active {
            Some(view) => div().size_full().child(view).into_any_element(),
            None => history(),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(strip)
            .child(div().flex_1().min_h(px(0.0)).child(body))
            .into_any_element()
    }

    /// The details area with extension tabs, or `None` when no extension
    /// registers one (the details pane then mounts exactly as before).
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
        let selected = router.selected(repo);
        let active = router.active_view(repo);
        let tabs = router.tabs();
        let strip = self.routed_strip(RoutedArea::Details, tabs, selected, cx);
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
