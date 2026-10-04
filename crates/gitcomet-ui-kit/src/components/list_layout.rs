//! How a list of changed files arranges them, and the one icon that shows
//! the arrangement and switches it.

use super::{Button, ButtonStyle};
use crate::icons::svg_icon;
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::SharedString;
use gpui::prelude::*;

/// How a list of changed files arranges them.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ListLayout {
    /// One row per path.
    #[default]
    Flat,
    /// Under their folders.
    Tree,
    /// Under group headers: by change kind, or by groups the list is given.
    Groups,
}

impl ListLayout {
    /// Every layout, in the order a click steps through them.
    pub const ALL: [Self; 3] = [Self::Flat, Self::Tree, Self::Groups];

    /// The stable name a preference is saved under.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Tree => "tree",
            Self::Groups => "groups",
        }
    }

    pub fn from_key(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layout| layout.key() == raw)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat list",
            Self::Tree => "Tree",
            Self::Groups => "Groups",
        }
    }

    /// The icon's shape, which is the layout's.
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Flat => "icons/menu.svg",
            Self::Tree => "icons/list_tree.svg",
            Self::Groups => "icons/list_groups.svg",
        }
    }

    /// The layout a click on the icon switches to.
    pub const fn next(self) -> Self {
        match self {
            Self::Flat => Self::Tree,
            Self::Tree => Self::Groups,
            Self::Groups => Self::Flat,
        }
    }

    pub fn tooltip(self) -> SharedString {
        format!(
            "Layout: {} — click to switch, right-click to choose",
            self.label()
        )
        .into()
    }
}

/// The layout icon of a list: one transparent button drawing `layout`'s
/// shape. The caller attaches a click that switches to [`ListLayout::next`]
/// and a right click (`PointerClickExt::on_pointer_click`) that opens a menu
/// of [`ListLayout::ALL`], and names it with [`ListLayout::tooltip`]. The
/// icon's debug selector is `{id}_{key}`, so a test can see its shape.
pub fn list_layout_button(
    id: impl Into<SharedString>,
    layout: ListLayout,
    theme: AppTheme,
    scale: UiScale,
) -> Button {
    let id = id.into();
    let selector = format!("{id}_{}", layout.key());
    Button::new(id, "")
        .style(ButtonStyle::Transparent)
        .start_slot(
            svg_icon(
                layout.icon(),
                theme.colors.foreground.secondary,
                scale.px(14.0),
            )
            .debug_selector(move || selector),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::click::PointerClickExt as _;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_click_steps_through_every_layout_and_keys_round_trip() {
        let mut layout = ListLayout::default();
        let mut seen = Vec::new();
        for _ in 0..ListLayout::ALL.len() {
            seen.push(layout);
            layout = layout.next();
        }
        assert_eq!(seen, ListLayout::ALL);
        assert_eq!(layout, ListLayout::Flat, "and back to the start");
        for layout in ListLayout::ALL {
            assert_eq!(ListLayout::from_key(layout.key()), Some(layout));
        }
        assert_eq!(ListLayout::from_key("nested"), None);
        assert_eq!(
            ListLayout::Groups.tooltip().as_ref(),
            "Layout: Groups — click to switch, right-click to choose"
        );
    }

    struct Picker {
        layout: ListLayout,
        menus: Rc<RefCell<u32>>,
        theme: AppTheme,
    }

    impl gpui::Render for Picker {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let menus = self.menus.clone();
            list_layout_button(
                "layout",
                self.layout,
                self.theme,
                UiScale::from_percent(100),
            )
            .on_click(self.theme, cx, |this, _, _, cx| {
                this.layout = this.layout.next();
                cx.notify();
            })
            .on_pointer_click(gpui::MouseButton::Right, move |_, _, _| {
                *menus.borrow_mut() += 1;
            })
        }
    }

    /// The icon's shape follows a click, and a right click asks for the menu
    /// without switching.
    #[gpui::test]
    fn the_icon_follows_the_layout_and_a_right_click_asks_for_the_menu(
        cx: &mut gpui::TestAppContext,
    ) {
        let _guard = crate::test_support::lock_visual_test();
        let menus = Rc::new(RefCell::new(0));
        let (view, cx) = cx.add_window_view(|_, _| Picker {
            layout: ListLayout::Flat,
            menus: menus.clone(),
            theme: AppTheme::gitcomet_dark(),
        });
        crate::test_support::redraw(cx);
        assert!(cx.debug_bounds("layout_flat").is_some());

        let center = cx.debug_bounds("layout_flat").unwrap().center();
        cx.simulate_click(center, gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert!(cx.debug_bounds("layout_tree").is_some());
        assert!(cx.debug_bounds("layout_flat").is_none());

        let center = cx.debug_bounds("layout_tree").unwrap().center();
        cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
        crate::test_support::redraw(cx);
        assert_eq!(*menus.borrow(), 1);
        assert_eq!(
            view.read_with(cx, |view, _| view.layout),
            ListLayout::Tree,
            "a right click does not switch"
        );
    }
}
