use crate::view::GitCometView;
use crate::view::mod_helpers::PopoverKind;
use gitcomet_core::domain::CommitId;
use gitcomet_state::model::RepoId;
use gpui::prelude::*;
use gpui::{
    ElementId, Entity, MouseUpEvent, Pixels, Point, SharedString, WeakEntity, Window, div, px,
};
use std::ops::Range;
use std::sync::Arc;

/// What a link inside a read-only text input points at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Commit {
        commit_id: CommitId,
        /// Whether the link may offer "Reveal commit". A commit's own SHA
        /// field cannot reveal itself.
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
    click: crate::kit::click::SubtargetClick<MessageLink>,
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
            click: Default::default(),
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
        if self.input != input || self.links != links || self.repo_id != repo_id {
            self.click = Default::default();
        }
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
        let link = self.link_at(event.position, cx).cloned();
        if self.click.release(link.as_ref(), event).is_none() {
            return;
        }
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
        let press_view = cx.entity();
        let move_view = cx.entity();
        let release_view = cx.entity();
        div()
            .id((ElementId::from("commit_link_menu_root"), self.id.clone()))
            .debug_selector({
                let id = self.id.clone();
                move || id.to_string()
            })
            .relative()
            .w_full()
            .min_w(px(0.0))
            .on_mouse_down_all(move |event, phase, hitbox, window, cx| {
                if phase == gpui::DispatchPhase::Capture {
                    press_view.update(cx, |this, cx| {
                        let target = hitbox
                            .is_hovered(window)
                            .then(|| this.link_at(event.position, cx).cloned())
                            .flatten();
                        this.click.press(target, event);
                    });
                }
            })
            .on_mouse_move_all(move |event, phase, _, _, cx| {
                if phase == gpui::DispatchPhase::Capture {
                    move_view.update(cx, |this, _| this.click.moved(event));
                }
            })
            .on_mouse_up_all(move |event, phase, hitbox, window, cx| {
                if phase == gpui::DispatchPhase::Capture && !hitbox.is_hovered(window) {
                    release_view.update(cx, |this, _| {
                        this.click.release(None, event);
                    });
                } else if phase == gpui::DispatchPhase::Bubble && hitbox.is_hovered(window) {
                    release_view.update(cx, |this, cx| this.on_mouse_up(event, window, cx));
                }
            })
            .child(self.input.clone())
    }
}
