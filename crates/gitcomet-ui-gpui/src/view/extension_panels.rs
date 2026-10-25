//! Extension bottom panels of one window: which are open per repository and
//! their views. The extension host and the root view share it, so opening,
//! closing, and asking never touch the root view.

use super::*;
use gitcomet_extension_api::{BottomPanelDescriptor, ContributionId};
use std::cell::RefCell;
use std::rc::Rc;

/// A repository as panels key it: ids are reused, lifetimes are not.
pub(in crate::view) type RepoKey = (RepoId, u64);

pub(in crate::view) fn repo_key(repo: &RepoState) -> RepoKey {
    (repo.id, repo.lifetime())
}

#[derive(Default)]
pub(in crate::view) struct BottomPanels {
    descriptors: Rc<[(ContributionId, BottomPanelDescriptor)]>,
    /// Open panels per repository in opening order, with their views once
    /// built.
    open: FxHashMap<RepoKey, Vec<(usize, Option<gpui::AnyView>)>>,
}

pub(in crate::view) type SharedBottomPanels = Rc<RefCell<BottomPanels>>;

impl BottomPanels {
    pub(in crate::view) fn new(descriptors: &[(ContributionId, BottomPanelDescriptor)]) -> Self {
        Self {
            descriptors: descriptors.to_vec().into(),
            open: FxHashMap::default(),
        }
    }

    pub(in crate::view) fn index_of(&self, id: &ContributionId) -> Option<usize> {
        self.descriptors
            .iter()
            .position(|(candidate, _)| candidate == id)
    }

    pub(in crate::view) fn descriptor(&self, index: usize) -> Option<&BottomPanelDescriptor> {
        self.descriptors
            .get(index)
            .map(|(_, descriptor)| descriptor)
    }

    pub(in crate::view) fn open(&mut self, key: RepoKey, index: usize) {
        let panels = self.open.entry(key).or_default();
        if !panels.iter().any(|(open, _)| *open == index) {
            panels.push((index, None));
        }
    }

    /// Whether it was open.
    pub(in crate::view) fn close(&mut self, key: RepoKey, index: usize) -> bool {
        let Some(panels) = self.open.get_mut(&key) else {
            return false;
        };
        let before = panels.len();
        panels.retain(|(open, _)| *open != index);
        let closed = panels.len() != before;
        if panels.is_empty() {
            self.open.remove(&key);
        }
        closed
    }

    pub(in crate::view) fn is_open(&self, key: RepoKey, index: usize) -> bool {
        self.open
            .get(&key)
            .is_some_and(|panels| panels.iter().any(|(open, _)| *open == index))
    }

    /// Whether `index` is open without a view yet.
    pub(in crate::view) fn needs_view(&self, key: RepoKey, index: usize) -> bool {
        self.open.get(&key).is_some_and(|panels| {
            panels
                .iter()
                .any(|(open, view)| *open == index && view.is_none())
        })
    }

    pub(in crate::view) fn set_view(&mut self, key: RepoKey, index: usize, view: gpui::AnyView) {
        if let Some(slot) = self
            .open
            .get_mut(&key)
            .and_then(|panels| panels.iter_mut().find(|(open, _)| *open == index))
        {
            slot.1 = Some(view);
        }
    }

    fn view(&self, key: RepoKey, index: usize) -> Option<gpui::AnyView> {
        self.open
            .get(&key)?
            .iter()
            .find(|(candidate, _)| *candidate == index)?
            .1
            .clone()
    }

    /// The open panels of `key` that have views, in opening order.
    #[cfg(test)]
    pub(in crate::view) fn shown(&self, key: RepoKey) -> Vec<(usize, gpui::AnyView)> {
        self.open
            .get(&key)
            .map(|panels| {
                panels
                    .iter()
                    .filter_map(|(index, view)| Some((*index, view.clone()?)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drops panels of repositories no longer open.
    pub(in crate::view) fn retain_open(&mut self, state: &AppState) {
        if self.open.is_empty() {
            return;
        }
        self.open.retain(|key, _| {
            state
                .repos
                .iter()
                .any(|repo| repo.id == key.0 && repo.lifetime() == key.1)
        });
    }
}

impl GitCometView {
    pub(in crate::view) fn extension_panel_view(&self, index: usize) -> Option<gpui::AnyView> {
        self.bottom_panels()?
            .borrow()
            .view(repo_key(self.active_repo()?), index)
    }

    fn bottom_panels(&self) -> Option<SharedBottomPanels> {
        self.extension_window
            .as_ref()
            .map(super::extension_host::ExtensionWindow::bottom_panels)
    }

    /// Builds extension panel `index` for `key` if it is open without a
    /// view, and brings it to the front.
    pub(in crate::view) fn show_extension_bottom_panel(
        &mut self,
        key: RepoKey,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(panels) = self.bottom_panels() else {
            return;
        };
        let Some(repo) = self
            .state
            .repos
            .iter()
            .find(|repo| repo_key(repo) == key)
            .cloned()
        else {
            panels.borrow_mut().close(key, index);
            return;
        };
        if !panels.borrow().is_open(key, index) {
            return;
        }
        if panels.borrow().needs_view(key, index) {
            let Some(build) = panels
                .borrow()
                .descriptor(index)
                .map(|descriptor| Rc::clone(&descriptor.build))
            else {
                return;
            };
            let Some(host) = self
                .extension_window
                .as_ref()
                .map(super::extension_host::ExtensionWindow::host)
            else {
                return;
            };
            let context = gitcomet_extension_api::RepositoryViewContext {
                window: host,
                repository: super::extension_host::repository_handle(
                    self.window_handle.window_id(),
                    &repo,
                ),
            };
            // Not borrowed while the extension builds: it may ask the host.
            super::perf::extension_dispatch();
            let view = build(context, window, cx);
            panels.borrow_mut().set_view(key, index, view);
        }
        self.active_bottom_panel
            .insert(repo.id, BottomPanelTab::Extension(index));
        cx.notify();
    }

    /// The panel closed: the strip falls back to the last one still open.
    pub(in crate::view) fn extension_bottom_panel_closed(
        &mut self,
        key: RepoKey,
        index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_bottom_panel.get(&key.0) == Some(&BottomPanelTab::Extension(index)) {
            self.active_bottom_panel.remove(&key.0);
        }
        cx.notify();
    }

    /// Closes extension panel `index` of the active repository from its tab.
    pub(in crate::view) fn close_extension_bottom_panel(
        &mut self,
        index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let (Some(panels), Some(key)) = (self.bottom_panels(), self.active_repo().map(repo_key))
        else {
            return;
        };
        let closed = panels.borrow_mut().close(key, index);
        if closed {
            self.extension_bottom_panel_closed(key, index, cx);
        }
    }

    pub(in crate::view) fn sync_extension_bottom_panels_with_state(&self) {
        if let Some(panels) = self.bottom_panels() {
            panels.borrow_mut().retain_open(&self.state);
        }
    }
}
