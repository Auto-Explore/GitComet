//! Extension settings pages: listed after the built-in categories, and only
//! the selected one is built.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_extension_api::{ContributionId, SettingsPageDescriptor};
use std::rc::Rc;

pub(super) type ExtensionPages = Rc<[(ContributionId, SettingsPageDescriptor)]>;

pub(super) fn extension_pages(cx: &App) -> ExtensionPages {
    crate::view::extension_host::registry(cx)
        .map(|registry| registry.settings_pages().to_vec().into())
        .unwrap_or_else(|| Rc::from([]))
}

pub(super) fn page_matches_query(page: &SettingsPageDescriptor, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || page.title.to_lowercase().contains(&query)
        || page.keywords.to_lowercase().contains(&query)
}

impl SettingsWindowView {
    /// Shows extension page `index`, building it; any built-in category
    /// selection is replaced.
    pub(super) fn select_extension_page(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.extension_page.as_ref().map(|(i, _)| *i) == Some(index) {
            return;
        }
        let Some((_, page)) = self.extension_pages.get(index) else {
            return;
        };
        // Pages exist only with a registry, and so does the window's host.
        let Some(host) = self.extension_window.as_ref().map(|e| e.host()) else {
            return;
        };
        let build = Rc::clone(&page.build);
        crate::view::perf::extension_dispatch();
        let view = build(
            gitcomet_extension_api::SettingsPageContext::new(host, self.theme),
            window,
            cx,
        );
        self.extension_page = Some((index, view));
        self.set_expanded_section(None, cx);
        self.settings_window_scroll
            .set_offset(gpui::point(px(0.0), px(0.0)));
        cx.notify();
    }

    /// The selected extension page's card, if one is selected.
    pub(super) fn extension_page_card(&self, theme: AppTheme) -> Option<Stateful<gpui::Div>> {
        let (index, view) = self.extension_page.as_ref()?;
        let (id, page) = self.extension_pages.get(*index)?;
        crate::view::perf::settings_page_rendered();
        Some(
            components::settings_card(
                format!("settings_window_extension_{id}"),
                page.title.clone(),
                theme,
            )
            .child(view.clone()),
        )
    }

    pub(super) fn extension_nav_items(
        &self,
        query: &str,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<Stateful<gpui::Div>> {
        let selected = self.extension_page.as_ref().map(|(index, _)| *index);
        self.extension_pages
            .iter()
            .enumerate()
            .filter(|(_, (_, page))| page_matches_query(page, query))
            .map(|(index, (id, page))| {
                components::settings_nav_item(
                    format!("settings_window_nav_extension_{id}"),
                    page.icon.clone(),
                    page.title.clone(),
                    selected == Some(index),
                    theme,
                    self.row_scale(theme),
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, window, cx| {
                        this.select_extension_page(index, window, cx);
                    }),
                )
            })
            .collect()
    }
}
