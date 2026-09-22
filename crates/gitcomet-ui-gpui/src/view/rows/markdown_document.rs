//! Flowing renderer for the single-document markdown preview.
//!
//! The diff preview paints into a uniform (fixed row height) list, because its
//! two columns must stay row-aligned and every row carries a change bar. A
//! single document has neither requirement, so it lays out naturally instead:
//! text wraps by itself, images sit inline at the size the document asked for,
//! and the gaps around headings are compact interactive spacer elements.
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
use crate::kit::click::PointerClickExt as _;
use crate::view::markdown_preview::{
    MAX_FLOWING_PREVIEW_ROWS, MarkdownBlock, MarkdownInlineImage, MarkdownInlineStyle,
    MarkdownPreviewDiff, MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind,
    MarkdownTableAlign, TOO_MANY_ROWS_TO_RENDER_MESSAGE, markdown_document_blocks,
};
use crate::view::perf::{self, ViewPerfRenderLane};
use rustc_hash::FxHashMap;
use std::cell::Cell;
use std::rc::Rc;

/// Everything the flowing renderer needs that is not in the document.
pub(in crate::view) struct MarkdownDocumentContext {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) editor_font_family: SharedString,
    /// Directory relative image sources resolve against.
    pub(in crate::view) image_base_dir: Option<Arc<std::path::Path>>,
    pub(in crate::view) remote_image_access: crate::view::rows::MarkdownRemoteImageAccess,
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
    /// The link under the pointer.
    pub(in crate::view) hovered_link: Option<crate::view::rows::MarkdownPreviewHoveredLink>,
    /// Where changed blocks were laid out, for the diff's scrollbar markers.
    pub(in crate::view) change_extents: Option<MarkdownChangeExtents>,
}

/// Vertical extents of changed blocks in scroll-content coordinates, recorded
/// during prepaint so scrollbar markers sit where the changes are drawn rather
/// than where a row count guesses.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownChangeExtents(
    std::rc::Rc<std::cell::RefCell<Vec<(f32, f32, u8)>>>,
);

impl MarkdownChangeExtents {
    /// The extents the last frame recorded, leaving the list empty for this one.
    pub(in crate::view) fn take(&self) -> Vec<(f32, f32, u8)> {
        std::mem::take(&mut self.0.borrow_mut())
    }

    fn record(&self, top: f32, bottom: f32, flag: u8) {
        self.0.borrow_mut().push((top, bottom, flag));
    }
}

/// Gap between two blocks, and the extra break a heading opens above itself.
const BLOCK_GAP_PX: f32 = 10.0;
const HEADING_GAP_PX: f32 = 22.0;
const CODE_BLOCK_PAD_Y_PX: f32 = 8.0;
const TABLE_CELL_PAD_X_PX: f32 = 10.0;
const TABLE_CELL_PAD_Y_PX: f32 = 4.0;

/// Width of the gutter marking a wholly added or removed file.
const MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX: f32 = 3.0;

/// Keep flowing fenced blocks on the same neutral surface as fixed-row
/// markdown previews. Accent-backed selection colors happen to look neutral in
/// GitComet Dark, but become amber in Amber Dark and are not a code surface.
fn markdown_document_code_background(theme: AppTheme) -> gpui::Rgba {
    super::history::markdown_preview_code_background(theme)
}

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
    let blocks = context.blocks.blocks(document);
    render_markdown_document_with_blocks(document, &blocks, context)
}

/// As [`render_markdown_document`], for a document whose blocks are already
/// grouped — the inline diff keeps them with the document.
pub(in crate::view) fn render_markdown_document_with_blocks(
    document: &MarkdownPreviewDocument,
    blocks: &[MarkdownBlock],
    context: &MarkdownDocumentContext,
) -> AnyElement {
    // The budget belongs to this renderer, so this is where it is enforced —
    // a caller that skips the check its document was built with still cannot
    // make the pane lay out an unbounded tree.
    if document.rows.len() > MAX_FLOWING_PREVIEW_ROWS {
        return too_many_rows(context);
    }
    record_flowing_rows(document, blocks);
    let column = render_block_column(document, blocks, 0, context);

    // The change bar is one element spanning the whole document rather than a
    // segment per row: a flowing layout puts gaps between blocks, and a
    // per-row bar would leave a gap in every one of them.
    let body = div()
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
        .child(column);

    let mut surface = div()
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .min_h_full()
        .child(body);
    if let Some(view) = context.view.clone() {
        surface = surface.child(flowing_diff_text_empty_space(view, context.text_region));
    }
    surface.into_any_element()
}

/// The split markdown diff: each band's old and new blocks side by side, so
/// the shorter side of a band is left blank and the two stay lined up.
pub(in crate::view) fn render_markdown_diff_split(
    diff: &MarkdownPreviewDiff,
    left: &MarkdownDocumentContext,
    right: &MarkdownDocumentContext,
) -> AnyElement {
    if diff.old.rows.len().max(diff.new.rows.len()) > MAX_FLOWING_PREVIEW_ROWS {
        return too_many_rows(left);
    }
    record_flowing_rows(&diff.old, &diff.old_blocks);
    record_flowing_rows(&diff.new, &diff.new_blocks);
    let divider = left.theme.colors.stroke.default;

    let mut column = div()
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, left))
        .text_color(left.theme.colors.foreground.primary);
    for (band_ix, band) in diff.bands.iter().enumerate() {
        let old_blocks = &diff.old_blocks[band.old_blocks.clone()];
        let new_blocks = &diff.new_blocks[band.new_blocks.clone()];
        if band_ix > 0 {
            let opens_heading = old_blocks
                .iter()
                .chain(new_blocks)
                .next()
                .is_some_and(|block| matches!(block, MarkdownBlock::Heading { .. }));
            let gap = if opens_heading {
                HEADING_GAP_PX
            } else {
                BLOCK_GAP_PX
            };
            let gap_side = |context: &MarkdownDocumentContext, side: usize| {
                div().flex_1().min_w(px(0.0)).child(render_block_gap(
                    band_ix * 2 + side,
                    band.rows.start,
                    gap,
                    context,
                ))
            };
            column = column.child(
                div()
                    .flex()
                    .w_full()
                    .child(gap_side(left, 0))
                    .child(div().w(px(1.0)).flex_none().bg(divider))
                    .child(gap_side(right, 1)),
            );
        }
        let side = |document, blocks, context: &MarkdownDocumentContext| {
            div()
                .flex_1()
                .min_w(px(0.0))
                .child(render_block_column(document, blocks, band_ix, context))
        };
        column = column.child(
            div()
                .id(("markdown_diff_band", band_ix))
                .debug_selector(move || format!("markdown_diff_band_{band_ix}"))
                .flex()
                .items_stretch()
                .w_full()
                .child(side(&diff.old, old_blocks, left))
                .child(div().w(px(1.0)).flex_none().bg(divider))
                .child(side(&diff.new, new_blocks, right)),
        );
    }

    let mut surface = div()
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .min_h_full()
        .child(column);
    if let (Some(left_view), Some(right_view)) = (left.view.clone(), right.view.clone()) {
        surface = surface.child(
            div()
                .flex()
                .flex_1()
                .w_full()
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(flowing_diff_text_empty_space(left_view, left.text_region)),
                )
                .child(div().w(px(1.0)).flex_none().bg(divider))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(flowing_diff_text_empty_space(right_view, right.text_region)),
                ),
        );
    }
    surface.into_any_element()
}

fn too_many_rows(context: &MarkdownDocumentContext) -> AnyElement {
    div()
        .w_full()
        .p(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
        .text_color(context.theme.colors.foreground.secondary)
        .child(TOO_MANY_ROWS_TO_RENDER_MESSAGE)
        .into_any_element()
}

/// The whole document lays out at once, so the rows the blocks cover are the
/// render cost. Spacers are not among them: the block builder drops them and
/// the flowing layout spends one interactive gap between blocks instead.
fn record_flowing_rows(document: &MarkdownPreviewDocument, blocks: &[MarkdownBlock]) {
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        document.rows.len(),
        blocks.iter().map(|block| block.row_range().len()).sum(),
    );
}

/// Blocks stacked with the gaps between them. `gap_key` keeps gap ids unique
/// when several columns render into one tree.
fn render_block_column(
    document: &MarkdownPreviewDocument,
    blocks: &[MarkdownBlock],
    gap_key: usize,
    context: &MarkdownDocumentContext,
) -> gpui::Div {
    let mut column = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .pl(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
        .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context))
        .text_color(context.theme.colors.foreground.primary);

    for (ix, block) in blocks.iter().enumerate() {
        if ix > 0 {
            let gap = if matches!(block, MarkdownBlock::Heading { .. }) {
                HEADING_GAP_PX
            } else {
                BLOCK_GAP_PX
            };
            // The worktree document keeps its original gap ids.
            let gap_ix = if gap_key == 0 {
                ix
            } else {
                (gap_key << 20) | (usize::from(context.text_region.order()) << 16) | ix
            };
            column = column.child(render_block_gap(
                gap_ix,
                block.row_range().start,
                gap,
                context,
            ));
        }
        let rendered = record_change_extent(
            document,
            block,
            context,
            render_block(document, block, context),
        );
        column = column.child(match block_change_bar(document, block, context) {
            // A wholly added or removed block is marked down its whole height,
            // gaps and padding included, so it reads as one change.
            Some(color) => div()
                .flex()
                .items_stretch()
                .w_full()
                .min_w(px(0.0))
                .child(
                    div()
                        .flex_none()
                        .w(scaled(MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX, context))
                        .mr(scaled(MARKDOWN_DOCUMENT_CHANGE_BAR_WIDTH_PX, context))
                        .bg(color)
                        .debug_selector(|| "markdown_preview_block_change_bar".to_string()),
                )
                .child(div().flex_1().min_w(px(0.0)).child(rendered))
                .into_any_element(),
            None => rendered,
        });
    }
    column
}

/// Wrap a changed block so it reports where it was laid out.
fn record_change_extent(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    context: &MarkdownDocumentContext,
    rendered: AnyElement,
) -> AnyElement {
    let (Some(extents), Some(scroll)) = (context.change_extents.clone(), context.scroll.clone())
    else {
        return rendered;
    };
    let flag = RowRun::new(document, block.row_range())
        .iter()
        .fold(0u8, |flag, (_, row)| {
            flag | crate::view::markdown_preview::scrollbar_flag_for_change_hint(row.change_hint)
        });
    if flag == 0 {
        return rendered;
    }
    div()
        .w_full()
        .min_w(px(0.0))
        .on_children_prepainted(move |children, _window, _cx| {
            let Some((top, height)) = crate::view::rows::markdown_preview_row_extent(&children)
            else {
                return;
            };
            // Prepaint bounds are in window space with the scroll applied.
            let top = top - scroll.bounds().origin.y - scroll.offset().y;
            extents.record(f32::from(top), f32::from(top + height), flag);
        })
        .child(rendered)
        .into_any_element()
}

/// The bar colour for a block every row of which was added, or removed.
fn block_change_bar(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    context: &MarkdownDocumentContext,
) -> Option<gpui::Rgba> {
    use crate::view::markdown_preview::MarkdownChangeHint;
    let rows = RowRun::new(document, block.row_range());
    let mut hints = rows.iter().map(|(_, row)| row.change_hint);
    let first = hints.next()?;
    if !hints.all(|hint| hint == first) {
        return None;
    }
    let colors = &context.theme.colors.status;
    match first {
        MarkdownChangeHint::Added => Some(colors.success.foreground),
        MarkdownChangeHint::Removed => Some(colors.danger.foreground),
        MarkdownChangeHint::Modified | MarkdownChangeHint::None => None,
    }
}

fn render_block_gap(
    block_ix: usize,
    next_source_visible_ix: usize,
    gap: f32,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let space = div()
        .id(("markdown_preview_block_gap", block_ix))
        .debug_selector(move || format!("markdown_preview_block_gap_{block_ix}"))
        .flex_none()
        .w_full()
        .h(scaled(gap, context));
    let Some(view) = context.view.clone() else {
        return space.into_any_element();
    };

    let left_view = view.clone();
    let right_view = view;
    let region = context.text_region;
    space
        .cursor(gpui::CursorStyle::IBeam)
        .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = left_view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            left_view.update(cx, |this, cx| {
                this.handle_diff_text_document_gap_mouse_down(
                    next_source_visible_ix,
                    region,
                    event.position,
                    window,
                    cx,
                );
                cx.notify();
            });
        })
        .on_pointer_click(gpui::MouseButton::Right, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = right_view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            right_view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu(
                    next_source_visible_ix,
                    region,
                    event.position,
                    window,
                    cx,
                );
                cx.notify();
            });
        })
        .into_any_element()
}

fn render_block(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let wrapper = div().w_full().min_w(px(0.0));
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
        MarkdownBlock::Image(_) => non_text_block_shell(document, block.row_range(), context)
            .when_some(rows.first(), |wrapper, (row_ix, row)| {
                wrapper.child(render_image(row_ix, row, context))
            })
            .into_any_element(),
        MarkdownBlock::ThematicBreak(row_ix) => {
            let row_ix = *row_ix;
            non_text_block_shell(document, block.row_range(), context)
                .child(
                    div()
                        .w_full()
                        .h(px(1.0))
                        .bg(with_alpha(
                            context.theme.colors.stroke.default,
                            if context.theme.is_dark { 0.92 } else { 0.88 },
                        ))
                        .debug_selector(move || {
                            format!("markdown_preview_thematic_break_{row_ix}")
                        }),
                )
                .into_any_element()
        }
    }
}

/// Give a non-text block a logical range for selection motion without
/// pretending that its accessible copy text was painted as glyphs.
fn non_text_block_shell(
    document: &MarkdownPreviewDocument,
    range: Range<usize>,
    context: &MarkdownDocumentContext,
) -> gpui::Div {
    let shell = div().w_full().min_w(px(0.0));
    let (Some(view), Some(first_row_ix), Some(last_row_ix)) = (
        context.view.clone(),
        range.clone().next(),
        range.clone().next_back(),
    ) else {
        return shell;
    };
    let Some(last_row) = document.rows.get(last_row_ix) else {
        return shell;
    };
    let region = context.text_region;
    let start = DiffTextPos {
        source_visible_ix: first_row_ix,
        region,
        offset: 0,
    };
    let end = DiffTextPos {
        source_visible_ix: last_row_ix,
        region,
        offset: last_row.text.len(),
    };

    shell.on_children_prepainted(move |children_bounds, _window, app| {
        let Some(bounds) = children_bounds.first().copied() else {
            return;
        };
        view.update(app, |this, _cx| {
            this.set_diff_text_motion_target(bounds, start, end);
        });
    })
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
            // Alignment padding inside a diff block draws nothing.
            .filter(|(_, row)| !matches!(row.kind, MarkdownPreviewRowKind::Spacer))
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
    matches!(kind, MarkdownPreviewRowKind::CodeLine { .. })
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
    let view = context.view.clone();
    shell.on_children_prepainted(move |children_bounds, window, _app| {
        let Some((revealed_ix, align)) = reveal.take() else {
            return;
        };
        if revealed_ix != row_ix {
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
            align,
            row_top_in_content,
            row_height,
            viewport.size.height,
            scroll.max_offset().y,
            offset.y,
        ) else {
            return;
        };
        scroll.set_offset(point(offset.x, target_y));
        // `refresh` is a no-op mid-draw, and this frame was laid out at the old
        // offset: repaint on the next one.
        match view.clone() {
            Some(view) => window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify())),
            None => window.request_animation_frame(),
        }
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

    row_interactions(shell, row_ix, context)
}

/// What makes a row's box behave as text: selection, links, the context menu.
/// A table row spreads these over each of its cells.
fn row_interactions(
    shell: gpui::Stateful<gpui::Div>,
    row_ix: usize,
    context: &MarkdownDocumentContext,
) -> gpui::Stateful<gpui::Div> {
    let Some(view) = context.view.clone() else {
        return shell;
    };
    let text_region = context.text_region;
    shell
        .cursor(crate::view::rows::MarkdownPreviewHoveredLink::cursor(
            context.hovered_link.as_ref(),
            text_region,
            row_ix,
        ))
        .on_mouse_move({
            let view = view.clone();
            move |event, _window, cx| {
                view.update(cx, |this, cx| {
                    this.update_markdown_preview_link_hover(
                        row_ix,
                        text_region,
                        event.position,
                        event.pressed_button.is_some(),
                        cx,
                    );
                });
            }
        })
        .on_hover({
            let view = view.clone();
            move |hovered, _window, cx| {
                if !*hovered {
                    view.update(cx, |this, cx| {
                        this.clear_markdown_preview_link_hover(row_ix, text_region, cx);
                    });
                }
            }
        })
        .on_mouse_down(gpui::MouseButton::Left, {
            let view = view.clone();
            move |event, window, cx| {
                let focus = view.read(cx).diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                let click_count = event.click_count;
                let position = event.position;
                view.update(cx, |this, cx| {
                    this.handle_markdown_preview_row_mouse_down(
                        row_ix,
                        text_region,
                        position,
                        click_count,
                        window,
                        cx,
                    );
                    cx.notify();
                });
            }
        })
        .on_pointer_click(gpui::MouseButton::Left, {
            let view = view.clone();
            move |event, window, cx| {
                view.update(cx, |this, cx| {
                    this.handle_markdown_preview_link_click(
                        row_ix,
                        text_region,
                        event.position,
                        event.click_count,
                        window,
                        cx,
                    );
                    cx.notify();
                });
            }
        })
        .on_pointer_click(gpui::MouseButton::Right, move |event, window, cx| {
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
            &context.remote_image_access,
        ));

    // A picture wrapped in a link opens the same menu its text would.
    let (Some(view), Some(url)) = (context.view.clone(), inline.link_url.clone()) else {
        return image.into_any_element();
    };
    let load_remote_image_url =
        if context.remote_image_access.policy == RemoteMarkdownImagePolicy::AskBeforeLoading {
            crate::view::rows::markdown_preview_remote_image_url(inline.image.source.as_ref())
                .filter(|image_url| !context.remote_image_access.permits(image_url))
        } else {
            None
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
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .cursor(gpui::CursorStyle::PointingHand)
                .on_pointer_click(gpui::MouseButton::Left, move |event, window, cx| {
                    // The row underneath would otherwise also treat this as a
                    // click on its text and arm a drag-selection behind the menu.
                    cx.stop_propagation();
                    let url = url.clone();
                    let load_remote_image_url = load_remote_image_url.clone();
                    let bounds = painted_bounds.get();
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        this.open_markdown_preview_link_menu(
                            url,
                            load_remote_image_url,
                            bounds,
                            position,
                            window,
                            cx,
                        );
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
        crate::view::rows::MarkdownPreviewHoveredLink::range_in_row(
            context.hovered_link.as_ref(),
            context.text_region,
            row_ix,
        ),
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
    text.cursor(crate::view::rows::MarkdownPreviewHoveredLink::cursor(
        context.hovered_link.as_ref(),
        context.text_region,
        row_ix,
    ))
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
    let last_row_ix = rows.iter().last().map(|(row_ix, _)| row_ix);
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

    if rows.first().is_some() {
        body = body.child(render_code_padding(first_row_ix, false, context));
    }
    for (row_ix, row) in rows.iter() {
        body = body
            .child(row_shell(row_ix, row, context).child(render_row_line(row_ix, row, context)));
    }
    if let Some(last_row_ix) = last_row_ix {
        body = body.child(render_code_padding(last_row_ix, true, context));
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
                .bg(markdown_document_code_background(context.theme))
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

/// The vertical breathing room inside a fenced-code shell is document space,
/// just like the gap between two Markdown blocks. Keeping it as explicit
/// elements preserves the code shell's geometry while allowing a selection to
/// begin immediately before its first row or after its last one.
fn render_code_padding(
    row_ix: usize,
    after_row: bool,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let edge = if after_row { "bottom" } else { "top" };
    let padding = div()
        .id((
            "markdown_preview_code_padding",
            row_ix
                .saturating_mul(2)
                .saturating_add(usize::from(after_row)),
        ))
        .debug_selector(move || format!("markdown_preview_code_padding_{edge}_{row_ix}"))
        .flex_none()
        .w_full()
        .h(scaled(CODE_BLOCK_PAD_Y_PX, context));
    let Some(view) = context.view.clone() else {
        return padding.into_any_element();
    };

    let left_view = view.clone();
    let text_region = context.text_region;
    padding
        .cursor(gpui::CursorStyle::IBeam)
        .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = left_view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            left_view.update(cx, |this, cx| {
                this.handle_markdown_preview_row_mouse_down(
                    row_ix,
                    text_region,
                    event.position,
                    event.click_count,
                    window,
                    cx,
                );
                cx.notify();
            });
        })
        .on_pointer_click(gpui::MouseButton::Right, move |event, window, cx| {
            crate::press_gesture::claim_press(cx);
            cx.stop_propagation();
            let focus = view.read(cx).diff_panel_focus_handle.clone();
            window.focus(&focus, cx);
            view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu(row_ix, text_region, event.position, window, cx);
                cx.notify();
            });
        })
        .into_any_element()
}

/// A table as a grid: columns sized to their content that shrink and wrap
/// when the pane is narrow, dividers between cells, a header band, and
/// alternating rows.
fn render_table(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let first_row_ix = rows.first().map(|(row_ix, _)| row_ix).unwrap_or_default();
    let Some(table) = rows
        .first()
        .and_then(|(_, row)| row.table.as_ref())
        .map(|cells| Arc::clone(&cells.table))
    else {
        return div().into_any_element();
    };
    let column_count = table.alignments.len();
    if column_count == 0 {
        return div().into_any_element();
    }
    let theme = context.theme;
    let divider = with_alpha(
        theme.colors.stroke.default,
        if theme.is_dark { 0.70 } else { 0.60 },
    );
    let header_band = with_alpha(
        theme.colors.surface.raised,
        if theme.is_dark { 0.64 } else { 0.86 },
    );
    let stripe = with_alpha(
        theme.colors.surface.raised,
        if theme.is_dark { 0.32 } else { 0.55 },
    );

    // `minmax(min-content, 1fr)` on a grid that sizes to its content: columns
    // hug their text while it fits, wrap when the pane is narrow, and stop at
    // the longest word, past which the table scrolls.
    let mut grid = div()
        .grid()
        .grid_cols_max_content(column_count as u16)
        .flex_none()
        .max_w(relative(1.0))
        .whitespace_normal()
        .border_1()
        .border_color(divider)
        .rounded(px(theme.radii.row));
    let mut body_ix = 0usize;
    for (grid_row, (row_ix, row)) in rows.iter().enumerate() {
        let Some(cells) = row.table.as_ref() else {
            continue;
        };
        let is_header = matches!(
            row.kind,
            MarkdownPreviewRowKind::TableRow { is_header: true }
        );
        // A change tint wins; otherwise the header band, then every other row.
        let background = crate::view::rows::markdown_preview_row_background(theme, row)
            .or(is_header.then_some(header_band))
            .or((!is_header && body_ix % 2 == 1).then_some(stripe));
        if !is_header {
            body_ix += 1;
        }
        for (column, range) in cells.cells.iter().enumerate() {
            let align = if is_header {
                MarkdownTableAlign::Center
            } else {
                table.alignments.get(column).copied().unwrap_or_default()
            };
            // The search reveal needs one box per row; the first cell stands in.
            let shell = if column == 0 {
                reveal_listener(row_ix, context)
            } else {
                div()
            };
            let cell = shell
                .id(("md_preview_table_cell", row_ix * column_count + column))
                .debug_selector(move || format!("markdown_preview_cell_box_{row_ix}_{column}"))
                .px(scaled(TABLE_CELL_PAD_X_PX, context))
                .py(scaled(TABLE_CELL_PAD_Y_PX, context))
                .when(column > 0, |cell| cell.border_l_1())
                .when(grid_row > 0, |cell| cell.border_t_1())
                .border_color(divider)
                .when_some(background, |cell, background| cell.bg(background))
                .when(is_header, |cell| cell.font_weight(FontWeight::SEMIBOLD))
                // The text box moves, not the glyphs inside it: `gpui` hit-tests
                // and places selections as if every line started at the left.
                .flex()
                .map(|cell| match align {
                    MarkdownTableAlign::Center => cell.justify_center(),
                    MarkdownTableAlign::Right => cell.justify_end(),
                    MarkdownTableAlign::None | MarkdownTableAlign::Left => cell,
                })
                .child(render_cell_text(
                    row_ix,
                    row,
                    column,
                    range.clone(),
                    context,
                ));
            grid = grid.child(row_interactions(cell, row_ix, context));
        }
    }

    // The table hugs its columns; a word too long to wrap still scrolls.
    scrolling_block(
        "markdown_document_table",
        "markdown_document_table_scrollbar",
        first_row_ix,
        context,
        |block| block.child(div().flex().w_full().min_w(px(0.0)).child(grid)),
    )
}

/// One table cell's text: its slice of the row, styled as the row is and
/// selectable in row coordinates.
fn render_cell_text(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    column: usize,
    range: std::ops::Range<usize>,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let styled = crate::view::rows::markdown_preview_styled_row_with_query(
        context.theme,
        row,
        row_ix,
        context.query.as_ref(),
        crate::view::rows::MarkdownPreviewHoveredLink::range_in_row(
            context.hovered_link.as_ref(),
            context.text_region,
            row_ix,
        ),
    );
    let styled = super::diff_text::slice_cached_diff_styled_text(
        styled.as_ref(),
        super::history::markdown_preview_expanded_slice_range(
            row.text.as_ref(),
            styled.text.len(),
            &range,
        ),
    );
    // No percentage width: while the grid sizes its columns a `w_full` box
    // measures its text at width 0, and `gpui` keeps that one-glyph-per-line
    // size for every later probe. The column stretches the box anyway.
    let text = div().min_w(px(0.0)).when(
        row.inline_spans[..].iter().any(|span| {
            span.style == MarkdownInlineStyle::Code
                && span.byte_range.start < range.end
                && range.start < span.byte_range.end
        }),
        |text| text.font_family(context.editor_font_family.clone()),
    );
    let Some(view) = context.view.clone() else {
        return text
            .child(crate::view::rows::markdown_preview_highlighted_text(
                styled.text.clone(),
                Arc::clone(&styled.highlights),
            ))
            .into_any_element();
    };
    text.cursor(crate::view::rows::MarkdownPreviewHoveredLink::cursor(
        context.hovered_link.as_ref(),
        context.text_region,
        row_ix,
    ))
    .debug_selector(move || format!("markdown_preview_cell_text_box_{row_ix}_{column}"))
    .child(
        MarkdownFlowText::new(
            view,
            row_ix,
            context.text_region,
            styled.text.clone(),
            styled.text.clone(),
            Arc::clone(&styled.highlights),
        )
        .cell(range, row.text.len()),
    )
    .into_any_element()
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
    std::rc::Rc<std::cell::RefCell<FxHashMap<usize, gpui::ScrollHandle>>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amber_fenced_code_blocks_follow_gitcomet_darks_neutral_surface_rule() {
        let amber = AppTheme::from_key(crate::theme::AMBER_DARK_THEME_KEY)
            .expect("Amber Dark theme should load");
        let gitcomet_dark = AppTheme::gitcomet_dark();

        for theme in [amber, gitcomet_dark] {
            assert_eq!(
                markdown_document_code_background(theme),
                with_alpha(theme.colors.surface.raised, 0.88)
            );
        }
        assert_ne!(
            markdown_document_code_background(amber),
            with_alpha(amber.colors.interaction.selected_background, 0.55),
            "Amber's accent-backed selection color must not tint fenced code blocks"
        );
    }
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
        &context.remote_image_access,
    )
}

fn scaled(value: f32, context: &MarkdownDocumentContext) -> Pixels {
    context.theme.markdown_px(value, context.ui_scale_percent)
}
