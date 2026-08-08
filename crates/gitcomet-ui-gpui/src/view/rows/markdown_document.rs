//! Flowing renderer for the single-document markdown preview.
//!
//! The diff preview paints into a uniform (fixed row height) list, because its
//! two columns must stay row-aligned and every row carries a change bar. A
//! single document has neither requirement, so it lays out naturally instead:
//! text wraps by itself, images sit inline at the size the document asked for,
//! and the gaps around headings are margins rather than blank rows.
//!
//! Modelled on Zed's markdown preview, which renders a whole document as one
//! element tree inside a scrolling container.
//!
//! Interaction still keys off the document's row indices. Selection, copy, hit
//! testing, and the link menu are all addressed by `(row index, region)`, so
//! handing the flowing renderer the same indices the row grid used keeps every
//! one of them working without a second code path.

use super::history::{
    MARKDOWN_PREVIEW_BASE_FONT_PX, MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX,
    MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, MARKDOWN_PREVIEW_INDENT_STEP_PX,
    MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX,
    MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
};
use super::markdown_flow_text::MarkdownFlowText;
use super::*;
use crate::view::markdown_preview::{
    MarkdownBlock, MarkdownInlineImage, MarkdownInlineStyle, MarkdownPreviewDocument,
    MarkdownPreviewRow, MarkdownPreviewRowKind, markdown_document_blocks,
};

/// Everything the flowing renderer needs that is not in the document.
pub(in crate::view) struct MarkdownDocumentContext {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) editor_font_family: SharedString,
    /// Directory relative image sources resolve against.
    pub(in crate::view) image_base_dir: Option<Arc<std::path::Path>>,
    /// Set when the preview is interactive: text selection, copy, the link
    /// menu, and the diff context menu all go through this view.
    pub(in crate::view) view: Option<Entity<MainPaneView>>,
    pub(in crate::view) text_region: DiffTextRegion,
    /// Gutter colour for a wholly added or removed file, `None` otherwise.
    pub(in crate::view) change_bar_color: Option<gpui::Rgba>,
}

/// Gap between two blocks, and the extra break a heading opens above itself.
const BLOCK_GAP_PX: f32 = 10.0;
const HEADING_GAP_PX: f32 = 22.0;
const CODE_BLOCK_PAD_Y_PX: f32 = 8.0;
const TABLE_CELL_PAD_X_PX: f32 = 10.0;
const TABLE_CELL_PAD_Y_PX: f32 = 4.0;

/// Width of the gutter marking a wholly added or removed file.
const MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX: f32 = 3.0;

/// Render a whole document as one flowing element tree.
pub(in crate::view) fn render_markdown_document(
    document: &MarkdownPreviewDocument,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let blocks = markdown_document_blocks(document);
    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .pl(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context))
        .text_color(context.theme.colors.text);

    for (ix, block) in blocks.iter().enumerate() {
        column = column.child(render_block(document, block, ix == 0, context));
    }

    // The change bar is one element spanning the whole document rather than a
    // segment per row: a flowing layout puts margins between blocks, and a
    // per-row bar would leave a gap in every one of them.
    div()
        .flex()
        .items_stretch()
        .w_full()
        .min_w(px(0.0))
        .when_some(context.change_bar_color, |row, color| {
            row.child(
                div()
                    .flex_none()
                    .w(scaled(MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX, context))
                    .bg(color)
                    .debug_selector(|| "markdown_preview_change_bar".to_string()),
            )
        })
        .child(column)
        .into_any_element()
}

fn render_block(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    is_first: bool,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    // A heading opens a wider break above it than the gap between ordinary
    // blocks, except at the very top of the document where there is nothing to
    // separate it from.
    let gap = match block {
        _ if is_first => 0.0,
        MarkdownBlock::Heading { .. } => HEADING_GAP_PX,
        _ => BLOCK_GAP_PX,
    };
    let wrapper = div().w_full().min_w(px(0.0)).mt(scaled(gap, context));
    let rows = RowRun::new(document, block.row_range());

    match block {
        MarkdownBlock::Heading { level, row_ix } => wrapper
            .child(render_heading(*level, *row_ix, document, context))
            .into_any_element(),
        MarkdownBlock::Paragraph(row_ix) => wrapper
            .when_some(document.rows.get(*row_ix), |wrapper, row| {
                wrapper.child(
                    row_shell(*row_ix, row, context).child(render_row_line(*row_ix, row, context)),
                )
            })
            .into_any_element(),
        MarkdownBlock::List(_) => wrapper.child(render_list(rows, context)).into_any_element(),
        MarkdownBlock::Blockquote(_) => wrapper
            .child(render_blockquote(rows, context))
            .into_any_element(),
        MarkdownBlock::Code(_) => wrapper.child(render_code(rows, context)).into_any_element(),
        MarkdownBlock::Table(_) => wrapper
            .child(render_table(rows, context))
            .into_any_element(),
        // Only the first band of an image carries its source; the rest exist so
        // the row grid can give the picture height.
        MarkdownBlock::Image(_) => wrapper
            .when_some(rows.first(), |wrapper, (row_ix, row)| {
                wrapper.child(render_image(row_ix, row, context))
            })
            .into_any_element(),
        MarkdownBlock::ThematicBreak(_) => wrapper
            .child(div().w_full().h(px(1.0)).bg(with_alpha(
                context.theme.colors.border,
                if context.theme.is_dark { 0.92 } else { 0.88 },
            )))
            .into_any_element(),
    }
}

/// The rows of one block, paired with the document index each one paints at.
struct RowRun<'a> {
    document: &'a MarkdownPreviewDocument,
    range: Range<usize>,
}

impl<'a> RowRun<'a> {
    fn new(document: &'a MarkdownPreviewDocument, range: Range<usize>) -> Self {
        Self { document, range }
    }

    fn iter(&self) -> impl Iterator<Item = (usize, &'a MarkdownPreviewRow)> {
        let document = self.document;
        self.range
            .clone()
            .filter_map(move |row_ix| document.rows.get(row_ix).map(|row| (row_ix, row)))
    }

    fn first(&self) -> Option<(usize, &'a MarkdownPreviewRow)> {
        self.iter().next()
    }
}

/// The container a row's text lives in: it carries the row's index, so mouse
/// events resolve to the same `(row, region)` pair selection and copy use.
fn row_shell(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> gpui::Stateful<gpui::Div> {
    let shell = div()
        .id(("md_preview_row", row_ix))
        .debug_selector(move || format!("markdown_preview_row_box_{row_ix}"))
        .w_full()
        .min_w(px(0.0))
        .flex()
        .items_start()
        .when_some(
            crate::view::rows::markdown_preview_row_background(context.theme, row),
            |shell, background| shell.bg(background),
        )
        // A line the parser could not interpret is shown verbatim on a warning
        // band, which needs room around the text.
        .when(
            matches!(row.kind, MarkdownPreviewRowKind::PlainFallback),
            |shell| shell.px(scaled(MARKDOWN_PREVIEW_SHELL_PAD_X_PX, context)),
        );

    let Some(view) = context.view.clone() else {
        return shell;
    };
    let text_region = context.text_region;
    shell
        .on_mouse_down(gpui::MouseButton::Left, {
            let view = view.clone();
            move |event, window, cx| {
                let focus = view.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                let click_count = event.click_count;
                let position = event.position;
                view.update(cx, |this, cx| {
                    if !this.handle_markdown_preview_link_click(
                        row_ix,
                        text_region,
                        position,
                        click_count,
                        window,
                        cx,
                    ) {
                        this.handle_diff_text_mouse_down(
                            row_ix,
                            text_region,
                            position,
                            click_count,
                            cx,
                        );
                    }
                    cx.notify();
                });
            }
        })
        .on_mouse_down(gpui::MouseButton::Right, move |event, window, cx| {
            view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu(row_ix, text_region, event.position, window, cx);
                cx.notify();
            });
        })
}

/// One row's line: its pictures and its text, laid out in document order.
///
/// The text stays one contiguous run so selection, copy, and hit testing keep
/// working on it, which means a picture written mid-sentence is drawn after the
/// text rather than between its words. Every other arrangement — badges alone,
/// a logo before a heading, an icon after a label — comes out in order.
fn render_row_line(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    if row.inline_images.is_empty() {
        return render_row_text(row_ix, row, context);
    }

    let (leading, trailing): (Vec<_>, Vec<_>) = row
        .inline_images
        .iter()
        .enumerate()
        .partition(|(_, inline)| inline.byte_offset == 0);

    let mut line = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(scaled(INLINE_IMAGE_GAP_PX, context))
        .flex_1()
        .min_w(px(0.0));
    for (image_ix, inline) in leading {
        line = line.child(render_inline_image(row_ix, image_ix, inline, context));
    }
    if !row.text.is_empty() {
        line = line.child(render_row_text(row_ix, row, context));
    }
    for (image_ix, inline) in trailing {
        line = line.child(render_inline_image(row_ix, image_ix, inline, context));
    }
    line.into_any_element()
}

/// Space between a picture and whatever shares its line.
const INLINE_IMAGE_GAP_PX: f32 = 6.0;

fn render_inline_image(
    row_ix: usize,
    image_ix: usize,
    inline: &MarkdownInlineImage,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let image = div()
        .flex_none()
        .child(crate::view::rows::markdown_preview_inline_image(
            inline,
            row_ix,
            image_ix,
            context.theme,
            context.ui_scale_percent,
            context.image_base_dir.as_deref(),
        ));

    // A picture wrapped in a link opens the same menu its text would.
    let (Some(view), Some(url)) = (context.view.clone(), inline.link_url.clone()) else {
        return image.into_any_element();
    };
    image
        .id(("markdown_preview_inline_image", row_ix * 64 + image_ix))
        .cursor(gpui::CursorStyle::PointingHand)
        .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
            let url = url.clone();
            let position = event.position;
            view.update(cx, |this, cx| {
                this.open_markdown_preview_link_menu(url, position, window, cx);
                cx.notify();
            });
        })
        .into_any_element()
}

/// One row's text, wrapping naturally and — when interactive — selectable.
fn render_row_text(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let styled = crate::view::rows::markdown_preview_styled_row(context.theme, row);

    let mut text = div().flex_1().min_w(px(0.0));
    if row
        .inline_spans
        .iter()
        .any(|span| span.style == MarkdownInlineStyle::Code)
    {
        // Inline code borrows the editor font for the whole line, matching how
        // the row preview renders it.
        text = text.font_family(context.editor_font_family.clone());
    }

    let Some(view) = context.view.clone() else {
        return if styled.highlights.is_empty() {
            text.child(styled.text.clone()).into_any_element()
        } else {
            text.child(crate::view::rows::markdown_preview_highlighted_text(
                styled.text.clone(),
                Arc::clone(&styled.highlights),
            ))
            .into_any_element()
        };
    };

    // The selection highlight is painted inside this box against the layout the
    // text was painted with, so the box must be the glyph box: any padding here
    // would slide the highlight off the text it covers.
    text.cursor(gpui::CursorStyle::IBeam)
        .debug_selector(move || format!("markdown_preview_text_box_{row_ix}"))
        .child(MarkdownFlowText::new(
            view,
            row_ix,
            context.text_region,
            row.text.clone(),
            styled.text.clone(),
            Arc::clone(&styled.highlights),
        ))
        .into_any_element()
}

fn render_heading(
    level: u8,
    row_ix: usize,
    document: &MarkdownPreviewDocument,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let Some(row) = document.rows.get(row_ix) else {
        return div().into_any_element();
    };
    let font_size = match level {
        1 => 22.0,
        2 => 18.0,
        3 => 16.0,
        4 => 14.5,
        _ => MARKDOWN_PREVIEW_BASE_FONT_PX,
    };
    let mut heading = row_shell(row_ix, row, context)
        .text_size(scaled(font_size, context))
        .font_weight(FontWeight::BOLD)
        .child(render_row_line(row_ix, row, context));

    // Only the top two levels get a rule under them, the way a rendered
    // README reads.
    if level <= 2 {
        heading = heading
            .pb(scaled(4.0, context))
            .border_b_1()
            .border_color(with_alpha(
                context.theme.colors.border,
                if context.theme.is_dark { 0.85 } else { 0.92 },
            ));
    }
    heading.into_any_element()
}

fn render_list(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let mut list = div().flex().flex_col().w_full().min_w(px(0.0));
    for (row_ix, row) in rows.iter() {
        let marker = crate::view::rows::markdown_preview_marker_label(row)
            .unwrap_or_else(|| SharedString::new_static(""));
        list = list.child(
            row_shell(row_ix, row, context)
                .pl(scaled(
                    f32::from(row.indent_level) * MARKDOWN_PREVIEW_INDENT_STEP_PX,
                    context,
                ))
                .child(
                    div()
                        .flex_none()
                        .min_w(scaled(MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX, context))
                        .mr(scaled(MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, context))
                        .text_color(context.theme.colors.text_muted)
                        .child(marker),
                )
                .child(render_row_line(row_ix, row, context)),
        );
    }
    list.into_any_element()
}

fn render_blockquote(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    // The block's kind is its first row's: blocks are split wherever an alert
    // starts, so every row in this one shares it.
    let first = rows.first();
    let alert = first.and_then(|(_, row)| row.alert_kind);
    let bar_color = alert
        .map(|kind| crate::view::rows::markdown_preview_alert_bar_color(context.theme, kind))
        .unwrap_or_else(|| {
            with_alpha(
                context.theme.colors.border,
                if context.theme.is_dark { 0.96 } else { 0.86 },
            )
        });

    let mut body = div().flex().flex_col().w_full().min_w(px(0.0));
    if let Some(label) = alert
        .filter(|_| first.is_some_and(|(_, row)| row.starts_alert))
        .and_then(crate::view::rows::markdown_preview_alert_label)
    {
        body = body.child(
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(bar_color)
                .child(label),
        );
    }
    for (row_ix, row) in rows.iter() {
        body = body
            .child(row_shell(row_ix, row, context).child(render_row_line(row_ix, row, context)));
    }

    div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .items_stretch()
        .child(
            div()
                .flex_none()
                .w(scaled(MARKDOWN_PREVIEW_BLOCKQUOTE_BAR_WIDTH_PX, context))
                .mr(scaled(MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, context))
                .bg(bar_color)
                .rounded(scaled(2.0, context)),
        )
        .child(body.text_color(context.theme.colors.text_muted))
        .into_any_element()
}

fn render_code(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let first_row_ix = rows.first().map(|(row_ix, _)| row_ix).unwrap_or_default();
    let mut body = div()
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .font_family(context.editor_font_family.clone())
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context));

    for (row_ix, row) in rows.iter() {
        body = body
            .child(row_shell(row_ix, row, context).child(render_row_line(row_ix, row, context)));
    }

    div()
        .id("markdown_document_code_block")
        .debug_selector(move || format!("markdown_preview_code_shell_{first_row_ix}"))
        .w_full()
        .min_w(px(0.0))
        // A code block scrolls on its own rather than widening the document or
        // rewrapping code that was written to specific columns.
        .overflow_x_scroll()
        .whitespace_nowrap()
        .px(scaled(MARKDOWN_PREVIEW_SHELL_PAD_X_PX, context))
        .py(scaled(CODE_BLOCK_PAD_Y_PX, context))
        .bg(with_alpha(
            context.theme.colors.active_section,
            if context.theme.is_dark { 0.55 } else { 0.45 },
        ))
        .border_1()
        .border_color(with_alpha(
            context.theme.colors.border,
            if context.theme.is_dark { 0.90 } else { 0.80 },
        ))
        .rounded(scaled(4.0, context))
        .child(body)
        .into_any_element()
}

fn render_table(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let mut table = div()
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .font_family(context.editor_font_family.clone());

    for (row_ix, row) in rows.iter() {
        let is_header = matches!(
            row.kind,
            MarkdownPreviewRowKind::TableRow { is_header: true }
        );
        // The header band is the stronger of the two so the first row reads as
        // labels rather than data.
        let cell_background = with_alpha(
            context.theme.colors.surface_bg_elevated,
            match (is_header, context.theme.is_dark) {
                (true, true) => 0.64,
                (true, false) => 0.86,
                (false, true) => 0.42,
                (false, false) => 0.72,
            },
        );
        let mut line = row_shell(row_ix, row, context)
            .px(scaled(TABLE_CELL_PAD_X_PX, context))
            .py(scaled(TABLE_CELL_PAD_Y_PX, context))
            .bg(cell_background)
            .child(render_row_line(row_ix, row, context));
        if is_header {
            line = line.font_weight(FontWeight::BOLD);
        }
        table = table.child(line.border_b_1().border_color(with_alpha(
            context.theme.colors.border,
            if context.theme.is_dark { 0.70 } else { 0.60 },
        )));
    }

    div()
        .id("markdown_document_table")
        .w_full()
        .min_w(px(0.0))
        .overflow_x_scroll()
        .whitespace_nowrap()
        .child(table)
        .into_any_element()
}

fn render_image(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    crate::view::rows::markdown_preview_flow_image(
        row,
        row_ix,
        context.theme,
        context.ui_scale_percent,
        context.image_base_dir.as_deref(),
    )
}

fn scaled(value: f32, context: &MarkdownDocumentContext) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, context.ui_scale_percent)
}
