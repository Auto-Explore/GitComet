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
use crate::kit::interaction::ControlInteractionExt as _;
use crate::view::markdown_preview::{
    MarkdownBlock, MarkdownInlineImage, MarkdownInlineStyle, MarkdownPreviewDiff,
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind, MarkdownTableAlign,
    MarkdownTaskMarker, markdown_document_blocks,
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
    /// Where relative picture sources resolve.
    pub(in crate::view) image_root: Option<crate::view::rows::MarkdownImageRoot>,
    pub(in crate::view) remote_image_access: crate::view::rows::MarkdownRemoteImageAccess,
    /// Sizes read from picture headers, so a picture that has not decoded yet
    /// still holds the box it is going to fill.
    pub(in crate::view) picture_sizes: crate::view::rows::MarkdownPreviewPictureSizes,
    /// Where the pictures a frame draws are listed, when the pane waits on
    /// their decode.
    pub(in crate::view) drawn_pictures: Option<crate::view::rows::MarkdownDrawnPictures>,
    /// Where this frame's rows and gaps are recorded for the document's
    /// pointer listeners; both sides of a split share one.
    pub(in crate::view) row_boxes: MarkdownRowBoxes,
    /// Where each sideways-scrolling block is scrolled to.
    pub(in crate::view) block_scrolls: MarkdownDocumentBlockScrolls,
    /// The block grouping of the document being rendered, kept across frames.
    pub(in crate::view) blocks: MarkdownDocumentBlockCache,
    /// How tall each top-level block (or split band) was when last drawn, so a
    /// frame builds only those near the viewport.
    pub(in crate::view) layout: MarkdownDocumentLayoutCache,
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
    /// Whether a task checkbox click writes to the file: only a document parsed
    /// from the working-tree file itself, so its byte offsets are the disk's.
    pub(in crate::view) tasks_editable: bool,
}

/// Vertical extents of changed blocks in scroll-content coordinates, placed
/// from the heights the windowed column keeps, so scrollbar markers sit where
/// the changes are drawn — or will be — rather than where a row count guesses.
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

/// How tall a flowing preview's top-level items — its blocks, or a split
/// diff's bands — were when last drawn.
///
/// The preview scrolls a plain container, and laying out every block of a
/// long document on every frame costs time in proportion to its length. Only
/// the items near the viewport are built; the rest stand in as two spacers of
/// their height — measured once an item has been drawn, estimated before.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownDocumentLayoutCache(Rc<std::cell::RefCell<MarkdownItemHeights>>);

#[derive(Default)]
struct MarkdownItemHeights {
    /// The items the heights describe: the document's address and how many.
    key: (usize, usize),
    /// The width they were measured at; text re-flows at another.
    width: Option<Pixels>,
    measured: Vec<Option<Pixels>>,
    /// Where the column began in the scroll content, as last drawn.
    top: Pixels,
    /// The window's height, which sets how far past the viewport text keeps
    /// hitboxes — and so how far past it items are built.
    window_height: Pixels,
    /// The item at the top of the viewport last frame, and where it began.
    anchor: Option<(usize, Pixels)>,
}

/// One frame's plan for a windowed column.
struct MarkdownWindow {
    heights: Vec<Pixels>,
    /// Where the column began in the scroll content, as last drawn.
    top: Pixels,
    /// Items laid out in the column: those around the viewport whose place
    /// is known exactly.
    flow: Range<usize>,
    /// Items above the viewport that have never been drawn, laid out out of
    /// sight to learn their height without moving what is in view.
    measure: Vec<usize>,
}

impl MarkdownDocumentLayoutCache {
    /// Plan a frame of `count` items identified by `key`: their heights — as
    /// last drawn, or `estimate`d — and which of them to build.
    ///
    /// Items above the viewport are never laid out in the column: the spacer
    /// above it stands for them. When that height changes because one of them
    /// was measured, the scroll offset moves by as much before this frame is
    /// laid out, so the item at the top of the viewport stays where it was —
    /// an adjustment made after layout would show, displaced, for a frame.
    fn plan(
        &self,
        key: (usize, usize),
        count: usize,
        scroll: Option<&gpui::ScrollHandle>,
        target: Option<usize>,
        estimate: impl Fn(usize, Option<Pixels>) -> Pixels,
    ) -> MarkdownWindow {
        let mut inner = self.0.borrow_mut();
        if inner.key != key || inner.measured.len() != count {
            *inner = MarkdownItemHeights {
                key,
                measured: vec![None; count],
                window_height: inner.window_height,
                ..MarkdownItemHeights::default()
            };
        }
        let width = inner.width;
        let heights: Vec<Pixels> = inner
            .measured
            .iter()
            .enumerate()
            .map(|(ix, measured)| measured.unwrap_or_else(|| estimate(ix, width)))
            .collect();
        let Some(scroll) = scroll else {
            return MarkdownWindow {
                heights,
                top: inner.top,
                flow: 0..count,
                measure: Vec::new(),
            };
        };

        // Keep last frame's top item where it was.
        if let Some((anchor, anchor_top)) = inner.anchor
            && anchor < count
        {
            let now: Pixels = heights[..anchor].iter().copied().sum();
            let moved = now - anchor_top;
            if moved != px(0.0) {
                let offset = scroll.offset();
                scroll.set_offset(point(offset.x, offset.y - moved));
            }
        }

        // Before its first layout the viewport has no height; a screen is a
        // fair guess, and the next frame corrects it.
        let viewport = scroll.bounds().size.height;
        let viewport = if viewport > px(0.0) {
            viewport
        } else {
            px(1200.0)
        };
        let top = -scroll.offset().y - inner.top;
        // The same reach the text keeps hitboxes for — two window heights —
        // so a drag that runs off the edge still finds the rows it heads for.
        let reach = inner.window_height.max(viewport) * 2.0;
        let (from, to) = (top - reach, top + viewport + reach);

        let mut y = px(0.0);
        let mut first = None;
        let mut anchor = None;
        let mut end = 0;
        for (ix, height) in heights.iter().enumerate() {
            if y >= to {
                break;
            }
            let bottom = y + *height;
            if bottom > from && first.is_none() {
                first = Some(ix);
            }
            if bottom > top && anchor.is_none() {
                anchor = Some(ix);
            }
            end = ix + 1;
            y = bottom;
        }
        // Scrolled past the end — the content shrank under the offset — the
        // last item is still built.
        let mut anchor = anchor.unwrap_or(count.saturating_sub(1));
        let mut first = first.unwrap_or(anchor).min(anchor);
        let mut end = end.max((anchor + 1).min(count));
        // A reveal builds the item it waits on, wherever it is; its listener
        // then scrolls there.
        if let Some(target) = target.filter(|target| *target < count)
            && !(anchor..end).contains(&target)
        {
            anchor = target;
            first = target.saturating_sub(1);
            end = (target + 2).min(count);
        }
        inner.anchor = Some((anchor, heights[..anchor].iter().copied().sum()));
        // Items above the viewport whose height is known lay out exactly where
        // the spacer would have put them, so the run of them just above it
        // joins the column; the rest are measured out of sight.
        let mut start = anchor;
        while start > first && inner.measured[start - 1].is_some() {
            start -= 1;
        }
        let measure = (first..start)
            .filter(|ix| inner.measured[*ix].is_none())
            .collect();
        MarkdownWindow {
            heights,
            top: inner.top,
            flow: start..end,
            measure,
        }
    }
}

/// A windowed column: `window.flow` between spacers standing in for the other
/// items, and `window.measure` laid out invisibly beside them. Heights learnt
/// while laying them out are recorded, and drawn from the next frame.
fn windowed_column(
    context: &MarkdownDocumentContext,
    key: (usize, usize),
    window: &MarkdownWindow,
    mut render_item: impl FnMut(usize) -> AnyElement,
) -> gpui::Div {
    let heights = &window.heights;
    let flow = window.flow.clone();
    let leading: Pixels = heights[..flow.start].iter().copied().sum();
    let trailing: Pixels = heights[flow.end..].iter().copied().sum();
    let mut column = div()
        .relative()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .min_w(px(0.0))
        .child(div().flex_none().w_full().h(leading));
    for ix in flow.clone() {
        column = column.child(render_item(ix));
    }
    column = column.child(div().flex_none().w_full().h(trailing));

    let cache = context.layout.clone();
    let view = context.view.clone();
    let record = move |cache: &MarkdownDocumentLayoutCache,
                       items: &[(usize, Bounds<Pixels>)],
                       window: &mut Window| {
        let mut layout = cache.0.borrow_mut();
        if layout.key != key {
            return;
        }
        let mut changed = false;
        for (ix, bounds) in items {
            if layout
                .width
                .is_none_or(|known| (known - bounds.size.width).abs() > px(0.5))
            {
                // Text re-flows at a new width, so every height taken at the
                // old one is stale.
                layout.width = Some(bounds.size.width);
                layout.measured.fill(None);
                changed = true;
            }
            if let Some(slot) = layout.measured.get_mut(*ix)
                && slot.is_none_or(|known| (known - bounds.size.height).abs() > px(0.5))
            {
                *slot = Some(bounds.size.height);
                changed = true;
            }
        }
        drop(layout);
        if changed {
            // `refresh` is a no-op mid-draw; the corrected spacers show next
            // frame.
            match view.clone() {
                Some(view) => {
                    window.on_next_frame(move |_, cx| view.update(cx, |_, cx| cx.notify()))
                }
                None => window.request_animation_frame(),
            }
        }
    };

    if !window.measure.is_empty() {
        let mut layer = div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .flex()
            .flex_col()
            .invisible();
        context.row_boxes.0.paused.set(true);
        for ix in &window.measure {
            layer = layer.child(render_item(*ix));
        }
        context.row_boxes.0.paused.set(false);
        let measured = window.measure.clone();
        let cache = cache.clone();
        let record = record.clone();
        column = column.child(layer.on_children_prepainted(move |bounds, window, _cx| {
            let items: Vec<_> = measured.iter().copied().zip(bounds).collect();
            record(&cache, &items, window);
        }));
    }

    let scroll = context.scroll.clone();
    let revealing = context.reveal.pending().is_some();
    column.on_children_prepainted(move |bounds, window, _cx| {
        let Some((spacer, rest)) = bounds.split_first() else {
            return;
        };
        {
            let mut layout = cache.0.borrow_mut();
            if layout.key != key {
                return;
            }
            // A reveal in this frame has already moved the offset past the
            // one this layout was placed at; the column itself has not moved.
            if let Some(scroll) = scroll.as_ref()
                && !revealing
            {
                layout.top = spacer.top() - scroll.bounds().top() - scroll.offset().y;
            }
            layout.window_height = window.viewport_size().height;
        }
        let items: Vec<_> = flow.clone().zip(rest.iter().copied()).collect();
        record(&cache, &items, window);
    })
}

/// A block's height before it has been drawn: its lines at the width the
/// column last had. Only the spacers and the scroll range rely on it, and
/// drawing the block replaces it.
fn estimated_block_height(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    context: &MarkdownDocumentContext,
    width: Option<Pixels>,
) -> Pixels {
    let font = f32::from(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context));
    let line = font * 1.5;
    let width = width.map_or(900.0, f32::from).max(120.0);
    let lines = |text: &str, indent: f32| {
        let per_line = ((width - indent) / (font * 0.55)).max(20.0);
        (text.chars().count() as f32 / per_line).ceil().max(1.0)
    };
    let rows = RowRun::new(document, block.row_range());
    let height = match block {
        MarkdownBlock::Heading { level, row_ix } => {
            let size = match level {
                1 => 22.0,
                2 => 18.0,
                3 => 16.0,
                4 => 14.5,
                _ => MARKDOWN_PREVIEW_BASE_FONT_PX,
            };
            let size = f32::from(scaled(size, context));
            let text = document
                .rows
                .get(*row_ix)
                .map_or("", |row| row.text.as_ref());
            lines(text, 0.0) * size * 1.3 + if *level <= 2 { 5.0 } else { 0.0 }
        }
        MarkdownBlock::Paragraph(row_ix) => {
            let text = document
                .rows
                .get(*row_ix)
                .map_or("", |row| row.text.as_ref());
            lines(text, 0.0) * line
        }
        MarkdownBlock::List(_) | MarkdownBlock::Blockquote(_) => rows
            .iter()
            .map(|(_, row)| lines(&row.text, 40.0 * f32::from(row.indent_level)) * line)
            .sum(),
        MarkdownBlock::Code(_) => {
            rows.iter().count() as f32 * line
                + 2.0 * f32::from(scaled(CODE_BLOCK_PAD_Y_PX, context))
        }
        MarkdownBlock::Table(_) => {
            rows.iter().count() as f32
                * (line + 2.0 * f32::from(scaled(TABLE_CELL_PAD_Y_PX, context)))
        }
        // The room its skeleton holds until the picture decodes.
        MarkdownBlock::Image(_) => rows
            .first()
            .and_then(|(_, row)| row.image.as_ref())
            .map_or(0.0, |image| {
                f32::from(scaled(image.reserved_height_px() as f32, context))
            }),
        MarkdownBlock::ThematicBreak(_) => 1.0,
    };
    px(height)
}

/// The gap a column opens above `block` when another block precedes it.
fn gap_above(block: &MarkdownBlock, context: &MarkdownDocumentContext) -> Pixels {
    scaled(
        if matches!(block, MarkdownBlock::Heading { .. }) {
            HEADING_GAP_PX
        } else {
            BLOCK_GAP_PX
        },
        context,
    )
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

/// The rows and gaps a frame laid out, recorded as they are placed: one set of
/// pointer listeners on the document finds the box under the pointer here,
/// instead of every box carrying listeners of its own.
#[derive(Clone, Default)]
pub(in crate::view) struct MarkdownRowBoxes(Rc<MarkdownRowBoxesState>);

#[derive(Default)]
struct MarkdownRowBoxesState {
    boxes: std::cell::RefCell<Vec<(Bounds<Pixels>, MarkdownBoxHit)>>,
    /// Set while the invisible measuring layer is built: its boxes lie under
    /// the visible ones and must not answer the pointer.
    paused: Cell<bool>,
}

/// What a recorded box stands for, and in which column.
#[derive(Clone, Copy)]
struct MarkdownBoxHit {
    kind: MarkdownBoxKind,
    region: DiffTextRegion,
}

#[derive(Clone, Copy)]
enum MarkdownBoxKind {
    /// A row's own box: a press selects in it, and its links follow.
    Row(usize),
    /// Room beside a row — a code block's padding — that selects as that row.
    RowPadding(usize),
    /// The gap before the block that starts at this row.
    Gap(usize),
}

impl MarkdownRowBoxes {
    fn hit_at(&self, position: Point<Pixels>) -> Option<MarkdownBoxHit> {
        self.0
            .boxes
            .borrow()
            .iter()
            .rev()
            .find(|(bounds, _)| bounds.contains(&position))
            .map(|(_, hit)| *hit)
    }
}

/// `child`, recorded as `kind` when the document takes pointer input.
fn recorded(
    child: impl IntoElement,
    kind: MarkdownBoxKind,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    if context.view.is_none() || context.row_boxes.0.paused.get() {
        return child.into_any_element();
    }
    RecordedBox {
        hit: MarkdownBoxHit {
            kind,
            region: context.text_region,
        },
        boxes: context.row_boxes.clone(),
        child: child.into_any_element(),
    }
    .into_any_element()
}

/// A box that records where it was placed; layout and paint are its child's.
struct RecordedBox {
    hit: MarkdownBoxHit,
    boxes: MarkdownRowBoxes,
    child: AnyElement,
}

impl IntoElement for RecordedBox {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RecordedBox {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.boxes.0.boxes.borrow_mut().push((bounds, self.hit));
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

/// The pointer listeners of every row and gap in a document, once, on the
/// column that holds them. Each press and move finds its box among those this
/// frame recorded and does what that box's own listeners used to.
fn document_pointer_listeners(column: gpui::Div, context: &MarkdownDocumentContext) -> AnyElement {
    let Some(view) = context.view.clone() else {
        return column.into_any_element();
    };
    let boxes = context.row_boxes.clone();
    let focus = |view: &Entity<MainPaneView>, window: &mut Window, cx: &mut App| {
        let focus = view.read(cx).diff_panel_focus_handle.clone();
        window.focus(&focus, cx);
    };
    column
        .id("markdown_preview_document")
        .on_mouse_move({
            let (view, boxes) = (view.clone(), boxes.clone());
            move |event, _window, cx| {
                let hit = boxes.hit_at(event.position);
                view.update(cx, |this, cx| match hit {
                    Some(MarkdownBoxHit {
                        kind: MarkdownBoxKind::Row(row_ix),
                        region,
                    }) => this.update_markdown_preview_link_hover(
                        row_ix,
                        region,
                        event.position,
                        event.pressed_button.is_some(),
                        cx,
                    ),
                    _ => this.clear_markdown_preview_link_hover(cx),
                });
            }
        })
        .on_hover({
            let view = view.clone();
            move |hovered, _window, cx| {
                if !*hovered {
                    view.update(cx, |this, cx| this.clear_markdown_preview_link_hover(cx));
                }
            }
        })
        .on_mouse_down(gpui::MouseButton::Left, {
            let (view, boxes) = (view.clone(), boxes.clone());
            move |event, window, cx| {
                let Some(MarkdownBoxHit { kind, region }) = boxes.hit_at(event.position) else {
                    return;
                };
                // Padding and gaps own their press outright; a row's press
                // still reaches the panel.
                if !matches!(kind, MarkdownBoxKind::Row(_)) {
                    crate::press_gesture::claim_press(cx);
                    cx.stop_propagation();
                }
                focus(&view, window, cx);
                view.update(cx, |this, cx| {
                    match kind {
                        MarkdownBoxKind::Row(row_ix) | MarkdownBoxKind::RowPadding(row_ix) => this
                            .handle_markdown_preview_row_mouse_down(
                                row_ix,
                                region,
                                event.position,
                                event.click_count,
                                window,
                                cx,
                            ),
                        MarkdownBoxKind::Gap(next_row_ix) => this
                            .handle_diff_text_document_gap_mouse_down(
                                next_row_ix,
                                region,
                                event.position,
                                window,
                                cx,
                            ),
                    }
                    cx.notify();
                });
            }
        })
        .on_pointer_click(gpui::MouseButton::Left, {
            let (view, boxes) = (view.clone(), boxes.clone());
            move |event, window, cx| {
                let Some(MarkdownBoxHit {
                    kind: MarkdownBoxKind::Row(row_ix),
                    region,
                }) = boxes.hit_at(event.position)
                else {
                    return;
                };
                view.update(cx, |this, cx| {
                    this.handle_markdown_preview_link_click(
                        row_ix,
                        region,
                        event.position,
                        event.click_count,
                        event.modifiers.secondary(),
                        window,
                        cx,
                    );
                    cx.notify();
                });
            }
        })
        .on_pointer_click(gpui::MouseButton::Right, move |event, window, cx| {
            let Some(MarkdownBoxHit { kind, region }) = boxes.hit_at(event.position) else {
                return;
            };
            let row_ix = match kind {
                MarkdownBoxKind::Row(row_ix) => row_ix,
                MarkdownBoxKind::RowPadding(row_ix) | MarkdownBoxKind::Gap(row_ix) => {
                    crate::press_gesture::claim_press(cx);
                    cx.stop_propagation();
                    focus(&view, window, cx);
                    row_ix
                }
            };
            view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu(row_ix, region, event.position, window, cx);
                cx.notify();
            });
        })
        .into_any_element()
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
    let column = document_pointer_listeners(
        render_windowed_block_column(document, blocks, context),
        context,
    );

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
    let divider = left.theme.colors.stroke.default;

    // Bands are what a split frame builds or skips: a band never cuts a block
    // on either side, and its taller side sets its height.
    let key = (std::ptr::from_ref(diff) as usize, diff.bands.len());
    let band_gap = |band: &crate::view::markdown_preview::MarkdownDiffBand| {
        diff.old_blocks[band.old_blocks.clone()]
            .iter()
            .chain(&diff.new_blocks[band.new_blocks.clone()])
            .next()
            .map_or(px(0.0), |block| gap_above(block, left))
    };
    let side_height =
        |document: &MarkdownPreviewDocument, blocks: &[MarkdownBlock], width: Option<Pixels>| {
            blocks
                .iter()
                .enumerate()
                .map(|(ix, block)| {
                    let gap = if ix > 0 {
                        gap_above(block, left)
                    } else {
                        px(0.0)
                    };
                    gap + estimated_block_height(document, block, left, width.map(|w| w / 2.0))
                })
                .sum::<Pixels>()
        };
    let target = left
        .reveal
        .pending()
        .and_then(|row| diff.bands.iter().position(|band| band.rows.contains(&row)));
    let window = left.layout.plan(
        key,
        diff.bands.len(),
        left.scroll.as_ref(),
        target,
        |band_ix, width| {
            let band = &diff.bands[band_ix];
            let gap = if band_ix > 0 { band_gap(band) } else { px(0.0) };
            gap + side_height(&diff.old, &diff.old_blocks[band.old_blocks.clone()], width).max(
                side_height(&diff.new, &diff.new_blocks[band.new_blocks.clone()], width),
            )
        },
    );
    if let Some(extents) = left.change_extents.as_ref() {
        let mut top = window.top;
        for (band, height) in diff.bands.iter().zip(&window.heights) {
            let flag = [
                (&diff.old, &diff.old_blocks[band.old_blocks.clone()]),
                (&diff.new, &diff.new_blocks[band.new_blocks.clone()]),
            ]
            .into_iter()
            .flat_map(|(document, blocks)| {
                blocks.iter().flat_map(move |block| {
                    document.rows.get(block.row_range()).into_iter().flatten()
                })
            })
            .fold(0u8, |flag, row| {
                flag | crate::view::markdown_preview::scrollbar_flag_for_change_hint(
                    row.change_hint,
                )
            });
            if flag != 0 {
                extents.record(f32::from(top), f32::from(top + *height), flag);
            }
            top += *height;
        }
    }
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        diff.old.rows.len().max(diff.new.rows.len()),
        diff.bands[window.flow.clone()]
            .iter()
            .map(|band| band.rows.len())
            .sum(),
    );

    let render_band = |band_ix: usize| {
        let band = &diff.bands[band_ix];
        let old_blocks = &diff.old_blocks[band.old_blocks.clone()];
        let new_blocks = &diff.new_blocks[band.new_blocks.clone()];
        let mut item = div().w_full().min_w(px(0.0));
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
            let gap_side = |context: &MarkdownDocumentContext| {
                let selector_ix = band_ix * 2 + usize::from(context.text_region.order());
                split_side(context).child(render_block_gap(
                    move || format!("markdown_preview_block_gap_{selector_ix}"),
                    band.rows.start,
                    gap,
                    context,
                ))
            };
            item = item.child(
                div()
                    .id(("markdown_diff_band_gap", band_ix))
                    .flex()
                    .w_full()
                    .child(gap_side(left))
                    .child(div().w(px(1.0)).flex_none().bg(divider))
                    .child(gap_side(right)),
            );
        }
        let side = |document: &MarkdownPreviewDocument,
                    blocks: &[MarkdownBlock],
                    all_blocks: &[MarkdownBlock],
                    notice: &'static str,
                    context: &MarkdownDocumentContext| {
            let side = split_side(context);
            // A side with no block at all says why.
            if band_ix == 0 && all_blocks.is_empty() {
                return side.child(empty_split_side(notice, context));
            }
            side.child(render_block_column(
                document,
                blocks,
                band.rows.start,
                context,
            ))
        };
        item.child(
            div()
                .id(("markdown_diff_band", band_ix))
                .debug_selector(move || format!("markdown_diff_band_{band_ix}"))
                .flex()
                .items_stretch()
                .w_full()
                .child(side(
                    &diff.old,
                    old_blocks,
                    &diff.old_blocks,
                    diff.old_empty_notice(),
                    left,
                ))
                .child(div().w(px(1.0)).flex_none().bg(divider))
                .child(side(
                    &diff.new,
                    new_blocks,
                    &diff.new_blocks,
                    diff.new_empty_notice(),
                    right,
                )),
        )
        .into_any_element()
    };
    let column = document_pointer_listeners(
        windowed_column(left, key, &window, render_band)
            .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, left))
            .text_color(left.theme.colors.foreground.primary),
        left,
    );

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

/// The notice standing in for a side with no block at all.
fn empty_split_side(notice: &'static str, context: &MarkdownDocumentContext) -> gpui::Div {
    let region = context.text_region;
    div()
        .debug_selector(move || format!("markdown_diff_empty_side_{region:?}"))
        .w_full()
        .flex()
        .justify_center()
        .py(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
        .text_size(context.theme.ui_text(14.0))
        .text_color(context.theme.colors.foreground.secondary)
        .child(
            div()
                .debug_selector(move || format!("markdown_diff_side_notice_{region:?}_{notice}"))
                .child(notice),
        )
}

/// One column of a split band. Both columns render the same row indices, so
/// the column's id is what keeps every element id beneath it distinct.
fn split_side(context: &MarkdownDocumentContext) -> gpui::Stateful<gpui::Div> {
    div()
        .id((
            "markdown_diff_side",
            usize::from(context.text_region.order()),
        ))
        .flex_1()
        .min_w(px(0.0))
}

/// A document's top-level blocks as a windowed column: the blocks near the
/// viewport, each with the gap above it, between spacers for the rest.
fn render_windowed_block_column(
    document: &MarkdownPreviewDocument,
    blocks: &[MarkdownBlock],
    context: &MarkdownDocumentContext,
) -> gpui::Div {
    let key = (std::ptr::from_ref(document) as usize, blocks.len());
    let gap = |ix: usize| {
        if ix > 0 {
            gap_above(&blocks[ix], context)
        } else {
            px(0.0)
        }
    };
    // A reveal waits on the block holding its row — or, for alignment padding,
    // the block after it.
    let target = context.reveal.pending().map(|row| {
        blocks
            .iter()
            .position(|block| row < block.row_range().end)
            .unwrap_or(blocks.len().saturating_sub(1))
    });
    let window = context.layout.plan(
        key,
        blocks.len(),
        context.scroll.as_ref(),
        target,
        |ix, width| gap(ix) + estimated_block_height(document, &blocks[ix], context, width),
    );

    // Scrollbar markers need every changed block, drawn or not, so they are
    // placed from the heights rather than from layout.
    if let Some(extents) = context.change_extents.as_ref() {
        let mut top = window.top;
        for (ix, (block, height)) in blocks.iter().zip(&window.heights).enumerate() {
            let flag =
                RowRun::new(document, block.row_range())
                    .iter()
                    .fold(0u8, |flag, (_, row)| {
                        flag | crate::view::markdown_preview::scrollbar_flag_for_change_hint(
                            row.change_hint,
                        )
                    });
            if flag != 0 {
                extents.record(f32::from(top + gap(ix)), f32::from(top + *height), flag);
            }
            top += *height;
        }
    }
    perf::record_row_batch(
        ViewPerfRenderLane::MarkdownPreview,
        document.rows.len(),
        blocks[window.flow.clone()]
            .iter()
            .map(|block| block.row_range().len())
            .sum(),
    );

    windowed_column(context, key, &window, |ix| {
        let rows_from = ix
            .checked_sub(1)
            .map_or(0, |previous| blocks[previous].row_range().end);
        let mut item = div().w_full().min_w(px(0.0));
        if ix > 0 {
            item = item.child(render_column_gap(blocks, ix, context, false));
        }
        item.child(render_column_block(
            document,
            &blocks[ix],
            rows_from,
            context,
            BlockNesting::default(),
        ))
        .into_any_element()
    })
    .pl(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
    .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context))
    .text_color(context.theme.colors.foreground.primary)
}

/// Where a column of blocks sits in the document: inside how many quotes, and
/// at which list indent its container already drew.
#[derive(Clone, Copy, Default)]
struct BlockNesting {
    quote_depth: u8,
    indent_base: u8,
}

/// Blocks stacked with the gaps between them. `first_row` is where the rows
/// this column shows begin.
fn render_block_column(
    document: &MarkdownPreviewDocument,
    blocks: &[MarkdownBlock],
    first_row: usize,
    context: &MarkdownDocumentContext,
) -> gpui::Div {
    render_nested_block_column(
        document,
        blocks,
        first_row,
        context,
        BlockNesting::default(),
    )
    .pl(scaled(MARKDOWN_PREVIEW_CONTENT_PAD_X_PX, context))
    .text_size(scaled(MARKDOWN_PREVIEW_BASE_FONT_PX, context))
    .text_color(context.theme.colors.foreground.primary)
}

/// The blocks of a column at any depth, all of them: a split band's side, or
/// a quote's contents inside its bar. Change marks belong to the top-level
/// blocks only.
fn render_nested_block_column(
    document: &MarkdownPreviewDocument,
    blocks: &[MarkdownBlock],
    first_row: usize,
    context: &MarkdownDocumentContext,
    nesting: BlockNesting,
) -> gpui::Div {
    let mut column = div().flex().flex_col().flex_1().min_w(px(0.0));
    let nested = nesting.quote_depth > 0;
    for (ix, block) in blocks.iter().enumerate() {
        if ix > 0 {
            column = column.child(render_column_gap(blocks, ix, context, nested));
        }
        let rows_from = ix
            .checked_sub(1)
            .map_or(first_row, |previous| blocks[previous].row_range().end);
        column = column.child(render_column_block(
            document, block, rows_from, context, nesting,
        ));
    }
    column
}

/// The interactive gap above block `ix` of a column.
fn render_column_gap(
    blocks: &[MarkdownBlock],
    ix: usize,
    context: &MarkdownDocumentContext,
    nested: bool,
) -> AnyElement {
    let block = &blocks[ix];
    let gap = if matches!(block, MarkdownBlock::Heading { .. }) {
        HEADING_GAP_PX
    } else {
        BLOCK_GAP_PX
    };
    let region = context.text_region;
    let next_row = block.row_range().start;
    render_block_gap(
        move || match (region, nested) {
            (_, true) => format!("markdown_preview_nested_block_gap_{region:?}_{next_row}"),
            (DiffTextRegion::Inline, false) => format!("markdown_preview_block_gap_{ix}"),
            (_, false) => format!("markdown_preview_block_gap_{region:?}_{next_row}"),
        },
        next_row,
        gap,
        context,
    )
}

/// One block of a column, answering a pending reveal when it holds the row
/// and, at the top level, marked down its side when it was wholly added or
/// removed. `rows_from` is where the rows it answers for begin: alignment
/// padding before it belongs to it.
fn render_column_block(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    rows_from: usize,
    context: &MarkdownDocumentContext,
    nesting: BlockNesting,
) -> AnyElement {
    let range = block.row_range();
    let mut rendered = render_block(document, block, context, nesting);
    // A row with no text box of its own — a picture, a rule, a gap — is
    // revealed by bringing its block into view. Alignment padding is left to
    // the other column, which draws the row it stands in for.
    if let Some(row) = context.reveal.pending()
        && (rows_from..range.end).contains(&row)
        && !row_reveals_itself(document, block, row)
        && !document
            .rows
            .get(row)
            .is_some_and(MarkdownPreviewRow::is_alignment_padding)
    {
        rendered = reveal_listener(row, context)
            .w_full()
            .min_w(px(0.0))
            .child(rendered)
            .into_any_element();
    }
    if nesting.quote_depth > 0 {
        return rendered;
    }
    match block_change_bar(document, block, context) {
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
    }
}

/// Whether `row` of `block` draws a text box that answers a reveal itself.
fn row_reveals_itself(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    row: usize,
) -> bool {
    !matches!(
        block,
        MarkdownBlock::Image(_) | MarkdownBlock::ThematicBreak(_)
    ) && block.row_range().contains(&row)
        && document
            .rows
            .get(row)
            .is_some_and(|row| !matches!(row.kind, MarkdownPreviewRowKind::Spacer))
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
    selector: impl FnOnce() -> String,
    next_source_visible_ix: usize,
    gap: f32,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let space = div()
        .debug_selector(selector)
        .flex_none()
        .w_full()
        .h(scaled(gap, context))
        .when(context.view.is_some(), |space| {
            space.cursor(gpui::CursorStyle::IBeam)
        });
    recorded(space, MarkdownBoxKind::Gap(next_source_visible_ix), context)
}

fn render_block(
    document: &MarkdownPreviewDocument,
    block: &MarkdownBlock,
    context: &MarkdownDocumentContext,
    nesting: BlockNesting,
) -> AnyElement {
    let rows = RowRun::new(document, block.row_range());
    // A block inside a list item — code, a table, a quote, a later paragraph —
    // starts at its item's text, not at the document margin. List rows indent
    // themselves.
    let indent = rows
        .first()
        .map(|(_, row)| row.indent_level.saturating_sub(nesting.indent_base))
        .unwrap_or(0);
    let wrapper = div().w_full().min_w(px(0.0)).when(
        indent > 0 && !matches!(block, MarkdownBlock::List(_)),
        |wrapper| wrapper.pl(list_content_indent(indent, context)),
    );

    match block {
        MarkdownBlock::Heading { level, row_ix } => wrapper
            .child(render_heading(*level, *row_ix, document, context))
            .into_any_element(),
        MarkdownBlock::Paragraph(row_ix) => match document.rows.get(*row_ix) {
            // A footnote definition reads as its label and then its text, like
            // a list item: the label hangs where a bullet would.
            Some(row) if row.footnote_label.is_some() => div()
                .w_full()
                .min_w(px(0.0))
                .child(render_marked_row(
                    *row_ix,
                    row,
                    indent.saturating_sub(1),
                    context,
                ))
                .into_any_element(),
            Some(row) => wrapper
                .child(recorded(
                    row_shell(*row_ix, row, context).child(render_row_line(*row_ix, row, context)),
                    MarkdownBoxKind::Row(*row_ix),
                    context,
                ))
                .into_any_element(),
            None => wrapper.into_any_element(),
        },
        MarkdownBlock::List(_) => wrapper
            .child(render_list(rows, context, nesting.indent_base))
            .into_any_element(),
        MarkdownBlock::Blockquote(range) => wrapper
            .child(render_blockquote(document, range.clone(), context, nesting))
            .into_any_element(),
        MarkdownBlock::Code(_) => wrapper.child(render_code(rows, context)).into_any_element(),
        MarkdownBlock::Table(_) => wrapper
            .child(render_table(rows, context))
            .into_any_element(),
        MarkdownBlock::Image(_) => non_text_block_shell(document, block.row_range(), context)
            .when_some(rows.first(), |wrapper, (row_ix, row)| {
                wrapper.child(render_image(row_ix, row, context))
            })
            .into_any_element(),
        MarkdownBlock::ThematicBreak(row_ix) => {
            let row_ix = *row_ix;
            non_text_block_shell(document, block.row_range(), context)
                .when(indent > 0, |shell| {
                    shell.pl(list_content_indent(indent, context))
                })
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

/// Where the text of a list item `levels` deep starts: its indent, then the
/// marker column and the gap after it.
fn list_content_indent(levels: u8, context: &MarkdownDocumentContext) -> Pixels {
    scaled(
        f32::from(levels) * MARKDOWN_PREVIEW_INDENT_STEP_PX
            + MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX
            + MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX,
        context,
    )
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

/// The container a row's text lives in. Recorded with its row's index (see
/// [`recorded`]), so pointer events resolve to the same `(row, region)` pair
/// selection and copy use.
fn row_shell(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    context: &MarkdownDocumentContext,
) -> gpui::Div {
    let shell = reveal_listener(row_ix, context)
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

    row_cursor(shell, row_ix, context)
}

/// A text row's pointer: over a link a click follows, the hand. Its presses
/// go to the document's listeners.
fn row_cursor(shell: gpui::Div, row_ix: usize, context: &MarkdownDocumentContext) -> gpui::Div {
    if context.view.is_none() {
        return shell;
    }
    shell.cursor(crate::view::rows::MarkdownPreviewHoveredLink::cursor(
        context.hovered_link.as_ref(),
        context.text_region,
        row_ix,
    ))
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
        line = line.child(render_inline_image(row_ix, inline, context));
    }
    // A row of nothing but pictures still has to paint its (empty) text: that
    // element is what registers the row's hit-test box, and without one a drag
    // across the row finds no target and the selection skips over it.
    if !row.text.is_empty() || context.view.is_some() {
        line = line.child(render_row_text(row_ix, row, context));
    }
    for inline in trailing() {
        line = line.child(render_inline_image(row_ix, inline, context));
    }
    line.into_any_element()
}

fn render_inline_image(
    row_ix: usize,
    inline: &MarkdownInlineImage,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let image = div()
        .flex_none()
        .child(crate::view::rows::markdown_preview_inline_image(
            inline,
            context.theme,
            context.ui_scale_percent,
            pictures(context),
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
    let region = context.text_region;
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
                    let follow = event.modifiers.secondary();
                    view.update(cx, |this, cx| {
                        this.open_markdown_preview_link_menu(
                            region,
                            row_ix,
                            url,
                            load_remote_image_url,
                            bounds,
                            position,
                            follow,
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
    let code_ranges = row
        .inline_spans
        .iter()
        .filter(|span| span.style == MarkdownInlineStyle::Code)
        .map(|span| span.byte_range.clone());

    let Some(view) = context.view.clone() else {
        // Without a view there is no flow text to set fonts per run, so the
        // whole line takes the editor font if any of it is code.
        if code_ranges.clone().next().is_some() {
            text = text.font_family(context.editor_font_family.clone());
        }
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
    .child(
        MarkdownFlowText::new(
            view,
            row_ix,
            context.text_region,
            row.text.clone(),
            styled.text.clone(),
            Arc::clone(&styled.highlights),
        )
        // Inline code is set in the editor font; the prose around it is not.
        .font_family_ranges(code_ranges, context.editor_font_family.clone()),
    )
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
    recorded(heading, MarkdownBoxKind::Row(row_ix), context)
}

fn render_list(rows: RowRun<'_>, context: &MarkdownDocumentContext, indent_base: u8) -> AnyElement {
    let mut list = div().flex().flex_col().w_full().min_w(px(0.0));
    for (row_ix, row) in rows.iter() {
        list = list.child(render_marked_row(
            row_ix,
            row,
            row.indent_level.saturating_sub(indent_base),
            context,
        ));
    }
    list.into_any_element()
}

/// A row that hangs its text off a marker: a list item's bullet, number, or
/// checkbox, or a footnote's label. A later row of the same item keeps the
/// marker column empty so its text lines up.
fn render_marked_row(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    indent: u8,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let marker_slot = div()
        .debug_selector(move || format!("markdown_preview_marker_{row_ix}"))
        .flex_none()
        .min_w(scaled(MARKDOWN_PREVIEW_LIST_MARKER_MIN_WIDTH_PX, context))
        .mr(scaled(MARKDOWN_PREVIEW_LIST_MARKER_GAP_PX, context))
        .text_color(context.theme.colors.foreground.secondary);
    let marker_slot = match row.task {
        // A task item's box stands where its bullet would; a numbered one
        // keeps its number in front of the box.
        Some(task) => marker_slot
            .flex()
            .items_center()
            .gap(scaled(4.0, context))
            .when(
                matches!(
                    row.kind,
                    MarkdownPreviewRowKind::ListItem { number: Some(_) }
                ),
                |slot| slot.children(crate::view::rows::markdown_preview_marker_label(row)),
            )
            .child(render_task_checkbox(row_ix, task, context)),
        None => marker_slot.child(
            crate::view::rows::markdown_preview_marker_label(row)
                .unwrap_or_else(|| SharedString::new_static("")),
        ),
    };
    recorded(
        row_shell(row_ix, row, context)
            .pl(scaled(
                f32::from(indent) * MARKDOWN_PREVIEW_INDENT_STEP_PX,
                context,
            ))
            .child(marker_slot)
            .child(render_row_line(row_ix, row, context)),
        MarkdownBoxKind::Row(row_ix),
        context,
    )
}

/// A task-list checkbox; clicking it toggles the item in the file when the
/// preview shows the working-tree copy.
fn render_task_checkbox(
    row_ix: usize,
    task: MarkdownTaskMarker,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let theme = context.theme;
    let checkbox = crate::view::rows::markdown_preview_task_checkbox(
        theme,
        task.checked,
        scaled(14.0, context),
    )
    .id(("markdown-task-checkbox", row_ix))
    .debug_selector(move || format!("markdown_task_checkbox_{row_ix}"));
    let (true, Some(view)) = (context.tasks_editable, context.view.clone()) else {
        return checkbox.into_any_element();
    };
    let region = context.text_region;
    // A nested control: its press is its own, so the row under it does not
    // start a selection, and it toggles on a completed click — a press dragged
    // off the box writes nothing.
    checkbox
        .cursor_pointer()
        .control_interaction(
            crate::kit::interaction::InteractionStyle::new(theme).hover(
                gpui::StyleRefinement::default().border_color(theme.colors.accent.foreground),
            ),
            crate::kit::interaction::InteractionState::default(),
        )
        .on_activate(
            false,
            crate::kit::interaction::ControlActivation::Nested,
            move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.toggle_markdown_preview_task(region, task, cx);
                });
            },
        )
        .into_any_element()
}

fn render_blockquote(
    document: &MarkdownPreviewDocument,
    range: Range<usize>,
    context: &MarkdownDocumentContext,
    nesting: BlockNesting,
) -> AnyElement {
    // The block's kind is its first row's: blocks are split wherever an alert
    // starts, so every row in this one shares it.
    let rows = RowRun::new(document, range.clone());
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

    // What the quote holds — lines, lists, code, tables, deeper quotes — is a
    // column of blocks of its own, drawn inside the bar.
    let inner = BlockNesting {
        quote_depth: nesting.quote_depth.saturating_add(1),
        indent_base: first.map_or(nesting.indent_base, |(_, row)| row.indent_level),
    };
    let blocks = crate::view::markdown_preview::markdown_blocks_in(
        document,
        range.clone(),
        inner.quote_depth,
    );
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
    body = body.child(render_nested_block_column(
        document,
        &blocks,
        range.start,
        context,
        inner,
    ));

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
        body = body.child(recorded(
            row_shell(row_ix, row, context).child(render_row_line(row_ix, row, context)),
            MarkdownBoxKind::Row(row_ix),
            context,
        ));
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
        .debug_selector(move || format!("markdown_preview_code_padding_{edge}_{row_ix}"))
        .flex_none()
        .w_full()
        .h(scaled(CODE_BLOCK_PAD_Y_PX, context))
        .when(context.view.is_some(), |padding| {
            padding.cursor(gpui::CursorStyle::IBeam)
        });
    recorded(padding, MarkdownBoxKind::RowPadding(row_ix), context)
}

/// A table as a grid: columns sized to their content that shrink and wrap
/// when the pane is narrow, dividers between cells, a header band, and
/// alternating rows.
///
/// An inline diff interleaves the old table's rows with the new one's, so a
/// block can mix tables of different widths: the widest sets the grid, and a
/// narrower row is filled out with empty cells.
fn render_table(rows: RowRun<'_>, context: &MarkdownDocumentContext) -> AnyElement {
    let first_row_ix = rows.first().map(|(row_ix, _)| row_ix).unwrap_or_default();
    let column_count = rows
        .iter()
        .filter_map(|(_, row)| row.table.as_ref())
        .map(|cells| cells.cells.len().max(cells.table.alignments.len()))
        .max()
        .unwrap_or(0);
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
        let cell_shell = |shell: gpui::Div, column: usize| {
            shell
                .px(scaled(TABLE_CELL_PAD_X_PX, context))
                .py(scaled(TABLE_CELL_PAD_Y_PX, context))
                .when(column > 0, |cell| cell.border_l_1())
                .when(grid_row > 0, |cell| cell.border_t_1())
                .border_color(divider)
                .when_some(background, |cell, background| cell.bg(background))
        };
        // Styled once for the row; each cell paints its slice.
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
        for (column, range) in cells.cells.iter().enumerate() {
            let align = if is_header {
                MarkdownTableAlign::Center
            } else {
                cells
                    .table
                    .alignments
                    .get(column)
                    .copied()
                    .unwrap_or_default()
            };
            // The search reveal needs one box per row; the first cell stands in.
            let shell = if column == 0 {
                reveal_listener(row_ix, context)
            } else {
                div()
            };
            let cell = cell_shell(shell, column)
                .debug_selector(move || format!("markdown_preview_cell_box_{row_ix}_{column}"))
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
                    styled.as_ref(),
                    context,
                ));
            grid = grid.child(recorded(
                row_cursor(cell, row_ix, context),
                MarkdownBoxKind::Row(row_ix),
                context,
            ));
        }
        for column in cells.cells.len()..column_count {
            grid = grid.child(cell_shell(div(), column));
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

/// One table cell's text: its slice of the row's styled text, selectable in
/// row coordinates.
fn render_cell_text(
    row_ix: usize,
    row: &MarkdownPreviewRow,
    column: usize,
    range: std::ops::Range<usize>,
    row_styled: &CachedDiffStyledText,
    context: &MarkdownDocumentContext,
) -> AnyElement {
    let styled = super::diff_text::slice_cached_diff_styled_text(
        row_styled,
        super::history::markdown_preview_expanded_slice_range(
            row.text.as_ref(),
            row_styled.text.len(),
            &range,
        ),
    );
    // No percentage width: while the grid sizes its columns a `w_full` box
    // measures its text at width 0, and `gpui` keeps that one-glyph-per-line
    // size for every later probe. The column stretches the box anyway.
    let text = div().min_w(px(0.0));
    // The cell's inline code, in the cell's own coordinates.
    let code_ranges = row
        .inline_spans
        .iter()
        .filter(|span| span.style == MarkdownInlineStyle::Code)
        .filter_map(|span| {
            let start = span.byte_range.start.max(range.start);
            let end = span.byte_range.end.min(range.end);
            (start < end).then(|| (start - range.start)..(end - range.start))
        })
        .collect::<Vec<_>>();
    let Some(view) = context.view.clone() else {
        let text = text.when(!code_ranges.is_empty(), |text| {
            text.font_family(context.editor_font_family.clone())
        });
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
        .cell(range, row.text.len())
        .font_family_ranges(code_ranges, context.editor_font_family.clone()),
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

    /// Frame-build cost of table-heavy previews. Run with
    /// `--ignored --nocapture`; compare raw output before and after a change.
    #[test]
    #[ignore]
    fn timing_table_preview_frames() {
        use crate::view::markdown_preview::parse_markdown;
        use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};

        let theme = AppTheme::gitcomet_dark();
        let mut source = String::from(
            "| c0 | c1 | c2 | c3 | c4 | c5 | c6 | c7 | c8 | c9 |\n\
             |---|---|---|---|---|---|---|---|---|---|\n",
        );
        for row in 0..300 {
            source.push_str(&format!(
                "| **r{row}** | `code {row}` | [link](https://x.invalid/{row}) | a{row} | b{row} \
                 | c{row} | d{row} | e{row} | f{row} | needle {row} |\n"
            ));
        }
        let query = crate::view::rows::MarkdownPreviewQuery {
            matcher: Arc::new(DiffSearchMatcher::new(
                "needle",
                DiffSearchOptions::default(),
            )),
            current_row: Some(5),
        };
        const FRAMES: u32 = 100;

        // With no scroll handle to window against, every row is built.
        let document = Arc::new(parse_markdown(&source).expect("parses"));
        for (label, query) in [("idle", None), ("search", Some(query))] {
            let context = MarkdownDocumentContext {
                theme,
                ui_scale_percent: 100,
                editor_font_family: "monospace".into(),
                image_root: None,
                remote_image_access: Default::default(),
                picture_sizes: Default::default(),
                drawn_pictures: None,
                row_boxes: Default::default(),
                block_scrolls: Default::default(),
                blocks: Default::default(),
                view: None,
                text_region: DiffTextRegion::Inline,
                change_bar_color: None,
                query,
                reveal: Default::default(),
                scroll: None,
                hovered_link: None,
                change_extents: None,
                layout: Default::default(),
                tasks_editable: false,
            };
            let start = std::time::Instant::now();
            for _ in 0..FRAMES {
                drop(render_markdown_document(&document, &context));
            }
            eprintln!(
                "timing flowing_table[{label}] {:?}/frame",
                start.elapsed() / FRAMES
            );
        }
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
        pictures(context),
    )
}

fn pictures(context: &MarkdownDocumentContext) -> crate::view::rows::MarkdownPictureContext<'_> {
    crate::view::rows::MarkdownPictureContext {
        image_root: context.image_root.as_ref(),
        picture_sizes: &context.picture_sizes,
        remote_image_access: &context.remote_image_access,
        drawn: context.drawn_pictures.as_ref(),
    }
}

fn scaled(value: f32, context: &MarkdownDocumentContext) -> Pixels {
    context.theme.markdown_px(value, context.ui_scale_percent)
}
