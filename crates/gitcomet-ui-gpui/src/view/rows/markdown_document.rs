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
    MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX, MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX,
    MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX, MARKDOWN_PREVIEW_SHELL_PAD_X_PX,
};
use super::markdown_flow_text::MarkdownFlowText;
use super::*;
use crate::view::markdown_preview::{
    MAX_FLOWING_PREVIEW_ROWS, MarkdownBlock, MarkdownInlineImage, MarkdownInlineStyle,
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind,
    TOO_MANY_ROWS_TO_RENDER_MESSAGE, markdown_document_blocks,
};
use crate::view::perf::{self, ViewPerfRenderLane};
use std::cell::Cell;
use std::rc::Rc;

/// Everything the flowing renderer needs that is not in the document.
pub(in crate::view) struct MarkdownDocumentContext {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) editor_font_family: SharedString,
    /// Directory relative image sources resolve against.
    pub(in crate::view) image_base_dir: Option<Arc<std::path::Path>>,
    /// Sizes read from picture headers, so a picture that has not decoded yet
    /// still holds the box it is going to fill.
    pub(in crate::view) picture_sizes: crate::view::rows::MarkdownPreviewPictureSizes,
    /// Where each sideways-scrolling block is scrolled to.
    pub(in crate::view) block_scrolls: MarkdownDocumentBlockScrolls,
    /// The block grouping of the document being rendered, kept across frames.
    pub(in crate::view) blocks: MarkdownDocumentBlockCache,
    /// Set when the preview is interactive: text selection, copy, the link
    /// menu, and the diff context menu all go through this view.
    pub(in crate::view) view: Option<Entity<MainPaneView>>,
    pub(in crate::view) text_region: DiffTextRegion,
    /// Gutter colour for a wholly added or removed file, `None` otherwise.
    pub(in crate::view) change_bar_color: Option<gpui::Rgba>,
    /// Quick-search state, when the search box is open over this preview.
    pub(in crate::view) query: Option<crate::view::rows::MarkdownPreviewQuery>,
    /// A row the search cursor wants brought into view, and where to report the
    /// bounds that reveal needs. The flowing document has no fixed row height,
    /// so the offset can only be computed once the row has been laid out.
    pub(in crate::view) reveal: crate::view::rows::MarkdownPreviewRevealRequest,
    /// The container the document scrolls in, which the reveal moves.
    pub(in crate::view) scroll: Option<gpui::ScrollHandle>,
}

/// Gap between two blocks, and the extra break a heading opens above itself.
const BLOCK_GAP_PX: f32 = 10.0;
const HEADING_GAP_PX: f32 = 22.0;
const CODE_BLOCK_PAD_Y_PX: f32 = 8.0;
const TABLE_CELL_PAD_X_PX: f32 = 10.0;
const TABLE_CELL_PAD_Y_PX: f32 = 4.0;

/// Width of the gutter marking a wholly added or removed file.
const MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX: f32 = 3.0;

/// Blocks the flowing renderer last grouped, and the document they describe.
///
/// Grouping depends only on the document, but this renderer runs on every
/// frame — a scroll, a hover, a cursor blink — and re-deriving it means a scan
/// of every row plus an allocation each time. Holding the document alongside
/// its blocks is what makes the identity check sound: while the cache keeps
/// that `Arc` alive, no later document can occupy the same address.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownDocumentBlockCache(MarkdownDocumentBlockCacheSlot);

/// The cached document and its blocks, shared between the clones of a
/// [`MarkdownDocumentBlockCache`].
type MarkdownDocumentBlockCacheSlot = std::rc::Rc<
    std::cell::RefCell<
        Option<(
            Arc<MarkdownPreviewDocument>,
            std::rc::Rc<Vec<MarkdownBlock>>,
        )>,
    >,
>;

impl MarkdownDocumentBlockCache {
    fn blocks(&self, document: &Arc<MarkdownPreviewDocument>) -> std::rc::Rc<Vec<MarkdownBlock>> {
        let mut slot = self.0.borrow_mut();
        if let Some((cached, blocks)) = slot.as_ref()
            && Arc::ptr_eq(cached, document)
        {
            return std::rc::Rc::clone(blocks);
        }
        let blocks = std::rc::Rc::new(markdown_document_blocks(document));
        *slot = Some((Arc::clone(document), std::rc::Rc::clone(&blocks)));
        blocks
    }
}

/// Render a whole document as one flowing element tree.
pub(in crate::view) fn render_markdown_document(
    document: &Arc<MarkdownPreviewDocument>,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    // The budget belongs to this renderer, so this is where it is enforced —
    // a caller that skips the check its document was built with still cannot
    // make the pane lay out an unbounded tree.
    if document.rows.len() > MAX_FLOWING_PREVIEW_ROWS {
        return div()
            .w_full()
            .p(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
            .text_color(context.theme.colors.foreground.secondary)
            .child(TOO_MANY_ROWS_TO_RENDER_MESSAGE)
            .into_any_element();
    }

    let blocks = context.blocks.blocks(document);
    // The whole document lays out at once, so the rows the blocks cover are the
    // render cost. Spacers are not among them: the block builder drops them and
    // the flowing layout spends a margin instead.
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        document.rows.len(),
        blocks.iter().map(|block| block.row_range().len()).sum(),
    );
    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .pl(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context))
        .text_color(context.theme.colors.foreground.primary);

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
                context.theme.colors.stroke.default,
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

/// Whether a row belongs to a block that scrolls sideways instead of wrapping.
///
/// A scroll container has something to scroll only when its content is allowed
/// to exceed it, so these rows size to their text. Every other row fills its
/// line, which is what lets its text wrap.
fn row_scrolls_sideways(kind: MarkdownPreviewRowKind) -> bool {
    matches!(
        kind,
        MarkdownPreviewRowKind::CodeLine { .. } | MarkdownPreviewRowKind::TableRow { .. }
    )
}

/// The row div, carrying the quick-search reveal when this is the target row.
///
/// The flowing document is not a `uniform_list`, so nothing can compute the
/// scroll offset from a row index: the row has to be laid out first. The
/// listener fires during prepaint, once, and clears the request so it does not
/// keep dragging the view back while the user scrolls away.
fn reveal_listener(row_ix: usize, context: &MarkdownDocumentContext) -> gpui::Div {
    let shell = div();
    if context.reveal.pending() != Some(row_ix) {
        return shell;
    }
    let Some(scroll) = context.scroll.clone() else {
        return shell;
    };
    let reveal = context.reveal.clone();
    shell.on_children_prepainted(move |children_bounds, window, _app| {
        if reveal.take() != Some(row_ix) {
            return;
        }
        let Some((row_top, row_height)) =
            crate::view::rows::markdown_preview_row_extent(&children_bounds)
        else {
            return;
        };
        let viewport = scroll.bounds();
        let offset = scroll.offset();
        // Prepaint bounds are in window space with the scroll already applied,
        // so undo it to get the row's place in the document.
        let row_top_in_content = row_top - viewport.origin.y - offset.y;
        let Some(target_y) = crate::view::rows::markdown_preview_reveal_offset_y(
            row_top_in_content,
            row_height,
            viewport.size.height,
            scroll.max_offset().y,
            offset.y,
        ) else {
            return;
        };
        scroll.set_offset(point(offset.x, target_y));
        window.refresh();
    })
}

/// The container a row's text lives in: it carries the row's index, so mouse
/// events resolve to the same `(row, region)` pair selection and copy use.
fn row_shell(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> gpui::Stateful<gpui::Div> {
    let shell = reveal_listener(row_ix, context)
        .id(("md_preview_row", row_ix))
        .debug_selector(move || format!("markdown_preview_row_box_{row_ix}"));
    let shell = if row_scrolls_sideways(row.kind) {
        shell.flex_none()
    } else {
        shell.w_full().min_w(px(0.0))
    };
    let shell = shell
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

    // A picture written at offset 0 comes before the text; everything else
    // follows it. Two passes over the same slice rather than partitioning into
    // a pair of vectors, which this would otherwise do on every frame.
    let leading = || row.inline_images.iter().filter(|i| i.byte_offset == 0);
    let trailing = || row.inline_images.iter().filter(|i| i.byte_offset != 0);

    let mut line = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(scaled(MARKDOWN_PREVIEW_INLINE_IMAGE_GAP_PX, context))
        .flex_1()
        .min_w(px(0.0));
    for inline in leading() {
        line = line.child(render_inline_image(inline, context));
    }
    // A row of nothing but pictures still has to paint its (empty) text: that
    // element is what registers the row's hit-test box, and without one a drag
    // across the row finds no target and the selection skips over it.
    if !row.text.is_empty() || context.view.is_some() {
        line = line.child(render_row_text(row_ix, row, context));
    }
    for inline in trailing() {
        line = line.child(render_inline_image(inline, context));
    }
    line.into_any_element()
}

fn render_inline_image(
    inline: &MarkdownInlineImage,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let image = div()
        .flex_none()
        .child(crate::view::rows::markdown_preview_inline_image(
            inline,
            context.theme,
            context.ui_scale_percent,
            context.image_base_dir.as_deref(),
            &context.picture_sizes,
        ));

    // A picture wrapped in a link opens the same menu its text would.
    let (Some(view), Some(url)) = (context.view.clone(), inline.link_url.clone()) else {
        return image.into_any_element();
    };
    // The menu hangs off the picture's box, which only paint knows. Prepaint of
    // this frame runs before it can dispatch a click, so the handler always
    // reads a box from the frame it fired on.
    let painted_bounds = Rc::new(Cell::new(None));
    let record_bounds = Rc::clone(&painted_bounds);
    div()
        // The wrapper stands where the picture stood, so it keeps the picture's
        // sizing in the line it sits on.
        .flex_none()
        .on_children_prepainted(move |children_bounds, _window, _cx| {
            record_bounds.set(children_bounds.first().copied());
        })
        .child(
            image
                .id(("markdown_preview_inline_image_link", inline.source_byte))
                .cursor(gpui::CursorStyle::PointingHand)
                .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                    // The row underneath would otherwise also treat this as a
                    // click on its text and arm a drag-selection behind the menu.
                    cx.stop_propagation();
                    let url = url.clone();
                    let bounds = painted_bounds.get();
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        this.open_markdown_preview_link_menu(url, bounds, position, window, cx);
                        cx.notify();
                    });
                }),
        )
        .into_any_element()
}

/// One row's text, wrapping naturally and — when interactive — selectable.
fn render_row_text(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    // The flowing document renders one element per source row, so the row
    // index is also the index the search cursor addresses.
    let styled = crate::view::rows::markdown_preview_styled_row_with_query(
        context.theme,
        row,
        row_ix,
        context.query.as_ref(),
    );
    let styled = styled.as_ref();

    // Text that scrolls takes the width it needs; text that wraps takes the
    // width it is given.
    let mut text = if row_scrolls_sideways(row.kind) {
        div().flex_none()
    } else {
        div().flex_1().min_w(px(0.0))
    };
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
                context.theme.colors.stroke.default,
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
                        .text_color(context.theme.colors.foreground.secondary)
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
                context.theme.colors.stroke.default,
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
        .child(body.text_color(context.theme.colors.foreground.secondary))
        .into_any_element()
}

fn render_code(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let first_row_ix = rows.first().map(|(row_ix, _)| row_ix).unwrap_or_default();
    let mut body = div()
        // The content moves under the shell as the block scrolls, which is the
        // only way to see that each block holds its own offset.
        .debug_selector(move || format!("markdown_preview_code_body_{first_row_ix}"))
        .flex()
        .flex_col()
        // Sized to its widest line rather than to the block, so a line longer
        // than the pane has somewhere to scroll to — but never narrower than the
        // block, so a short one still fills it.
        .flex_none()
        .min_w(relative(1.0))
        .font_family(context.editor_font_family.clone())
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context));

    for (row_ix, row) in rows.iter() {
        body = body
            .child(row_shell(row_ix, row, context).child(render_row_line(row_ix, row, context)));
    }

    scrolling_block(
        "markdown_document_code_block",
        "markdown_document_code_block_scrollbar",
        first_row_ix,
        context,
        |block| {
            block
                .debug_selector(move || format!("markdown_preview_code_shell_{first_row_ix}"))
                .px(scaled(MARKDOWN_PREVIEW_SHELL_PAD_X_PX, context))
                .py(scaled(CODE_BLOCK_PAD_Y_PX, context))
                .bg(with_alpha(
                    context.theme.colors.interaction.selected_background,
                    if context.theme.is_dark { 0.55 } else { 0.45 },
                ))
                .border_1()
                .border_color(with_alpha(
                    context.theme.colors.stroke.default,
                    if context.theme.is_dark { 0.90 } else { 0.80 },
                ))
                .rounded(scaled(4.0, context))
                .child(body)
        },
    )
}

fn render_table(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let first_row_ix = rows.first().map(|(row_ix, _)| row_ix).unwrap_or_default();
    let mut table = div()
        .flex()
        .flex_col()
        // As with a code block: sized to its widest row so it can scroll, and at
        // least as wide as the block so a narrow table still spans it.
        .flex_none()
        .min_w(relative(1.0))
        .font_family(context.editor_font_family.clone());

    for (row_ix, row) in rows.iter() {
        let is_header = matches!(
            row.kind,
            MarkdownPreviewRowKind::TableRow { is_header: true }
        );
        // The header band is the stronger of the two so the first row reads as
        // labels rather than data.
        let cell_background = with_alpha(
            context.theme.colors.surface.raised,
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
            context.theme.colors.stroke.default,
            if context.theme.is_dark { 0.70 } else { 0.60 },
        )));
    }

    scrolling_block(
        "markdown_document_table",
        "markdown_document_table_scrollbar",
        first_row_ix,
        context,
        |block| block.child(table),
    )
}

/// Where each sideways-scrolling block is scrolled to, kept across frames.
///
/// `gpui` remembers a scroll offset against an element id by itself, which is
/// enough to scroll but not to *draw* a scrollbar: the bar has to read the
/// offset and the extent, and that needs a handle. Blocks come and go with the
/// document, so a handle is made the first time a block is drawn rather than
/// listed up front.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownDocumentBlockScrolls(
    std::rc::Rc<std::cell::RefCell<std::collections::HashMap<usize, gpui::ScrollHandle>>>,
);

impl MarkdownDocumentBlockScrolls {
    fn for_block(&self, first_row_ix: usize) -> gpui::ScrollHandle {
        self.0.borrow_mut().entry(first_row_ix).or_default().clone()
    }

    /// Forget every position: the document these blocks belonged to is gone.
    pub(in crate::view) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// How far a block can be scrolled sideways, which is what decides whether
    /// its scrollbar has a thumb to draw.
    #[cfg(test)]
    pub(in crate::view) fn max_scroll_for_tests(&self, first_row_ix: usize) -> Option<Pixels> {
        self.0
            .borrow()
            .get(&first_row_ix)
            .map(|handle| handle.max_offset().x)
    }
}

/// A block that scrolls sideways on its own rather than widening the document
/// or rewrapping content that was written to specific columns, with a scrollbar
/// along its bottom edge once there is somewhere to scroll to.
///
/// The id is keyed on the block's first row: `gpui` stores the scroll offset
/// against it, so blocks sharing one id would scroll as a single unit.
///
/// The scroller is a flex container so its content can be a `flex_none` item,
/// which is what lets that content exceed the block and give the scroll
/// something to do.
fn scrolling_block(
    name: &'static str,
    // Distinct from `name`: the bar is a sibling of the scroller, and two
    // siblings sharing an id share the state `gpui` keeps against it.
    scrollbar_name: &'static str,
    first_row_ix: usize,
    context: &MarkdownDocumentContext,
    build: impl FnOnce(gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div>,
) -> AnyElement {
    let handle = context.block_scrolls.for_block(first_row_ix);
    let mut block = div().id((name, first_row_ix));
    // Without this, `gpui` sends a plain wheel to whichever axis the element
    // scrolls — so a block that only scrolls sideways swallows the page scroll
    // the moment the pointer crosses it, and the document stops moving.
    block.style().restrict_scroll_to_axis = Some(true);
    let block = block
        .w_full()
        .min_w(px(0.0))
        .flex()
        .overflow_x_scroll()
        .whitespace_nowrap()
        .track_scroll(&handle)
        // Room for the bar, but only while there is one, so a block that fits
        // is not left with a strip of dead space under it.
        .pb(components::Scrollbar::visible_gutter(
            handle.clone(),
            components::ScrollbarAxis::Horizontal,
        ));

    let scrollbar = components::Scrollbar::horizontal((scrollbar_name, first_row_ix), handle);
    #[cfg(test)]
    let scrollbar = scrollbar.debug_selector(scrollbar_name);

    // The bar is positioned against this wrapper rather than the scroller: it
    // has to stay put while the content slides under it.
    div()
        .relative()
        .w_full()
        .min_w(px(0.0))
        .child(build(block))
        .child(scrollbar.render(context.theme))
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
        &context.picture_sizes,
    )
}

fn scaled(value: f32, context: &MarkdownDocumentContext) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, context.ui_scale_percent)
}
