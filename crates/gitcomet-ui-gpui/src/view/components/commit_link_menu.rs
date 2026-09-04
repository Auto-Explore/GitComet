use crate::view::GitCometView;
use crate::view::mod_helpers::PopoverKind;
use gitcomet_core::domain::CommitId;
use gitcomet_state::model::RepoId;
use gpui::prelude::*;
use gpui::{
    ElementId, Entity, MouseButton, MouseUpEvent, Pixels, Point, SharedString, WeakEntity, Window,
    div, px,
};
use std::ops::Range;
use std::sync::Arc;

/// What a link inside a read-only text input points at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Commit {
        commit_id: CommitId,
        /// Whether the link may offer "Navigate". A commit's own SHA field
        /// cannot navigate to itself.
        allow_navigate: bool,
    },
    Url(SharedString),
}

/// A clickable span of a read-only text input, as a byte range into its text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageLink {
    pub range: Range<usize>,
    pub target: LinkTarget,
}

/// Wraps a read-only [`TextInput`](crate::kit::TextInput) and turns single
/// clicks that land on one of its links into a menu anchored under the link.
///
/// The menu itself belongs to the popover host, which is what gives commit
/// messages the same link menu the markdown preview already shows.
pub struct CommitLinkMenu {
    input: Entity<crate::kit::TextInput>,
    repo_id: RepoId,
    links: Arc<[MessageLink]>,
    id: SharedString,
    root_view: WeakEntity<GitCometView>,
}

impl CommitLinkMenu {
    pub fn new(
        input: Entity<crate::kit::TextInput>,
        repo_id: RepoId,
        links: Arc<[MessageLink]>,
        id: impl Into<SharedString>,
        root_view: WeakEntity<GitCometView>,
    ) -> Self {
        Self {
            input,
            repo_id,
            links,
            id: id.into(),
            root_view,
        }
    }

    pub fn sync(
        &mut self,
        input: Entity<crate::kit::TextInput>,
        repo_id: RepoId,
        links: Arc<[MessageLink]>,
        id: impl Into<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.input = input;
        self.repo_id = repo_id;
        self.links = links;
        self.id = id.into();
        cx.notify();
    }

    /// The spans this menu treats as links — what the detector found, after the
    /// whole pipeline from message text to a clickable run.
    #[cfg(test)]
    pub fn links_for_tests(&self) -> Arc<[MessageLink]> {
        Arc::clone(&self.links)
    }

    fn link_at(&self, position: Point<Pixels>, cx: &gpui::App) -> Option<&MessageLink> {
        let ranges = self
            .links
            .iter()
            .map(|link| link.range.clone())
            .collect::<Vec<_>>();
        let ix = self.input.read_with(cx, |input, _| {
            input.hotspot_range_index_at_position(position, &ranges)
        })?;
        self.links.get(ix)
    }

    fn popover_kind_for(&self, target: &LinkTarget) -> PopoverKind {
        match target {
            LinkTarget::Commit {
                commit_id,
                allow_navigate,
            } => PopoverKind::CommitShaLinkMenu {
                repo_id: self.repo_id,
                commit_id: commit_id.clone(),
                allow_navigate: *allow_navigate,
            },
            LinkTarget::Url(url) => PopoverKind::WebLinkMenu {
                url: url.clone(),
                load_remote_image_url: None,
            },
        }
    }

    fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // A double or triple click is still a text selection, and so is a drag
        // that ended on the link — only a plain click follows it, so selecting
        // the words of a link keeps working.
        if event.click_count > 1 {
            return;
        }
        if self
            .input
            .read_with(cx, |input, _| !input.selected_range().is_empty())
        {
            return;
        }

        let Some(link) = self.link_at(event.position, cx) else {
            return;
        };
        let kind = self.popover_kind_for(&link.target);
        let range = link.range.clone();

        // Anchor on the link's own box, so the menu opens flush under the words
        // it describes rather than on top of them or off beside the panel.
        let Some(anchor) = self
            .input
            .read_with(cx, |input, _| input.hotspot_bounds(&range))
        else {
            return;
        };

        cx.stop_propagation();

        // The popover lives on the root view, and this handler was reached from
        // it — opening inline would re-enter the update that is already running.
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_for_bounds(kind, anchor, window, cx);
                });
            });
        });
    }
}

impl Render for CommitLinkMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let debug_selector = self.id.to_string();
        div()
            .id((ElementId::from("commit_link_menu_root"), self.id.clone()))
            .debug_selector(move || debug_selector.clone())
            .relative()
            .w_full()
            .min_w(px(0.0))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .child(self.input.clone())
    }
}
