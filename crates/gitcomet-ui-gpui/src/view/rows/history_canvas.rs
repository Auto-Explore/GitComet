use super::*;
use crate::kit::interaction_paint::InteractionPaint;
use crate::view::panes::HistoryRowHoverArea;
use gitcomet_state::msg::CommitSelectMode;
use gitcomet_ui_kit::text_layout::TextLayoutExt as _;
use gpui::{
    Bounds, ContentMask, CursorStyle, DispatchPhase, MouseButton, TruncateFrom, fill, point, px,
    size,
};
use palette::IntoColor;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::cell::RefCell;

const HISTORY_TAG_CHIP_HEIGHT_PX: f32 = 18.0;
const HISTORY_TAG_CHIP_PADDING_X_PX: f32 = 6.0;
/// Comfortable raises the history row; the ref chip and its padding follow so
/// the badge keeps its proportion.
const HISTORY_TAG_CHIP_COMFORTABLE_HEIGHT_PX: f32 = 24.0;
const HISTORY_TAG_CHIP_COMFORTABLE_PADDING_X_PX: f32 = 8.0;
const HISTORY_TAG_CHIP_GAP_PX: f32 = 4.0;
const HISTORY_BRANCH_CHIP_ICON_PX: f32 = 11.0;
const HISTORY_BRANCH_CHIP_COMBINED_ICON_PX: f32 = 16.0;
const HISTORY_BRANCH_CHIP_TEXT_ICON_GAP_PX: f32 = 3.0;
const HISTORY_BRANCH_CHIP_ICON_GAP_PX: f32 = 2.0;
const HISTORY_REF_CONNECTOR_DOT_PX: f32 = 1.5;
const HISTORY_REF_CONNECTOR_PITCH_PX: f32 = 4.0;

const HISTORY_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 8_192;

#[derive(Clone, Copy, Debug, PartialEq)]
struct HistoryCanvasColumnWidths {
    branch: Pixels,
    graph: Pixels,
    author: Pixels,
    date: Pixels,
    sha: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HistoryCanvasColumnVisibility {
    graph: bool,
    author: bool,
    date: bool,
    sha: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HistoryCanvasColumnLayout {
    branch: Bounds<Pixels>,
    graph: Bounds<Pixels>,
    summary: Bounds<Pixels>,
    author: Bounds<Pixels>,
    date: Bounds<Pixels>,
    sha: Bounds<Pixels>,
}

/// Resolves the canvas's hand-built column layout the same way GPUI resolves the
/// header and flex-row lengths: each padding/width is snapped independently to
/// the current display's device-pixel grid before the column edges accumulate.
///
/// Keeping this at paint time leaves the stored drag widths in logical pixels,
/// while ensuring fractional pointer positions cannot move the graph inside a
/// device pixel that the surrounding layout still treats as stationary.
fn history_canvas_column_layout(
    bounds: Bounds<Pixels>,
    horizontal_pad: Pixels,
    widths: HistoryCanvasColumnWidths,
    visibility: HistoryCanvasColumnVisibility,
    pixel_snap: impl Fn(Pixels) -> Pixels,
) -> HistoryCanvasColumnLayout {
    let snap_non_negative = |value: Pixels| pixel_snap(value.max(px(0.0)));
    let horizontal_pad = snap_non_negative(horizontal_pad);
    let widths = HistoryCanvasColumnWidths {
        branch: snap_non_negative(widths.branch),
        graph: snap_non_negative(widths.graph),
        author: snap_non_negative(widths.author),
        date: snap_non_negative(widths.date),
        sha: snap_non_negative(widths.sha),
    };
    let inner = Bounds::new(
        point(bounds.left() + horizontal_pad, bounds.top()),
        size(
            (bounds.size.width - horizontal_pad * 2.0).max(px(0.0)),
            bounds.size.height,
        ),
    );

    let mut x = inner.left();
    let branch = Bounds::new(
        point(x, bounds.top()),
        size(widths.branch, bounds.size.height),
    );
    x += widths.branch;
    let graph_width = if visibility.graph {
        widths.graph
    } else {
        px(0.0)
    };
    let graph = Bounds::new(
        point(x, bounds.top()),
        size(graph_width, bounds.size.height),
    );
    x += graph_width;

    let mut right_x = inner.right();
    let sha = if visibility.sha {
        right_x -= widths.sha;
        Bounds::new(
            point(right_x, bounds.top()),
            size(widths.sha, bounds.size.height),
        )
    } else {
        Bounds::new(
            point(right_x, bounds.top()),
            size(px(0.0), bounds.size.height),
        )
    };
    let date = if visibility.date {
        right_x -= widths.date;
        Bounds::new(
            point(right_x, bounds.top()),
            size(widths.date, bounds.size.height),
        )
    } else {
        Bounds::new(
            point(right_x, bounds.top()),
            size(px(0.0), bounds.size.height),
        )
    };
    let author = if visibility.author {
        right_x -= widths.author;
        Bounds::new(
            point(right_x, bounds.top()),
            size(widths.author, bounds.size.height),
        )
    } else {
        Bounds::new(
            point(right_x, bounds.top()),
            size(px(0.0), bounds.size.height),
        )
    };

    let summary_right = right_x.max(x);
    let summary = Bounds::new(
        point(x, bounds.top()),
        size((summary_right - x).max(px(0.0)), bounds.size.height),
    );

    HistoryCanvasColumnLayout {
        branch,
        graph,
        summary,
        author,
        date,
        sha,
    }
}

thread_local! {
    static HISTORY_TEXT_LAYOUT_CACHE: RefCell<FxLruCache<u64, gpui::ShapedLine>> =
        RefCell::new(new_fx_lru_cache(HISTORY_TEXT_LAYOUT_CACHE_MAX_ENTRIES));
}

fn shape_truncated_line_cached(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    text_hash: u64,
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
) -> gpui::ShapedLine {
    shape_truncated_line_cached_from(
        window,
        base_style,
        font_size,
        text,
        text_hash,
        max_width,
        color,
        font_family,
        TruncateFrom::End,
    )
}

#[allow(clippy::too_many_arguments)]
fn shape_truncated_line_cached_from(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    text_hash: u64,
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
    truncate_from: TruncateFrom,
) -> gpui::ShapedLine {
    shape_truncated_line_cached_from_with_affix(
        window,
        base_style,
        font_size,
        text,
        text_hash,
        max_width,
        color,
        font_family,
        truncate_from,
        "…",
        &[],
    )
}

/// [`shape_truncated_line_cached`] with find matches drawn in `highlights`.
#[allow(clippy::too_many_arguments)]
fn shape_highlighted_line_cached(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    text_hash: u64,
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
    highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
) -> gpui::ShapedLine {
    shape_truncated_line_cached_from_with_affix(
        window,
        base_style,
        font_size,
        text,
        text_hash,
        max_width,
        color,
        font_family,
        TruncateFrom::End,
        "…",
        highlights,
    )
}

#[allow(clippy::too_many_arguments)]
fn shape_clipped_chip_line_cached_from(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    text_hash: u64,
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
    truncate_from: TruncateFrom,
) -> gpui::ShapedLine {
    shape_truncated_line_cached_from_with_affix(
        window,
        base_style,
        font_size,
        text,
        text_hash,
        max_width,
        color,
        font_family,
        truncate_from,
        "",
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn shape_truncated_line_cached_from_with_affix(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    text_hash: u64,
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
    truncate_from: TruncateFrom,
    truncation_affix: &'static str,
    highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
) -> gpui::ShapedLine {
    use std::hash::{Hash, Hasher};

    let key = {
        let mut hasher = FxHasher::default();
        text_hash.hash(&mut hasher);
        max_width.hash(&mut hasher);
        font_size.hash(&mut hasher);
        base_style.font_weight.hash(&mut hasher);
        font_family
            .unwrap_or_else(|| base_style.font_family.as_ref())
            .hash(&mut hasher);
        color.red.to_bits().hash(&mut hasher);
        color.green.to_bits().hash(&mut hasher);
        color.blue.to_bits().hash(&mut hasher);
        color.alpha.to_bits().hash(&mut hasher);
        matches!(truncate_from, TruncateFrom::Start).hash(&mut hasher);
        truncation_affix.hash(&mut hasher);
        highlights.len().hash(&mut hasher);
        for (range, style) in highlights {
            range.hash(&mut hasher);
            crate::text_runs::hash_highlight_style(style, &mut hasher);
        }
        hasher.finish()
    };

    if let Some(shaped) =
        HISTORY_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned())
    {
        return shaped;
    }

    let mut style = base_style.clone();
    style.color = color.into_color();
    if let Some(family) = font_family {
        style.font_family = family.into();
    }
    let runs = crate::text_runs::text_runs_for_highlights(text, &style, highlights);
    let text_system = Arc::clone(window.text_system());
    let shaped = if highlights.is_empty() || truncate_from != TruncateFrom::End {
        let (truncated, runs) = gpui::TextLayout::truncate_line(
            text.clone(),
            font_size,
            max_width.max(px(0.0)),
            truncation_affix,
            &runs,
            truncate_from,
            window,
        );
        text_system.shape_line(truncated, font_size, runs.as_ref())
    } else {
        // The wrapper cuts by base-font widths, so bold matches would
        // overflow and be clipped mid-glyph; cut the shaped line instead.
        let full = text_system.shape_line(text.clone(), font_size, &runs);
        let affix = || {
            text_system
                .shape_line(
                    truncation_affix.into(),
                    font_size,
                    &[style.to_run(truncation_affix.len())],
                )
                .width
        };
        match highlighted_cut(full.width, max_width, affix, |x| full.index_for_x(x)) {
            None => full,
            Some(cut) => {
                // Like the wrapper, drop trailing blanks and punctuation.
                let prefix = text[..cut]
                    .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation());
                let kept: Vec<_> = highlights
                    .iter()
                    .filter_map(|(range, highlight)| {
                        let end = range.end.min(prefix.len());
                        (range.start < end).then_some((range.start..end, *highlight))
                    })
                    .collect();
                let mut runs = crate::text_runs::text_runs_for_highlights(prefix, &style, &kept);
                runs.push(style.to_run(truncation_affix.len()));
                text_system.shape_line(
                    format!("{prefix}{truncation_affix}").into(),
                    font_size,
                    &runs,
                )
            }
        }
    };

    HISTORY_TEXT_LAYOUT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, shaped.clone());
    });

    shaped
}

/// Where to cut a highlighted line so its shaped width, with the affix,
/// fits `max_width`; `None` when the whole line fits. `index_for_x` is the
/// shaped line's: the glyph under `x`, whose start is at or before it.
fn highlighted_cut(
    width: Pixels,
    max_width: Pixels,
    affix_width: impl FnOnce() -> Pixels,
    index_for_x: impl FnOnce(Pixels) -> Option<usize>,
) -> Option<usize> {
    if width <= max_width {
        return None;
    }
    let room = (max_width - affix_width()).max(px(0.0));
    Some(index_for_x(room).unwrap_or(0))
}

/// Ref identity is independent of lane/row selection and the menu-open state.
#[derive(Clone, Copy)]
enum HistoryChipStyleKind {
    Tag,
    Head { selected: bool },
    Branch { selected: bool },
}

struct HistoryChipVisual {
    border: gpui::Rgba,
    bg: gpui::Rgba,
    text: gpui::Rgba,
    selected: bool,
}

fn relative_luminance(color: gpui::Rgba) -> f32 {
    let linear_channel = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };

    0.2126 * linear_channel(color.red)
        + 0.7152 * linear_channel(color.green)
        + 0.0722 * linear_channel(color.blue)
}

fn contrast_ratio(a: gpui::Rgba, b: gpui::Rgba) -> f32 {
    let a = relative_luminance(a);
    let b = relative_luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Keeps the semantic foreground when it is readable on the actual chip fill.
/// A custom theme can provide valid solid/selection pairs that stop being valid
/// after the accent is reduced to a wash, so fall back to its strongest body
/// foreground instead of assuming either semantic token still fits.
fn readable_chip_foreground(
    theme: AppTheme,
    rendered_bg: gpui::Rgba,
    preferred: gpui::Rgba,
) -> gpui::Rgba {
    const MIN_TEXT_CONTRAST: f32 = 4.5;

    if contrast_ratio(preferred, rendered_bg) >= MIN_TEXT_CONTRAST {
        return preferred;
    }

    [
        theme.colors.foreground.primary,
        theme.colors.foreground.emphasis,
        theme.colors.accent.foreground,
        theme.colors.accent.on_solid,
        gpui::rgba(0x000000ff),
        gpui::rgba(0xffffffff),
    ]
    .into_iter()
    .max_by(|a, b| contrast_ratio(*a, rendered_bg).total_cmp(&contrast_ratio(*b, rendered_bg)))
    .unwrap_or(preferred)
}

/// Shared by chip fills, graph nodes, and their decorative connectors. The
/// selected lane is resolved once by the history view, including its lifetime;
/// a recycled palette colour on an unrelated row must not activate that row.
#[derive(Clone, Copy, Debug)]
pub(super) struct HistoryRowVisual {
    pub(super) lane: gpui::Rgba,
    pub(super) background: gpui::Rgba,
    pub(super) selected: bool,
    related_to_selection: Option<bool>,
}

impl HistoryRowVisual {
    pub(super) fn new(
        theme: AppTheme,
        row: &history_graph::GraphRow,
        row_ix: usize,
        selected_lane: Option<super::history_graph_paint::SelectedLane>,
        selected: bool,
        background: gpui::Rgba,
    ) -> Self {
        Self {
            lane: super::history_graph_paint::lane_wash_color(
                theme,
                row.node_color_ix,
                row_ix,
                selected_lane,
            ),
            background,
            selected,
            related_to_selection: selected_lane
                .map(|lane| lane.covers(theme, row_ix, row.node_color_ix)),
        }
    }

    fn refs_active(self) -> bool {
        self.selected || self.related_to_selection == Some(true)
    }
}

fn history_chip_visual(
    theme: AppTheme,
    row: HistoryRowVisual,
    kind: HistoryChipStyleKind,
    context_menu_open: bool,
) -> HistoryChipVisual {
    let selected = matches!(
        kind,
        HistoryChipStyleKind::Head { selected: true }
            | HistoryChipStyleKind::Branch { selected: true }
    );
    let active = row.refs_active() || selected;
    let bg = crate::theme::composite_over(
        row.background,
        with_alpha(row.lane, if active { 1.0 } else { 0.12 }),
    );
    let text = readable_chip_foreground(theme, bg, row.lane);
    HistoryChipVisual {
        border: if context_menu_open {
            crate::theme::mix_colors(text, row.lane, 0.35)
        } else if selected {
            readable_chip_foreground(theme, bg, theme.colors.foreground.emphasis)
        } else {
            crate::theme::composite_over(row.background, with_alpha(row.lane, 0.65))
        },
        bg,
        text,
        selected,
    }
}

/// Paint an opaque disc above the lane strokes, with cached, cap-height centred
/// initials. Case expansion can produce more than two glyphs (e.g. SSE); fit
/// those glyphs instead of replacing the author's identity with an ellipsis.
pub(super) fn paint_history_author_node(
    theme: AppTheme,
    bounds: Bounds<Pixels>,
    row: HistoryRowVisual,
    author: &HistoryAuthorVm,
    initials_reveal: f32,
    window: &mut Window,
) {
    let scale = ui_scale::UiScale::from_window(window).with_appearance(theme.metrics);
    let disc = crate::theme::composite_over(row.background, with_alpha(row.lane, 0.12));
    let background = crate::theme::mix_colors(row.lane, disc, initials_reveal);
    let foreground = readable_chip_foreground(theme, background, theme.colors.foreground.primary);
    super::history_shapes::paint_rounded_rect(
        bounds,
        bounds.size.width * 0.5,
        background,
        super::history_node::ring_width(scale, window.scale_factor()) * initials_reveal,
        row.lane,
        window,
    );
    let text_opacity = ((initials_reveal - 0.5) * 2.0).clamp(0.0, 1.0);
    if text_opacity == 0.0 {
        return;
    }
    paint_author_initials(
        bounds,
        author,
        scale.ui_text(9.0),
        scale.ui_text(16.0),
        foreground,
        text_opacity,
        window,
    );
}

fn paint_author_initials(
    bounds: Bounds<Pixels>,
    author: &HistoryAuthorVm,
    mut font: Pixels,
    available: Pixels,
    foreground: gpui::Rgba,
    opacity: f32,
    window: &mut Window,
) {
    let mut style = window.text_style();
    style.font_weight = gpui::FontWeight::SEMIBOLD;
    // Shape at full opacity: animation must not add a cache entry per frame.
    let mut shaped = shape_truncated_line_cached(
        window,
        &style,
        font,
        author.initials.shared(),
        author.initials.text_hash(),
        font * 16.0,
        gpui::rgba(0xffffffff),
        None,
    );
    // Keep glyph metrics stable while the dot expands beneath the text.
    if shaped.width > available && shaped.width > px(0.0) {
        font *= available / shaped.width;
        shaped = shape_truncated_line_cached(
            window,
            &style,
            font,
            author.initials.shared(),
            author.initials.text_hash(),
            font * 16.0,
            gpui::rgba(0xffffffff),
            None,
        );
    }
    let cap_height = shaped
        .paint_fragments
        .first()
        .map(|run| window.text_system().cap_height(run.font_id, font))
        .unwrap_or(font * 0.7);
    // Glyph positions stay cached; color and opacity are paint-only state.
    // These single-line initials have no decorations or inline elements.
    let origin = point(
        bounds.left() + (bounds.size.width - shaped.width).max(px(0.0)) * 0.5,
        bounds.center().y + cap_height * 0.5,
    );
    let color = with_alpha(foreground, opacity).into_color();
    for fragment in &shaped.paint_fragments {
        for glyph in &*fragment.glyphs {
            let position = origin + glyph.position;
            let _ = if glyph.is_emoji {
                window.paint_emoji_with_color(
                    position,
                    fragment.font_id,
                    glyph.id,
                    fragment.font_size,
                    color,
                )
            } else {
                window.paint_glyph(
                    position,
                    fragment.font_id,
                    glyph.id,
                    fragment.font_size,
                    color,
                )
            };
        }
    }
}

/// Whether one rendered ref is the branch selected in the sidebar. Comparison
/// is on ref identity, not the chip's label: a local and a remote branch of the
/// same name are different refs that can share a row, and label matching cannot
/// tell them apart. The checked-out branch matches through its attached-HEAD
/// ref item, whose visible label is now just the branch name.
fn history_ref_is_selected_branch(
    kind: &HistoryRefListItemKind,
    selected_branch: Option<&SelectedHistoryBranch>,
) -> bool {
    let Some(selected) = selected_branch else {
        return false;
    };
    match kind {
        HistoryRefListItemKind::AttachedHead { branch }
        | HistoryRefListItemKind::LocalBranch { name: branch } => {
            selected.target == BranchMenuTarget::local(branch)
        }
        HistoryRefListItemKind::RemoteBranch { remote, branch, .. } => {
            selected.target == BranchMenuTarget::remote(remote, branch)
        }
        HistoryRefListItemKind::Tag { .. } | HistoryRefListItemKind::DetachedHead => false,
    }
}

/// Whether this row is the tip of the branch selected in the sidebar.
///
/// Deliberately derived from the row's own refs rather than from
/// `selected_branch.is_some()`: the sidebar's pick outlives the reveal, so
/// after picking a branch and then clicking some other commit, the newly
/// selected row would otherwise claim the branch too.
fn history_row_is_selected_branch_tip(
    ref_items: &[HistoryRefListItem],
    selected_branch: Option<&SelectedHistoryBranch>,
) -> bool {
    ref_items
        .iter()
        .any(|item| history_ref_is_selected_branch(&item.kind, selected_branch))
}

/// How far an unrelated row's lane colour is pushed toward muted, so the message
/// border and the graph-column fade recede with the text instead of staying
/// fully saturated on rows that have nothing to do with the selection.
const UNRELATED_LANE_COLOR_MIX: f32 = 0.75;

/// The commit summary carries the history's own selection.
///
/// `related_to_selection` is `None` when no single commit is selected, and only
/// then does the row render as ordinary body text. While a commit *is* selected
/// the column splits in two: every row the selected commit's own graph lane runs
/// through goes to the theme's emphasis foreground, and everything else drops to
/// muted -- so that lane reads as a continuous run down the list.
///
/// A lane, not an ancestry walk: a merge's second parent lives on a lane of its
/// own and washes out with the rest, even though the commit is genuinely an
/// ancestor. That is what the graph draws, so it is what the highlight follows.
///
/// A row background tint was tried first and was far too intrusive: it washed
/// most of the list and fought with the table's own shading for selection, HEAD
/// and hover. Moving only the message text leaves all of that legible underneath.
///
/// The revealed branch tip keeps its branch colour when nothing is selected, so
/// it still reads while the relation cache has not caught up.
fn history_summary_color(
    theme: AppTheme,
    is_selected_branch_tip: bool,
    related_to_selection: Option<bool>,
) -> gpui::Rgba {
    match related_to_selection {
        Some(true) => full_contrast_text(theme),
        // All the way to muted: against pure white/black on the related rows,
        // anything short of this left the two too close to separate at a glance.
        Some(false) => theme.colors.foreground.secondary,
        None if is_selected_branch_tip => full_contrast_text(theme),
        None => theme.colors.foreground.primary,
    }
}

/// Summary colour for a row, given only its relation to the selection. The
/// working-tree summary row has no refs, so it needs the relation rule without
/// the branch-tip case.
pub(super) fn selection_related_summary_color(
    theme: AppTheme,
    related_to_selection: Option<bool>,
) -> gpui::Rgba {
    history_summary_color(theme, false, related_to_selection)
}

/// Lane colour muted to match an unrelated row, shared with the working-tree
/// summary row so its connector recedes with the history it joins.
pub(super) fn selection_related_lane_color(
    theme: AppTheme,
    lane_color: gpui::Rgba,
    related_to_selection: Option<bool>,
) -> gpui::Rgba {
    if related_to_selection == Some(false) {
        crate::theme::mix_colors(
            lane_color,
            theme.colors.foreground.secondary,
            UNRELATED_LANE_COLOR_MIX,
        )
    } else {
        lane_color
    }
}

/// The theme's highest-contrast text against the list background.
///
/// This is what `foreground.emphasis` already means, and the bundled themes set
/// it to exactly the pure white / pure black this used to hardcode -- so the
/// built-in themes render identically, while a custom theme that deliberately
/// softens its emphasis colour is no longer overridden here.
fn full_contrast_text(theme: AppTheme) -> gpui::Rgba {
    theme.colors.foreground.emphasis
}

fn history_branch_chip_style_kind(
    chip: &HistoryBranchChipVm,
    selected_branch: Option<&SelectedHistoryBranch>,
) -> HistoryChipStyleKind {
    match &chip.kind {
        HistoryBranchChipKind::DetachedHead => HistoryChipStyleKind::Head { selected: false },
        HistoryBranchChipKind::Branch { targets, is_head } => {
            let selected = selected_branch
                .is_some_and(|selected| targets.iter().any(|target| target == &selected.target));
            if *is_head {
                HistoryChipStyleKind::Head { selected }
            } else {
                HistoryChipStyleKind::Branch { selected }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryBranchChipIcon {
    Local,
    Remote,
    LocalRemote,
    Tag,
    Check,
}

impl HistoryBranchChipIcon {
    fn path(self) -> &'static str {
        match self {
            Self::Local => "icons/computer.svg",
            Self::Remote => "icons/cloud.svg",
            Self::LocalRemote => "icons/computer-cloud-background.svg",
            Self::Tag => "icons/tag.svg",
            Self::Check => "icons/check.svg",
        }
    }

    fn foreground_path(self) -> Option<&'static str> {
        match self {
            Self::LocalRemote => Some("icons/computer-cloud-foreground.svg"),
            Self::Local | Self::Remote | Self::Tag | Self::Check => None,
        }
    }

    fn size(self, icon_size: Pixels, combined_icon_size: Pixels) -> Pixels {
        match self {
            Self::Local | Self::Remote | Self::Tag | Self::Check => icon_size,
            Self::LocalRemote => combined_icon_size,
        }
    }
}

fn history_branch_chip_icons(chip: &HistoryBranchChipVm) -> SmallVec<[HistoryBranchChipIcon; 2]> {
    let HistoryBranchChipKind::Branch { targets, is_head } = &chip.kind else {
        return smallvec::smallvec![HistoryBranchChipIcon::Check];
    };
    let has_local = targets
        .iter()
        .any(|target| target.section() == BranchSection::Local);
    let has_remote = targets
        .iter()
        .any(|target| target.section() == BranchSection::Remote);
    let mut icons = SmallVec::new();
    if *is_head {
        icons.push(HistoryBranchChipIcon::Check);
    }
    if has_local && has_remote {
        icons.push(HistoryBranchChipIcon::LocalRemote);
    } else if has_local {
        icons.push(HistoryBranchChipIcon::Local);
    } else if has_remote {
        icons.push(HistoryBranchChipIcon::Remote);
    }
    icons
}

fn history_branch_chip_icon_width(
    icons: &[HistoryBranchChipIcon],
    icon_size: Pixels,
    combined_icon_size: Pixels,
    text_icon_gap: Pixels,
    icon_gap: Pixels,
) -> Pixels {
    if icons.is_empty() {
        return px(0.0);
    }
    text_icon_gap
        + icons
            .iter()
            .map(|icon| icon.size(icon_size, combined_icon_size))
            .fold(px(0.0), |width, icon_width| width + icon_width)
        + icon_gap * icons.len().saturating_sub(1) as f32
}

#[allow(clippy::too_many_arguments)]
fn paint_history_chip(
    window: &mut Window,
    cx: &mut gpui::App,
    chip_bounds: Bounds<Pixels>,
    visual: &HistoryChipVisual,
    shaped: &gpui::ShapedLine,
    radius: Pixels,
    border_w: Pixels,
    pad_x: Pixels,
    line_height: Pixels,
    icons: &[HistoryBranchChipIcon],
    icon_size: Pixels,
    combined_icon_size: Pixels,
    text_icon_gap: Pixels,
    icon_gap: Pixels,
) {
    let border_w = if visual.selected {
        border_w * 2.0
    } else {
        border_w
    };
    super::history_shapes::paint_rounded_rect(
        chip_bounds,
        radius,
        visual.bg,
        border_w,
        visual.border,
        window,
    );

    // Center the line box on the chip even when it is taller than the chip
    // (clamping the offset to zero pushed the glyphs visibly below center).
    let text_y = chip_bounds.top() + (chip_bounds.size.height - line_height) * 0.5;
    let _ = shaped.paint(
        point(chip_bounds.left() + pad_x, text_y),
        line_height,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );

    let mut icon_x = chip_bounds.left() + pad_x + shaped.width;
    if !icons.is_empty() {
        icon_x += text_icon_gap;
    }
    for (ix, icon) in icons.iter().enumerate() {
        if ix > 0 {
            icon_x += icon_gap;
        }
        let painted_icon_size = icon.size(icon_size, combined_icon_size);
        let cell = Bounds::new(
            point(icon_x, chip_bounds.top()),
            size(painted_icon_size, chip_bounds.size.height),
        );
        super::diff_canvas::paint_centered_svg_icon(
            icon.path(),
            cell,
            painted_icon_size,
            visual.text,
            window,
            cx,
        );
        if let Some(foreground_path) = icon.foreground_path() {
            super::diff_canvas::paint_centered_svg_icon(
                foreground_path,
                cell,
                painted_icon_size,
                visual.text,
                window,
                cx,
            );
        }
        icon_x += painted_icon_size;
    }
}

fn fx_hash_str(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

fn history_tag_chip_menu_invoker(
    repo_id: RepoId,
    commit_id: &CommitId,
    tag_name: &str,
) -> SharedString {
    format!(
        "history_tag_chip_menu_{}_{}_{}",
        repo_id.0,
        commit_id.as_ref(),
        tag_name
    )
    .into()
}

fn history_branch_chip_menu_invoker(
    repo_id: RepoId,
    commit_id: &CommitId,
    chip: &HistoryBranchChipVm,
) -> SharedString {
    format!(
        "history_branch_chip_menu_{}_{}_{}",
        repo_id.0,
        commit_id.as_ref(),
        chip.text.as_ref()
    )
    .into()
}

struct HistoryRefChipPaint {
    bounds: Bounds<Pixels>,
    visual: HistoryChipVisual,
    shaped: gpui::ShapedLine,
    icons: SmallVec<[HistoryBranchChipIcon; 2]>,
}

#[derive(Default)]
struct HistoryChipMetrics {
    radius: Pixels,
    border: Pixels,
    pad_x: Pixels,
    line_height: Pixels,
    icon_size: Pixels,
    combined_icon_size: Pixels,
    text_icon_gap: Pixels,
    icon_gap: Pixels,
}

struct HistoryRefsPaintLayout {
    bounds: Bounds<Pixels>,
    content_bounds: Bounds<Pixels>,
    chips: SmallVec<[HistoryRefChipPaint; 4]>,
    metrics: HistoryChipMetrics,
    #[cfg(test)]
    shown: usize,
    #[cfg(test)]
    hidden: usize,
}

// Includes every chip and overflow badge; the caller reserves the final gap.
fn history_inline_refs_max_width(message_width: Pixels, gap: Pixels) -> Pixels {
    (message_width / 3.0 - gap).max(px(0.0))
}

fn history_message_after_inline_refs(
    content: Bounds<Pixels>,
    refs_width: Pixels,
    gap: Pixels,
) -> Bounds<Pixels> {
    let prefix = if refs_width > px(0.0) {
        (refs_width + gap).min(content.size.width / 3.0)
    } else {
        px(0.0)
    };
    Bounds::new(
        point(content.left() + prefix, content.top()),
        size(
            (content.size.width - prefix).max(px(0.0)),
            content.size.height,
        ),
    )
}

#[allow(clippy::too_many_arguments)]
fn layout_history_ref_chips(
    theme: AppTheme,
    row_visual: HistoryRowVisual,
    content_bounds: Bounds<Pixels>,
    repo_id: RepoId,
    commit_id: &CommitId,
    tag_names: &[HistoryTextVm],
    branch_chips: &[HistoryBranchChipVm],
    selected_branch: Option<&SelectedHistoryBranch>,
    active_context_menu_invoker: Option<&SharedString>,
    base_style: &gpui::TextStyle,
    xxs_font: Pixels,
    xxs_line_height: Pixels,
    window: &mut Window,
) -> HistoryRefsPaintLayout {
    if tag_names.is_empty() && branch_chips.is_empty() {
        return HistoryRefsPaintLayout {
            bounds: Bounds::new(
                content_bounds.origin,
                size(px(0.0), content_bounds.size.height),
            ),
            chips: SmallVec::new(),
            content_bounds,
            metrics: HistoryChipMetrics::default(),
            #[cfg(test)]
            shown: 0,
            #[cfg(test)]
            hidden: 0,
        };
    }
    let scaled_px = ui_scale::scaler(ui_scale::UiScale::from_window(window));
    let chip_height = scaled_px(theme.metrics.row_height(
        HISTORY_TAG_CHIP_HEIGHT_PX,
        HISTORY_TAG_CHIP_COMFORTABLE_HEIGHT_PX,
    ));
    let chip_pad_x = scaled_px(theme.metrics.ramp(
        HISTORY_TAG_CHIP_PADDING_X_PX,
        HISTORY_TAG_CHIP_COMFORTABLE_PADDING_X_PX,
    ));
    let chip_gap = scaled_px(HISTORY_TAG_CHIP_GAP_PX);
    // paint_quad has no layout-level radius clamping, so a
    // pill radius (999) must be capped to half the height.
    let chip_radius = px(theme.radii.pill).min(chip_height * 0.5);
    let chip_border_w = scaled_px(1.0);
    let branch_icon_size = scaled_px(HISTORY_BRANCH_CHIP_ICON_PX);
    let branch_combined_icon_size = scaled_px(HISTORY_BRANCH_CHIP_COMBINED_ICON_PX);
    let branch_text_icon_gap = scaled_px(HISTORY_BRANCH_CHIP_TEXT_ICON_GAP_PX);
    let branch_icon_gap = scaled_px(HISTORY_BRANCH_CHIP_ICON_GAP_PX);
    let chip_y =
        content_bounds.top() + (content_bounds.size.height - chip_height).max(px(0.0)) * 0.5;
    let min_text_w = scaled_px(12.0);
    let total_chips = tag_names.len() + branch_chips.len();

    // Reserved width for a trailing "+N" chip; sized for the
    // worst-case count so mid-loop reservations never come up short.
    let overflow_reserve = if total_chips > 1 {
        let probe: SharedString = format!("+{}", total_chips - 1).into();
        let shaped = shape_truncated_line_cached(
            window,
            base_style,
            xxs_font,
            &probe,
            fx_hash_str(probe.as_ref()),
            content_bounds.size.width,
            theme.colors.foreground.secondary,
            None,
        );
        shaped.width + chip_pad_x * 2.0 + chip_gap
    } else {
        px(0.0)
    };

    let mut chips = SmallVec::new();
    let mut head_style = None;
    let mut x = content_bounds.left();
    let mut shown = 0usize;

    enum ChipEntry<'a> {
        Tag(&'a HistoryTextVm),
        Branch(&'a HistoryBranchChipVm),
    }
    // HEAD first (the strongest signal), then tags, then
    // plain branches; overflow beyond the column collapses
    // into a "+N" chip resolved by the refs hover menu.
    let head_entries = branch_chips
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                HistoryBranchChipKind::Branch { is_head: true, .. }
                    | HistoryBranchChipKind::DetachedHead
            )
        })
        .map(ChipEntry::Branch);
    let branch_entries = branch_chips
        .iter()
        .filter(|item| {
            matches!(
                item.kind,
                HistoryBranchChipKind::Branch { is_head: false, .. }
            )
        })
        .map(ChipEntry::Branch);
    let entries = head_entries
        .chain(tag_names.iter().map(ChipEntry::Tag))
        .chain(branch_entries);

    for entry in entries {
        let pending_after = total_chips - shown - 1;
        let reserve = if pending_after > 0 {
            overflow_reserve
        } else {
            px(0.0)
        };
        let icons = match &entry {
            ChipEntry::Tag(_) => smallvec::smallvec![HistoryBranchChipIcon::Tag],
            ChipEntry::Branch(chip) => history_branch_chip_icons(chip),
        };
        let icons_width = history_branch_chip_icon_width(
            icons.as_slice(),
            branch_icon_size,
            branch_combined_icon_size,
            branch_text_icon_gap,
            branch_icon_gap,
        );
        let max_text_w = content_bounds.right() - x - reserve - chip_pad_x * 2.0 - icons_width;
        // Later chips need enough room to be legible;
        // otherwise fold the remainder into the "+N" chip
        // instead of painting an "…x" stub.
        let needed_text_w = if shown == 0 {
            min_text_w
        } else {
            scaled_px(28.0)
        };
        if max_text_w < needed_text_w {
            break;
        }

        let style_kind = match &entry {
            ChipEntry::Tag(_) => HistoryChipStyleKind::Tag,
            ChipEntry::Branch(chip) => history_branch_chip_style_kind(chip, selected_branch),
        };
        let context_menu_open = active_context_menu_invoker.is_some_and(|active| match &entry {
            ChipEntry::Tag(name) => {
                active == &history_tag_chip_menu_invoker(repo_id, commit_id, name.as_ref())
            }
            ChipEntry::Branch(chip) => {
                active == &history_branch_chip_menu_invoker(repo_id, commit_id, chip)
            }
        });
        let visual = history_chip_visual(theme, row_visual, style_kind, context_menu_open);
        let shaped = match &entry {
            ChipEntry::Tag(name) => shape_clipped_chip_line_cached_from(
                window,
                base_style,
                xxs_font,
                name.shared(),
                name.text_hash(),
                max_text_w,
                visual.text,
                None,
                TruncateFrom::End,
            ),
            ChipEntry::Branch(chip) => {
                // Clip from the start so the leaf segment
                // ("feature_name") stays visible without
                // spending chip width on an ellipsis.
                shape_clipped_chip_line_cached_from(
                    window,
                    if matches!(style_kind, HistoryChipStyleKind::Head { .. }) {
                        head_style.get_or_insert_with(|| {
                            let mut style = base_style.clone();
                            style.font_weight = gpui::FontWeight::BOLD;
                            style
                        })
                    } else {
                        base_style
                    },
                    xxs_font,
                    chip.text.shared(),
                    chip.text.text_hash(),
                    max_text_w,
                    visual.text,
                    None,
                    TruncateFrom::Start,
                )
            }
        };

        let chip_w = shaped.width + icons_width + chip_pad_x * 2.0;
        let chip_bounds = Bounds::new(point(x, chip_y), size(chip_w, chip_height));
        chips.push(HistoryRefChipPaint {
            bounds: chip_bounds,
            visual,
            shaped,
            icons,
        });
        shown += 1;
        x += chip_w + chip_gap;
    }

    let hidden = total_chips - shown;
    let mut occupied_right = (x - chip_gap).max(content_bounds.left());
    // Shown chips reserve room for it. Without any, a clipped "+N" is still
    // the row's only refs marker and its inline hover target.
    if hidden > 0 {
        let visual = history_chip_visual(
            theme,
            row_visual,
            HistoryChipStyleKind::Branch { selected: false },
            false,
        );
        let label: SharedString = format!("+{hidden}").into();
        let shaped = shape_truncated_line_cached(
            window,
            base_style,
            xxs_font,
            &label,
            fx_hash_str(label.as_ref()),
            (content_bounds.right() - x - chip_pad_x * 2.0).max(px(0.0)),
            visual.text,
            None,
        );
        let chip_bounds = Bounds::new(
            point(x, chip_y),
            size(shaped.width + chip_pad_x * 2.0, chip_height),
        );
        occupied_right = chip_bounds.right();

        chips.push(HistoryRefChipPaint {
            bounds: chip_bounds,
            visual,
            shaped,
            icons: SmallVec::new(),
        });
    }
    HistoryRefsPaintLayout {
        content_bounds,
        chips,
        metrics: HistoryChipMetrics {
            radius: chip_radius,
            border: chip_border_w,
            pad_x: chip_pad_x,
            line_height: xxs_line_height,
            icon_size: branch_icon_size,
            combined_icon_size: branch_combined_icon_size,
            text_icon_gap: branch_text_icon_gap,
            icon_gap: branch_icon_gap,
        },
        bounds: Bounds::new(
            content_bounds.origin,
            size(
                (occupied_right.min(content_bounds.right()) - content_bounds.left()).max(px(0.0)),
                content_bounds.size.height,
            ),
        ),
        #[cfg(test)]
        shown,
        #[cfg(test)]
        hidden,
    }
}

/// Paint the exact layout used to attach the connector and size hover regions;
/// neither text shaping nor overflow decisions are repeated in this pass.
fn paint_history_ref_chips(layout: &HistoryRefsPaintLayout, window: &mut Window, cx: &mut App) {
    if layout.chips.is_empty() {
        return;
    }
    let m = &layout.metrics;
    window.with_content_mask(
        Some(ContentMask {
            bounds: layout.content_bounds,
            ..Default::default()
        }),
        |window| {
            for chip in &layout.chips {
                paint_history_chip(
                    window,
                    cx,
                    chip.bounds,
                    &chip.visual,
                    &chip.shaped,
                    m.radius,
                    m.border,
                    m.pad_x,
                    m.line_height,
                    &chip.icons,
                    m.icon_size,
                    m.combined_icon_size,
                    m.text_icon_gap,
                    m.icon_gap,
                );
            }
        },
    );
}

/// Leaders stop at the facing edge of the node in either ref layout. They are
/// decoration, not hit targets or topology: crossing a lane adds no junction.
fn history_ref_connector_bounds(
    mode: HistoryBranchNamesMode,
    refs: Bounds<Pixels>,
    node: Bounds<Pixels>,
    gap: Pixels,
    stroke: Pixels,
) -> Option<Bounds<Pixels>> {
    if refs.size.width <= px(0.0) {
        return None;
    }
    let (left, right) = match mode {
        HistoryBranchNamesMode::SeparateColumn => (refs.right() + gap, node.left()),
        HistoryBranchNamesMode::Inline => (node.right(), refs.left() - gap),
    };
    (right > left).then(|| {
        Bounds::new(
            point(left, node.center().y - stroke * 0.5),
            size(right - left, stroke),
        )
    })
}

fn history_ref_connector_dots(
    mode: HistoryBranchNamesMode,
    bounds: Bounds<Pixels>,
    leading_annotation: Bounds<Pixels>,
    pitch: Pixels,
) -> impl Iterator<Item = Bounds<Pixels>> {
    let diameter = bounds.size.height;
    let count = if diameter > px(0.0) && pitch >= diameter && bounds.size.width >= diameter {
        (f32::from(bounds.size.width - diameter) / f32::from(pitch)).floor() as usize + 1
    } else {
        0
    };
    // Inline refs may follow an extension's leading mark. Leave its reserved
    // width clear without restarting the dot pattern across the interruption.
    // Start at the node in both layouts; only full dots fit inside the leader.
    (0..count).filter_map(move |index| {
        let offset = pitch * index as f32;
        let left = match mode {
            HistoryBranchNamesMode::SeparateColumn => bounds.right() - diameter - offset,
            HistoryBranchNamesMode::Inline => bounds.left() + offset,
        };
        let dot = Bounds::new(point(left, bounds.top()), size(diameter, diameter));
        let overlaps_annotation = leading_annotation.size.width > px(0.0)
            && dot.left() < leading_annotation.right()
            && dot.right() > leading_annotation.left();
        (!overlaps_annotation).then_some(dot)
    })
}

fn paint_history_ref_connector(
    mode: HistoryBranchNamesMode,
    bounds: Bounds<Pixels>,
    leading_annotation: Bounds<Pixels>,
    color: gpui::Rgba,
    window: &mut Window,
) {
    let pitch = ui_scale::UiScale::from_window(window).px(HISTORY_REF_CONNECTOR_PITCH_PX);
    for dot in history_ref_connector_dots(mode, bounds, leading_annotation, pitch) {
        super::history_shapes::paint_disc(dot, color, window);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn history_commit_row_canvas(
    theme: AppTheme,
    view: Entity<HistoryView>,
    branch_names: HistoryBranchNamesMode,
    row_id: usize,
    repo_id: RepoId,
    commit_id: CommitId,
    col_branch: Pixels,
    col_graph: Pixels,
    col_author: Pixels,
    col_date: Pixels,
    col_sha: Pixels,
    show_graph: bool,
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    show_graph_color_marker: bool,
    is_stash_node: bool,
    selected: bool,
    connect_from_top_col: Option<usize>,
    graph_rows: Arc<[history_graph::GraphRow]>,
    graph_row_ix: usize,
    tag_names: Arc<[HistoryTextVm]>,
    branch_chips: Arc<[HistoryBranchChipVm]>,
    ref_items: Arc<[HistoryRefListItem]>,
    selected_branch: Option<SelectedHistoryBranch>,
    selected_lane: Option<super::history_graph_paint::SelectedLane>,
    lane_branch_name: Option<SharedString>,
    author: HistoryAuthorVm,
    author_run_interior: bool,
    summary: HistoryTextVm,
    when: HistoryTextVm,
    // Raw commit time, so the date cell's tooltip can be rendered in full
    // only when someone actually hovers it.
    commit_time: std::time::SystemTime,
    short_sha: HistoryTextVm,
    find_highlights: Option<crate::view::panes::history::find::HistoryFindHighlights>,
    active_context_menu_invoker: Option<SharedString>,
    // The same resolver paints the row and its graph-node cutouts.
    row_paint: InteractionPaint,
    annotation: gitcomet_extension_api::HistoryRowAnnotation,
) -> AnyElement {
    let interaction = row_paint.clone();
    let prepaint_interaction = row_paint.clone();
    super::canvas::keyed_canvas_with_id(
        "history_commit_row_canvas",
        move |_, _, _| {
            prepaint_interaction
                .canvas_hitbox()
                .expect("canvas interaction prepaint")
        },
        move |bounds, hitbox, element_id, window, cx| {
            let Some(graph_row) = graph_rows.get(graph_row_ix) else {
                return;
            };
            let mut row_background = row_paint.background(theme.colors.surface.canvas, window);
            // Purple highlight on the commit being browsed historically, unless
            // it is the selected row: its selection highlight already says so.
            if view.read(cx).active_repo().is_some_and(|repo| {
                repo.browsing_commit() == Some(&commit_id)
                    && repo.history_state.selected_commit.as_ref() != Some(&commit_id)
            }) {
                let tint =
                    crate::theme::with_alpha(crate::theme::historical_outline(theme.is_dark), 0.22);
                window.paint_quad(fill(bounds, tint));
                row_background = crate::theme::composite_over(row_background, tint);
            }
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);

            let is_selected_branch_tip =
                history_row_is_selected_branch_tip(&ref_items, selected_branch.as_ref());

            let scale = ui_scale::UiScale::from_window(window).with_appearance(theme.metrics);
            let scaled_px = ui_scale::scaler(scale);
            let base_style = window.text_style();
            let sm_font = base_style.font_size.to_pixels(window.rem_size());
            let sm_line_height = base_style
                .line_height
                .to_pixels(sm_font.into(), window.rem_size());
            let xs_font = sm_font * 0.86;
            let xs_line_height = base_style
                .line_height
                .to_pixels(xs_font.into(), window.rem_size());
            let xxs_font = sm_font * 0.78;
            let xxs_line_height = base_style
                .line_height
                .to_pixels(xxs_font.into(), window.rem_size());
            let cell_pad_x = scaled_px(HISTORY_COL_HANDLE_PX / 2.0);
            // Find matches look like the sidebar's search matches.
            let find_match_style = gpui::HighlightStyle {
                color: Some(theme.colors.accent.foreground.into_color()),
                font_weight: Some(gpui::FontWeight::BOLD),
                ..gpui::HighlightStyle::default()
            };
            let find_match_runs = |ranges: &[std::ops::Range<usize>]| -> Vec<_> {
                ranges
                    .iter()
                    .map(|range| (range.clone(), find_match_style))
                    .collect()
            };
            let (summary_matches, author_matches, sha_matches) = find_highlights
                .as_ref()
                .map(|found| {
                    let sha = (found.sha > 0).then_some((0..found.sha, find_match_style));
                    (
                        find_match_runs(&found.summary),
                        find_match_runs(&found.author),
                        sha.into_iter().collect::<Vec<_>>(),
                    )
                })
                .unwrap_or_default();

            let center_y = |line_height: Pixels| {
                let extra = (bounds.size.height - line_height).max(px(0.0));
                bounds.top() + extra * 0.5
            };

            if let Some(range) = &annotation.range {
                let color: gpui::Rgba = range.color.into_color();
                let start = if range.starts {
                    bounds.size.height * 0.5
                } else {
                    px(0.0)
                };
                let end = if range.ends {
                    bounds.size.height * 0.5
                } else {
                    bounds.size.height
                };
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), bounds.top() + start),
                        size(px(3.0), (end - start).max(px(2.0))),
                    ),
                    color,
                ));
            }

            let column_layout = history_canvas_column_layout(
                bounds,
                window.rem_size() * 0.5,
                HistoryCanvasColumnWidths {
                    branch: col_branch,
                    graph: col_graph,
                    author: col_author,
                    date: col_date,
                    sha: col_sha,
                },
                HistoryCanvasColumnVisibility {
                    graph: show_graph,
                    author: show_author,
                    date: show_date,
                    sha: show_sha,
                },
                |value| window.pixel_snap(value),
            );
            let branch_bounds = column_layout.branch;
            let graph_bounds = column_layout.graph;
            let mut summary_bounds = column_layout.summary;
            let author_bounds = column_layout.author;
            let date_bounds = column_layout.date;
            let sha_bounds = column_layout.sha;
            let selection = if show_graph {
                super::history_node::selection_progress(element_id, selected, window, cx)
            } else {
                0.0
            };
            let initials_reveal = if author_run_interior
                && branch_chips.is_empty()
                && tag_names.is_empty()
                && connect_from_top_col.is_none()
            {
                selection
            } else {
                1.0
            };
            let graph_grid =
                show_graph.then(|| super::history_graph_paint::GraphGrid::new(window, scale));
            let graph_node = graph_grid.map(|grid| {
                super::history_graph_paint::graph_node_bounds(
                    graph_row,
                    is_stash_node,
                    initials_reveal,
                    graph_bounds,
                    grid,
                    scale,
                )
            });
            // Dots retain the same generous author tooltip target as discs.
            let author_hover_bounds = graph_node.map(|node| {
                let diameter = super::history_node::author_diameter(scale, 1.0);
                Bounds::new(
                    point(
                        node.center().x - diameter * 0.5,
                        node.center().y - diameter * 0.5,
                    ),
                    size(diameter, diameter),
                )
            });

            // Marks share the existing summary canvas, reserving their width so
            // neither the commit subject nor adjacent columns are obscured. A
            // text glyph shapes with the label; an icon glyph leads it.
            for (leading, mark) in [
                (true, annotation.leading.as_ref()),
                (false, annotation.trailing.as_ref()),
            ] {
                let Some(mark) = mark else {
                    continue;
                };
                let color: gpui::Rgba = mark.color.into_color();
                let (icon, text) = match &mark.glyph {
                    Some(gitcomet_extension_api::RowGlyph::Icon(path)) => {
                        (Some(path), mark.label.clone())
                    }
                    Some(gitcomet_extension_api::RowGlyph::Text(glyph)) => (
                        None,
                        Some(match &mark.label {
                            Some(label) => format!("{glyph} {label}").into(),
                            None => glyph.clone(),
                        }),
                    ),
                    _ => (None, mark.label.clone()),
                };
                let icon_size = xxs_line_height.min(bounds.size.height);
                let icon_width = if icon.is_some() {
                    icon_size + scaled_px(2.0)
                } else {
                    px(0.0)
                };
                let available = (summary_bounds.size.width * 0.3).min(scaled_px(96.0));
                let text = text.map(|text| {
                    let mut hasher = FxHasher::default();
                    text.hash(&mut hasher);
                    shape_truncated_line_cached(
                        window,
                        &base_style,
                        xxs_font,
                        &text,
                        hasher.finish(),
                        (available - icon_width).max(px(0.0)),
                        color,
                        None,
                    )
                });
                let content = icon_width + text.as_ref().map_or(px(0.0), |text| text.width);
                if content <= px(0.0) {
                    continue;
                }
                let width = (content + scaled_px(8.0)).min(summary_bounds.size.width);
                let x = if leading {
                    summary_bounds.left() + scaled_px(4.0)
                } else {
                    summary_bounds.right() - width
                };
                if let Some(path) = icon {
                    super::diff_canvas::paint_centered_svg_icon(
                        path.clone(),
                        Bounds::new(point(x, center_y(icon_size)), size(icon_size, icon_size)),
                        icon_size,
                        color,
                        window,
                        cx,
                    );
                }
                if let Some(text) = text {
                    let _ = text.paint(
                        point(x + icon_width, center_y(xxs_line_height)),
                        xxs_line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
                summary_bounds.size.width -= width;
                if leading {
                    summary_bounds.origin.x += width;
                }
            }

            // Everything coloured from this row's lane -- the node, the
            // message border, the fade wash and the hover badge -- washes with
            // that lane, so a row never shows two different strengths of the
            // same colour.
            let row_visual = HistoryRowVisual::new(
                theme,
                graph_row,
                graph_row_ix,
                selected_lane,
                selected,
                row_background,
            );
            let related_to_selection = row_visual.related_to_selection;
            let node_color = row_visual.lane;

            // A lane-coloured wash across the right of the graph column, fading
            // into the border on the message cell so a commit's dot and its
            // message read as one unit. Painted before the lanes so the strokes
            // stay crisp on top of it, and a single gradient quad either way.
            //
            // `graph_bounds.right()` is exactly `summary_bounds.left()`, so the
            // gradient ends where the border begins.
            if show_graph && show_graph_color_marker {
                super::history_graph_paint::paint_graph_fade(
                    node_color,
                    graph_bounds,
                    scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX),
                    window,
                );
            }

            let mut summary_text_left =
                summary_bounds.left() + scaled_px(history_message_text_left_px(false));
            if show_graph_color_marker {
                // A lane-coloured border down the left edge of the message cell,
                // where the graph column's fade lands. Full row height, so it
                // changes colour row by row like the graph line beside it.
                let border_w = scaled_px(HISTORY_MESSAGE_BORDER_W_PX);
                window.paint_quad(fill(
                    Bounds::new(
                        point(summary_bounds.left(), bounds.top()),
                        size(border_w, bounds.size.height),
                    ),
                    node_color,
                ));
                summary_text_left =
                    summary_bounds.left() + scaled_px(history_message_text_left_px(true));
            }

            let summary_content_bounds = Bounds::new(
                point(summary_text_left, bounds.top()),
                size(
                    (summary_bounds.right() - cell_pad_x - summary_text_left).max(px(0.0)),
                    bounds.size.height,
                ),
            );
            let chip_gap = scaled_px(HISTORY_TAG_CHIP_GAP_PX);

            let branch_content_bounds = if branch_names == HistoryBranchNamesMode::Inline {
                Bounds::new(
                    summary_content_bounds.origin,
                    size(
                        history_inline_refs_max_width(summary_content_bounds.size.width, chip_gap),
                        summary_content_bounds.size.height,
                    ),
                )
            } else {
                Bounds::new(
                    point(branch_bounds.left() + cell_pad_x, branch_bounds.top()),
                    size(
                        (branch_bounds.size.width
                            - cell_pad_x * 2.0
                            - if show_graph && (!branch_chips.is_empty() || !tag_names.is_empty()) {
                                scaled_px(12.0)
                            } else {
                                px(0.0)
                            })
                        .max(px(0.0)),
                        branch_bounds.size.height,
                    ),
                )
            };

            let branch_ref_count = branch_chips.len();
            // While the row is hovered, name the branch it belongs to in the ref
            // column. Only for rows that carry no ref of their own: those
            // already say which branch they are, and a badge would collide with
            // their chips. This is the gap the feature fills -- the ref column
            // is empty on the great majority of rows.
            if branch_names == HistoryBranchNamesMode::SeparateColumn
                && tag_names.is_empty()
                && branch_ref_count == 0
                && hitbox.is_hovered(window)
                && let Some(name) = lane_branch_name.as_ref()
                && !name.is_empty()
                && branch_content_bounds.size.width >= scaled_px(HISTORY_BRANCH_BADGE_MIN_W_PX)
            {
                let shaped = shape_truncated_line_cached_from(
                    window,
                    &base_style,
                    xxs_font,
                    name,
                    fx_hash_str(name.as_ref()),
                    branch_content_bounds.size.width,
                    with_alpha(node_color, HISTORY_BRANCH_BADGE_ALPHA),
                    None,
                    // Keeps the leaf of `origin/feature/long-name` readable.
                    TruncateFrom::Start,
                );
                let text_y = bounds.top() + (bounds.size.height - xxs_line_height) * 0.5;
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: branch_content_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(branch_content_bounds.left(), text_y),
                            xxs_line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            let refs = layout_history_ref_chips(
                theme,
                row_visual,
                branch_content_bounds,
                repo_id,
                &commit_id,
                &tag_names,
                &branch_chips,
                selected_branch.as_ref(),
                active_context_menu_invoker.as_ref(),
                &base_style,
                xxs_font,
                xxs_line_height,
                window,
            );

            if let Some(node) = graph_node {
                let stroke = scaled_px(HISTORY_REF_CONNECTOR_DOT_PX);
                if let Some(connector) = history_ref_connector_bounds(
                    branch_names,
                    refs.bounds,
                    node,
                    scaled_px(4.0),
                    stroke,
                ) {
                    let annotation_bounds = Bounds::new(
                        column_layout.summary.origin,
                        size(
                            summary_bounds.left() - column_layout.summary.left(),
                            bounds.size.height,
                        ),
                    );
                    paint_history_ref_connector(
                        branch_names,
                        connector,
                        annotation_bounds,
                        node_color,
                        window,
                    );
                }
            }

            if let (Some(grid), Some(node_bounds)) = (graph_grid, graph_node) {
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: graph_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        window.paint_layer(graph_bounds, |window| {
                            super::history_graph_paint::paint_history_graph(
                                theme,
                                graph_row,
                                graph_row_ix,
                                connect_from_top_col,
                                is_stash_node,
                                selected_lane,
                                row_visual,
                                &author,
                                initials_reveal,
                                selection,
                                graph_bounds,
                                grid,
                                node_bounds,
                                window,
                                cx,
                            );
                        });
                    },
                );
            }

            paint_history_ref_chips(&refs, window, cx);

            let summary_text_bounds = if branch_names == HistoryBranchNamesMode::Inline {
                history_message_after_inline_refs(
                    summary_content_bounds,
                    refs.bounds.size.width,
                    chip_gap,
                )
            } else {
                summary_content_bounds
            };
            let refs_hover_bounds = if branch_names == HistoryBranchNamesMode::Inline {
                refs.bounds
            } else {
                branch_bounds
            };
            let message_hover_bounds = if branch_names == HistoryBranchNamesMode::Inline {
                summary_text_bounds
            } else {
                summary_bounds
            };
            if !summary.is_empty() {
                let shaped = shape_highlighted_line_cached(
                    window,
                    &base_style,
                    sm_font,
                    summary.shared(),
                    summary.text_hash(),
                    summary_text_bounds.size.width.max(px(0.0)),
                    history_summary_color(theme, is_selected_branch_tip, related_to_selection),
                    None,
                    &summary_matches,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: summary_text_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(summary_text_bounds.left(), center_y(sm_line_height)),
                            sm_line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            // Hover regions resolved during paint and read by the shared move
            // listener below.
            let mut signature_hover: Option<(
                Bounds<Pixels>,
                gitcomet_core::domain::CommitSignature,
            )> = None;
            let mut date_hover_bounds: Option<Bounds<Pixels>> = None;

            if show_author && !author.is_empty() {
                let avatar_d = scale.ui_text(components::AVATAR_DIAMETER_PX);
                let avatar_gap = scaled_px(6.0);
                // Matches the header's extra left inset that clears the
                // column resize handle.
                let avatar_left = author_bounds.left() + cell_pad_x * 2.0;
                let identity_color = components::author_color(theme, author.as_ref());
                if !show_graph && author_bounds.size.width >= avatar_d + cell_pad_x * 2.0 {
                    let avatar_top = author_bounds.top()
                        + (author_bounds.size.height - avatar_d).max(px(0.0)) * 0.5;
                    let avatar = super::history_shapes::paint_disc(
                        Bounds::new(point(avatar_left, avatar_top), size(avatar_d, avatar_d)),
                        with_alpha(identity_color, 0.22),
                        window,
                    );

                    paint_author_initials(
                        avatar,
                        &author,
                        scale.ui_text(components::AVATAR_FONT_PX),
                        avatar_d,
                        identity_color,
                        1.0,
                        window,
                    );
                }

                // Signature badge, parked at the trailing edge of the author
                // cell. Keeping it off the avatar-to-name run leaves the names
                // flush against their avatars, and the badges line up in their
                // own column. Icon only: the details pane carries the signer,
                // key and format.
                //
                // The width is reserved for every row as soon as the repository
                // has any verdict at all, so a name truncates at the same place
                // whether or not its own commit is signed. A repository that
                // signs nothing gives up no width.
                let (signature, reserve_signature_gutter) = view
                    .read(cx)
                    .active_repo()
                    .map(|repo| {
                        let signatures = &repo.history_state.commit_signatures;
                        // Cloned, not rendered: building the badge's tooltip here
                        // would allocate a multi-line string per signed row per
                        // frame, for text only the hovered row ever reads.
                        (signatures.get(&commit_id).cloned(), !signatures.is_empty())
                    })
                    .unwrap_or((None, false));
                let signature_glyph = scaled_px(12.0);
                let text_left = if show_graph {
                    avatar_left
                } else {
                    avatar_left + avatar_d + avatar_gap
                };
                let signature_width = if reserve_signature_gutter {
                    signature_glyph + avatar_gap
                } else {
                    px(0.0)
                };
                let author_text_right =
                    (author_bounds.right() - cell_pad_x - signature_width).max(text_left);
                if let Some(signature) = &signature {
                    let badge_bounds = Bounds::new(
                        point(
                            author_bounds.right() - cell_pad_x - signature_glyph,
                            author_bounds.top(),
                        ),
                        size(signature_glyph, author_bounds.size.height),
                    );
                    let (icon, palette) =
                        crate::view::commit_signature::signature_glyph(theme, signature);
                    super::diff_canvas::paint_centered_svg_icon(
                        icon,
                        badge_bounds,
                        signature_glyph,
                        palette.foreground,
                        window,
                        cx,
                    );
                    signature_hover = Some((badge_bounds, signature.clone()));
                }

                let author_text_bounds = Bounds::new(
                    point(text_left, author_bounds.top()),
                    size(
                        (author_text_right - text_left).max(px(0.0)),
                        author_bounds.size.height,
                    ),
                );
                let shaped = shape_highlighted_line_cached(
                    window,
                    &base_style,
                    xs_font,
                    author.shared(),
                    author.text_hash(),
                    author_text_bounds.size.width.max(px(0.0)),
                    theme.colors.foreground.secondary,
                    None,
                    &author_matches,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: author_text_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(author_text_bounds.left(), center_y(xs_line_height)),
                            xs_line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            if show_date && !when.is_empty() {
                date_hover_bounds = Some(date_bounds);
                let date_text_bounds = Bounds::new(
                    point(date_bounds.left() + cell_pad_x, date_bounds.top()),
                    size(
                        (date_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                        date_bounds.size.height,
                    ),
                );
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    xxs_font,
                    when.shared(),
                    when.text_hash(),
                    date_text_bounds.size.width.max(px(0.0)),
                    theme.colors.foreground.secondary,
                    Some(UI_MONOSPACE_FONT_FAMILY),
                );
                let origin_x =
                    (date_text_bounds.right() - shaped.width).max(date_text_bounds.left());
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: date_text_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(origin_x, center_y(xxs_line_height)),
                            xxs_line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            if show_sha && !short_sha.is_empty() {
                let sha_text_bounds = Bounds::new(
                    point(sha_bounds.left() + cell_pad_x, sha_bounds.top()),
                    size(
                        (sha_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                        sha_bounds.size.height,
                    ),
                );
                let shaped = shape_highlighted_line_cached(
                    window,
                    &base_style,
                    xxs_font,
                    short_sha.shared(),
                    short_sha.text_hash(),
                    sha_text_bounds.size.width.max(px(0.0)),
                    theme.colors.foreground.secondary,
                    Some(UI_MONOSPACE_FONT_FAMILY),
                    &sha_matches,
                );
                let origin_x = (sha_text_bounds.right() - shaped.width).max(sha_text_bounds.left());
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: sha_text_bounds,
                        ..Default::default()
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(origin_x, center_y(xxs_line_height)),
                            xxs_line_height,
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    },
                );
            }

            // Share one move listener and row-level hit test across the canvas
            // hover areas, avoiding an extra boxed capture set per affordance
            // on every frame.
            window.on_mouse_event({
                let view = view.clone();
                let commit_id = commit_id.clone();
                let summary = summary.shared().clone();
                let hover_author = author.shared().clone();
                let hover_when = when.shared().clone();
                let ref_items = Arc::clone(&ref_items);
                let hitbox = hitbox.clone();
                let signature_hover = signature_hover.clone();
                move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    // The row's hitbox — not its bounds — decides whether this
                    // row owns the pointer: window-level listeners run whatever
                    // is painted on top, so anything overlaying the history (the
                    // collapsed sidebar's popover, a panel, a menu) must win.
                    let pointer_on_row = hitbox.is_hovered(window);

                    // Canvas cells cannot carry `.tooltip()`, so the signature
                    // badge, date cell and author node drive the shared host.
                    //
                    // Retracting is deliberately NOT gated on the hitbox: the row
                    // that owns the tooltip is by definition the one the pointer
                    // just left, so gating it would strand the host's text and
                    // make every later pointer event in the window respawn the
                    // host's delay timer.
                    let hovered_area = if !pointer_on_row {
                        None
                    } else if signature_hover
                        .as_ref()
                        .is_some_and(|(bounds, _)| bounds.contains(&event.position))
                    {
                        Some(HistoryRowHoverArea::Signature)
                    } else if date_hover_bounds
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        Some(HistoryRowHoverArea::Date)
                    } else if author_hover_bounds
                        .is_some_and(|bounds| bounds.contains(&event.position))
                    {
                        Some(HistoryRowHoverArea::Author)
                    } else {
                        None
                    };
                    let next_hover = hovered_area.map(|area| (row_id, area));
                    let current_hover = view.read(cx).row_hover(cx);
                    // Gate hard: this listener runs for every visible row on
                    // every pixel of movement. Only this row's own hover is ever
                    // retracted, so rows never fight over the host.
                    if current_hover != next_hover
                        && (next_hover.is_some()
                            || matches!(current_hover, Some((ix, _)) if ix == row_id))
                    {
                        let signature = match hovered_area {
                            Some(HistoryRowHoverArea::Signature) => signature_hover
                                .as_ref()
                                .map(|(_, signature)| signature.clone()),
                            _ => None,
                        };
                        view.update(cx, |this, cx| {
                            // Built here, once per hover transition, rather than
                            // once per row per frame.
                            let tooltip = match hovered_area {
                                Some(HistoryRowHoverArea::Signature) => signature
                                    .as_ref()
                                    .map(crate::view::commit_signature::signature_tooltip),
                                Some(HistoryRowHoverArea::Date) => {
                                    Some(this.full_commit_time_text(commit_time))
                                }
                                Some(HistoryRowHoverArea::Author) => {
                                    Some(if hover_author.trim().is_empty() {
                                        "Unknown author".into()
                                    } else {
                                        hover_author.clone()
                                    })
                                }
                                None => None,
                            };
                            this.update_history_row_hover(next_hover, tooltip, cx);
                        });
                    }

                    if !pointer_on_row {
                        return;
                    }

                    if !ref_items.is_empty() && refs_hover_bounds.contains(&event.position) {
                        view.update(cx, |this, cx| {
                            this.show_history_refs_hover(
                                repo_id,
                                commit_id.clone(),
                                refs_hover_bounds,
                                Arc::clone(&ref_items),
                                event.position,
                                cx,
                            );
                        });
                        return;
                    }

                    // The card is scoped to the message cell rather than the
                    // whole row: it is about the message, and pointing at the
                    // graph, the refs, the author or the date should not summon
                    // it. Closing is driven centrally from the window root, so
                    // rows the pointer merely passes over do no work.
                    if !message_hover_bounds.contains(&event.position) {
                        return;
                    }
                    // Deliberately no "is the card already open on this commit"
                    // shortcut here. Answering it means reading the root view
                    // and the hover host, and `Entity::read` aborts the process
                    // when its target is mid-update -- which the `cx.defer`
                    // below exists precisely because it can be. `show` is
                    // already a no-op for a card that is open on this commit.
                    let next = CommitMessageHoverState {
                        repo_id,
                        commit_id: commit_id.clone(),
                        summary: summary.clone(),
                        author: hover_author.clone(),
                        when: hover_when.clone(),
                        source_bounds: message_hover_bounds,
                        source_pointer_x: event.position.x,
                    };
                    let view = view.clone();
                    let pointer = event.position;
                    // Deferred so the update does not nest inside whatever
                    // update is already in flight on the root view.
                    cx.defer(move |cx| {
                        view.update(cx, |this, cx| {
                            this.show_commit_message_hover(next, pointer, cx)
                        });
                    });
                }
            });

            window.on_mouse_event({
                let view = view.clone();
                move |_: &gpui::MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble {
                        view.update(cx, |this, _| this.reset_history_row_hover());
                    }
                }
            });
            let target = crate::kit::click::canvas_target(
                "history-menu",
                (view.entity_id(), repo_id.0, commit_id.as_ref()),
            );
            let view = view.clone();
            let commit_id = commit_id.clone();
            crate::kit::click::on_canvas_click(
                window,
                target,
                &hitbox,
                MouseButton::Right,
                true,
                move |event, window, cx| {
                    view.update(cx, |this, cx| {
                        // Right-clicking inside an active multi-selection must
                        // not collapse it — the menu acts on the whole set — but
                        // focus must still move to the clicked commit so the
                        // details pane matches the menu target. Outside the
                        // selection this collapses to the clicked commit.
                        this.cancel_history_find_navigation();
                        this.select_history_commit(
                            repo_id,
                            commit_id.clone(),
                            CommitSelectMode::PreserveIfSelected,
                            None,
                        );
                        let context_menu_invoker =
                            format!("history_commit_menu_{}_{}", repo_id.0, commit_id.as_ref())
                                .into();

                        let kind = PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        };
                        this.open_popover_at(
                            kind.invoked_by(context_menu_invoker),
                            event.position(),
                            window,
                            cx,
                        );
                        cx.notify();
                    });
                },
            );
        },
    )
    .h_full()
    .w_full()
    .track_interaction(interaction)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn test_row_visual(theme: AppTheme) -> HistoryRowVisual {
        HistoryRowVisual {
            lane: history_graph::lane_color(theme, 3),
            background: theme.colors.surface.canvas,
            selected: false,
            related_to_selection: None,
        }
    }

    struct RefChipsTestView {
        percent: u32,
        painted: Rc<std::cell::Cell<Option<u32>>>,
    }

    impl Render for RefChipsTestView {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let percent = self.percent;
            let painted = self.painted.clone();
            super::super::canvas::keyed_canvas(
                "ref_budget_test",
                |_, _, _| (),
                move |_, _, window, cx| {
                    assert_inline_ref_paint_budget(percent, window, cx);
                    painted.set(Some(percent));
                },
            )
            .w_full()
            .h_full()
        }
    }

    struct InitialsCacheTestView;

    impl Render for InitialsCacheTestView {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            super::super::canvas::keyed_canvas(
                "initials_cache_test",
                |_, _, _| (),
                |_, _, window, _cx| {
                    let theme = AppTheme::gitcomet_dark();
                    let author = HistoryAuthorVm::new("Initials Cache".into());
                    let row = test_row_visual(theme);
                    let cached = || HISTORY_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow().len());
                    let paint = |progress: f32, window: &mut Window| {
                        let d = super::super::history_node::author_diameter(
                            ui_scale::UiScale::from_window(window).with_appearance(theme.metrics),
                            progress,
                        );
                        paint_history_author_node(
                            theme,
                            Bounds::new(point(px(20.0), px(20.0)), size(d, d)),
                            row,
                            &author,
                            progress,
                            window,
                        );
                    };
                    paint(1.0, window);
                    let warm = cached();
                    for frame in 1..60 {
                        paint(0.5 + frame as f32 / 120.0, window);
                    }
                    assert_eq!(
                        cached(),
                        warm,
                        "animated alpha must not grow the text layout cache"
                    );
                },
            )
            .w_full()
            .h_full()
        }
    }

    #[gpui::test]
    fn history_initials_animation_reuses_shaped_text(cx: &mut gpui::TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| InitialsCacheTestView);
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }

    #[gpui::test]
    fn inline_history_refs_fit_the_budget_and_only_use_their_actual_width(
        cx: &mut gpui::TestAppContext,
    ) {
        let painted = Rc::new(std::cell::Cell::new(None));
        let (view, cx) = cx.add_window_view(|_, _| RefChipsTestView {
            percent: 100,
            painted: painted.clone(),
        });
        for percent in [100, 125, 200] {
            cx.update(|window, app| {
                crate::ui_scale::apply_to_window(window, percent);
                view.update(app, |view, cx| {
                    view.percent = percent;
                    cx.notify();
                });
                let _ = window.draw(app);
            });
            assert_eq!(painted.get(), Some(percent));
        }
    }

    fn assert_inline_ref_paint_budget(percent: u32, window: &mut Window, cx: &mut App) {
        let commit_id = CommitId("tip".into());
        let branches = [branch_chip(
            "main",
            true,
            &[
                (BranchSection::Local, "main"),
                (BranchSection::Remote, "origin/main"),
            ],
        )];
        let tags: Vec<_> = (0..12)
            .map(|ix| HistoryTextVm::new(format!("release-{ix}").into()))
            .collect();
        let scale = crate::ui_scale::UiScale::from_percent(percent);
        let style = window.text_style();
        let gap = scale.px(HISTORY_TAG_CHIP_GAP_PX);
        // 54px leaves a refs budget too narrow for a whole "+N" chip.
        for width in [0.0, 10.0, 54.0, 90.0, 300.0, 900.0] {
            let content = Bounds::new(point(px(25.0), px(10.0)), scale.size(width, 28.0));
            let budget = Bounds::new(
                content.origin,
                size(
                    history_inline_refs_max_width(content.size.width, gap),
                    content.size.height,
                ),
            );
            let refs = layout_history_ref_chips(
                AppTheme::gitcomet_dark(),
                test_row_visual(AppTheme::gitcomet_dark()),
                budget,
                RepoId(1),
                &commit_id,
                &tags,
                &branches,
                None,
                None,
                &style,
                scale.px(11.0),
                scale.px(16.0),
                window,
            );
            paint_history_ref_chips(&refs, window, cx);
            assert_eq!(
                refs.shown + refs.hidden,
                13,
                "grouped local/remote refs count as one chip"
            );
            assert!(refs.hidden > 0);
            let message = history_message_after_inline_refs(content, refs.bounds.size.width, gap);
            assert!(refs.bounds.right() <= budget.right());
            assert!(message.left() >= refs.bounds.right());
            assert!(message.left() - content.left() <= content.size.width / 3.0 + px(0.001));
            assert!((message.right() - content.right()).abs() < px(0.001));
            if width <= 90.0 {
                assert_eq!(refs.shown, 0);
            }
            // A clipped "+N" still says the row has refs, and is the inline
            // hover target that opens the refs menu.
            if budget.size.width > px(0.0) {
                assert!(
                    refs.bounds.size.width > px(0.0),
                    "{width}px at {percent}%: hidden refs left no marker"
                );
            }
        }
        let content = Bounds::new(point(px(25.0), px(10.0)), scale.size(900.0, 28.0));
        let budget = Bounds::new(
            content.origin,
            size(
                history_inline_refs_max_width(content.size.width, gap),
                content.size.height,
            ),
        );
        for branch_list in [&branches[..], &[]] {
            let refs = layout_history_ref_chips(
                AppTheme::gitcomet_dark(),
                test_row_visual(AppTheme::gitcomet_dark()),
                budget,
                RepoId(1),
                &commit_id,
                &[],
                branch_list,
                None,
                None,
                &style,
                scale.px(11.0),
                scale.px(16.0),
                window,
            );
            paint_history_ref_chips(&refs, window, cx);
            assert_eq!(refs.shown, branch_list.len());
            assert_eq!(refs.hidden, 0);
            let message = history_message_after_inline_refs(content, refs.bounds.size.width, gap);
            assert!(message.left() - content.left() < content.size.width / 3.0);
            if branch_list.is_empty() {
                assert_eq!(message, content);
            }
        }
        let long = [branch_chip(
            "origin/feature/a-very-long-name-that-needs-clipping",
            false,
            &[(
                BranchSection::Remote,
                "origin/feature/a-very-long-name-that-needs-clipping",
            )],
        )];
        let refs = layout_history_ref_chips(
            AppTheme::gitcomet_dark(),
            test_row_visual(AppTheme::gitcomet_dark()),
            Bounds::new(content.origin, scale.size(95.0, 28.0)),
            RepoId(1),
            &commit_id,
            &[],
            &long,
            None,
            None,
            &style,
            scale.px(11.0),
            scale.px(16.0),
            window,
        );
        paint_history_ref_chips(&refs, window, cx);
        assert_eq!((refs.shown, refs.hidden), (1, 0));
        assert!(refs.bounds.size.width <= scale.px(95.0));
    }

    /// The ref chip is painted into the row, so a fixed chip in a taller row
    /// reads as a badge that shrank.
    #[test]
    fn the_ref_chip_keeps_its_proportion_in_a_comfortable_row() {
        use crate::appearance::{Appearance, UiDensity};
        let compact = Appearance {
            density: UiDensity::Compact,
            ..Appearance::default()
        };
        let comfortable = Appearance {
            density: UiDensity::Comfortable,
            ..Appearance::default()
        };
        let chip = |metrics: Appearance| {
            metrics.row_height(
                HISTORY_TAG_CHIP_HEIGHT_PX,
                HISTORY_TAG_CHIP_COMFORTABLE_HEIGHT_PX,
            )
        };

        assert!(chip(comfortable) > chip(compact));
        const {
            assert!(
                HISTORY_TAG_CHIP_COMFORTABLE_PADDING_X_PX > HISTORY_TAG_CHIP_PADDING_X_PX,
                "the padding has to follow the chip or the label crowds its edges"
            )
        };

        for metrics in UiDensity::ALL.into_iter().map(|density| Appearance {
            density,
            ..Appearance::default()
        }) {
            let row = crate::view::rows::history_row_height(
                crate::ui_scale::UiScale::from_percent(100).with_appearance(metrics),
            );
            assert!(
                px(chip(metrics)) < row,
                "the chip must stay inside its {row:?} row"
            );
        }
    }

    fn canvas_layout_for_branch_width(
        window: &Window,
        branch: Pixels,
        horizontal_pad: Pixels,
    ) -> HistoryCanvasColumnLayout {
        history_canvas_column_layout(
            Bounds::new(point(px(16.0), px(4.0)), size(px(1_000.0), px(28.0))),
            horizontal_pad,
            HistoryCanvasColumnWidths {
                branch,
                graph: px(80.2),
                author: px(140.2),
                date: px(160.2),
                sha: px(88.2),
            },
            HistoryCanvasColumnVisibility {
                graph: true,
                author: true,
                date: true,
                sha: true,
            },
            |value| window.pixel_snap(value),
        )
    }

    fn assert_canvas_columns_close(layout: HistoryCanvasColumnLayout, expected_right: Pixels) {
        assert_eq!(layout.branch.right(), layout.graph.left());
        assert_eq!(layout.graph.right(), layout.summary.left());
        assert_eq!(layout.summary.right(), layout.author.left());
        assert_eq!(layout.author.right(), layout.date.left());
        assert_eq!(layout.date.right(), layout.sha.left());
        assert_eq!(layout.sha.right(), expected_right);
    }

    #[gpui::test]
    fn history_canvas_graph_origin_tracks_one_x_device_pixel_rounding(
        cx: &mut gpui::TestAppContext,
    ) {
        let window_handle = cx.add_window(|_window, _cx| gpui::Empty);
        cx.update_window(window_handle.into(), |_, window, _| {
            window.set_scale_factor(1.0);

            let first = canvas_layout_for_branch_width(window, px(130.1), px(8.2));
            let same_pixel = canvas_layout_for_branch_width(window, px(130.4), px(8.2));
            let next_pixel = canvas_layout_for_branch_width(window, px(130.6), px(8.2));

            assert_eq!(first.graph.left(), px(154.0));
            assert_eq!(same_pixel.graph.left(), first.graph.left());
            assert_eq!(next_pixel.graph.left() - first.graph.left(), px(1.0));
            assert_canvas_columns_close(first, px(1_008.0));
        })
        .expect("history canvas test window should stay open");
    }

    #[gpui::test]
    fn history_canvas_graph_origin_tracks_fractional_device_pixel_rounding(
        cx: &mut gpui::TestAppContext,
    ) {
        let window_handle = cx.add_window(|_window, _cx| gpui::Empty);
        cx.update_window(window_handle.into(), |_, window, _| {
            window.set_scale_factor(1.5);

            // Near the default branch width at 125% UI scale, two fractional
            // drag positions occupy one device pixel and the third crosses into
            // exactly the next one.
            let first = canvas_layout_for_branch_width(window, px(162.7), px(10.1));
            let same_pixel = canvas_layout_for_branch_width(window, px(162.9), px(10.1));
            let next_pixel = canvas_layout_for_branch_width(window, px(163.2), px(10.1));

            assert_eq!(same_pixel.graph.left(), first.graph.left());
            let device_delta =
                f32::from(next_pixel.graph.left() - first.graph.left()) * window.scale_factor();
            assert!((device_delta - 1.0).abs() < 1e-4, "got {device_delta}");

            let graph_device_x = f32::from(first.graph.left()) * window.scale_factor();
            assert!(
                (graph_device_x - graph_device_x.round()).abs() < 1e-4,
                "graph origin must land on a device pixel, got {graph_device_x}"
            );
            assert_canvas_columns_close(first, px(1_006.0));
        })
        .expect("history canvas test window should stay open");
    }

    #[gpui::test]
    fn chip_label_clipping_omits_the_ellipsis_and_reclaims_its_width(
        cx: &mut gpui::TestAppContext,
    ) {
        let window_handle = cx.add_window(|_window, _cx| gpui::Empty);
        cx.update_window(window_handle.into(), |_, window, _| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let text: SharedString = "origin/feature/a-very-long-branch-name".into();
            let max_width = px(96.0);
            let color = AppTheme::gitcomet_dark().colors.foreground.primary;

            // Use identical layout inputs so this also proves the truncation
            // affix participates in the shared layout-cache key.
            let ellipsized = shape_truncated_line_cached_from(
                window,
                &style,
                font_size,
                &text,
                fx_hash_str(text.as_ref()),
                max_width,
                color,
                None,
                TruncateFrom::Start,
            );
            let clipped = shape_clipped_chip_line_cached_from(
                window,
                &style,
                font_size,
                &text,
                fx_hash_str(text.as_ref()),
                max_width,
                color,
                None,
                TruncateFrom::Start,
            );

            assert!(ellipsized.text.starts_with('…'));
            assert!(!clipped.text.contains('…'));
            assert!(text.ends_with(clipped.text.as_ref()));
            assert!(
                clipped.text.chars().count()
                    > ellipsized.text.trim_start_matches('…').chars().count(),
                "removing the ellipsis should expose more of the branch name"
            );
        })
        .expect("history canvas test window should stay open");
    }

    /// Test text systems measure bold like regular text, so this models a
    /// shaped line instead: 10px glyphs, bold ones 14px, a 10px ellipsis.
    /// The wrapper's base-font cut let bold matches overflow the cell.
    #[test]
    fn highlighted_lines_are_cut_where_their_bold_matches_really_end() {
        let bold = |ix: usize| (4..10).contains(&ix) || (14..20).contains(&ix);
        let glyphs = 24usize;
        let start = |ix: usize| {
            (0..ix)
                .map(|ix| if bold(ix) { 14.0 } else { 10.0 })
                .sum::<f32>()
        };
        let width = px(start(glyphs));
        // As `LineLayout::index_for_x`: the last glyph starting at or before x.
        let index_for_x = |x: Pixels| {
            (x < width).then(|| {
                (0..glyphs)
                    .rev()
                    .find(|&ix| px(start(ix)) <= x)
                    .unwrap_or(0)
            })
        };
        for max in 0..=320 {
            let max_width = px(max as f32);
            let drawn = match highlighted_cut(width, max_width, || px(10.0), index_for_x) {
                None => start(glyphs),
                Some(cut) => start(cut) + 10.0,
            };
            assert!(
                drawn <= max as f32 || max < 10,
                "{drawn}px drawn in a {max}px cell"
            );
        }
        assert_eq!(
            highlighted_cut(width, width, || px(10.0), index_for_x),
            None
        );
    }

    #[gpui::test]
    fn highlighted_lines_truncate_on_character_boundaries(cx: &mut gpui::TestAppContext) {
        let window_handle = cx.add_window(|_window, _cx| gpui::Empty);
        cx.update_window(window_handle.into(), |_, window, _| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let text: SharedString = "Élodie fixé — naïve déjà vu, and more words".into();
            let color = AppTheme::gitcomet_dark().colors.foreground.primary;
            let bold = gpui::HighlightStyle {
                font_weight: Some(gpui::FontWeight::BOLD),
                ..gpui::HighlightStyle::default()
            };
            let full = shape_highlighted_line_cached(
                window,
                &style,
                font_size,
                &text,
                fx_hash_str(text.as_ref()),
                px(10_000.0),
                color,
                None,
                &[(7..13, bold)],
            );
            assert_eq!(full.text, text);
            for max in [0.0, 20.0, 60.0, 120.0, f32::from(full.width) - 1.0] {
                let shaped = shape_highlighted_line_cached(
                    window,
                    &style,
                    font_size,
                    &text,
                    fx_hash_str(text.as_ref()),
                    px(max),
                    color,
                    None,
                    &[(7..13, bold)],
                );
                assert!(shaped.text.ends_with('…'), "{max}: {}", shaped.text);
                assert!(
                    text.starts_with(shaped.text.trim_end_matches('…')),
                    "{max}: {}",
                    shaped.text
                );
                assert!(
                    shaped.width <= px(max) || shaped.text.as_ref() == "…",
                    "{max}: {:?}",
                    shaped.width
                );
            }
        })
        .expect("history canvas test window should stay open");
    }

    /// A row painted before the find bar opened must not keep its plain
    /// cached line once its text matches.
    #[gpui::test]
    fn find_match_highlights_are_part_of_the_layout_cache_key(cx: &mut gpui::TestAppContext) {
        let window_handle = cx.add_window(|_window, _cx| gpui::Empty);
        cx.update_window(window_handle.into(), |_, window, _| {
            let style = window.text_style();
            let font_size = style.font_size.to_pixels(window.rem_size());
            let text: SharedString = "highlight cache key: Fix login bug".into();
            let color = AppTheme::gitcomet_dark().colors.foreground.primary;
            let bold = gpui::HighlightStyle {
                font_weight: Some(gpui::FontWeight::BOLD),
                ..gpui::HighlightStyle::default()
            };
            let cached = || HISTORY_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow().len());
            let shape = |window: &mut Window, highlights: &[(std::ops::Range<usize>, _)]| {
                shape_highlighted_line_cached(
                    window,
                    &style,
                    font_size,
                    &text,
                    fx_hash_str(text.as_ref()),
                    px(1000.0),
                    color,
                    None,
                    highlights,
                );
            };

            let before = cached();
            shape(window, &[]);
            shape(window, &[(21..24, bold)]);
            shape(window, &[(21..24, bold)]);
            shape(window, &[(25..30, bold)]);
            assert_eq!(
                cached() - before,
                3,
                "plain, one match and another match are three layouts"
            );
        })
        .expect("history canvas test window should stay open");
    }

    fn test_branch_target(section: BranchSection, name: &str) -> BranchMenuTarget {
        match section {
            BranchSection::Local => BranchMenuTarget::local(name),
            BranchSection::Remote => {
                let (remote, branch) = name.split_once('/').expect("test remote branch");
                BranchMenuTarget::remote(remote, branch)
            }
        }
    }

    fn selected(section: BranchSection, name: &str) -> SelectedHistoryBranch {
        SelectedHistoryBranch {
            target: test_branch_target(section, name),
        }
    }

    fn branch_chip(
        text: &str,
        is_head: bool,
        targets: &[(BranchSection, &str)],
    ) -> HistoryBranchChipVm {
        let targets = targets
            .iter()
            .map(|(section, name)| test_branch_target(*section, name))
            .collect::<Vec<_>>()
            .into();
        HistoryBranchChipVm {
            text: HistoryTextVm::new(SharedString::from(text.to_string())),
            kind: HistoryBranchChipKind::Branch { is_head, targets },
        }
    }

    #[test]
    fn grouped_branch_chip_marks_any_selected_exact_ref() {
        let chip = branch_chip(
            "main",
            false,
            &[
                (BranchSection::Local, "main"),
                (BranchSection::Remote, "origin/main"),
                (BranchSection::Remote, "upstream/main"),
            ],
        );
        assert!(matches!(
            history_branch_chip_style_kind(&chip, Some(&selected(BranchSection::Local, "main"))),
            HistoryChipStyleKind::Branch { selected: true }
        ));
        assert!(matches!(
            history_branch_chip_style_kind(
                &chip,
                Some(&selected(BranchSection::Remote, "upstream/main"))
            ),
            HistoryChipStyleKind::Branch { selected: true }
        ));
        assert!(matches!(
            history_branch_chip_style_kind(
                &chip,
                Some(&selected(BranchSection::Remote, "fork/main"))
            ),
            HistoryChipStyleKind::Branch { selected: false }
        ));
        assert!(matches!(
            history_branch_chip_style_kind(&chip, None),
            HistoryChipStyleKind::Branch { selected: false }
        ));
    }

    #[test]
    fn grouped_head_chip_stays_in_the_head_style() {
        // HEAD keeps its identity while also tracking the sidebar-selected ref.
        let head = branch_chip(
            "main",
            true,
            &[
                (BranchSection::Local, "main"),
                (BranchSection::Remote, "origin/main"),
            ],
        );
        assert!(matches!(
            history_branch_chip_style_kind(&head, Some(&selected(BranchSection::Local, "main"))),
            HistoryChipStyleKind::Head { selected: true }
        ));
        assert!(matches!(
            history_branch_chip_style_kind(
                &head,
                Some(&selected(BranchSection::Remote, "origin/main"))
            ),
            HistoryChipStyleKind::Head { selected: true }
        ));
        let detached = HistoryBranchChipVm {
            text: HistoryTextVm::new("HEAD".into()),
            kind: HistoryBranchChipKind::DetachedHead,
        };
        assert!(matches!(
            history_branch_chip_style_kind(&detached, None),
            HistoryChipStyleKind::Head { selected: false }
        ));
    }

    #[test]
    fn grouped_branch_chip_icons_keep_exact_refs() {
        let combined = branch_chip(
            "feature/x",
            false,
            &[
                (BranchSection::Local, "feature/x"),
                (BranchSection::Remote, "origin/feature/x"),
            ],
        );

        assert_eq!(
            history_branch_chip_icons(&combined).as_slice(),
            [HistoryBranchChipIcon::LocalRemote]
        );
        assert_eq!(HistoryBranchChipIcon::Local.path(), "icons/computer.svg");
        assert_eq!(HistoryBranchChipIcon::Remote.path(), "icons/cloud.svg");
        assert_eq!(
            HistoryBranchChipIcon::LocalRemote.path(),
            "icons/computer-cloud-background.svg"
        );
        assert_eq!(
            HistoryBranchChipIcon::Local.size(px(11.0), px(16.0)),
            px(11.0)
        );
        assert_eq!(
            HistoryBranchChipIcon::Remote.size(px(11.0), px(16.0)),
            px(11.0)
        );
        assert_eq!(
            HistoryBranchChipIcon::LocalRemote.size(px(11.0), px(16.0)),
            px(16.0)
        );
        assert_eq!(
            history_branch_chip_icon_width(
                &[HistoryBranchChipIcon::LocalRemote],
                px(11.0),
                px(16.0),
                px(3.0),
                px(2.0),
            ),
            px(19.0)
        );

        let local_only = branch_chip("feature/x", false, &[(BranchSection::Local, "feature/x")]);
        assert_eq!(
            history_branch_chip_icons(&local_only).as_slice(),
            [HistoryBranchChipIcon::Local]
        );

        let remote_only = branch_chip(
            "feature/x",
            false,
            &[(BranchSection::Remote, "origin/feature/x")],
        );
        assert_eq!(
            history_branch_chip_icons(&remote_only).as_slice(),
            [HistoryBranchChipIcon::Remote]
        );

        let detached = HistoryBranchChipVm {
            text: HistoryTextVm::new("HEAD".into()),
            kind: HistoryBranchChipKind::DetachedHead,
        };
        assert_eq!(
            history_branch_chip_icons(&detached).as_slice(),
            [HistoryBranchChipIcon::Check]
        );
    }

    fn ref_item(kind: HistoryRefListItemKind) -> HistoryRefListItem {
        HistoryRefListItem {
            text: HistoryTextVm::new(SharedString::from("chip")),
            kind,
        }
    }

    #[test]
    fn summary_lifts_on_the_selected_branch_tip_including_the_checked_out_branch() {
        // The summary and chip must both recognize the checked-out branch's
        // attached-HEAD ref when it is revealed from the sidebar.
        let head_row = [ref_item(HistoryRefListItemKind::AttachedHead {
            branch: "main".to_string(),
        })];
        assert!(history_row_is_selected_branch_tip(
            &head_row,
            Some(&selected(BranchSection::Local, "main"))
        ));

        let feature_row = [ref_item(HistoryRefListItemKind::LocalBranch {
            name: "feature".to_string(),
        })];
        assert!(history_row_is_selected_branch_tip(
            &feature_row,
            Some(&selected(BranchSection::Local, "feature"))
        ));

        let remote_row = [ref_item(HistoryRefListItemKind::RemoteBranch {
            name: "origin/main".to_string(),
            remote: "origin".to_string(),
            branch: "main".to_string(),
        })];
        assert!(history_row_is_selected_branch_tip(
            &remote_row,
            Some(&selected(BranchSection::Remote, "origin/main"))
        ));
    }

    #[test]
    fn summary_stays_body_text_on_rows_that_do_not_carry_the_selected_branch() {
        // The sidebar's pick outlives the reveal: clicking a different commit
        // afterwards must not brighten that row's summary too.
        let other_row = [ref_item(HistoryRefListItemKind::LocalBranch {
            name: "other".to_string(),
        })];
        assert!(!history_row_is_selected_branch_tip(
            &other_row,
            Some(&selected(BranchSection::Local, "feature"))
        ));

        // A row with no refs at all is the common case for that stale pick.
        assert!(!history_row_is_selected_branch_tip(
            &[],
            Some(&selected(BranchSection::Local, "feature"))
        ));

        // Local and remote refs of the same name stay distinct.
        let local_main = [ref_item(HistoryRefListItemKind::LocalBranch {
            name: "main".to_string(),
        })];
        assert!(!history_row_is_selected_branch_tip(
            &local_main,
            Some(&selected(BranchSection::Remote, "origin/main"))
        ));

        // Tags never stand in for a branch, and nothing is picked by default.
        let tag_row = [ref_item(HistoryRefListItemKind::Tag {
            name: "v1.0".to_string(),
        })];
        assert!(!history_row_is_selected_branch_tip(
            &tag_row,
            Some(&selected(BranchSection::Local, "v1.0"))
        ));
        assert!(!history_row_is_selected_branch_tip(&local_main, None));
    }

    #[test]
    fn commits_related_to_the_selection_go_to_full_contrast() {
        let dark = AppTheme::gitcomet_dark();
        let on_chain = history_summary_color(dark, false, Some(true));
        assert_eq!(
            (on_chain.red, on_chain.green, on_chain.blue),
            (1.0, 1.0, 1.0)
        );

        let light = AppTheme::gitcomet_light();
        let on_chain = history_summary_color(light, false, Some(true));
        assert_eq!(
            (on_chain.red, on_chain.green, on_chain.blue),
            (0.0, 0.0, 0.0)
        );

        for theme in [dark, light] {
            let on_chain = history_summary_color(theme, false, Some(true));
            // Has to actually move off body text, or the cue says nothing.
            assert_ne!(on_chain, theme.colors.foreground.primary);
            // The relation covers the tip too, so it wins over tip styling.
            assert_eq!(history_summary_color(theme, true, Some(true)), on_chain);
        }
    }

    #[test]
    fn rows_unrelated_to_the_selection_recede_behind_it() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let off = history_summary_color(theme, false, Some(false));
            let on = history_summary_color(theme, false, Some(true));

            // Pushed all the way to muted, and clearly apart from both the
            // related rows' full contrast and ordinary body text.
            assert_eq!(off, theme.colors.foreground.secondary);
            assert_ne!(off, theme.colors.foreground.primary);
            assert_ne!(off, on);
        }
    }

    #[test]
    fn the_uncommitted_changes_row_follows_the_history_it_connects_to() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let lane = history_graph::lane_color(theme, 0);

            // Nothing selected: the connector keeps its lane colour and the
            // label its body text, exactly as before the feature existed.
            assert_eq!(selection_related_lane_color(theme, lane, None), lane);
            assert_eq!(
                selection_related_summary_color(theme, None),
                theme.colors.foreground.primary
            );

            // On the selected chain: full contrast, matching the commit rows.
            assert_eq!(selection_related_lane_color(theme, lane, Some(true)), lane);
            assert_eq!(
                selection_related_summary_color(theme, Some(true)),
                full_contrast_text(theme)
            );

            // Off it: both recede, so the lane does not break at the top of a
            // dimmed run.
            assert_ne!(selection_related_lane_color(theme, lane, Some(false)), lane);
            assert_eq!(
                selection_related_summary_color(theme, Some(false)),
                theme.colors.foreground.secondary
            );
        }
    }

    #[test]
    fn summaries_are_plain_body_text_while_nothing_is_selected() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            assert_eq!(
                history_summary_color(theme, false, None),
                theme.colors.foreground.primary
            );
        }
    }

    #[test]
    fn selected_branch_tip_summary_is_brighter_than_body_text() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let plain = history_summary_color(theme, false, None);
            let tip = history_summary_color(theme, true, None);
            assert_ne!(
                (plain.red, plain.green, plain.blue, plain.alpha),
                (tip.red, tip.green, tip.blue, tip.alpha),
                "the revealed branch tip's summary must not reuse the body text color"
            );
            // Dark themes lift toward white, light themes toward black; either
            // way the tip must move away from body text, not sit between it and
            // the muted text used for de-emphasized columns.
            assert_eq!(
                tip, theme.colors.foreground.emphasis,
                "the tip summary should use the theme's emphasis color"
            );
        }
    }

    /// Every chip on a selected row is filled with the lane, so the sidebar's
    /// pick is carried by its ring alone. A merely different ring is what once
    /// made this selection invisible; it must meet the selection-indicator floor.
    #[test]
    fn sidebar_selected_branch_ring_stands_out_on_a_selected_shared_tip() {
        for (key, _) in crate::theme::bundled_theme_keys() {
            let theme = AppTheme::from_key(&key).unwrap();
            for color in 0..crate::theme::GRAPH_LANE_PALETTE_SIZE {
                let row = HistoryRowVisual {
                    lane: history_graph::lane_color(theme, color as u8),
                    selected: true,
                    related_to_selection: Some(true),
                    ..test_row_visual(theme)
                };
                for (name, is_head) in [("feature/x", false), ("main", true)] {
                    let selected = SelectedBranch {
                        repo_id: RepoId(7),
                        target: BranchMenuTarget::local(name),
                    };
                    let selected =
                        selected_branch_for_history_row(Some(&selected), RepoId(7), true);
                    let chip = branch_chip(name, is_head, &[(BranchSection::Local, name)]);
                    let picked = history_chip_visual(
                        theme,
                        row,
                        history_branch_chip_style_kind(&chip, selected.as_ref()),
                        false,
                    );
                    assert!(picked.selected, "{key}: the picked ring is drawn thicker");
                    assert!(
                        contrast_ratio(picked.border, picked.bg) >= 3.0,
                        "{key} lane {color}: the sidebar-selected {name} ring blends into its fill"
                    );
                    for other in [
                        HistoryChipStyleKind::Tag,
                        history_branch_chip_style_kind(&chip, None),
                    ] {
                        let other = history_chip_visual(theme, row, other, false);
                        assert!(!other.selected);
                        assert_ne!(picked.border, other.border);
                    }
                }
            }
        }
    }

    #[test]
    fn chips_share_the_lane_color_and_activate_for_the_row_or_lane() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let row = test_row_visual(theme);
            for kind in [
                HistoryChipStyleKind::Tag,
                HistoryChipStyleKind::Head { selected: false },
                HistoryChipStyleKind::Branch { selected: false },
            ] {
                let plain = history_chip_visual(theme, row, kind, false);
                let selected = history_chip_visual(
                    theme,
                    HistoryRowVisual {
                        selected: true,
                        ..row
                    },
                    kind,
                    false,
                );
                let on_lane = history_chip_visual(
                    theme,
                    HistoryRowVisual {
                        related_to_selection: Some(true),
                        ..row
                    },
                    kind,
                    false,
                );
                assert_ne!(plain.bg, selected.bg);
                assert_eq!(selected.bg, row.lane);
                assert_eq!(on_lane.bg, selected.bg);
            }
            let head = branch_chip("main", true, &[(BranchSection::Local, "main")]);
            assert_eq!(
                history_branch_chip_icons(&head).as_slice(),
                [HistoryBranchChipIcon::Check, HistoryBranchChipIcon::Local]
            );
        }
    }

    #[test]
    fn chips_with_open_context_menus_keep_a_distinct_outline() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            for selected in [false, true] {
                let row = HistoryRowVisual {
                    selected,
                    ..test_row_visual(theme)
                };
                for kind in [
                    HistoryChipStyleKind::Tag,
                    HistoryChipStyleKind::Head { selected: false },
                    HistoryChipStyleKind::Head { selected: true },
                    HistoryChipStyleKind::Branch { selected: false },
                    HistoryChipStyleKind::Branch { selected: true },
                ] {
                    let closed = history_chip_visual(theme, row, kind, false);
                    let open = history_chip_visual(theme, row, kind, true);
                    assert_ne!(closed.border, open.border);
                    assert_eq!(closed.bg, open.bg);
                    assert_eq!(closed.text, open.text);
                }
            }
        }
    }

    #[test]
    fn chip_menu_invokers_identify_the_exact_chip() {
        let repo_id = RepoId(7);
        let commit_id = CommitId("deadbeef".into());
        let local = branch_chip(
            "feature/local",
            false,
            &[(BranchSection::Local, "feature/local")],
        );
        let remote = branch_chip(
            "feature/remote",
            false,
            &[(BranchSection::Remote, "origin/feature/remote")],
        );

        let local_invoker = history_branch_chip_menu_invoker(repo_id, &commit_id, &local);
        assert_ne!(
            local_invoker,
            history_branch_chip_menu_invoker(repo_id, &commit_id, &remote)
        );
        assert_ne!(
            local_invoker,
            history_tag_chip_menu_invoker(repo_id, &commit_id, "feature/local")
        );
        assert_eq!(
            local_invoker,
            history_branch_chip_menu_invoker(repo_id, &commit_id, &local)
        );
    }

    #[test]
    fn chip_icons_keep_contrast_on_lane_colored_fills() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            for selected in [false, true] {
                let row = HistoryRowVisual {
                    selected,
                    ..test_row_visual(theme)
                };
                let visual = history_chip_visual(
                    theme,
                    row,
                    HistoryChipStyleKind::Head { selected: false },
                    false,
                );
                // Every icon layer is painted in the chip's text colour.
                assert!(contrast_ratio(visual.text, visual.bg) >= 4.5);
            }
        }
    }

    #[test]
    fn chip_foregrounds_fit_every_lane_and_custom_theme_surface() {
        let mut custom = AppTheme::gitcomet_light();
        custom.colors.foreground.primary = gpui::rgba(0x777777ff);
        custom.colors.foreground.emphasis = gpui::rgba(0x777777ff);
        custom.colors.accent.foreground = gpui::rgba(0x777777ff);
        custom.colors.accent.on_solid = gpui::rgba(0x777777ff);
        for theme in [
            AppTheme::gitcomet_dark(),
            AppTheme::gitcomet_light(),
            custom,
            AppTheme::from_key(crate::theme::AMBER_DARK_THEME_KEY).unwrap(),
            AppTheme::from_key("tokyo_night").unwrap(),
            AppTheme::from_key("sunset_veil").unwrap(),
        ] {
            for color in 0..crate::theme::GRAPH_LANE_PALETTE_SIZE {
                for selected in [false, true] {
                    let row = HistoryRowVisual {
                        lane: history_graph::lane_color(theme, color as u8),
                        selected,
                        ..test_row_visual(theme)
                    };
                    for open in [false, true] {
                        let visual =
                            history_chip_visual(theme, row, HistoryChipStyleKind::Tag, open);
                        assert_eq!(visual.bg.alpha, 1.0);
                        assert!(contrast_ratio(visual.text, visual.bg) >= 4.5);
                    }
                }
            }
        }
    }

    #[test]
    fn ref_leaders_reverse_direction_and_stop_at_the_facing_node_edge() {
        let node = Bounds::new(point(px(140.0), px(4.0)), size(px(20.0), px(20.0)));
        let left_refs = Bounds::new(point(px(8.0), px(0.0)), size(px(100.0), px(28.0)));
        let right_refs = Bounds::new(point(px(180.0), px(0.0)), size(px(100.0), px(28.0)));
        let separate = history_ref_connector_bounds(
            HistoryBranchNamesMode::SeparateColumn,
            left_refs,
            node,
            px(4.0),
            px(1.0),
        )
        .unwrap();
        let inline = history_ref_connector_bounds(
            HistoryBranchNamesMode::Inline,
            right_refs,
            node,
            px(4.0),
            px(1.0),
        )
        .unwrap();
        assert_eq!(separate.left(), left_refs.right() + px(4.0));
        assert_eq!(separate.right(), node.left());
        assert_eq!(inline.left(), node.right());
        assert_eq!(inline.right(), right_refs.left() - px(4.0));
        assert_eq!(separate.center().y, node.center().y);
        assert_eq!(inline.center().y, node.center().y);
        for mode in [
            HistoryBranchNamesMode::SeparateColumn,
            HistoryBranchNamesMode::Inline,
        ] {
            let empty = Bounds::new(left_refs.origin, size(px(0.0), px(28.0)));
            assert!(history_ref_connector_bounds(mode, empty, node, px(4.0), px(1.0)).is_none());
            assert!(history_ref_connector_bounds(mode, node, node, px(4.0), px(1.0)).is_none());
        }
    }

    #[test]
    fn ref_dots_anchor_to_the_node_and_stay_inside_fractional_bounds() {
        for scale in [0.8, 1.0, 1.25, 1.5, 2.0] {
            let diameter = px(HISTORY_REF_CONNECTOR_DOT_PX * scale);
            let pitch = px(HISTORY_REF_CONNECTOR_PITCH_PX * scale);
            let bounds = Bounds::new(point(px(10.25), px(20.5)), size(px(21.5 * scale), diameter));
            let separate: Vec<_> = history_ref_connector_dots(
                HistoryBranchNamesMode::SeparateColumn,
                bounds,
                Bounds::default(),
                pitch,
            )
            .collect();
            let inline: Vec<_> = history_ref_connector_dots(
                HistoryBranchNamesMode::Inline,
                bounds,
                Bounds::default(),
                pitch,
            )
            .collect();
            assert_eq!(separate.len(), 6);
            assert_eq!(inline.len(), 6);
            assert!((f32::from(separate[0].right() - bounds.right())).abs() < 0.0001);
            assert_eq!(inline[0].left(), bounds.left());
            for dots in [&separate, &inline] {
                for dot in dots {
                    assert!(dot.left() >= bounds.left() - px(0.0001));
                    assert!(dot.right() <= bounds.right() + px(0.0001));
                    assert_eq!(dot.center().y, bounds.center().y);
                    assert_eq!(dot.size, size(diameter, diameter));
                }
                for pair in dots.windows(2) {
                    assert!(
                        f32::from((pair[0].center().x - pair[1].center().x).abs() - pitch).abs()
                            < 0.0001
                    );
                }
            }
        }
    }

    #[test]
    fn ref_dots_skip_annotations_without_restarting_the_pattern() {
        let bounds = Bounds::new(point(px(10.25), px(20.5)), size(px(40.5), px(1.875)));
        let annotation = Bounds::new(point(px(23.0), px(0.0)), size(px(12.0), px(40.0)));
        for mode in [
            HistoryBranchNamesMode::SeparateColumn,
            HistoryBranchNamesMode::Inline,
        ] {
            let full: Vec<_> =
                history_ref_connector_dots(mode, bounds, Bounds::default(), px(5.0)).collect();
            let interrupted: Vec<_> =
                history_ref_connector_dots(mode, bounds, annotation, px(5.0)).collect();
            assert!(!interrupted.is_empty() && interrupted.len() < full.len());
            assert_eq!(
                interrupted,
                full.into_iter()
                    .filter(|dot| {
                        dot.right() <= annotation.left() || dot.left() >= annotation.right()
                    })
                    .collect::<Vec<_>>()
            );
            assert!(
                interrupted
                    .iter()
                    .any(|dot| dot.right() <= annotation.left())
            );
            assert!(
                interrupted
                    .iter()
                    .any(|dot| dot.left() >= annotation.right())
            );
            assert_eq!(
                history_ref_connector_dots(mode, bounds, bounds, px(5.0)).count(),
                0
            );
        }
    }

    #[test]
    fn ref_dots_require_room_for_one_whole_dot() {
        for mode in [
            HistoryBranchNamesMode::SeparateColumn,
            HistoryBranchNamesMode::Inline,
        ] {
            for (width, expected) in [(0.0, 0), (1.49, 0), (1.5, 1), (5.49, 1), (5.5, 2)] {
                let bounds = Bounds::new(point(px(0.25), px(10.5)), size(px(width), px(1.5)));
                assert_eq!(
                    history_ref_connector_dots(mode, bounds, Bounds::default(), px(4.0)).count(),
                    expected
                );
            }
        }
    }

    #[test]
    fn chip_lane_activation_respects_lifetime_and_selected_rows() {
        let theme = AppTheme::gitcomet_dark();
        let row = history_graph::GraphRow {
            lanes_now: Default::default(),
            lanes_next: Default::default(),
            joins_in: Default::default(),
            edges_out: Default::default(),
            node_col: 0,
            node_color_ix: 3,
            is_merge: false,
            from_node_cols: Default::default(),
        };
        let lane = Some(super::super::history_graph_paint::SelectedLane::span(
            3, 10, 20,
        ));
        let background = theme.colors.surface.canvas;
        assert!(HistoryRowVisual::new(theme, &row, 15, lane, false, background).refs_active());
        assert!(!HistoryRowVisual::new(theme, &row, 30, lane, false, background).refs_active());
        assert!(HistoryRowVisual::new(theme, &row, 30, lane, true, background).refs_active());
        assert!(!HistoryRowVisual::new(theme, &row, 15, None, false, background).refs_active());
        assert!(HistoryRowVisual::new(theme, &row, 15, None, true, background).refs_active());
    }
}
