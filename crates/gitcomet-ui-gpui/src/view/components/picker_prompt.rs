use super::control_height_md;
use crate::kit::{Scrollbar, ScrollbarAxis, TextInput};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use crate::view::restrict_scroll_to_vertical_axis;
use crate::view::tooltip_host::TooltipHost;
use gpui::prelude::*;
use gpui::{
    ClickEvent, CursorStyle, Div, Entity, FontWeight, HighlightStyle, MouseButton, MouseDownEvent,
    MouseMoveEvent, Pixels, ScrollHandle, SharedString, WeakEntity, Window, div, px,
};
use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use super::{TextTruncationProfile, TruncatedText, TruncatedTextFlex};

pub struct PickerPrompt {
    query_input: Entity<TextInput>,
    scroll_handle: ScrollHandle,
    items: Rc<[PickerPromptItem]>,
    /// Layout the caller already resolved for these items. Pickers that memoise
    /// their rows pass it so `render` does not filter and sort them a second
    /// time on every frame; the rest let `render` resolve it.
    layout: Option<Rc<PickerPromptLayout>>,
    /// Renders only the rows the viewport can show once the list is long enough.
    /// Opt-in, because a picker whose rows can be left unbuilt must scroll its
    /// keyboard selection into view through [`PickerPromptGeometry`] rather than
    /// through `ScrollHandle::scroll_to_item`, which needs the row to exist.
    empty_text: SharedString,
    max_height: gpui::Pixels,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    selected_index: Option<usize>,
    marked_index: Option<usize>,
    leading_icon: Option<&'static str>,
    selected_hint: Option<SharedString>,
    accent_selection: bool,
    attached_list_surface: bool,
    padded_query_row: bool,
    select_on_mouse_down: bool,
    query_row_trailing: Option<gpui::AnyElement>,
    list_override: Option<gpui::AnyElement>,
    remove_tooltip: Option<SharedString>,
    on_toggle_section: Option<Rc<OnToggleSectionFn>>,
    on_context_menu: Option<Rc<OnContextMenuFn>>,
}

/// Which row a right-click landed on, in both index spaces, and where the
/// pointer was — a menu needs the pre-filter index to name the item, the
/// display index to highlight the row, and the position to anchor itself.
#[derive(Clone, Debug)]
pub struct PickerPromptContextMenuEvent {
    pub original_index: usize,
    pub display_index: usize,
    pub position: gpui::Point<Pixels>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPromptItem {
    display_text: SharedString,
    match_text: SharedString,
    parts: Vec<PickerPromptItemPart>,
    /// Supporting detail, rendered on a second, quieter line below `parts`.
    /// Empty for the single-line rows most pickers use.
    secondary: Vec<PickerPromptItemPart>,
    icon: Option<&'static str>,
    repository_initials: Option<SharedString>,
    section: Option<SharedString>,
    removable: bool,
}

/// Row and header metrics, taken from Zed's title-bar menus so the two read the
/// same: a row's fill is inset from the popover edge rather than spanning it, and
/// the text sits a further 6px inside that fill (10px from the edge in total).
/// See `zed/crates/ui/src/components/list/list_item.rs` (`inset` + `Sparse`).
const LIST_PAD_PX: f32 = 4.0;
const ROW_PAD_X_PX: f32 = 6.0;
/// Air above and below a row's text (Zed's `ListItemSpacing::Sparse` → `py_1`).
///
/// A row is otherwise exactly as tall as its own text — each line's box is its
/// font size times the line height — so a row with a detail line grows by one
/// line box and the whole menu keeps its proportions at any UI scale. Pinning the
/// line boxes to scaled pixel heights instead made the lines crowd, because the
/// rem-based font sizes did not scale with them.
const ROW_PAD_Y_PX: f32 = 4.0;
/// Gap between a row's leading icon and its text (Zed's `gap_2p5`).
const ROW_ICON_GAP_PX: f32 = 10.0;
/// Zed's rows are `rounded_sm` (`rems(0.25)`), tighter than `radii.row`, which
/// stays as it is for the sidebar and history rows.
const ROW_CORNER_PX: f32 = 4.0;
/// Zed's picker query row is `h_9`.
const QUERY_ROW_HEIGHT_PX: f32 = 36.0;
/// Zed's `ListSubHeader` label box.
const SECTION_HEADER_HEIGHT_PX: f32 = 20.0;
/// Air a section header adds around its label box: `pt_1p5` + `pb_1`.
const SECTION_HEADER_PAD_TOP_PX: f32 = 6.0;
const SECTION_HEADER_PAD_BOTTOM_PX: f32 = 4.0;
/// Every header but the first is introduced by a 1px rule with a margin above it.
const SECTION_HEADER_MARGIN_PX: f32 = 4.0;
/// Type sizes of a row's two lines, in rems — see [`picker_item_label`], which
/// sets these on the lines themselves.
const PRIMARY_TEXT_REMS: f32 = 0.875;
const SECONDARY_TEXT_REMS: f32 = 0.75;
/// GPUI's default text line height (`gpui::phi()`), which nothing in the popover
/// tree overrides. `row_height_is_the_height_rows_actually_paint_at` pins this
/// to what a drawn row really measures, so a future global override cannot
/// silently desynchronise the windowed list's spacers from its rows.
const LINE_HEIGHT_RATIO: f32 = 1.618_034;
/// The list renders every row until its content is this many viewports tall;
/// past that it renders only what can be seen. Short lists — every picker in the
/// app bar one — therefore keep exactly the geometry they had before windowing.
const WINDOWED_LIST_VIEWPORTS: f32 = 2.0;
/// Rows rendered beyond each edge of the viewport, so a scroll of a few pixels
/// does not expose an unrendered row before the next frame.
const WINDOW_OVERDRAW_ROWS: usize = 4;
/// Height the badge pickers cap their row list at. Shared so the panel that
/// renders the list and the keyboard navigation that scrolls it agree on the
/// viewport they are working in.
pub const PICKER_LIST_MAX_HEIGHT_PX: f32 = 300.0;

/// One section header in a resolved layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPromptHeader {
    pub label: SharedString,
    /// True when the section's rows are folded away. Its rows are absent from
    /// [`PickerPromptLayout::item_indices`], so keyboard navigation skips them
    /// without knowing anything about collapse.
    pub collapsed: bool,
    /// Rows this section matched that collapse is hiding, shown as a count on
    /// the header so a folded section still says how much it holds.
    pub hidden_count: usize,
    /// The first header of a list draws no rule above it.
    pub is_first: bool,
}

/// Where a filtered picker list ends up on screen: which items survived the
/// query, in render order, which scroll child each one is (section headers take
/// child slots too, so `scroll_to_item` needs the translated index), and where
/// the query matched inside each one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerPromptLayout {
    pub item_indices: Vec<usize>,
    pub child_indices: Vec<usize>,
    /// Match span, in the item's match-text coordinates, for the row's
    /// highlight. Parallel to `item_indices`; `None` for an empty query.
    pub match_ranges: Vec<Option<Range<usize>>>,
    /// Section headers in render order, each tagged with the display row it is
    /// drawn before. A slot of `item_indices.len()` means the header trails the
    /// last row, which is where a section whose rows are all collapsed away
    /// ends up.
    ///
    /// Sparse — one entry per header, not per row — because these layouts are
    /// cached per query for lists of tens of thousands of rows, where a slot
    /// vector would cost more than the headers it describes.
    ///
    /// Placing headers here rather than rediscovering them while rendering is
    /// what lets a collapsed section keep its header after its rows are gone,
    /// and lets a windowed list start mid-list without replaying the sections
    /// above it.
    pub headers: Vec<(usize, PickerPromptHeader)>,
}

/// Resolve the display layout the same way [`PickerPrompt::render`] does, so
/// keyboard navigation over a filtered list stays in lockstep with the rows the
/// user sees.
///
/// The shorthand for a picker with no sections to fold — one flat list, so there
/// is nothing for a collapsed set to name. Sectioned pickers go through
/// [`picker_prompt_layout_with_collapsed`].
pub fn picker_prompt_layout(items: &[PickerPromptItem], query: &str) -> PickerPromptLayout {
    picker_prompt_layout_with_collapsed(items, query, &BTreeSet::new())
}

/// [`picker_prompt_layout`] with sections folded away: a section named in
/// `collapsed` keeps its header and drops its rows.
///
/// The panel and its keyboard navigation must call this with the same collapsed
/// set, or Enter activates a different row than the one highlighted.
pub fn picker_prompt_layout_with_collapsed(
    items: &[PickerPromptItem],
    query: &str,
    collapsed: &BTreeSet<SharedString>,
) -> PickerPromptLayout {
    let matches = match_items(items, &section_groups(items), query);
    let mut layout = PickerPromptLayout {
        item_indices: Vec::with_capacity(matches.len()),
        child_indices: Vec::with_capacity(matches.len()),
        match_ranges: Vec::with_capacity(matches.len()),
        headers: Vec::new(),
    };
    let mut child_ix = 0usize;
    let mut sections = SectionRun::default();
    for m in &matches {
        let section = items[m.index].section.as_ref();
        let is_collapsed = section.is_some_and(|section| collapsed.contains(section));
        if sections.starts_new_section(section)
            && let Some(label) = section.cloned()
        {
            // A header is drawn before the row this loop has not pushed yet.
            layout.headers.push((
                layout.item_indices.len(),
                PickerPromptHeader {
                    label,
                    collapsed: is_collapsed,
                    hidden_count: 0,
                    is_first: child_ix == 0,
                },
            ));
            child_ix += 1;
        }
        if is_collapsed {
            if let Some((_, header)) = layout.headers.last_mut() {
                header.hidden_count += 1;
            }
            continue;
        }
        layout.item_indices.push(m.index);
        layout.child_indices.push(child_ix);
        layout.match_ranges.push(m.range.clone());
        child_ix += 1;
    }
    layout
}

/// Where every row of a rendered picker list sits vertically.
///
/// Row heights are fully determined by the type sizes and paddings above, so the
/// list can be windowed — rendering only the rows the viewport can show, with
/// spacers standing in for the rest — without measuring anything. Keyboard
/// navigation shares this, because a row outside the window has no element for
/// `ScrollHandle::scroll_to_item` to find.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PickerPromptGeometry {
    /// Top edge of each displayed row, measured from the first row rather than
    /// from the scroll container: the list's own padding is a property of the
    /// container, which the spacers must not repeat. `pad` converts to scroll
    /// coordinates.
    tops: Vec<Pixels>,
    /// Height of each displayed row, excluding any section header above it.
    heights: Vec<Pixels>,
    /// Height of the header(s) that precede a row, or zero.
    header_heights: Vec<Pixels>,
    /// Height of every row and header together.
    rows_height: Pixels,
    /// Height of the headers that follow the last row — collapsed sections with
    /// no rows left to introduce. Counted in `rows_height`, and held separately
    /// so a window that renders them does not also reserve a spacer for them.
    trailing_header_height: Pixels,
    /// The list's vertical padding, above the first row and below the last.
    pad: Pixels,
}

impl PickerPromptGeometry {
    pub fn new(
        items: &[PickerPromptItem],
        layout: &PickerPromptLayout,
        ui_scale: impl Into<UiScale>,
    ) -> Self {
        let ui_scale = ui_scale.into();
        let row_count = layout.item_indices.len();
        let mut geometry = Self {
            tops: Vec::with_capacity(row_count),
            heights: Vec::with_capacity(row_count),
            header_heights: Vec::with_capacity(row_count),
            rows_height: px(0.0),
            trailing_header_height: px(0.0),
            pad: ui_scale.px(LIST_PAD_PX),
        };

        // The headers are sorted by slot and this walks the slots in order, so
        // one cursor covers them all rather than rescanning per row.
        let mut next_header = 0usize;
        let mut take_headers = |slot: usize| {
            let mut height = px(0.0);
            while let Some((header_slot, header)) = layout.headers.get(next_header) {
                if *header_slot > slot {
                    break;
                }
                height += section_header_height(ui_scale, header.is_first);
                next_header += 1;
            }
            height
        };

        let mut y = px(0.0);
        for (display_ix, item_ix) in layout.item_indices.iter().enumerate() {
            let Some(item) = items.get(*item_ix) else {
                continue;
            };
            let header = take_headers(display_ix);
            let height = row_height(ui_scale, !item.secondary.is_empty());
            geometry.header_heights.push(header);
            geometry.tops.push(y + header);
            geometry.heights.push(height);
            y += header + height;
        }

        geometry.trailing_header_height = take_headers(usize::MAX);
        geometry.rows_height = y + geometry.trailing_header_height;
        geometry
    }

    /// The scrollable height of the list: every row and header, plus the list's
    /// padding above and below them.
    pub fn total_height(&self) -> Pixels {
        self.rows_height + self.pad * 2.0
    }

    /// Displayed rows a frame builds elements for at this scroll offset — every
    /// row for a short list, a viewport's worth plus overdraw for a long one.
    #[cfg(feature = "benchmarks")]
    pub fn visible_rows(&self, offset: Pixels, viewport: Pixels) -> Range<usize> {
        self.window(offset, viewport).rows
    }

    /// True when the list is long enough to be worth windowing.
    fn is_windowed(&self, viewport: Pixels) -> bool {
        viewport > px(0.0) && self.total_height() > viewport * WINDOWED_LIST_VIEWPORTS
    }

    /// The rows to render for a viewport of `viewport` at scroll offset `offset`
    /// (0 at the top, growing downwards), plus the space the rows before and
    /// after them occupy.
    fn window(&self, offset: Pixels, viewport: Pixels) -> PickerPromptWindow {
        let row_count = self.tops.len();
        if row_count == 0 || !self.is_windowed(viewport) {
            return PickerPromptWindow::everything(row_count);
        }

        // Into row coordinates: the scroll offset counts from above the list's
        // top padding, which sits before the first row.
        let top = (offset - self.pad).max(px(0.0));
        let bottom = top + viewport;
        let first_visible = (0..row_count)
            .find(|ix| self.tops[*ix] + self.heights[*ix] > top)
            .unwrap_or(0);
        let last_visible = (first_visible..row_count)
            .take_while(|ix| self.tops[*ix] < bottom)
            .last()
            .unwrap_or(first_visible);

        let first = first_visible.saturating_sub(WINDOW_OVERDRAW_ROWS);
        let last = (last_visible + WINDOW_OVERDRAW_ROWS).min(row_count - 1);

        // Spacers stand in for the rows outside the window, so the content stays
        // exactly as tall as it would be with every row rendered — the scrollbar
        // and the scroll offset must not move when the window does. They cover
        // the rows only: the list's own padding is still the list's.
        let space_before = self.tops[first] - self.header_heights[first];
        let mut space_after = self.rows_height - (self.tops[last] + self.heights[last]);
        if last + 1 == row_count {
            // The window reaches the end, so the trailing headers are drawn
            // rather than stood in for.
            space_after -= self.trailing_header_height;
        }
        PickerPromptWindow {
            rows: first..(last + 1),
            space_before,
            space_after: space_after.max(px(0.0)),
        }
    }

    /// Scroll offset that brings displayed row `ix` into view, given where the
    /// list is scrolled now. Returns the current offset when the row is already
    /// fully visible, so arrowing within the visible rows does not scroll.
    pub fn reveal_offset(&self, ix: usize, viewport: Pixels, current: Pixels) -> Pixels {
        let Some(row_top) = self.tops.get(ix).copied() else {
            return current;
        };
        let height = self.heights.get(ix).copied().unwrap_or(px(0.0));
        // Scrolling to the first row of a section should show its header too.
        let header = self.header_heights.get(ix).copied().unwrap_or(px(0.0));
        // Into scroll coordinates, which the caller's `current` is also in.
        let top = self.pad + row_top - header;
        let bottom = self.pad + row_top + height;
        let current = current.max(px(0.0));
        let max_offset = (self.total_height() - viewport).max(px(0.0));

        let offset = if top < current {
            top
        } else if bottom > current + viewport {
            bottom - viewport
        } else {
            current
        };
        offset.clamp(px(0.0), max_offset)
    }

    /// Top edge of displayed row `ix`, below any header that introduces it.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn row_top(&self, ix: usize) -> Pixels {
        self.tops[ix]
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn row_height(&self, ix: usize) -> Pixels {
        self.heights[ix]
    }
}

struct PickerPromptWindow {
    rows: Range<usize>,
    space_before: Pixels,
    space_after: Pixels,
}

impl PickerPromptWindow {
    /// Every row, with no spacers — a list short enough not to need windowing,
    /// and every picker that has not opted into it.
    fn everything(row_count: usize) -> Self {
        Self {
            rows: 0..row_count,
            space_before: px(0.0),
            space_after: px(0.0),
        }
    }
}

/// Height of one line of row text at `size_rems`.
fn text_line_height(ui_scale: UiScale, size_rems: f32) -> Pixels {
    let rem_size: f32 = crate::ui_scale::rem_size_for_percent(ui_scale.percent()).into();
    // Matches `TextStyle::line_height_in_pixels`, which rounds.
    px((rem_size * size_rems * LINE_HEIGHT_RATIO).round())
}

/// Height of a picker row, which is its own text plus the air around it — or the
/// standard control height, whichever is larger (`min_h`).
fn row_height(ui_scale: UiScale, has_secondary: bool) -> Pixels {
    let mut content =
        ui_scale.px(ROW_PAD_Y_PX) * 2.0 + text_line_height(ui_scale, PRIMARY_TEXT_REMS);
    if has_secondary {
        content += text_line_height(ui_scale, SECONDARY_TEXT_REMS);
    }
    content.max(control_height_md(ui_scale))
}

fn section_header_height(ui_scale: UiScale, is_first: bool) -> Pixels {
    let mut height = ui_scale
        .px(SECTION_HEADER_PAD_TOP_PX + SECTION_HEADER_PAD_BOTTOM_PX + SECTION_HEADER_HEIGHT_PX);
    if !is_first {
        // The rule itself is `border_t_1`, which is one physical pixel at any
        // scale, unlike the margin above it.
        height += ui_scale.px(SECTION_HEADER_MARGIN_PX) + px(1.0);
    }
    height
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerPromptItemPart {
    text: SharedString,
    profile: TextTruncationProfile,
    flexible: bool,
    searchable: bool,
    /// Renders one step quieter than the rest of its line — the line's own base
    /// color already differs between the primary and secondary lines.
    dim: bool,
    /// Whether this part shows its full text on hover when it is truncated.
    ///
    /// Off costs nothing; on gives the part an element id, a hover listener and
    /// a hitbox, which the pointer then has to be hit-tested against on every
    /// move. Parts that cannot truncate — separators, a short sha, a relative
    /// date — have nothing to reveal, so they opt out.
    tooltip: bool,
    match_range: Option<Range<usize>>,
}

type OnSelectFn<V> =
    dyn Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static;
type OnRemoveFn<V> = dyn Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static;
/// Section-header and right-click handlers are supplied as ready-made
/// `cx.listener` closures rather than as `render` arguments, so they can be
/// stored on the (view-agnostic) builder instead of widening every `render`
/// signature.
type OnToggleSectionFn = dyn Fn(&SharedString, &mut Window, &mut gpui::App) + 'static;
type OnContextMenuFn = dyn Fn(&PickerPromptContextMenuEvent, &mut Window, &mut gpui::App) + 'static;

impl PickerPrompt {
    pub fn new(query_input: Entity<TextInput>, scroll_handle: ScrollHandle) -> Self {
        Self {
            query_input,
            scroll_handle,
            items: Rc::from(Vec::new()),
            layout: None,
            empty_text: "No matches".into(),
            max_height: px(360.0),
            tooltip_host: None,
            selected_index: None,
            marked_index: None,
            leading_icon: None,
            selected_hint: None,
            accent_selection: false,
            attached_list_surface: false,
            padded_query_row: false,
            select_on_mouse_down: false,
            query_row_trailing: None,
            list_override: None,
            remove_tooltip: None,
            on_toggle_section: None,
            on_context_menu: None,
        }
    }

    /// Items and the layout already resolved for them, shared rather than
    /// rebuilt. For pickers that memoise their row model across frames (see
    /// `popover::rows_cache`); `layout` must have come from
    /// [`picker_prompt_layout`] over these very items and the query the input
    /// currently holds, or the rendered rows and the rows keyboard navigation
    /// walks would disagree.
    pub fn prebuilt_items(
        mut self,
        items: Rc<[PickerPromptItem]>,
        layout: Rc<PickerPromptLayout>,
    ) -> Self {
        self.items = items;
        self.layout = Some(layout);
        self
    }

    pub fn tooltip_host(mut self, tooltip_host: WeakEntity<TooltipHost>) -> Self {
        self.tooltip_host = Some(tooltip_host);
        self
    }

    pub fn empty_text(mut self, text: impl Into<SharedString>) -> Self {
        self.empty_text = text.into();
        self
    }

    pub fn max_height(mut self, height: gpui::Pixels) -> Self {
        self.max_height = height;
        self
    }

    pub fn selected_index(mut self, ix: Option<usize>) -> Self {
        self.selected_index = ix;
        self
    }

    /// Item (by original index, pre-filter) rendered with a trailing check —
    /// e.g. the currently checked-out branch in the branch picker.
    pub fn marked_index(mut self, ix: Option<usize>) -> Self {
        self.marked_index = ix;
        self
    }

    pub fn leading_icon(mut self, icon: &'static str) -> Self {
        self.leading_icon = Some(icon);
        self
    }

    pub fn selected_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.selected_hint = Some(hint.into());
        self
    }

    pub fn accent_selection(mut self) -> Self {
        self.accent_selection = true;
        self
    }

    pub fn attached_list_surface(mut self) -> Self {
        self.attached_list_surface = true;
        self
    }

    /// Pads and vertically centers the query row (matching the attached-surface
    /// layout) without drawing the surface border. Use when the picker already
    /// sits inside a bordered card (e.g. a popover) but the input is chromeless.
    pub fn padded_query_row(mut self) -> Self {
        self.padded_query_row = true;
        self
    }

    pub fn select_on_mouse_down(mut self) -> Self {
        self.select_on_mouse_down = true;
        self
    }

    /// Control pinned to the right of the query row, e.g. a sort toggle.
    pub fn query_row_trailing(mut self, element: impl IntoElement) -> Self {
        self.query_row_trailing = Some(element.into_any_element());
        self
    }

    /// Replaces the result rows with `element` — for menus that take over the
    /// list area while the query row stays put (the sort menu).
    pub fn list_override(mut self, element: impl IntoElement) -> Self {
        self.list_override = Some(element.into_any_element());
        self
    }

    /// Tooltip for the trailing remove button on
    /// [`PickerPromptItem::removable`] rows.
    pub fn remove_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.remove_tooltip = Some(tooltip.into());
        self
    }

    /// Makes section headers interactive: a disclosure chevron plus a click that
    /// hands back the section's label. Pass a `cx.listener(...)`.
    pub fn on_toggle_section(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_toggle_section = Some(Rc::new(handler));
        self
    }

    /// Right-click on a row. Pass a `cx.listener(...)`; the event carries both
    /// index spaces and the pointer position.
    pub fn on_context_menu(
        mut self,
        handler: impl Fn(&PickerPromptContextMenuEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_context_menu = Some(Rc::new(handler));
        self
    }

    pub fn render<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        self.render_with_remove(theme, ui_scale, cx, on_select, |_, _, _, _| {})
    }

    /// Like [`Self::render`], but also wires the trailing remove button that
    /// [`PickerPromptItem::removable`] rows carry. `on_remove` receives the
    /// item's original (pre-filter) index, like `on_select`.
    pub fn render_with_remove<V: 'static>(
        self,
        theme: AppTheme,
        ui_scale: impl Into<UiScale>,
        cx: &gpui::Context<V>,
        on_select: impl Fn(&mut V, usize, &ClickEvent, &mut Window, &mut gpui::Context<V>) + 'static,
        on_remove: impl Fn(&mut V, usize, &mut Window, &mut gpui::Context<V>) + 'static,
    ) -> Div {
        let on_select: Arc<OnSelectFn<V>> = Arc::new(on_select);
        let on_remove: Arc<OnRemoveFn<V>> = Arc::new(on_remove);
        let remove_tooltip = self.remove_tooltip;
        let scroll_handle = self.scroll_handle;
        let leading_icon = self.leading_icon;
        let selected_hint = self.selected_hint;
        let accent_selection = self.accent_selection;
        let attached_list_surface = self.attached_list_surface;
        let padded_query_row = self.padded_query_row;
        let select_on_mouse_down = self.select_on_mouse_down;
        let ui_scale = ui_scale.into();
        let scaled_px = |value| ui_scale.px(value);

        // Reuse the caller's layout when it supplied one; otherwise filter here.
        // A picker that folds sections away resolves its own layout with
        // `picker_prompt_layout_with_collapsed` and passes it through
        // [`Self::prebuilt_items`] — nothing is folded on this path.
        let layout = match self.layout.clone() {
            Some(layout) => layout,
            None => {
                let query = self
                    .query_input
                    .read_with(cx, |input, _| input.text().trim().to_string());
                Rc::new(picker_prompt_layout_with_collapsed(
                    &self.items,
                    &query,
                    &BTreeSet::new(),
                ))
            }
        };
        let row_count = layout.item_indices.len();
        let on_toggle_section = self.on_toggle_section;
        let on_context_menu = self.on_context_menu;

        let selected_index = self.selected_index.and_then(|ix| {
            if row_count == 0 {
                None
            } else {
                Some(ix.min(row_count - 1))
            }
        });

        let body = div()
            .flex()
            .flex_col()
            .w_full()
            .when(attached_list_surface, |surface| {
                surface
                    .border_1()
                    .border_color(theme.colors.stroke.subtle)
                    .rounded(px(theme.radii.control))
                    .bg(theme.colors.surface.raised)
                    .overflow_hidden()
            })
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w(px(0.0))
                    .when(attached_list_surface || padded_query_row, |query_row| {
                        query_row
                            .h(scaled_px(QUERY_ROW_HEIGHT_PX))
                            .items_center()
                            .px(scaled_px(10.0))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(self.query_input.clone()),
                    )
                    .when_some(self.query_row_trailing, |query_row, trailing| {
                        query_row.child(div().flex_shrink_0().child(trailing))
                    }),
            )
            .child(div().h(px(1.0)).w_full().bg(if attached_list_surface {
                theme.colors.stroke.default
            } else {
                theme.colors.stroke.subtle
            }));

        if let Some(list_override) = self.list_override {
            return body.child(div().w_full().min_w(px(0.0)).child(list_override));
        }

        let mut list = div()
            .id("picker_prompt_list")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .max_h(self.max_height)
            .py(scaled_px(LIST_PAD_PX))
            .pl(scaled_px(LIST_PAD_PX))
            .track_scroll(&scroll_handle);
        list = restrict_scroll_to_vertical_axis(list);

        // A list whose every section is collapsed has no rows but is not empty:
        // its headers are the only way back to the rows, so they must still draw.
        if row_count == 0 && layout.headers.is_empty() {
            list = list.child(
                div()
                    .h(control_height_md(ui_scale))
                    .w_full()
                    .flex()
                    .items_center()
                    .px(scaled_px(ROW_PAD_X_PX))
                    .text_sm()
                    .line_height(scaled_px(18.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(self.empty_text),
            );
        } else {
            // Long lists render only what the viewport can show; the rows above
            // and below it become two spacers of exactly their height, so the
            // scrollbar and the scroll offset behave as if all of them were
            // there.
            // Only what the viewport can show, once the list is long enough
            // to be worth it — `PickerPromptGeometry::window` hands back every
            // row for a short one, so a small picker keeps exactly the geometry
            // it had before any of this existed.
            let geometry = PickerPromptGeometry::new(&self.items, &layout, ui_scale);
            let window = geometry.window(-scroll_handle.offset().y, self.max_height);
            if window.space_before > px(0.0) {
                list = list.child(div().flex_shrink_0().w_full().h(window.space_before));
            }
            // Scanned rather than walked with a cursor because a windowed list
            // starts partway down: only the rendered rows ask for headers, and
            // a list has a handful of them at most.
            let section_header = |list: gpui::Stateful<Div>, slot: usize| {
                layout
                    .headers
                    .iter()
                    .filter(|(header_slot, _)| *header_slot == slot)
                    .fold(list, |list, (_, header)| {
                        list.child(section_header_row(
                            theme,
                            ui_scale,
                            header,
                            on_toggle_section.clone(),
                        ))
                    })
            };
            for display_ix in window.rows.clone() {
                let original_index = layout.item_indices[display_ix];
                let match_range = layout
                    .match_ranges
                    .get(display_ix)
                    .cloned()
                    .unwrap_or_default();
                list = section_header(list, display_ix);
                let label = picker_item_label(
                    theme,
                    &self.items[original_index],
                    match_range,
                    self.tooltip_host.clone(),
                    cx,
                );
                let on_select = Arc::clone(&on_select);
                let row_initials = self.items[original_index].repository_initials.clone();
                let has_initials = row_initials.is_some();
                let is_selected = selected_index == Some(display_ix);
                let is_marked = self.marked_index == Some(original_index);
                let row_icon = (!has_initials)
                    .then(|| row_leading_icon(&self.items[original_index], leading_icon, is_marked))
                    .flatten();
                let is_removable = self.items[original_index].removable;
                // Only the remove button reveals itself on row hover, so only
                // removable rows pay for a hover group — naming one costs a
                // formatted string and a group-hitbox registration per row, per
                // frame.
                let row_group: Option<SharedString> =
                    is_removable.then(|| format!("picker_prompt_row_{original_index}").into());
                let mut row = div()
                    .id(("picker_prompt_item", original_index))
                    .debug_selector(move || format!("picker_prompt_item_{original_index}"))
                    .when_some(row_group.clone(), |row, group| row.group(group))
                    // Sized by its own text rather than pinned to a height, so a
                    // row with a detail line grows by exactly one line box.
                    // `flex_shrink_0` is what keeps that honest: once the rows
                    // overflow the list's max height, a shrinkable row would be
                    // squashed below its content and its two lines would overlap.
                    .flex_shrink_0()
                    .min_h(control_height_md(ui_scale))
                    .py(scaled_px(ROW_PAD_Y_PX))
                    .w_full()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(scaled_px(ROW_ICON_GAP_PX))
                    .px(scaled_px(ROW_PAD_X_PX))
                    .rounded(scaled_px(ROW_CORNER_PX))
                    .cursor(CursorStyle::PointingHand)
                    .when_some(row_icon, |row, icon| {
                        row.child(
                            crate::view::icons::svg_icon(
                                icon,
                                if is_marked {
                                    theme.colors.accent.foreground
                                } else {
                                    theme.colors.foreground.secondary
                                },
                                scaled_px(14.0),
                            )
                            .debug_selector(move || {
                                format!("picker_prompt_item_icon_{original_index}")
                            }),
                        )
                    })
                    .when_some(row_initials, |row, initials| {
                        row.child(
                            super::repository_initials_box(
                                theme,
                                ui_scale,
                                initials,
                                is_selected || is_marked,
                            )
                            .debug_selector(move || {
                                format!("picker_prompt_repository_badge_{original_index}")
                            }),
                        )
                    })
                    .child(div().flex_1().min_w(px(0.0)).child(label))
                    // Only rows with repository initials still need this: they have
                    // no icon slot to turn into a check.
                    .when(is_marked && has_initials, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .pl(scaled_px(6.0))
                                .debug_selector(move || {
                                    format!("picker_prompt_item_trailing_check_{original_index}")
                                })
                                .child(crate::view::icons::svg_icon(
                                    "icons/check.svg",
                                    theme.colors.accent.foreground,
                                    scaled_px(12.0),
                                )),
                        )
                    })
                    .when(is_selected, |row| {
                        row.when_some(selected_hint.clone(), |row, hint| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .min_w(scaled_px(34.0))
                                    .h(scaled_px(22.0))
                                    .px(scaled_px(6.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(scaled_px(4.0))
                                    .bg(with_alpha(
                                        theme.colors.foreground.primary,
                                        if theme.is_dark { 0.06 } else { 0.035 },
                                    ))
                                    .font_family(
                                        crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                    )
                                    .text_xs()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(hint),
                            )
                        })
                    })
                    .when_some(row_group.clone(), |row, row_group| {
                        row.child(remove_row_button(
                            theme,
                            ui_scale,
                            original_index,
                            row_group,
                            // Keyboard users never hover, so the row the
                            // selection sits on keeps its button visible.
                            is_selected,
                            remove_tooltip.clone(),
                            self.tooltip_host.clone(),
                            Arc::clone(&on_remove),
                            cx,
                        ))
                    });
                if select_on_mouse_down {
                    row = row.on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            (on_select)(this, original_index, &ClickEvent::default(), window, cx);
                        }),
                    );
                } else {
                    row = row.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        (on_select)(this, original_index, event, window, cx);
                    }));
                }
                if let Some(on_context_menu) = on_context_menu.clone() {
                    row = row.on_mouse_down(
                        MouseButton::Right,
                        move |event: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            (on_context_menu)(
                                &PickerPromptContextMenuEvent {
                                    original_index,
                                    display_index: display_ix,
                                    position: event.position,
                                },
                                window,
                                cx,
                            );
                        },
                    );
                }
                // Text-alpha overlays keep the highlight visible on the
                // elevated popover surface, unlike the canvas-tuned tokens.
                let hover_overlay = theme.hover_overlay();
                let active_overlay = theme.active_overlay();
                if is_selected {
                    row = row.bg(active_overlay).when(accent_selection, |row| {
                        row.rounded_tl(px(0.0)).rounded_bl(px(0.0)).child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(scaled_px(3.0))
                                .rounded_tr(px(theme.radii.row))
                                .rounded_br(px(theme.radii.row))
                                .bg(theme.colors.accent.foreground),
                        )
                    });
                }
                row = row
                    .hover(move |s| s.bg(hover_overlay))
                    .active(move |s| s.bg(active_overlay));
                list = list.child(row);
            }
            if window.rows.end == row_count {
                // Sections whose rows are all folded away sit after the last
                // row, so they are only reached once the window ends the list.
                list = section_header(list, row_count);
            }
            if window.space_after > px(0.0) {
                list = list.child(div().flex_shrink_0().w_full().h(window.space_after));
            }
        }

        let scrollbar_gutter =
            Scrollbar::visible_gutter(scroll_handle.clone(), ScrollbarAxis::Vertical);
        // Mirrors the list's left padding so rows stay centred in the list, with
        // the scrollbar's own gutter added on top of it when one is showing.
        let list = list.pr(scrollbar_gutter + scaled_px(LIST_PAD_PX));
        let scrollbar = {
            let scrollbar = Scrollbar::new("picker_prompt_scrollbar", scroll_handle);
            #[cfg(test)]
            let scrollbar = scrollbar.debug_selector("picker_prompt_scrollbar");
            scrollbar.render(theme)
        };

        body.child(
            div()
                .id("picker_prompt_list_container")
                .relative()
                .w_full()
                .min_w(px(0.0))
                .child(list)
                .child(scrollbar),
        )
    }
}

impl PickerPromptItem {
    pub fn plain(text: impl Into<SharedString>) -> Self {
        Self::single(text, TextTruncationProfile::End)
    }

    pub fn single(text: impl Into<SharedString>, profile: TextTruncationProfile) -> Self {
        Self::from_parts([PickerPromptItemPart::new(text).profile(profile)])
    }

    pub fn from_parts<I>(parts: I) -> Self
    where
        I: IntoIterator<Item = PickerPromptItemPart>,
    {
        let mut display_text = String::new();
        let mut match_text = String::new();
        let built_parts = accumulate_parts(parts, Some(&mut display_text), &mut match_text);

        Self {
            display_text: display_text.into(),
            match_text: match_text.into(),
            parts: built_parts,
            secondary: Vec::new(),
            icon: None,
            repository_initials: None,
            section: None,
            removable: false,
        }
    }

    /// Adds a second, quieter line of supporting detail below the primary parts,
    /// making the row two lines tall.
    ///
    /// Searchable secondary parts still filter and highlight — a worktree's path
    /// finds its row from the second line just as its name does from the first.
    /// They stay out of `display_text` on purpose: that is the picker's sort key,
    /// which should order rows by what the eye reads first.
    pub fn secondary_parts<I>(mut self, parts: I) -> Self
    where
        I: IntoIterator<Item = PickerPromptItemPart>,
    {
        let mut match_text = self.match_text.to_string();
        self.secondary = accumulate_parts(parts, None, &mut match_text);
        self.match_text = match_text.into();
        self
    }

    pub fn icon(mut self, icon: &'static str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Uses the shared repository initials box in the row's leading slot.
    /// This takes precedence over both item and picker-level SVG icons.
    pub fn repository_initials(mut self, repository_name: &str) -> Self {
        self.repository_initials = Some(super::repository_initials(repository_name).into());
        self
    }

    /// Gives the row a trailing `x` that drops the entry from the list instead
    /// of activating it. Requires [`PickerPrompt::render_with_remove`].
    pub fn removable(mut self) -> Self {
        self.removable = true;
        self
    }

    /// Groups the item under a labelled section header. Items sharing a label
    /// must be contiguous in the list passed to [`PickerPrompt::items`].
    pub fn section(mut self, section: impl Into<SharedString>) -> Self {
        self.section = Some(section.into());
        self
    }

    /// The section label this item was grouped under, if any.
    pub fn section_label(&self) -> Option<&SharedString> {
        self.section.as_ref()
    }

    fn display_text(&self) -> &str {
        self.display_text.as_ref()
    }

    fn match_text(&self) -> &str {
        self.match_text.as_ref()
    }

    fn parts(&self) -> &[PickerPromptItemPart] {
        self.parts.as_slice()
    }

    fn secondary(&self) -> &[PickerPromptItemPart] {
        self.secondary.as_slice()
    }

    /// The secondary line's text, part by part — what a row's detail line says.
    #[cfg(any(test, feature = "benchmarks"))]
    pub(crate) fn debug_secondary_text(&self) -> String {
        self.secondary
            .iter()
            .map(|part| part.text.as_ref())
            .collect()
    }

    #[cfg(feature = "benchmarks")]
    pub fn debug_display_text(&self) -> &str {
        self.display_text.as_ref()
    }

    /// Text parts across both of the row's lines — one element each.
    #[cfg(feature = "benchmarks")]
    pub fn debug_part_count(&self) -> usize {
        self.parts.len() + self.secondary.len()
    }

    /// Parts carrying a hover tooltip, and so an element id, a hover listener
    /// and a hitbox.
    #[cfg(feature = "benchmarks")]
    pub fn debug_tooltip_part_count(&self) -> usize {
        self.parts
            .iter()
            .chain(self.secondary.iter())
            .filter(|part| part.tooltip)
            .count()
    }
}

/// Appends `parts` to a row's match text — and, for the primary line, its display
/// text — recording each searchable part's span in match-text coordinates so a
/// query's hit can be highlighted in the part that actually contains it.
///
/// `display_text` is `None` for the secondary line, which stays out of the sort
/// key by design.
fn accumulate_parts<I>(
    parts: I,
    mut display_text: Option<&mut String>,
    match_text: &mut String,
) -> Vec<PickerPromptItemPart>
where
    I: IntoIterator<Item = PickerPromptItemPart>,
{
    let mut built = Vec::new();
    for mut part in parts {
        if let Some(display_text) = display_text.as_deref_mut() {
            display_text.push_str(part.text.as_ref());
        }

        if part.searchable {
            let start = match_text.len();
            match_text.push_str(part.text.as_ref());
            part.match_range = Some(start..match_text.len());
        }

        built.push(part);
    }
    built
}

impl PickerPromptItemPart {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            profile: TextTruncationProfile::End,
            flexible: true,
            searchable: true,
            dim: false,
            tooltip: true,
            match_range: None,
        }
    }

    /// Punctuation and connective text between the parts that carry meaning, so
    /// it renders one step quieter than they do. Never truncated, so it carries
    /// no tooltip.
    pub fn separator(text: impl Into<SharedString>) -> Self {
        Self::new(text)
            .flexible(false)
            .searchable(false)
            .dim()
            .tooltip(false)
    }

    /// Opts this part out of the truncated-text hover tooltip. Use for text that
    /// cannot be cut off — a short sha, a relative date, a fixed label.
    pub fn tooltip(mut self, tooltip: bool) -> Self {
        self.tooltip = tooltip;
        self
    }

    pub fn path(text: impl Into<SharedString>) -> Self {
        Self::new(text).profile(TextTruncationProfile::Path)
    }

    pub fn profile(mut self, profile: TextTruncationProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn flexible(mut self, flexible: bool) -> Self {
        self.flexible = flexible;
        self
    }

    /// Renders this part one step quieter than the rest of its line.
    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        if !searchable {
            self.match_range = None;
        }
        self
    }

    fn local_match_range(&self, range: Option<&Range<usize>>) -> Option<Range<usize>> {
        let range = range?;
        let part_range = self.match_range.as_ref()?;
        let start = range.start.max(part_range.start);
        let end = range.end.min(part_range.end);
        (start < end).then(|| (start - part_range.start)..(end - part_range.start))
    }
}

impl From<SharedString> for PickerPromptItem {
    fn from(value: SharedString) -> Self {
        Self::plain(value)
    }
}

impl From<String> for PickerPromptItem {
    fn from(value: String) -> Self {
        Self::plain(value)
    }
}

impl From<&str> for PickerPromptItem {
    fn from(value: &str) -> Self {
        Self::plain(value.to_owned())
    }
}

#[derive(Clone, Debug)]
struct Match {
    index: usize,
    range: Option<Range<usize>>,
    sort_key: (usize, usize, usize, SharedString),
}

/// Tracks the section of the previously emitted row so a header is rendered
/// exactly once per contiguous run of items sharing a section label.
#[derive(Default)]
struct SectionRun<'a> {
    previous: Option<Option<&'a SharedString>>,
}

impl<'a> SectionRun<'a> {
    /// True when `section` opens a labelled run that needs a header row.
    fn starts_new_section(&mut self, section: Option<&'a SharedString>) -> bool {
        let changed = self.previous != Some(section);
        self.previous = Some(section);
        changed && section.is_some()
    }
}

/// Numbers each contiguous run of items sharing a section label. Matches sort
/// within their group, so filtering never interleaves sections.
fn section_groups(items: &[PickerPromptItem]) -> Vec<usize> {
    let mut groups = Vec::with_capacity(items.len());
    let mut group = 0usize;
    let mut previous: Option<&SharedString> = None;
    for (ix, item) in items.iter().enumerate() {
        if ix > 0 && item.section.as_ref() != previous {
            group += 1;
        }
        previous = item.section.as_ref();
        groups.push(group);
    }
    groups
}

fn match_items(items: &[PickerPromptItem], groups: &[usize], query: &str) -> Vec<Match> {
    let group_of = |index: usize| groups.get(index).copied().unwrap_or(0);

    if query.is_empty() {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| Match {
                index,
                range: None,
                sort_key: (
                    group_of(index),
                    0,
                    item.display_text().len(),
                    item.display_text.clone(),
                ),
            })
            .collect();
    }

    let mut out = Vec::with_capacity(items.len());
    let needle_bytes = query.as_bytes();
    let first_lower = needle_bytes[0].to_ascii_lowercase();
    let first_upper = needle_bytes[0].to_ascii_uppercase();

    for (index, item) in items.iter().enumerate() {
        let match_text = item.match_text();
        if match_text.is_empty() {
            continue;
        }

        let Some(range) = find_ascii_case_insensitive_precomputed(
            match_text.as_bytes(),
            needle_bytes,
            first_lower,
            first_upper,
        ) else {
            continue;
        };
        let start = range.start;
        out.push(Match {
            index,
            range: Some(range),
            sort_key: (
                group_of(index),
                start,
                item.display_text().len(),
                item.display_text.clone(),
            ),
        });
    }

    out.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
    out
}

/// The icon in a row's leading slot.
///
/// The marked row says "this is the current one" by turning its icon into a check
/// rather than keeping its own icon and adding a second one at the far end — which
/// also leaves the row's trailing edge free for its hover actions.
fn row_leading_icon(
    item: &PickerPromptItem,
    leading_icon: Option<&'static str>,
    is_marked: bool,
) -> Option<&'static str> {
    if is_marked {
        return Some("icons/check.svg");
    }
    item.icon.or(leading_icon)
}

/// A group label above a run of rows. Plain muted text rather than a heavier
/// treatment, and every section after the first is introduced by a rule, so the
/// groups separate without the labels having to shout.
///
/// With a toggle handler the label gains the sidebar's disclosure chevron and
/// becomes clickable; a folded section also says how many rows it is hiding.
fn section_header_row(
    theme: AppTheme,
    ui_scale: UiScale,
    header: &PickerPromptHeader,
    on_toggle: Option<Rc<OnToggleSectionFn>>,
) -> Div {
    let scaled_px = |value| ui_scale.px(value);
    let label = header.label.clone();
    let collapsed = header.collapsed;

    let mut label_row = div()
        .h(scaled_px(SECTION_HEADER_HEIGHT_PX))
        .w_full()
        .flex()
        .items_center()
        .gap(scaled_px(4.0))
        .px(scaled_px(ROW_PAD_X_PX))
        .text_xs()
        .text_color(theme.colors.foreground.secondary)
        .whitespace_nowrap()
        .overflow_hidden();

    if on_toggle.is_some() {
        // Same disclosure treatment as the branch sidebar's section headers, in
        // a fixed-width slot so the labels stay aligned either way.
        label_row = label_row.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .w(scaled_px(12.0))
                .child(crate::view::icons::svg_icon(
                    if collapsed {
                        "icons/arrow_right.svg"
                    } else {
                        "icons/chevron_down.svg"
                    },
                    theme.colors.foreground.secondary,
                    scaled_px(10.0),
                )),
        );
    }
    label_row = label_row.child(label.clone());
    if collapsed && header.hidden_count > 0 {
        label_row = label_row.child(
            div()
                .text_color(with_alpha(theme.colors.foreground.secondary, 0.7))
                .child(SharedString::from(header.hidden_count.to_string())),
        );
    }

    let mut row = div()
        .w_full()
        .flex_shrink_0()
        .when(!header.is_first, |header| {
            header
                .mt(scaled_px(4.0))
                .border_t_1()
                .border_color(theme.colors.stroke.subtle)
        })
        .pt(scaled_px(6.0))
        .pb(scaled_px(4.0));

    let Some(on_toggle) = on_toggle else {
        return row.child(label_row);
    };

    let debug_label = label.clone();
    row = row.child(
        div()
            .id(SharedString::from(format!("picker_prompt_section_{label}")))
            .debug_selector(move || format!("picker_prompt_section_{debug_label}"))
            .w_full()
            .rounded(scaled_px(ROW_CORNER_PX))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.hover_overlay()))
            .active(move |s| s.bg(theme.active_overlay()))
            .child(label_row)
            .on_click(move |_event: &ClickEvent, window, cx| {
                (on_toggle)(&label, window, cx);
            }),
    );
    row
}

fn picker_item_label<V: 'static>(
    theme: AppTheme,
    item: &PickerPromptItem,
    range: Option<Range<usize>>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let has_secondary = !item.secondary().is_empty();
    // The title line stays at normal weight even with detail below it: the smaller,
    // muted detail line supplies the contrast, so bolding the title only makes the
    // list noisier.
    let primary = picker_item_line(
        theme,
        LineRole {
            text_id: "picker_prompt_label_part_text",
            parts: item.parts(),
            base_color: theme.colors.foreground.primary,
            dim_color: theme.colors.foreground.secondary,
            text_size: gpui::rems(0.875),
            weight: FontWeight::NORMAL,
        },
        range.clone(),
        tooltip_host.clone(),
        cx,
    );

    if !has_secondary {
        return primary;
    }

    let secondary = picker_item_line(
        theme,
        LineRole {
            text_id: "picker_prompt_secondary_part_text",
            parts: item.secondary(),
            base_color: theme.colors.foreground.secondary,
            // Half-strength muted, the weight Zed gives the dots between a row's
            // detail fields.
            dim_color: with_alpha(theme.colors.foreground.secondary, 0.5),
            text_size: gpui::rems(0.75),
            weight: FontWeight::NORMAL,
        },
        range,
        tooltip_host,
        cx,
    );

    div()
        .flex()
        .flex_col()
        .w_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(primary)
        .child(secondary)
}

/// How one line of a picker row renders: which parts it holds and the type and
/// color treatment that mark it as the title or the supporting detail.
struct LineRole<'a> {
    /// Element-id stem for this line's tooltip-bearing parts. Both lines of a row
    /// live under the same parent, so their part ids must not collide.
    text_id: &'static str,
    parts: &'a [PickerPromptItemPart],
    base_color: gpui::Rgba,
    dim_color: gpui::Rgba,
    /// Set on the line *and* on each `TruncatedText`. The latter measures itself
    /// against the text style in effect during layout, which does not yet carry
    /// the line's own `text_sm`/`text_xs`; leaving it to inherit reserved a line
    /// box for 1rem text and left both lines sitting loose in it.
    text_size: gpui::Rems,
    weight: FontWeight,
}

fn picker_item_line<V: 'static>(
    theme: AppTheme,
    role: LineRole<'_>,
    range: Option<Range<usize>>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    cx: &gpui::Context<V>,
) -> Div {
    let match_highlight = HighlightStyle {
        color: Some(theme.colors.accent.foreground.into()),
        font_weight: Some(FontWeight::BOLD),
        ..HighlightStyle::default()
    };

    // `TruncatedText` shapes against the inherited text style, so size and
    // weight are set here on the line rather than per part.
    let mut line = div()
        .flex()
        .w_full()
        .min_w(px(0.0))
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .font_weight(role.weight)
        .text_size(role.text_size);

    for (ix, part) in role.parts.iter().enumerate() {
        let highlight_range = part.local_match_range(range.as_ref());
        // `TruncatedText` already wraps its element in a clipping box, so the
        // flex behaviour goes on that box rather than nesting a second div
        // around it — this is one element per part instead of two.
        let flex = if part.flexible {
            TruncatedTextFlex::Grow
        } else if part.searchable {
            TruncatedTextFlex::Shrink
        } else {
            TruncatedTextFlex::Fixed
        };

        let mut text = TruncatedText::new(part.text.clone())
            .profile(part.profile)
            .flex(flex)
            .text_size(role.text_size)
            .text_color(if part.dim {
                role.dim_color
            } else {
                role.base_color
            });
        if let Some(highlight_range) = highlight_range.clone() {
            text = text
                .focus_range(Some(highlight_range.clone()))
                .highlights([(highlight_range, match_highlight)]);
        }
        // The id exists for the tooltip's hover state; without one the part is a
        // plain clipping box and needs no element state at all.
        if let Some(tooltip_host) = tooltip_host.clone().filter(|_| part.tooltip) {
            text = text.id((role.text_id, ix)).full_text_tooltip(tooltip_host);
        }

        line = line.child(text.render(cx));
    }

    line
}

/// The trailing `x` on a removable row — drops the entry from the list the
/// picker draws from, rather than activating it. Mirrors the repository tab's
/// close affordance: hidden until the row is hovered (or carries the keyboard
/// selection) and tinted with the danger colour.
#[allow(clippy::too_many_arguments)]
fn remove_row_button<V: 'static>(
    theme: AppTheme,
    ui_scale: UiScale,
    index: usize,
    row_group: SharedString,
    always_visible: bool,
    tooltip: Option<SharedString>,
    tooltip_host: Option<WeakEntity<TooltipHost>>,
    on_remove: Arc<OnRemoveFn<V>>,
    cx: &gpui::Context<V>,
) -> impl IntoElement {
    let scaled_px = |value| ui_scale.px(value);
    let tooltip_for_move = tooltip.clone();
    let host_for_move = tooltip_host.clone();
    let host_for_hover = tooltip_host;

    div()
        .id(("picker_prompt_item_remove", index))
        .debug_selector(move || format!("picker_prompt_item_remove_{index}"))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .size(scaled_px(super::REMOVE_BUTTON_SIZE_PX))
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .when(!always_visible, |button| {
            button
                .invisible()
                .group_hover(row_group, |style| style.visible())
        })
        .hover(move |s| {
            s.bg(with_alpha(
                theme.colors.status.danger.foreground,
                super::REMOVE_BUTTON_HOVER_ALPHA,
            ))
        })
        .active(move |s| {
            s.bg(with_alpha(
                theme.colors.status.danger.foreground,
                super::REMOVE_BUTTON_PRESSED_ALPHA,
            ))
        })
        .child(crate::view::icons::svg_icon(
            super::REMOVE_BUTTON_ICON,
            theme.colors.status.danger.foreground,
            scaled_px(super::REMOVE_BUTTON_ICON_SIZE_PX),
        ))
        .on_mouse_move(cx.listener(move |_this, event: &MouseMoveEvent, _w, cx| {
            let (Some(host), Some(tooltip)) = (host_for_move.as_ref(), tooltip_for_move.as_ref())
            else {
                return;
            };
            let _ = host.update(cx, |host, cx| {
                host.on_mouse_moved(event.position, cx);
                host.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
            });
        }))
        .on_hover(cx.listener(move |_this, hovering: &bool, _w, cx| {
            let (false, Some(host), Some(tooltip)) =
                (*hovering, host_for_hover.as_ref(), tooltip.as_ref())
            else {
                return;
            };
            let _ = host.update(cx, |host, cx| {
                host.clear_tooltip_if_matches(tooltip, cx);
            });
        }))
        // Keeps the press off the row, which may activate on mouse-down.
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event: &MouseDownEvent, _w, cx| cx.stop_propagation()),
        )
        .on_click(cx.listener(move |this, _event: &ClickEvent, window, cx| {
            cx.stop_propagation();
            (on_remove)(this, index, window, cx);
        }))
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}

/// Substring search with precomputed first-byte lowercase/uppercase values.
/// Skips positions where the first byte cannot match, avoiding the inner loop
/// overhead for most non-matching positions.
fn find_ascii_case_insensitive_precomputed(
    haystack_bytes: &[u8],
    needle_bytes: &[u8],
    first_lower: u8,
    first_upper: u8,
) -> Option<Range<usize>> {
    if needle_bytes.is_empty() {
        return Some(0..0);
    }
    if haystack_bytes.len() < needle_bytes.len() {
        return None;
    }

    let end = haystack_bytes.len() - needle_bytes.len();
    'outer: for start in 0..=end {
        let first = haystack_bytes[start];
        if first != first_lower && first != first_upper {
            continue;
        }
        for (offset, needle_byte) in needle_bytes.iter().copied().enumerate().skip(1) {
            let haystack_byte = haystack_bytes[start + offset];
            if !haystack_byte.eq_ignore_ascii_case(&needle_byte) {
                continue 'outer;
            }
        }
        return Some(start..(start + needle_bytes.len()));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sectioned_items() -> Vec<PickerPromptItem> {
        vec![
            PickerPromptItem::plain("pinned-repo").section("Pinned"),
            PickerPromptItem::plain("open-repo").section("Open Repositories"),
            PickerPromptItem::plain("closed-one").section("Recently Closed"),
            PickerPromptItem::plain("closed-two").section("Recently Closed"),
        ]
    }

    fn header_labels(layout: &PickerPromptLayout) -> Vec<(usize, &str, bool, usize)> {
        layout
            .headers
            .iter()
            .map(|(slot, header)| {
                (
                    *slot,
                    header.label.as_ref(),
                    header.collapsed,
                    header.hidden_count,
                )
            })
            .collect()
    }

    #[test]
    fn an_uncollapsed_layout_places_one_header_before_each_sections_first_row() {
        let items = sectioned_items();
        let layout = picker_prompt_layout(&items, "");

        assert_eq!(layout.item_indices, vec![0, 1, 2, 3]);
        // Every header takes a scroll child slot ahead of its first row.
        assert_eq!(layout.child_indices, vec![1, 3, 5, 6]);
        assert_eq!(
            header_labels(&layout),
            vec![
                (0, "Pinned", false, 0),
                (1, "Open Repositories", false, 0),
                (2, "Recently Closed", false, 0),
            ]
        );
        assert!(
            layout
                .headers
                .iter()
                .all(|(slot, _)| *slot < layout.item_indices.len()),
            "with nothing collapsed every header still introduces a row"
        );
    }

    #[test]
    fn a_collapsed_section_keeps_its_header_and_drops_its_rows() {
        let items = sectioned_items();
        let collapsed = BTreeSet::from([SharedString::from("Recently Closed")]);
        let layout = picker_prompt_layout_with_collapsed(&items, "", &collapsed);

        assert_eq!(
            layout.item_indices,
            vec![0, 1],
            "the collapsed section's rows are gone, so keyboard navigation skips them"
        );
        assert_eq!(layout.child_indices, vec![1, 3]);
        assert_eq!(
            header_labels(&layout),
            vec![
                (0, "Pinned", false, 0),
                (1, "Open Repositories", false, 0),
                // Its rows are gone, so the header trails the last visible row.
                (2, "Recently Closed", true, 2),
            ]
        );
    }

    #[test]
    fn collapsing_every_section_leaves_the_headers_as_the_only_rows() {
        let items = sectioned_items();
        let collapsed = BTreeSet::from([
            SharedString::from("Pinned"),
            SharedString::from("Open Repositories"),
            SharedString::from("Recently Closed"),
        ]);
        let layout = picker_prompt_layout_with_collapsed(&items, "", &collapsed);

        assert!(layout.item_indices.is_empty());
        assert_eq!(
            header_labels(&layout),
            vec![
                (0, "Pinned", true, 1),
                (0, "Open Repositories", true, 1),
                (0, "Recently Closed", true, 2),
            ],
            "with no rows at all every header trails slot 0"
        );
        assert!(
            layout
                .headers
                .first()
                .is_some_and(|(_, header)| header.is_first),
            "only the first header skips the rule above it"
        );
        assert!(
            layout.headers[1..]
                .iter()
                .all(|(_, header)| !header.is_first)
        );
    }

    #[test]
    fn a_collapsed_section_still_counts_only_the_rows_the_query_matched() {
        let items = sectioned_items();
        let collapsed = BTreeSet::from([SharedString::from("Recently Closed")]);
        let layout = picker_prompt_layout_with_collapsed(&items, "closed-one", &collapsed);

        assert!(layout.item_indices.is_empty());
        assert_eq!(
            header_labels(&layout),
            vec![(0, "Recently Closed", true, 1)],
            "sections with no matches contribute no header, and the count is of matches"
        );
    }

    #[test]
    fn match_items_skips_queries_longer_than_candidate_labels() {
        let items = vec![
            PickerPromptItem::plain("ab"),
            PickerPromptItem::plain("alphabet"),
        ];

        let matches = match_items(&items, &section_groups(&items), "alphabet soup");

        assert!(matches.is_empty());
    }

    #[test]
    fn ascii_matcher_returns_none_when_needle_is_longer_than_haystack() {
        let needle = b"alphabet soup";

        let range = find_ascii_case_insensitive_precomputed(
            b"ab",
            needle,
            needle[0].to_ascii_lowercase(),
            needle[0].to_ascii_uppercase(),
        );

        assert_eq!(range, None);
    }

    #[test]
    fn picker_prompt_item_maps_search_hits_into_part_local_ranges() {
        let item = PickerPromptItem::from_parts([
            PickerPromptItemPart::new("feature/worktree").flexible(false),
            PickerPromptItemPart::separator("  "),
            PickerPromptItemPart::path("/tmp/repo/src/main.rs"),
        ]);

        let matches = match_items(std::slice::from_ref(&item), &[0], "main");
        let range = matches
            .first()
            .and_then(|m| m.range.clone())
            .expect("expected a match");

        assert_eq!(item.parts()[0].local_match_range(Some(&range)), None);
        assert_eq!(item.parts()[1].local_match_range(Some(&range)), None);
        assert_eq!(
            item.parts()[2].local_match_range(Some(&range)),
            Some(14..18)
        );
    }

    #[test]
    fn the_marked_row_shows_a_check_instead_of_its_own_icon() {
        let item = PickerPromptItem::plain("main").icon("icons/git_branch.svg");

        assert_eq!(
            row_leading_icon(&item, None, true),
            Some("icons/check.svg"),
            "the marked row gives up its icon for the check"
        );
        assert_eq!(
            row_leading_icon(&item, None, false),
            Some("icons/git_branch.svg")
        );
    }

    #[test]
    fn unmarked_rows_without_an_icon_fall_back_to_the_picker_wide_one() {
        let item = PickerPromptItem::plain("main");

        assert_eq!(
            row_leading_icon(&item, Some("icons/git_branch.svg"), false),
            Some("icons/git_branch.svg")
        );
        assert_eq!(row_leading_icon(&item, None, false), None);
    }

    #[test]
    fn secondary_parts_keep_filtering_and_highlighting_the_row() {
        // The worktree picker moved the path onto the second line; a path query
        // must still find the row and light up the matched span there.
        let item =
            PickerPromptItem::from_parts([PickerPromptItemPart::new("feature").flexible(false)])
                .secondary_parts([PickerPromptItemPart::path("/tmp/ws/feature/src/main.rs")]);

        let matches = match_items(std::slice::from_ref(&item), &[0], "src");
        let range = matches
            .first()
            .and_then(|m| m.range.clone())
            .expect("a path query must still match");

        assert_eq!(item.parts()[0].local_match_range(Some(&range)), None);
        assert_eq!(
            item.secondary()[0].local_match_range(Some(&range)),
            Some(16..19),
            "the hit belongs to the secondary part that contains it"
        );
    }

    #[test]
    fn secondary_parts_stay_out_of_the_sort_key() {
        // Sorting is by what the eye reads first, so a long detail line must not
        // push a short-titled row down the list.
        let items = vec![
            PickerPromptItem::from_parts([PickerPromptItemPart::new("main")]).secondary_parts([
                PickerPromptItemPart::path("/a/very/long/path/that/dwarfs/the/title"),
            ]),
            PickerPromptItem::from_parts([PickerPromptItemPart::new("maintenance")]),
        ];

        let matches = match_items(&items, &section_groups(&items), "main");

        assert_eq!(
            matches.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![0, 1],
            "the shorter title sorts first regardless of its detail line"
        );
    }

    #[test]
    fn picker_prompt_layout_reserves_a_child_slot_per_section_header() {
        let items = vec![
            PickerPromptItem::plain("alpha").section("Open"),
            PickerPromptItem::plain("beta").section("Open"),
            PickerPromptItem::plain("gamma").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "");

        assert_eq!(layout.item_indices, vec![0, 1, 2]);
        // Header, alpha, beta, header, gamma.
        assert_eq!(layout.child_indices, vec![1, 2, 4]);
    }

    #[test]
    fn picker_prompt_layout_keeps_sections_contiguous_when_filtering() {
        let items = vec![
            PickerPromptItem::plain("zulu-repo").section("Open"),
            PickerPromptItem::plain("repo-one").section("Recently Closed"),
            PickerPromptItem::plain("repo-two").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "repo");

        // The "Recently Closed" hits sort earlier on match position, but must
        // not be hoisted above the "Open" section.
        assert_eq!(layout.item_indices, vec![0, 1, 2]);
        assert_eq!(layout.child_indices, vec![1, 3, 4]);
    }

    #[test]
    fn picker_prompt_layout_drops_headers_for_sections_without_matches() {
        let items = vec![
            PickerPromptItem::plain("alpha").section("Open"),
            PickerPromptItem::plain("gamma").section("Recently Closed"),
        ];

        let layout = picker_prompt_layout(&items, "gam");

        assert_eq!(layout.item_indices, vec![1]);
        assert_eq!(layout.child_indices, vec![1]);
    }

    fn two_line_items(count: usize) -> Vec<PickerPromptItem> {
        (0..count)
            .map(|ix| {
                PickerPromptItem::plain(format!("branch-{ix}"))
                    .secondary_parts([PickerPromptItemPart::new("Ada  •  2 days ago")])
            })
            .collect()
    }

    fn geometry_for(items: &[PickerPromptItem]) -> PickerPromptGeometry {
        let layout = picker_prompt_layout(items, "");
        PickerPromptGeometry::new(items, &layout, UiScale::from_percent(100))
    }

    #[test]
    fn a_list_that_barely_scrolls_renders_every_row() {
        // Windowing only earns its keep on long lists; short ones keep exactly
        // the geometry they had before it existed.
        let items = two_line_items(8);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

        let window = geometry.window(px(0.0), viewport);

        assert!(!geometry.is_windowed(viewport));
        assert_eq!(window.rows, 0..8);
        assert_eq!(window.space_before, px(0.0));
        assert_eq!(window.space_after, px(0.0));
    }

    #[test]
    fn a_long_list_renders_only_what_can_be_seen() {
        let items = two_line_items(1_200);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

        let window = geometry.window(px(0.0), viewport);

        assert!(geometry.is_windowed(viewport));
        // A 300px viewport over 50px rows is six rows, plus the overdraw below.
        assert!(
            window.rows.len() < 20,
            "expected a viewport-sized window, rendered {} rows",
            window.rows.len()
        );
        assert_eq!(window.rows.start, 0);
    }

    #[test]
    fn the_windowed_list_is_exactly_as_tall_as_the_full_one() {
        // The spacers stand in for the rows outside the window — and only for
        // those: the list's own padding stays the list's, or the rows would shift
        // down and the scrollbar would grow the moment windowing kicked in. If
        // this did not add up, scrolling would jump as the window moved.
        let items = two_line_items(1_200);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);
        let pad = UiScale::from_percent(100).px(LIST_PAD_PX);

        for offset in [px(0.0), px(640.0), px(20_000.0)] {
            let window = geometry.window(offset, viewport);
            let rendered: Pixels = window
                .rows
                .clone()
                .map(|ix| geometry.heights[ix] + geometry.header_heights[ix])
                .fold(px(0.0), |sum, height| sum + height);

            assert_eq!(
                pad + window.space_before + rendered + window.space_after + pad,
                geometry.total_height(),
                "spacers must account for every row outside the window at {offset:?}"
            );
        }
        // And an unwindowed list adds no spacers at all.
        let short = geometry_for(&two_line_items(4));
        let window = short.window(px(0.0), viewport);
        assert_eq!(window.space_before, px(0.0));
        assert_eq!(window.space_after, px(0.0));
    }

    #[test]
    fn scrolling_down_keeps_the_window_around_the_viewport() {
        let items = two_line_items(1_200);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);
        let offset = geometry.reveal_offset(600, viewport, px(0.0));

        let window = geometry.window(offset, viewport);

        assert!(window.rows.contains(&600));
        assert!(window.rows.start > 0, "rows above the viewport are dropped");
        assert!(window.space_before > px(0.0));
        assert!(window.space_after > px(0.0));
    }

    #[test]
    fn reveal_offset_scrolls_to_rows_outside_the_viewport_and_leaves_visible_ones_alone() {
        let items = two_line_items(1_200);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

        // Offsets are in the scroll container's coordinates, so they count the
        // list's top padding as well as the rows above.
        let pad = UiScale::from_percent(100).px(LIST_PAD_PX);

        // A row far below: scrolled to sit at the bottom edge of the viewport.
        let down = geometry.reveal_offset(40, viewport, px(0.0));
        assert_eq!(
            down,
            pad + geometry.row_top(40) + geometry.row_height(40) - viewport
        );

        // A row above the current offset: scrolled to the top edge.
        let up = geometry.reveal_offset(10, viewport, down);
        assert_eq!(up, pad + geometry.row_top(10));

        // A row already fully visible: left where it is.
        let unchanged = geometry.reveal_offset(1, viewport, px(0.0));
        assert_eq!(unchanged, px(0.0));
    }

    #[test]
    fn reveal_offset_stays_within_the_scrollable_range() {
        let items = two_line_items(1_200);
        let geometry = geometry_for(&items);
        let viewport = px(PICKER_LIST_MAX_HEIGHT_PX);

        let last = geometry.reveal_offset(1_199, viewport, px(0.0));

        // Scrolls no further than it has to: the last row sits flush with the
        // bottom edge, which is inside the scrollable range (the list's bottom
        // padding is the remaining few pixels).
        let pad = UiScale::from_percent(100).px(LIST_PAD_PX);
        assert_eq!(
            last,
            pad + geometry.row_top(1_199) + geometry.row_height(1_199) - viewport
        );
        assert!(last <= geometry.total_height() - viewport);
    }

    #[test]
    fn a_section_header_is_counted_once_above_the_row_that_opens_it() {
        let items = vec![
            PickerPromptItem::plain("alpha").section("Local Branches"),
            PickerPromptItem::plain("beta").section("Local Branches"),
            PickerPromptItem::plain("origin/gamma").section("Remote Branches"),
        ];
        let geometry = geometry_for(&items);
        let ui_scale = UiScale::from_percent(100);

        assert_eq!(
            geometry.header_heights[0],
            section_header_height(ui_scale, true)
        );
        assert_eq!(geometry.header_heights[1], px(0.0));
        assert_eq!(
            geometry.header_heights[2],
            section_header_height(ui_scale, false),
            "every header but the first also carries its rule"
        );
        // The second section's rows sit below both headers and the rows above.
        let single = row_height(ui_scale, false);
        assert_eq!(
            geometry.row_top(2),
            section_header_height(ui_scale, true)
                + single * 2.0
                + section_header_height(ui_scale, false)
        );
    }

    #[test]
    fn a_detail_line_makes_a_row_one_line_box_taller() {
        let ui_scale = UiScale::from_percent(100);

        assert_eq!(
            row_height(ui_scale, true) - row_height(ui_scale, false),
            text_line_height(ui_scale, SECONDARY_TEXT_REMS)
        );
    }

    #[test]
    fn a_single_line_row_is_never_shorter_than_a_control() {
        // `min_h(control_height_md)` is what stops a one-line row from collapsing.
        let ui_scale = UiScale::from_percent(80);

        assert!(row_height(ui_scale, false) >= control_height_md(ui_scale));
    }

    #[test]
    fn picker_prompt_item_search_skips_non_searchable_separators() {
        let item = PickerPromptItem::from_parts([
            PickerPromptItemPart::new("repo").flexible(false),
            PickerPromptItemPart::separator(" - "),
            PickerPromptItemPart::path("/tmp/workspace"),
        ]);

        let matches = match_items(&[item], &[0], " - ");

        assert!(matches.is_empty());
    }
}
