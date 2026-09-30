//! The repository-view router: History or one of the extensions' repository
//! views, chosen per repository in each window.
//!
//! Exists only when an extension registers a repository view; otherwise the
//! main area renders exactly as before and no strip is shown. An inactive
//! History pane is not rendered but keeps its state, and an extension's view
//! is built on first selection and kept until its repository closes.

use super::*;
use gitcomet_extension_api::{ContributionId, RepositoryViewDescriptor};
use std::rc::Rc;

/// A repository as the router keys it: ids are reused, lifetimes are not.
type RepoKey = (RepoId, u64);

pub(in crate::view) struct RepositoryViewRouter {
    views: Rc<[(ContributionId, RepositoryViewDescriptor)]>,
    /// Selected view index per repository; absent means History.
    selected: FxHashMap<RepoKey, usize>,
    built: FxHashMap<(RepoKey, usize), gpui::AnyView>,
}

impl RepositoryViewRouter {
    /// `None` unless an extension registered a repository view.
    pub(in crate::view) fn for_window(cx: &App) -> Option<Self> {
        let registry = super::extension_host::registry(cx)?;
        if registry.repository_views().is_empty() {
            return None;
        }
        Some(Self {
            views: registry.repository_views().to_vec().into(),
            selected: FxHashMap::default(),
            built: FxHashMap::default(),
        })
    }

    fn key(repo: &RepoState) -> RepoKey {
        (repo.id, repo.lifetime())
    }

    /// The selected view for `repo`: `None` is History.
    pub(in crate::view) fn selected(&self, repo: &RepoState) -> Option<usize> {
        self.selected.get(&Self::key(repo)).copied()
    }

    /// The view to show instead of History for `repo`, if one is selected.
    pub(in crate::view) fn active_view(&self, repo: &RepoState) -> Option<gpui::AnyView> {
        let key = Self::key(repo);
        let index = *self.selected.get(&key)?;
        self.built.get(&(key, index)).cloned()
    }

    /// Forgets views and selections of repositories no longer open.
    pub(in crate::view) fn retain_open(&mut self, state: &AppState) {
        if self.selected.is_empty() && self.built.is_empty() {
            return;
        }
        let open = |key: &RepoKey| {
            state
                .repos
                .iter()
                .any(|repo| repo.id == key.0 && repo.lifetime() == key.1)
        };
        self.selected.retain(|key, _| open(key));
        self.built.retain(|(key, _), _| open(key));
    }
}

impl GitCometView {
    /// Shows History (`None`) or repository view `index` for the active
    /// repository, building the view the first time it is chosen.
    pub(in crate::view) fn select_repository_view(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let window_id = self.window_handle.window_id();
        let Some(repo) = self.active_repo().cloned() else {
            return;
        };
        let Some(router) = self.repository_views.as_mut() else {
            return;
        };
        let key = RepositoryViewRouter::key(&repo);
        let Some(index) = index.filter(|index| *index < router.views.len()) else {
            if router.selected.remove(&key).is_some() {
                cx.notify();
            }
            return;
        };
        if router.selected.get(&key) == Some(&index) {
            return;
        }
        if !router.built.contains_key(&(key, index)) {
            let build = Rc::clone(&router.views[index].1.build);
            let Some(host) = self
                .extension_window
                .as_ref()
                .map(super::extension_host::ExtensionWindow::host)
            else {
                return;
            };
            let context = gitcomet_extension_api::RepositoryViewContext {
                window: host,
                repository: super::extension_host::repository_handle(window_id, &repo),
            };
            let view = build(context, window, cx);
            // Rebind: the builder may have touched the router through the host.
            if let Some(router) = self.repository_views.as_mut() {
                router.built.insert((key, index), view);
            }
        }
        if let Some(router) = self.repository_views.as_mut() {
            router.selected.insert(key, index);
        }
        cx.notify();
    }

    /// The main area for the active repository: History, or the selected
    /// extension view under a strip naming both.
    pub(in crate::view) fn repository_main_content(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let history = || stable_cached_fill_view(self.main_pane.clone());
        let (Some(router), Some(repo)) = (self.repository_views.as_ref(), self.active_repo())
        else {
            return history();
        };
        let selected = router.selected(repo);
        let active = router.active_view(repo);
        let titles: Vec<SharedString> = router
            .views
            .iter()
            .map(|(_, view)| view.title.clone())
            .collect();
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let mut strip = components::navigation_tab_strip(theme.colors.surface.canvas, ui_scale)
            .id("repository_view_strip")
            .debug_selector(|| "repository_view_strip".to_string())
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(components::navigation_tab_metrics(
                components::navigation_tab(
                    "repository_view_history",
                    "History",
                    selected.is_none(),
                    None,
                    theme,
                )
                .on_click(theme, cx, |this, _, window, cx| {
                    this.select_repository_view(None, window, cx);
                }),
                theme,
                ui_scale,
            ));
        for (index, title) in titles.into_iter().enumerate() {
            strip = strip.child(components::navigation_tab_metrics(
                components::navigation_tab(
                    format!("repository_view_{index}"),
                    title,
                    selected == Some(index),
                    None,
                    theme,
                )
                .on_click(theme, cx, move |this, _, window, cx| {
                    this.select_repository_view(Some(index), window, cx);
                }),
                theme,
                ui_scale,
            ));
        }
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
}
