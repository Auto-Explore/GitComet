use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_extension_api::{
    RepositoryHandle, RepositoryViewContext, SidebarProvider, SidebarSectionRows,
};
use std::rc::Rc;

pub(super) struct SidebarContributions {
    providers: Vec<(gitcomet_extension_api::ContributionId, SidebarProvider)>,
    revisions: Vec<u64>,
    repository: Option<(RepoId, u64)>,
    sections: Vec<SidebarSectionRows>,
    collapsed: FxHashSet<SharedString>,
    projection: Option<(Rc<[BranchSidebarRow]>, SidebarPresentation)>,
}
impl SidebarContributions {
    pub(super) fn new(cx: &App) -> Option<Self> {
        let registry = crate::view::extension_host::registry(cx)?;
        if registry.sidebar_providers().is_empty() {
            return None;
        }
        Some(Self {
            providers: registry.sidebar_providers().to_vec(),
            revisions: Vec::new(),
            repository: None,
            sections: Vec::new(),
            collapsed: Default::default(),
            projection: None,
        })
    }

    pub(super) fn project(&mut self, mut base: SidebarPresentation) -> SidebarPresentation {
        if let Some((rows, cached)) = &self.projection
            && Rc::ptr_eq(rows, &base.rows)
            && Rc::ptr_eq(&cached.search, &base.search)
        {
            return cached.clone();
        }
        let source = base.rows.clone();
        let mut rows = base.rows.to_vec();
        for (section, data) in self.sections.iter().enumerate() {
            let collapsed = self.collapsed.contains(&data.id);
            let items: Vec<_> = data
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| base.search.matches_ref(&row.label))
                .collect();
            if items.is_empty() {
                continue;
            }
            rows.push(BranchSidebarRow::ContributionHeader {
                section,
                title: data.title.clone(),
                collapsed,
                collapse_key: data.id.clone(),
            });
            if !collapsed {
                rows.extend(items.into_iter().map(|(row, data)| {
                    BranchSidebarRow::ContributionItem {
                        section,
                        row,
                        label: data.label.clone(),
                        key: format!("{}/{}", self.sections[section].id, data.id).into(),
                    }
                }));
            }
        }
        base.rows = rows.into();
        base.row_keys = base
            .rows
            .iter()
            .map(crate::view::sidebar_sticky::row_key)
            .collect();
        base.structure = Rc::new(crate::view::sidebar_sticky::SidebarStructure::with_pins(
            &base.rows,
            base.pins.len(),
        ));
        self.projection = Some((source, base.clone()));
        base
    }
}

impl SidebarPaneView {
    pub(super) fn sync_contributed_rows(&mut self, cx: &App) {
        let Some(providers) = &mut self.contributions else {
            return;
        };
        let repo = self
            .state
            .active_repo
            .and_then(|id| self.state.repos.iter().find(|repo| repo.id == id));
        let repository = repo.map(|repo| (repo.id, repo.lifetime()));
        if providers.repository == repository
            && providers.revisions.len() == providers.providers.len()
            && providers
                .providers
                .iter()
                .zip(&providers.revisions)
                .all(|((_, provider), rev)| provider.signal.revision() == *rev)
        {
            return;
        }
        providers.repository = repository;
        providers.revisions = providers
            .providers
            .iter()
            .map(|(_, provider)| provider.signal.revision())
            .collect();
        providers.sections.clear();
        providers.projection = None;
        let Some(repo) = repo else {
            return;
        };
        let Ok(Some(host)) = self.root_view.read_with(cx, |root, _| {
            root.extension_window
                .as_ref()
                .map(|extension| extension.host())
        }) else {
            return;
        };
        let context = RepositoryViewContext {
            repository: RepositoryHandle::new(
                host.id(),
                repo.id,
                repo.lifetime(),
                repo.spec.workdir.clone(),
            ),
            window: host,
        };
        for (id, provider) in &providers.providers {
            crate::view::perf::extension_dispatch();
            for mut section in (provider.sections)(&context, cx) {
                section.id = format!("section:extension:{id}/{}", section.id).into();
                if let Some(files) = &section.files {
                    section.rows.extend(files.rows());
                }
                providers.sections.push(section);
            }
        }
    }

    pub(in crate::view) fn contributed_header(
        &self,
        section: usize,
        title: SharedString,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        div()
            .id(("sidebar_contribution_header", section))
            .h(crate::view::rows::sidebar::sidebar_list_row_height(
                self.theme,
                crate::ui_scale::current(cx).percent,
            ))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            // Sized like the built-in section headers and rows.
            .text_size(self.theme.ui_text(14.0))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(if collapsed { "▸" } else { "▾" })
            .child(title)
            .control_interaction(
                controls::InteractionStyle::new(self.theme),
                controls::InteractionState::default(),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |view, _, _, cx| {
                    if let Some(providers) = &mut view.contributions
                        && let Some(data) = providers.sections.get(section)
                    {
                        let key = data.id.clone();
                        if !providers.collapsed.remove(&key) {
                            providers.collapsed.insert(key);
                        }
                        providers.projection = None;
                        cx.notify();
                    }
                }),
            )
            .into_any_element()
    }
    pub(in crate::view) fn contributed_row(
        &self,
        section: usize,
        row: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let Some(data) = self
            .contributions
            .as_ref()
            .and_then(|p| p.sections.get(section))
            .and_then(|s| s.rows.get(row))
        else {
            return div().into_any_element();
        };
        let action = data.action.clone();
        let id = format!("sidebar_contribution_{section}_{row}");
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .h(crate::view::rows::sidebar::sidebar_list_row_height(
                self.theme,
                crate::ui_scale::current(cx).percent,
            ))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_size(self.theme.ui_text(14.0))
            .when_some(data.icon.clone(), |row, icon| {
                // `svg()` is a mask in the text colour; without one it paints nothing.
                row.child(
                    gpui::svg()
                        .path(icon)
                        .size_4()
                        .text_color(self.theme.colors.foreground.secondary),
                )
            })
            .child(div().flex_1().overflow_hidden().child(data.label.clone()))
            .when_some(data.mark.clone(), |row, mark| {
                row.child(row_mark(mark, ui_scale::UiScale::current(cx)))
            })
            .control_interaction(
                controls::InteractionStyle::new(self.theme),
                controls::InteractionState::default(),
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                move |_, _, cx| action.invoke(cx),
            )
            .into_any_element()
    }
}

/// A contributed row's mark: its glyph, then its label, in its colour.
fn row_mark(mark: gitcomet_extension_api::RowMark, ui_scale: ui_scale::UiScale) -> gpui::Div {
    let color = mark.color;
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(ui_scale.px(4.0))
        .text_color(color)
        .children(mark.glyph.map(|glyph| {
            match glyph {
                gitcomet_extension_api::RowGlyph::Icon(path) => gpui::svg()
                    .path(path)
                    .size(ui_scale.px(12.0))
                    .text_color(color)
                    .into_any_element(),
                gitcomet_extension_api::RowGlyph::Text(text) => {
                    div().child(text).into_any_element()
                }
                _ => gpui::Empty.into_any_element(),
            }
        }))
        .children(mark.label)
}
