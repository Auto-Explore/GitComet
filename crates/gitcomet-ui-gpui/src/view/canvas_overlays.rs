//! Extension canvas overlays of one window: which overlay covers each
//! repository's main canvas, and the views built for them. The extension
//! host and the root view share it, so showing, hiding and asking never touch
//! the root view. An overlay replaces the main slot's element only; History
//! and the selected repository view stay built behind it, and neither
//! `ViewChanged` nor `MainViewChanged` is emitted either way.

use super::extension_panels::{RepoKey, repo_key};
use super::repository_views::MainSlot;
use super::*;
use gitcomet_extension_api::{
    CanvasOverlayDescriptor, CanvasOverlayVisibility, ContributionId, RepositoryHandle,
    RepositoryViewContext, WindowHost,
};
use std::cell::RefCell;
use std::rc::Rc;

/// An overlay shown at least once over a repository, kept until dropped.
struct Entry {
    /// The handle it was shown for: its view's context, and what `Dropped`
    /// names after the repository closed.
    repository: RepositoryHandle,
    /// Built by the root after the show that recorded the entry.
    view: Option<gpui::AnyView>,
}

#[derive(Default)]
pub(in crate::view) struct CanvasOverlays {
    descriptors: Rc<[(ContributionId, CanvasOverlayDescriptor)]>,
    /// The overlay covering each repository's canvas.
    shown: FxHashMap<RepoKey, usize>,
    entries: FxHashMap<(RepoKey, usize), Entry>,
}

pub(in crate::view) type SharedCanvasOverlays = Rc<RefCell<CanvasOverlays>>;

impl CanvasOverlays {
    pub(in crate::view) fn new(descriptors: &[(ContributionId, CanvasOverlayDescriptor)]) -> Self {
        Self {
            descriptors: descriptors.to_vec().into(),
            shown: FxHashMap::default(),
            entries: FxHashMap::default(),
        }
    }

    fn none_registered(&self) -> bool {
        self.descriptors.is_empty()
    }

    pub(in crate::view) fn index_of(&self, id: &ContributionId) -> Option<usize> {
        self.descriptors
            .iter()
            .position(|(candidate, _)| candidate == id)
    }

    fn descriptor(&self, index: usize) -> Option<&CanvasOverlayDescriptor> {
        self.descriptors
            .get(index)
            .map(|(_, descriptor)| descriptor)
    }

    /// Covers `key` with `index`; returns the overlay that covered it before.
    pub(in crate::view) fn show(
        &mut self,
        key: RepoKey,
        index: usize,
        repository: RepositoryHandle,
    ) -> Option<usize> {
        self.entries.entry((key, index)).or_insert(Entry {
            repository,
            view: None,
        });
        self.shown.insert(key, index)
    }

    /// Whether `index` was the one shown.
    pub(in crate::view) fn hide(&mut self, key: RepoKey, index: usize) -> bool {
        if self.shown.get(&key) != Some(&index) {
            return false;
        }
        self.shown.remove(&key);
        true
    }

    /// Hides whichever overlay covers `key`, with the handle it was shown for.
    fn hide_any(&mut self, key: RepoKey) -> Option<(usize, RepositoryHandle)> {
        let index = self.shown.remove(&key)?;
        Some((index, self.repository(key, index)?))
    }

    /// Forgets `index` over `key` and drops its view. Returns whether it was
    /// shown, and the handle it was shown for.
    pub(in crate::view) fn take(
        &mut self,
        key: RepoKey,
        index: usize,
    ) -> Option<(bool, RepositoryHandle)> {
        let entry = self.entries.remove(&(key, index))?;
        Some((self.hide(key, index), entry.repository))
    }

    pub(in crate::view) fn is_shown(&self, key: RepoKey, index: usize) -> bool {
        self.shown.get(&key) == Some(&index)
    }

    fn shown_index(&self, key: RepoKey) -> Option<usize> {
        self.shown.get(&key).copied()
    }

    fn repository(&self, key: RepoKey, index: usize) -> Option<RepositoryHandle> {
        Some(self.entries.get(&(key, index))?.repository.clone())
    }

    fn needs_view(&self, key: RepoKey, index: usize) -> bool {
        self.entries
            .get(&(key, index))
            .is_some_and(|entry| entry.view.is_none())
    }

    fn set_view(&mut self, key: RepoKey, index: usize, view: gpui::AnyView) {
        if let Some(entry) = self.entries.get_mut(&(key, index)) {
            entry.view = Some(view);
        }
    }

    pub(in crate::view) fn view(&self, key: RepoKey, index: usize) -> Option<gpui::AnyView> {
        self.entries.get(&(key, index))?.view.clone()
    }

    /// Forgets everything of repositories no longer open, dropping their
    /// views; returns what was forgotten.
    fn retain_open(&mut self, state: &AppState) -> Vec<(usize, RepositoryHandle)> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let open = |key: &RepoKey| {
            state
                .repos
                .iter()
                .any(|repo| repo.id == key.0 && repo.lifetime() == key.1)
        };
        self.shown.retain(|key, _| open(key));
        let mut forgotten = Vec::new();
        self.entries.retain(|(key, index), entry| {
            let keep = open(key);
            if !keep {
                forgotten.push((*index, entry.repository.clone()));
            }
            keep
        });
        forgotten
    }
}

/// Tells overlay `index`'s extension of `visibility`, after the current
/// update and never inside the call that caused it.
pub(in crate::view) fn announce(
    overlays: &SharedCanvasOverlays,
    host: &WindowHost,
    index: usize,
    repository: &RepositoryHandle,
    visibility: CanvasOverlayVisibility,
    cx: &mut App,
) {
    let observer = overlays
        .borrow()
        .descriptor(index)
        .and_then(|descriptor| descriptor.on_visibility.clone());
    let Some(observer) = observer else {
        return;
    };
    let context = RepositoryViewContext {
        window: host.clone(),
        repository: repository.clone(),
    };
    cx.defer(move |cx| {
        super::perf::extension_dispatch();
        observer(context, visibility, cx);
    });
}

/// Runs [`GitCometView::canvas_overlay_left`] after the current update,
/// where the window is at hand.
pub(in crate::view) fn leave_later(
    window: gpui::AnyWindowHandle,
    root: WeakEntity<GitCometView>,
    key: RepoKey,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            let _ = root.update(cx, |root, cx| root.canvas_overlay_left(key, window, cx));
        });
    });
}

impl GitCometView {
    fn canvas_overlays(&self) -> Option<SharedCanvasOverlays> {
        self.extension_window
            .as_ref()
            .map(super::extension_host::ExtensionWindow::canvas_overlays)
    }

    fn extension_host(&self) -> Option<WindowHost> {
        self.extension_window
            .as_ref()
            .map(super::extension_host::ExtensionWindow::host)
    }

    pub(in crate::view) fn canvas_overlays_registered(&self) -> bool {
        self.canvas_overlays()
            .is_some_and(|overlays| !overlays.borrow().none_registered())
    }

    /// The overlay covering the active repository, built or not.
    pub(in crate::view) fn shown_canvas_overlay(&self) -> Option<usize> {
        let overlays = self.canvas_overlays()?;
        let key = repo_key(self.active_repo()?);
        overlays.borrow().shown_index(key)
    }

    /// Whether an overlay fills the active repository's main slot.
    pub(in crate::view) fn canvas_overlay_shown(&self) -> bool {
        matches!(self.main_slot(), MainSlot::Overlay(_))
    }

    /// Overlay `index`'s view over the active repository, once built.
    pub(in crate::view) fn canvas_overlay_view(&self, index: usize) -> Option<gpui::AnyView> {
        let overlays = self.canvas_overlays()?;
        let key = repo_key(self.active_repo()?);
        overlays.borrow().view(key, index)
    }

    /// Builds overlay `index` for `key` if it has no view yet and, over the
    /// active repository, leaves any document and gives it the keyboard.
    pub(in crate::view) fn present_canvas_overlay(
        &mut self,
        key: RepoKey,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(overlays) = self.canvas_overlays() else {
            return;
        };
        // A hide in the same update, or the repository closing, wins.
        if !overlays.borrow().is_shown(key, index) {
            return;
        }
        if overlays.borrow().needs_view(key, index) {
            let build = overlays
                .borrow()
                .descriptor(index)
                .map(|descriptor| Rc::clone(&descriptor.build));
            let repository = overlays.borrow().repository(key, index);
            let (Some(build), Some(repository), Some(host)) =
                (build, repository, self.extension_host())
            else {
                return;
            };
            let context = RepositoryViewContext {
                window: host,
                repository,
            };
            // Not borrowed while the extension builds: it may ask the host.
            super::perf::extension_dispatch();
            let view = build(context, window, cx);
            overlays.borrow_mut().set_view(key, index, view);
        }
        if self.active_repo().map(repo_key) == Some(key) {
            if self.documents_active {
                self.show_repository_canvas(cx);
            }
            let focus = {
                let overlays = overlays.borrow();
                overlays
                    .descriptor(index)
                    .and_then(|descriptor| descriptor.focus.clone())
                    .zip(overlays.view(key, index))
            };
            match focus.and_then(|(focus, view)| focus(&view, cx)) {
                Some(focus) => window.focus(&focus, cx),
                None => window.focus(&self.canvas_overlay_focus, cx),
            }
        }
        self.sync_extension_navigation(cx);
        cx.notify();
    }

    /// An overlay over `key` was hidden: the keyboard goes back to the view
    /// behind only if the overlay held it, so a hide in the background never
    /// takes focus from elsewhere.
    pub(in crate::view) fn canvas_overlay_left(
        &mut self,
        key: RepoKey,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(repo) = self.active_repo().filter(|repo| repo_key(repo) == key)
            && self.canvas_overlay_focus.contains_focused(window, cx)
        {
            self.focus_main_view(repo, window, cx);
        }
        self.sync_extension_navigation(cx);
        cx.notify();
    }

    /// Hides whichever overlay covers `key`, as the host does whenever the
    /// main canvas is asked for. Returns whether one was shown.
    pub(in crate::view) fn hide_canvas_overlay_for(
        &self,
        key: RepoKey,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let (Some(overlays), Some(host)) = (self.canvas_overlays(), self.extension_host()) else {
            return false;
        };
        let hidden = overlays.borrow_mut().hide_any(key);
        let Some((index, repository)) = hidden else {
            return false;
        };
        announce(
            &overlays,
            &host,
            index,
            &repository,
            CanvasOverlayVisibility::Hidden,
            cx,
        );
        leave_later(self.window_handle, cx.weak_entity(), key, cx);
        true
    }

    /// Hides the active repository's overlay, if one is shown.
    pub(in crate::view) fn hide_active_canvas_overlay(&self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(key) = self.active_repo().map(repo_key) else {
            return false;
        };
        self.hide_canvas_overlay_for(key, cx)
    }

    /// Drops overlays of repositories no longer open and says so.
    pub(in crate::view) fn retain_open_canvas_overlays(&self, cx: &mut gpui::Context<Self>) {
        let (Some(overlays), Some(host)) = (self.canvas_overlays(), self.extension_host()) else {
            return;
        };
        let forgotten = overlays.borrow_mut().retain_open(&self.state);
        for (index, repository) in forgotten {
            announce(
                &overlays,
                &host,
                index,
                &repository,
                CanvasOverlayVisibility::Dropped,
                cx,
            );
        }
    }
}
