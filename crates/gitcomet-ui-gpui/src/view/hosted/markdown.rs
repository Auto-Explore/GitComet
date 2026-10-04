//! A read-only markdown view for extensions: History's markdown renderer
//! over a string, without the diff preview's selection, links or search.

use super::*;
use crate::view::markdown_preview::{MarkdownPreviewDocument, parse_markdown};
use std::cell::Cell;
use std::rc::Rc;

pub(crate) struct MarkdownView {
    theme: Rc<Cell<AppTheme>>,
    document: Option<Arc<MarkdownPreviewDocument>>,
    blocks: crate::view::rows::MarkdownDocumentBlockCache,
    block_scrolls: crate::view::rows::MarkdownDocumentBlockScrolls,
    layout: crate::view::rows::MarkdownDocumentLayoutCache,
}

impl MarkdownView {
    pub(crate) fn new(theme: Rc<Cell<AppTheme>>) -> Self {
        Self {
            theme,
            document: None,
            blocks: Default::default(),
            block_scrolls: Default::default(),
            layout: Default::default(),
        }
    }

    /// What each parsed row is, for tests.
    #[cfg(test)]
    pub(in crate::view) fn row_kinds(
        &self,
    ) -> Vec<crate::view::markdown_preview::MarkdownPreviewRowKind> {
        self.document
            .iter()
            .flat_map(|document| document.rows.iter().map(|row| row.kind))
            .collect()
    }

    fn set_source(&mut self, source: &str, cx: &mut gpui::Context<Self>) {
        self.document = parse_markdown(source).map(Arc::new);
        self.blocks = Default::default();
        self.block_scrolls = Default::default();
        self.layout = Default::default();
        cx.notify();
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(document) = self.document.clone() else {
            return div().debug_selector(|| "hosted_markdown_empty".to_string());
        };
        let context = crate::view::rows::MarkdownDocumentContext {
            theme: self.theme.get(),
            ui_scale_percent: crate::ui_scale::current(cx).percent,
            editor_font_family: crate::font_preferences::current_editor_font_family(cx).into(),
            image_root: None,
            remote_image_access: Default::default(),
            picture_sizes: Default::default(),
            drawn_pictures: None,
            row_boxes: Default::default(),
            block_scrolls: self.block_scrolls.clone(),
            blocks: self.blocks.clone(),
            layout: self.layout.clone(),
            view: None,
            text_region: DiffTextRegion::Inline,
            change_bar_color: None,
            query: None,
            reveal: Default::default(),
            scroll: None,
            hovered_link: None,
            change_extents: None,
            tasks_editable: false,
        };
        div()
            .debug_selector(|| "hosted_markdown".to_string())
            .w_full()
            .child(crate::view::rows::render_markdown_document(
                usize::from(crate::view::tab_width::DEFAULT_TAB_WIDTH),
                &document,
                &context,
            ))
    }
}

/// The extension-facing handle; it owns the view.
pub(crate) struct HostedMarkdownView {
    pub(crate) entity: Entity<MarkdownView>,
}

impl gitcomet_extension_api::MarkdownViewImpl for HostedMarkdownView {
    fn view(&self) -> gpui::AnyView {
        self.entity.clone().into()
    }

    fn set_source(&self, source: SharedString, cx: &mut App) {
        self.entity
            .update(cx, |view, cx| view.set_source(source.as_ref(), cx));
    }
}
