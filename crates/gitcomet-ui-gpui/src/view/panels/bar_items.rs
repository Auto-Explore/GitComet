//! Route chrome elements while their original view keeps their state and listeners.
use super::*;
use gitcomet_extension_api::{BarItemLocation, BuiltinBarItem, Registry};
use std::rc::Rc;

pub(super) type ExtensionBarItem = (
    BarItemLocation,
    Option<gitcomet_extension_api::ViewTarget>,
    gpui::AnyView,
);
pub(super) type RelocatedBarItems = Vec<(BarItemLocation, gpui::AnyView)>;

pub(super) struct BarItems {
    registry: Option<Rc<Registry>>,
    items: Vec<(BarItemLocation, Option<AnyElement>)>,
}

impl BarItems {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            registry: crate::view::extension_host::registry(cx),
            items: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, item: BuiltinBarItem, element: impl IntoElement) {
        self.items
            .push((self.location(item), Some(element.into_any_element())));
    }

    pub(super) fn location(&self, item: BuiltinBarItem) -> BarItemLocation {
        self.registry.as_ref().map_or_else(
            || item.default_location(),
            |registry| registry.bar_item_location(item),
        )
    }

    pub(super) fn extensions(
        &mut self,
        items: &[ExtensionBarItem],
        active_view: &gitcomet_extension_api::ViewTarget,
    ) {
        self.items.extend(
            items
                .iter()
                .filter(|(_, view, _)| view.as_ref().is_none_or(|view| view == active_view))
                .map(|(location, _, view)| (*location, Some(view.clone().into_any_element()))),
        );
    }

    pub(super) fn take(&mut self, location: BarItemLocation) -> Vec<AnyElement> {
        self.items
            .iter_mut()
            .filter(|(placed, _)| *placed == location)
            .filter_map(|(_, element)| element.take())
            .collect()
    }
}

pub(super) trait BarItemOwner: Render {
    fn bar_items(
        &mut self,
        action_bar: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> BarItems;
}

/// Only allocated for an actual move between bars. Observing the owner keeps
/// relocated items fresh across the destination bar's independent cache.
struct RelocatedItems<O: BarItemOwner> {
    owner: WeakEntity<O>,
    location: BarItemLocation,
    _subscription: gpui::Subscription,
}

impl<O: BarItemOwner> Render for RelocatedItems<O> {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let location = self.location;
        let items = self
            .owner
            .update(cx, |owner, cx| {
                owner
                    .bar_items(location.is_action_bar(), window, cx)
                    .take(location)
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_none()
            .h_full()
            .items_center()
            .gap(crate::ui_scale::UiScale::current(cx).px(4.0))
            .children(items)
    }
}

fn relocated<O: BarItemOwner>(
    owner: &Entity<O>,
    location: BarItemLocation,
    cx: &mut App,
) -> gpui::AnyView {
    cx.new(|cx| RelocatedItems {
        owner: owner.downgrade(),
        location,
        _subscription: cx.observe(owner, |_, _, cx| cx.notify()),
    })
    .into()
}

pub(in crate::view) fn connect(
    action_bar: &Entity<ActionBarView>,
    status_bar: &Entity<BottomStatusBarView>,
    cx: &mut App,
) {
    let Some(registry) = crate::view::extension_host::registry(cx) else {
        return;
    };
    for location in [
        BarItemLocation::ActionBarStart,
        BarItemLocation::ActionBarEnd,
        BarItemLocation::StatusBarStart,
        BarItemLocation::StatusBarEnd,
    ] {
        if !registry.bar_item_locations().iter().any(|(item, placed)| {
            *placed == location
                && item.default_location().is_action_bar() != location.is_action_bar()
        }) {
            continue;
        }
        if location.is_action_bar() {
            let items = relocated(status_bar, location, cx);
            action_bar.update(cx, |bar, _| bar.relocated_items.push((location, items)));
        } else {
            let items = relocated(action_bar, location, cx);
            status_bar.update(cx, |bar, _| bar.relocated_items.push((location, items)));
        }
    }
}

pub(super) fn placed_views(
    items: &RelocatedBarItems,
    location: BarItemLocation,
) -> impl Iterator<Item = gpui::AnyView> + '_ {
    items
        .iter()
        .filter(move |(placed, _)| *placed == location)
        .map(|(_, view)| view.clone())
}
