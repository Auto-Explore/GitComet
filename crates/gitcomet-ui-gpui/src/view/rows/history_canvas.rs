use super::*;
use gitcomet_state::msg::CommitSelectMode;
use gpui::{
    Bounds, ContentMask, CursorStyle, DispatchPhase, HitboxBehavior, MouseButton, TruncateFrom,
    fill, point, px, size,
};
use palette::IntoColor;
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::cell::RefCell;

const HISTORY_TAG_CHIP_HEIGHT_PX: f32 = 18.0;
const HISTORY_TAG_CHIP_PADDING_X_PX: f32 = 6.0;
const HISTORY_TAG_CHIP_GAP_PX: f32 = 4.0;
const HISTORY_BRANCH_CHIP_ICON_PX: f32 = 11.0;
const HISTORY_BRANCH_CHIP_COMBINED_ICON_PX: f32 = 16.0;
const HISTORY_BRANCH_CHIP_TEXT_ICON_GAP_PX: f32 = 3.0;
const HISTORY_BRANCH_CHIP_ICON_GAP_PX: f32 = 2.0;

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
    let runs = vec![style.to_run(text.len())];
    let mut wrapper = window.text_system().line_wrapper(style.font(), font_size);
    let (truncated, runs) = wrapper.truncate_line(
        text.clone(),
        max_width.max(px(0.0)),
        truncation_affix,
        &runs,
        truncate_from,
    );
    let shaped = window
        .text_system()
        .shape_line(truncated, font_size, runs.as_ref(), None);

    HISTORY_TEXT_LAYOUT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, shaped.clone());
    });

    shaped
}

/// Which visual family a ref chip belongs to. Tags stay accent-tinted pills,
/// the HEAD chip is a solid accent pill, plain branches are neutral so a busy
/// ref column doesn't shout, and a plain branch selected in the sidebar is
/// tinted so the revealed commit visibly belongs to it.
#[derive(Clone, Copy)]
enum HistoryChipStyleKind {
    Tag,
    Head,
    Branch { selected: bool },
}

struct HistoryChipVisual {
    border: gpui::Rgba,
    bg: gpui::Rgba,
    text: gpui::Rgba,
    local_branch_icon: gpui::Rgba,
    remote_branch_icon: gpui::Rgba,
}

fn history_chip_visual(
    theme: AppTheme,
    kind: HistoryChipStyleKind,
    context_menu_open: bool,
) -> HistoryChipVisual {
    let active_branch_icon = gpui::rgba(0xffffffff);
    let visual = match kind {
        HistoryChipStyleKind::Tag => HistoryChipVisual {
            border: with_alpha(theme.colors.accent.foreground, 0.35),
            bg: with_alpha(theme.colors.accent.foreground, 0.12),
            text: theme.colors.accent.foreground,
            local_branch_icon: theme.colors.accent.foreground,
            remote_branch_icon: theme.colors.foreground.secondary,
        },
        // The HEAD chip carries no selection state: a ring around a pill that is
        // already a solid accent fill reads as a rendering artifact, not as a
        // selection. Selecting the checked-out branch is left unmarked here.
        HistoryChipStyleKind::Head => HistoryChipVisual {
            // `accent.solid`, not a near-opaque `accent.foreground`: this is the
            // one solid-accent surface in the app, and the theme asserts 4.5:1
            // of `on_solid` against `solid`. Mixing the fill from a different
            // token let a theme pass that gate and still ship an unreadable chip.
            border: theme.colors.accent.solid,
            bg: theme.colors.accent.solid,
            text: theme.colors.accent.on_solid,
            // Keep the icon legible against the solid accent chip. Plain local
            // branch chips use `accent.foreground`, matching the sidebar.
            local_branch_icon: theme.colors.accent.on_solid,
            remote_branch_icon: with_alpha(theme.colors.accent.on_solid, 0.70),
        },
        // The branch picked in the sidebar is tinted rather than merely
        // re-bordered: on a busy ref column a border alone reads as noise, and
        // this chip is the only thing marking which branch the revealed commit
        // is the tip of. It stays short of the solid HEAD fill so the two
        // remain distinguishable on the same row.
        HistoryChipStyleKind::Branch { selected: true } => HistoryChipVisual {
            border: with_alpha(theme.colors.accent.foreground, 0.85),
            bg: with_alpha(theme.colors.accent.foreground, 0.22),
            text: selected_branch_label_color(theme),
            // Every branch-location glyph is an outline. Keep both layers of
            // the combined computer/cloud glyph pure white on the active fill.
            local_branch_icon: active_branch_icon,
            remote_branch_icon: active_branch_icon,
        },
        HistoryChipStyleKind::Branch { selected: false } => HistoryChipVisual {
            border: with_alpha(theme.colors.stroke.default, 0.90),
            bg: theme.colors.surface.raised,
            text: theme.colors.foreground.secondary,
            local_branch_icon: theme.colors.accent.foreground,
            remote_branch_icon: theme.colors.foreground.secondary,
        },
    };

    if !context_menu_open {
        return visual;
    }

    match kind {
        // HEAD already owns the strongest accent surface. A light ring is the
        // only open-state treatment that remains visible without weakening its
        // established foreground contrast.
        HistoryChipStyleKind::Head => HistoryChipVisual {
            border: theme.colors.accent.on_solid,
            ..visual
        },
        // The menu-open state is deliberately stronger than sidebar selection,
        // so a selected branch still changes when its own menu is pinned open.
        HistoryChipStyleKind::Tag => HistoryChipVisual {
            border: theme.colors.accent.foreground,
            bg: with_alpha(theme.colors.accent.foreground, 0.30),
            text: selected_branch_label_color(theme),
            local_branch_icon: theme.colors.accent.foreground,
            remote_branch_icon: theme.colors.foreground.secondary,
        },
        HistoryChipStyleKind::Branch { .. } => HistoryChipVisual {
            border: theme.colors.accent.foreground,
            bg: with_alpha(theme.colors.accent.foreground, 0.30),
            text: selected_branch_label_color(theme),
            local_branch_icon: active_branch_icon,
            remote_branch_icon: active_branch_icon,
        },
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
    let is = |section: BranchSection, name: &str| {
        selected.section == section && selected.name.as_ref() == name
    };

    match kind {
        HistoryRefListItemKind::AttachedHead { branch } => is(BranchSection::Local, branch),
        HistoryRefListItemKind::LocalBranch { name } => is(BranchSection::Local, name),
        HistoryRefListItemKind::RemoteBranch { name } => is(BranchSection::Remote, name),
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
        None if is_selected_branch_tip => selected_branch_label_color(theme),
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
        HistoryBranchChipKind::DetachedHead => HistoryChipStyleKind::Head,
        HistoryBranchChipKind::Branch { is_head: true, .. } => HistoryChipStyleKind::Head,
        HistoryBranchChipKind::Branch { targets, .. } => HistoryChipStyleKind::Branch {
            selected: selected_branch.is_some_and(|selected| {
                targets.iter().any(|target| {
                    target.section == selected.section
                        && target.name.as_str() == selected.name.as_ref()
                })
            }),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryBranchChipIcon {
    Local,
    Remote,
    LocalRemote,
}

impl HistoryBranchChipIcon {
    fn path(self) -> &'static str {
        match self {
            Self::Local => "icons/computer.svg",
            Self::Remote => "icons/cloud.svg",
            Self::LocalRemote => "icons/computer-cloud-background.svg",
        }
    }

    fn color(self, visual: &HistoryChipVisual) -> gpui::Rgba {
        match self {
            Self::Local => visual.local_branch_icon,
            Self::Remote | Self::LocalRemote => visual.remote_branch_icon,
        }
    }

    fn foreground_layer(self, visual: &HistoryChipVisual) -> Option<(&'static str, gpui::Rgba)> {
        match self {
            Self::LocalRemote => Some((
                "icons/computer-cloud-foreground.svg",
                visual.local_branch_icon,
            )),
            Self::Local | Self::Remote => None,
        }
    }

    fn size(self, icon_size: Pixels, combined_icon_size: Pixels) -> Pixels {
        match self {
            Self::Local | Self::Remote => icon_size,
            Self::LocalRemote => combined_icon_size,
        }
    }
}

fn history_branch_chip_icons(chip: &HistoryBranchChipVm) -> SmallVec<[HistoryBranchChipIcon; 2]> {
    let HistoryBranchChipKind::Branch { targets, .. } = &chip.kind else {
        return SmallVec::new();
    };
    let has_local = targets
        .iter()
        .any(|target| target.section == BranchSection::Local);
    let has_remote = targets
        .iter()
        .any(|target| target.section == BranchSection::Remote);
    let mut icons = SmallVec::new();
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
    window.paint_quad(fill(chip_bounds, visual.border).corner_radii(radius));
    let inner = Bounds::new(
        point(chip_bounds.left() + border_w, chip_bounds.top() + border_w),
        size(
            (chip_bounds.size.width - border_w * 2.0).max(px(0.0)),
            (chip_bounds.size.height - border_w * 2.0).max(px(0.0)),
        ),
    );
    window.paint_quad(fill(inner, visual.bg).corner_radii((radius - border_w).max(px(0.0))));

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
            icon.color(visual),
            window,
            cx,
        );
        if let Some((foreground_path, foreground_color)) = icon.foreground_layer(visual) {
            super::diff_canvas::paint_centered_svg_icon(
                foreground_path,
                cell,
                painted_icon_size,
                foreground_color,
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

fn hit_test_index(bounds: &[Bounds<Pixels>], p: gpui::Point<Pixels>) -> Option<usize> {
    bounds.iter().position(|b| b.contains(&p))
}

fn hit_test_branch_chip(
    chips: &[(Bounds<Pixels>, HistoryBranchChipVm)],
    p: gpui::Point<Pixels>,
) -> Option<&HistoryBranchChipVm> {
    chips
        .iter()
        .find_map(|(bounds, chip)| bounds.contains(&p).then_some(chip))
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

fn history_branch_chip_popover_kind(
    repo_id: RepoId,
    chip: &HistoryBranchChipVm,
) -> Option<PopoverKind> {
    let HistoryBranchChipKind::Branch { targets, .. } = &chip.kind else {
        return None;
    };
    match targets.as_ref() {
        [] => None,
        [target] => Some(target.popover_kind(repo_id)),
        _ => Some(PopoverKind::BranchRefsMenu {
            repo_id,
            display_name: chip.text.as_ref().to_string(),
            targets: targets.to_vec(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn history_commit_row_canvas(
    theme: AppTheme,
    view: Entity<HistoryView>,
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
    connect_from_top_col: Option<usize>,
    graph_rows: Arc<[history_graph::GraphRow]>,
    graph_row_ix: usize,
    tag_names: Arc<[HistoryTextVm]>,
    branch_chips: Arc<[HistoryBranchChipVm]>,
    ref_items: Arc<[HistoryRefListItem]>,
    selected_branch: Option<SelectedHistoryBranch>,
    selected_lane: Option<super::history_graph_paint::SelectedLane>,
    lane_branch_name: Option<SharedString>,
    author: HistoryTextVm,
    summary: HistoryTextVm,
    when: HistoryTextVm,
    short_sha: HistoryTextVm,
    active_context_menu_invoker: Option<SharedString>,
    // The background the row's own `div` carries (selection, HEAD, open context
    // menu), and the one it swaps in while hovered. Mirrored rather than painted
    // again: the graph's icon nodes knock their glyphs out in the row background,
    // so the canvas has to know what the row is actually showing.
    row_bg_overlay: Option<gpui::Rgba>,
    hover_bg_overlay: gpui::Rgba,
) -> AnyElement {
    super::canvas::keyed_canvas(
        ("history_commit_row_canvas", row_id),
        move |bounds, window, _cx| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let Some(graph_row) = graph_rows.get(graph_row_ix) else {
                return;
            };
            // What the row is painted over, flattened as it is built up, so the
            // graph's icon nodes can knock their glyphs out in it. The row's own
            // `div` already paints the hover tint (or `row_bg_overlay` when not
            // hovered) behind this canvas, so that one is only accounted for
            // here, never painted a second time.
            let mut row_background = theme.colors.surface.canvas;
            if let Some(overlay) = if hitbox.is_hovered(window) {
                Some(hover_bg_overlay)
            } else {
                row_bg_overlay
            } {
                row_background = crate::theme::composite_over(row_background, overlay);
            }
            // Purple highlight on the commit currently being browsed historically.
            if view
                .read(cx)
                .active_repo()
                .is_some_and(|repo| repo.browsing_commit() == Some(&commit_id))
            {
                let tint =
                    crate::theme::with_alpha(crate::theme::historical_outline(theme.is_dark), 0.22);
                window.paint_quad(fill(bounds, tint));
                row_background = crate::theme::composite_over(row_background, tint);
            }
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);

            let is_selected_branch_tip =
                history_row_is_selected_branch_tip(&ref_items, selected_branch.as_ref());

            let design_scale_factor = ui_scale::design_scale_factor_from_window(window);
            let scaled_px = |value| px(value * design_scale_factor);
            let base_style = window.text_style();
            // Avatar initials are semibold, matching `components::author_avatar`.
            let initials_style = {
                let mut style = base_style.clone();
                style.font_weight = gpui::FontWeight::SEMIBOLD;
                style
            };
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

            let center_y = |line_height: Pixels| {
                let extra = (bounds.size.height - line_height).max(px(0.0));
                bounds.top() + extra * 0.5
            };

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
            let summary_bounds = column_layout.summary;
            let author_bounds = column_layout.author;
            let date_bounds = column_layout.date;
            let sha_bounds = column_layout.sha;

            // Everything coloured from this row's lane -- the node, the
            // message border, the fade wash and the hover badge -- washes with
            // that lane, so a row never shows two different strengths of the
            // same colour.
            let related_to_selection = selected_lane
                .map(|selected| selected.covers(theme, graph_row_ix, graph_row.node_color_ix));
            let node_color = super::history_graph_paint::lane_wash_color(
                theme,
                graph_row.node_color_ix,
                graph_row_ix,
                selected_lane,
            );

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

            if show_graph {
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: graph_bounds,
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
                                row_background,
                                graph_bounds,
                                window,
                                cx,
                            );
                        });
                    },
                );
            }

            let chip_height = scaled_px(HISTORY_TAG_CHIP_HEIGHT_PX);
            let chip_pad_x = scaled_px(HISTORY_TAG_CHIP_PADDING_X_PX);
            let chip_gap = scaled_px(HISTORY_TAG_CHIP_GAP_PX);

            let branch_content_bounds = Bounds::new(
                point(branch_bounds.left() + cell_pad_x, branch_bounds.top()),
                size(
                    (branch_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                    branch_bounds.size.height,
                ),
            );

            let mut tag_chip_bounds: SmallVec<[Bounds<Pixels>; 4]> =
                SmallVec::with_capacity(tag_names.len());
            let mut branch_chip_hits: SmallVec<[(Bounds<Pixels>, HistoryBranchChipVm); 4]> =
                SmallVec::with_capacity(branch_chips.len());
            let branch_ref_count = branch_chips.len();
            // While the row is hovered, name the branch it belongs to in the ref
            // column. Only for rows that carry no ref of their own: those
            // already say which branch they are, and a badge would collide with
            // their chips. This is the gap the feature fills -- the ref column
            // is empty on the great majority of rows.
            if tag_names.is_empty()
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

            if !tag_names.is_empty() || branch_ref_count > 0 {
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: branch_content_bounds,
                    }),
                    |window| {
                        // paint_quad has no layout-level radius clamping, so a
                        // pill radius (999) must be capped to half the height.
                        let chip_radius = px(theme.radii.pill).min(chip_height * 0.5);
                        let chip_border_w = scaled_px(1.0);
                        let branch_icon_size = scaled_px(HISTORY_BRANCH_CHIP_ICON_PX);
                        let branch_combined_icon_size =
                            scaled_px(HISTORY_BRANCH_CHIP_COMBINED_ICON_PX);
                        let branch_text_icon_gap = scaled_px(HISTORY_BRANCH_CHIP_TEXT_ICON_GAP_PX);
                        let branch_icon_gap = scaled_px(HISTORY_BRANCH_CHIP_ICON_GAP_PX);
                        let chip_y =
                            bounds.top() + (bounds.size.height - chip_height).max(px(0.0)) * 0.5;
                        let min_text_w = scaled_px(12.0);
                        let total_chips = tag_names.len() + branch_ref_count;

                        // Reserved width for a trailing "+N" chip; sized for the
                        // worst-case count so mid-loop reservations never come up short.
                        let overflow_reserve = if total_chips > 1 {
                            let probe: SharedString = format!("+{}", total_chips - 1).into();
                            let shaped = shape_truncated_line_cached(
                                window,
                                &base_style,
                                xxs_font,
                                &probe,
                                fx_hash_str(probe.as_ref()),
                                branch_content_bounds.size.width,
                                theme.colors.foreground.secondary,
                                None,
                            );
                            shaped.width + chip_pad_x * 2.0 + chip_gap
                        } else {
                            px(0.0)
                        };

                        let mut x = branch_content_bounds.left();
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
                                ChipEntry::Tag(_) => SmallVec::new(),
                                ChipEntry::Branch(chip) => history_branch_chip_icons(chip),
                            };
                            let icons_width = history_branch_chip_icon_width(
                                icons.as_slice(),
                                branch_icon_size,
                                branch_combined_icon_size,
                                branch_text_icon_gap,
                                branch_icon_gap,
                            );
                            let max_text_w = branch_content_bounds.right()
                                - x
                                - reserve
                                - chip_pad_x * 2.0
                                - icons_width;
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
                                ChipEntry::Branch(chip) => {
                                    history_branch_chip_style_kind(chip, selected_branch.as_ref())
                                }
                            };
                            let context_menu_open = active_context_menu_invoker
                                .as_ref()
                                .is_some_and(|active| match &entry {
                                    ChipEntry::Tag(name) => {
                                        active
                                            == &history_tag_chip_menu_invoker(
                                                repo_id,
                                                &commit_id,
                                                name.as_ref(),
                                            )
                                    }
                                    ChipEntry::Branch(chip) => {
                                        active
                                            == &history_branch_chip_menu_invoker(
                                                repo_id, &commit_id, chip,
                                            )
                                    }
                                });
                            let visual = history_chip_visual(theme, style_kind, context_menu_open);
                            let shaped = match &entry {
                                ChipEntry::Tag(name) => shape_clipped_chip_line_cached_from(
                                    window,
                                    &base_style,
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
                                        &base_style,
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
                            let chip_bounds =
                                Bounds::new(point(x, chip_y), size(chip_w, chip_height));
                            paint_history_chip(
                                window,
                                cx,
                                chip_bounds,
                                &visual,
                                &shaped,
                                chip_radius,
                                chip_border_w,
                                chip_pad_x,
                                xxs_line_height,
                                icons.as_slice(),
                                branch_icon_size,
                                branch_combined_icon_size,
                                branch_text_icon_gap,
                                branch_icon_gap,
                            );
                            match entry {
                                ChipEntry::Tag(_) => tag_chip_bounds.push(chip_bounds),
                                ChipEntry::Branch(chip) => {
                                    branch_chip_hits.push((chip_bounds, chip.clone()));
                                }
                            }

                            shown += 1;
                            x += chip_w + chip_gap;
                        }

                        let hidden = total_chips - shown;
                        if hidden > 0 {
                            let label: SharedString = format!("+{hidden}").into();
                            let shaped = shape_truncated_line_cached(
                                window,
                                &base_style,
                                xxs_font,
                                &label,
                                fx_hash_str(label.as_ref()),
                                (branch_content_bounds.right() - x - chip_pad_x * 2.0).max(px(0.0)),
                                theme.colors.foreground.secondary,
                                None,
                            );
                            let chip_bounds = Bounds::new(
                                point(x, chip_y),
                                size(shaped.width + chip_pad_x * 2.0, chip_height),
                            );
                            let visual = history_chip_visual(
                                theme,
                                HistoryChipStyleKind::Branch { selected: false },
                                false,
                            );
                            paint_history_chip(
                                window,
                                cx,
                                chip_bounds,
                                &visual,
                                &shaped,
                                chip_radius,
                                chip_border_w,
                                chip_pad_x,
                                xxs_line_height,
                                &[],
                                branch_icon_size,
                                branch_combined_icon_size,
                                branch_text_icon_gap,
                                branch_icon_gap,
                            );
                        }
                    },
                );
            }

            let mut summary_text_left =
                summary_bounds.left() + scaled_px(history_message_text_left_px(false));
            if show_graph_color_marker {
                // A lane-coloured border down the left edge of the message cell,
                // where the graph column's fade lands. Inset vertically so
                // consecutive rows read as separate borders rather than as one
                // continuous stripe down the list.
                let border_w = scaled_px(HISTORY_MESSAGE_BORDER_W_PX);
                let inset_y = scaled_px(HISTORY_MESSAGE_BORDER_INSET_Y_PX);
                let border_h = (bounds.size.height - inset_y * 2.0).max(px(0.0));
                window.paint_quad(
                    fill(
                        Bounds::new(
                            point(summary_bounds.left(), bounds.top() + inset_y),
                            size(border_w, border_h),
                        ),
                        node_color,
                    )
                    .corner_radii(border_w * 0.5),
                );
                summary_text_left =
                    summary_bounds.left() + scaled_px(history_message_text_left_px(true));
            }

            let summary_text_bounds = Bounds::new(
                point(summary_text_left, bounds.top()),
                size(
                    (summary_bounds.right() - cell_pad_x - summary_text_left).max(px(0.0)),
                    bounds.size.height,
                ),
            );
            if !summary.is_empty() {
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    sm_font,
                    summary.shared(),
                    summary.text_hash(),
                    summary_text_bounds.size.width.max(px(0.0)),
                    history_summary_color(theme, is_selected_branch_tip, related_to_selection),
                    None,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: summary_text_bounds,
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

            if show_author && !author.is_empty() {
                let avatar_d = scaled_px(components::AVATAR_DIAMETER_PX);
                let avatar_gap = scaled_px(6.0);
                // Matches the header's extra left inset that clears the
                // column resize handle.
                let avatar_left = author_bounds.left() + cell_pad_x * 2.0;
                let identity_color = components::author_color(theme, author.as_ref());
                if author_bounds.size.width >= avatar_d + cell_pad_x * 2.0 {
                    let avatar_top = author_bounds.top()
                        + (author_bounds.size.height - avatar_d).max(px(0.0)) * 0.5;
                    window.paint_quad(
                        fill(
                            Bounds::new(point(avatar_left, avatar_top), size(avatar_d, avatar_d)),
                            with_alpha(identity_color, 0.22),
                        )
                        .corner_radii(avatar_d * 0.5),
                    );

                    let initials: SharedString =
                        components::author_initials(author.as_ref()).into();
                    let initials_font = scaled_px(components::AVATAR_FONT_PX);
                    let initials_line_height = initials_style
                        .line_height
                        .to_pixels(initials_font.into(), window.rem_size());
                    let initials_shaped = shape_truncated_line_cached(
                        window,
                        &initials_style,
                        initials_font,
                        &initials,
                        fx_hash_str(initials.as_ref()),
                        avatar_d,
                        identity_color,
                        None,
                    );
                    let initials_cap_height = initials_shaped
                        .runs
                        .first()
                        .map(|run| window.text_system().cap_height(run.font_id, initials_font))
                        .unwrap_or(initials_font * 0.7);
                    let _ = initials_shaped.paint(
                        point(
                            avatar_left + (avatar_d - initials_shaped.width).max(px(0.0)) * 0.5,
                            components::initials_paint_origin_y(
                                avatar_top,
                                avatar_d,
                                initials_line_height,
                                initials_shaped.ascent,
                                initials_shaped.descent,
                                initials_cap_height,
                            ),
                        ),
                        initials_line_height,
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }

                let author_text_bounds = Bounds::new(
                    point(avatar_left + avatar_d + avatar_gap, author_bounds.top()),
                    size(
                        (author_bounds.right()
                            - cell_pad_x
                            - (avatar_left + avatar_d + avatar_gap))
                            .max(px(0.0)),
                        author_bounds.size.height,
                    ),
                );
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    xs_font,
                    author.shared(),
                    author.text_hash(),
                    author_text_bounds.size.width.max(px(0.0)),
                    theme.colors.foreground.secondary,
                    None,
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: author_text_bounds,
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
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    xxs_font,
                    short_sha.shared(),
                    short_sha.text_hash(),
                    sha_text_bounds.size.width.max(px(0.0)),
                    theme.colors.foreground.secondary,
                    Some(UI_MONOSPACE_FONT_FAMILY),
                );
                let origin_x = (sha_text_bounds.right() - shaped.width).max(sha_text_bounds.left());
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: sha_text_bounds,
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

            // One move listener for both hover affordances. They share the
            // row-level hit test below and differ only in which cell they watch,
            // and a second closure per row would re-box its whole capture set on
            // every frame.
            window.on_mouse_event({
                let view = view.clone();
                let commit_id = commit_id.clone();
                let summary = summary.shared().clone();
                let hover_author = author.shared().clone();
                let hover_when = when.shared().clone();
                let ref_items = Arc::clone(&ref_items);
                let hitbox = hitbox.clone();
                move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    // The row's hitbox — not its bounds — decides whether this
                    // row owns the pointer: window-level listeners run whatever
                    // is painted on top, so anything overlaying the history (the
                    // collapsed sidebar's popover, a panel, a menu) must win.
                    if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                        return;
                    }

                    if !ref_items.is_empty() && branch_bounds.contains(&event.position) {
                        view.update(cx, |this, cx| {
                            this.show_history_refs_hover(
                                repo_id,
                                commit_id.clone(),
                                branch_bounds,
                                Arc::clone(&ref_items),
                                event.position,
                                window,
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
                    if !summary_bounds.contains(&event.position) {
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
                        source_bounds: summary_bounds,
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
                let commit_id = commit_id.clone();
                move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    // Hitbox, not bounds: see the hover listener above. Without
                    // this, right-clicking an overlay that happens to sit over
                    // the history opens this commit's menu through it.
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Right
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }

                    let tag_menu = hit_test_index(&tag_chip_bounds, event.position)
                        .and_then(|ix| tag_names.get(ix))
                        .map(|tag| {
                            let name = tag.as_ref().to_string();
                            let invoker =
                                history_tag_chip_menu_invoker(repo_id, &commit_id, name.as_str());
                            (name, invoker)
                        });
                    let branch_menu = if tag_menu.is_none() {
                        hit_test_branch_chip(&branch_chip_hits, event.position).and_then(|chip| {
                            let kind = history_branch_chip_popover_kind(repo_id, chip)?;
                            let invoker =
                                history_branch_chip_menu_invoker(repo_id, &commit_id, chip);
                            Some((kind, invoker))
                        })
                    } else {
                        None
                    };
                    view.update(cx, |this, cx| {
                        // Right-clicking inside an active multi-selection must
                        // not collapse it — the menu acts on the whole set — but
                        // focus must still move to the clicked commit so the
                        // details pane matches the menu target. Outside the
                        // selection this collapses to the clicked commit.
                        this.store.dispatch(Msg::SelectCommitMulti {
                            repo_id,
                            commit_id: commit_id.clone(),
                            mode: CommitSelectMode::PreserveIfSelected,
                            clicked_index: None,
                            visible_order: None,
                        });
                        let context_menu_invoker = tag_menu
                            .as_ref()
                            .map(|(_, invoker)| invoker.clone())
                            .or_else(|| branch_menu.as_ref().map(|(_, invoker)| invoker.clone()))
                            .unwrap_or_else(|| {
                                format!("history_commit_menu_{}_{}", repo_id.0, commit_id.as_ref())
                                    .into()
                            });
                        this.activate_context_menu_invoker(context_menu_invoker, cx);
                        let kind = match (tag_menu, branch_menu) {
                            (Some((name, _)), _) => PopoverKind::TagRefMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                                name,
                            },
                            (None, Some((kind, _))) => kind,
                            (None, None) => PopoverKind::CommitMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                            },
                        };
                        this.open_popover_at(kind, event.position, window, cx);
                        cx.notify();
                    });
                }
            });
        },
    )
    .h_full()
    .w_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn selected(section: BranchSection, name: &str) -> SelectedHistoryBranch {
        SelectedHistoryBranch {
            section,
            name: name.into(),
        }
    }

    fn branch_chip(
        text: &str,
        is_head: bool,
        targets: &[(BranchSection, &str)],
    ) -> HistoryBranchChipVm {
        let targets = targets
            .iter()
            .map(|(section, name)| BranchMenuTarget {
                section: *section,
                name: (*name).to_string(),
            })
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
        // Selecting the checked-out branch must not restyle its HEAD pill: a
        // ring around an already-solid accent fill reads as an artifact.
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
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_branch_chip_style_kind(
                &head,
                Some(&selected(BranchSection::Remote, "origin/main"))
            ),
            HistoryChipStyleKind::Head
        ));
        let detached = HistoryBranchChipVm {
            text: HistoryTextVm::new("HEAD".into()),
            kind: HistoryBranchChipKind::DetachedHead,
        };
        assert!(matches!(
            history_branch_chip_style_kind(&detached, None),
            HistoryChipStyleKind::Head
        ));
    }

    #[test]
    fn grouped_branch_chip_icons_and_context_menu_keep_exact_refs() {
        let repo_id = RepoId(5);
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
        assert!(matches!(
            history_branch_chip_popover_kind(repo_id, &combined),
            Some(PopoverKind::BranchRefsMenu {
                repo_id: routed_repo,
                ref display_name,
                ref targets,
            }) if routed_repo == repo_id
                && display_name == "feature/x"
                && targets == &vec![
                    BranchMenuTarget {
                        section: BranchSection::Local,
                        name: "feature/x".to_string(),
                    },
                    BranchMenuTarget {
                        section: BranchSection::Remote,
                        name: "origin/feature/x".to_string(),
                    },
                ]
        ));

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
        assert!(matches!(
            history_branch_chip_popover_kind(repo_id, &remote_only),
            Some(PopoverKind::BranchMenu {
                repo_id: routed_repo,
                section: BranchSection::Remote,
                ref name,
            }) if routed_repo == repo_id && name == "origin/feature/x"
        ));

        let detached = HistoryBranchChipVm {
            text: HistoryTextVm::new("HEAD".into()),
            kind: HistoryBranchChipKind::DetachedHead,
        };
        assert!(history_branch_chip_icons(&detached).is_empty());
        assert!(history_branch_chip_popover_kind(repo_id, &detached).is_none());
    }

    fn ref_item(kind: HistoryRefListItemKind) -> HistoryRefListItem {
        HistoryRefListItem {
            text: HistoryTextVm::new(SharedString::from("chip")),
            kind,
        }
    }

    #[test]
    fn summary_lifts_on_the_selected_branch_tip_including_the_checked_out_branch() {
        // The HEAD chip carries no selection state, so the summary is the only
        // cue left when the picked branch is the one that is checked out.
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

    #[test]
    fn selected_chips_are_visibly_apart_from_their_unselected_form() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let rgba = |c: gpui::Rgba| (c.red, c.green, c.blue, c.alpha);
            let sel = history_chip_visual(
                theme,
                HistoryChipStyleKind::Branch { selected: true },
                false,
            );
            let plain = history_chip_visual(
                theme,
                HistoryChipStyleKind::Branch { selected: false },
                false,
            );
            let head = history_chip_visual(theme, HistoryChipStyleKind::Head, false);

            // A border-only difference is what made the sidebar's branch
            // selection invisible on the revealed row; the fill has to carry it.
            assert_ne!(
                rgba(sel.bg),
                rgba(plain.bg),
                "selected branch chip must not reuse the plain chip fill"
            );
            assert_ne!(
                rgba(sel.bg),
                rgba(head.bg),
                "selected branch chip must stay distinguishable from HEAD"
            );
        }
    }

    #[test]
    fn chips_with_open_context_menus_have_a_distinct_visual() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let rgba = |c: gpui::Rgba| (c.red, c.green, c.blue, c.alpha);
            for kind in [
                HistoryChipStyleKind::Tag,
                HistoryChipStyleKind::Branch { selected: false },
                HistoryChipStyleKind::Branch { selected: true },
            ] {
                let closed = history_chip_visual(theme, kind, false);
                let open = history_chip_visual(theme, kind, true);
                assert_ne!(
                    rgba(closed.bg),
                    rgba(open.bg),
                    "opening a chip menu must strengthen its fill"
                );
                assert_ne!(
                    rgba(closed.border),
                    rgba(open.border),
                    "opening a chip menu must strengthen its outline"
                );
            }

            let closed_head = history_chip_visual(theme, HistoryChipStyleKind::Head, false);
            let open_head = history_chip_visual(theme, HistoryChipStyleKind::Head, true);
            assert_eq!(open_head.bg, closed_head.bg);
            assert_eq!(open_head.text, closed_head.text);
            assert_eq!(open_head.border, theme.colors.accent.on_solid);
            assert_ne!(rgba(open_head.border), rgba(closed_head.border));
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
    fn local_branch_chip_icons_match_the_sidebar_accent() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let plain = history_chip_visual(
                theme,
                HistoryChipStyleKind::Branch { selected: false },
                false,
            );
            assert_eq!(
                HistoryBranchChipIcon::Local.color(&plain),
                theme.colors.accent.foreground
            );
            assert_eq!(
                HistoryBranchChipIcon::LocalRemote.color(&plain),
                theme.colors.foreground.secondary,
                "the combined icon's cloud base should stay neutral"
            );
            let (foreground_path, foreground_color) = HistoryBranchChipIcon::LocalRemote
                .foreground_layer(&plain)
                .expect("the combined icon should paint a computer foreground");
            assert_eq!(foreground_path, "icons/computer-cloud-foreground.svg");
            assert_eq!(foreground_color, theme.colors.accent.foreground);
            assert_eq!(
                HistoryBranchChipIcon::Remote.color(&plain),
                plain.remote_branch_icon
            );

            let head = history_chip_visual(theme, HistoryChipStyleKind::Head, false);
            assert_eq!(
                HistoryBranchChipIcon::Local.color(&head),
                theme.colors.accent.on_solid,
                "the current branch icon must retain contrast on its solid accent chip"
            );
            assert_eq!(
                HistoryBranchChipIcon::LocalRemote.color(&head),
                with_alpha(theme.colors.accent.on_solid, 0.70),
                "the combined cloud must retain contrast on a solid HEAD chip"
            );
            assert_eq!(
                HistoryBranchChipIcon::LocalRemote
                    .foreground_layer(&head)
                    .expect("the combined icon should retain its foreground layer")
                    .1,
                theme.colors.accent.on_solid,
                "the combined computer must retain contrast on a solid HEAD chip"
            );
        }
    }

    #[test]
    fn active_branch_chip_icons_are_white_outlines() {
        let white = gpui::rgba(0xffffffff);
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            for visual in [
                history_chip_visual(
                    theme,
                    HistoryChipStyleKind::Branch { selected: true },
                    false,
                ),
                history_chip_visual(
                    theme,
                    HistoryChipStyleKind::Branch { selected: false },
                    true,
                ),
                history_chip_visual(theme, HistoryChipStyleKind::Branch { selected: true }, true),
            ] {
                assert_eq!(HistoryBranchChipIcon::Local.color(&visual), white);
                assert_eq!(HistoryBranchChipIcon::Remote.color(&visual), white);
                assert_eq!(HistoryBranchChipIcon::LocalRemote.color(&visual), white);
                assert_eq!(
                    HistoryBranchChipIcon::LocalRemote
                        .foreground_layer(&visual)
                        .expect("the combined icon should paint its computer layer")
                        .1,
                    white
                );
            }
        }
    }

    #[test]
    fn hit_test_index_returns_clicked_chip_index() {
        let chips = vec![
            Bounds::new(point(px(0.0), px(0.0)), size(px(10.0), px(10.0))),
            Bounds::new(point(px(20.0), px(0.0)), size(px(10.0), px(10.0))),
        ];
        assert_eq!(hit_test_index(&chips, point(px(5.0), px(5.0))), Some(0));
        assert_eq!(hit_test_index(&chips, point(px(25.0), px(5.0))), Some(1));
        assert_eq!(hit_test_index(&chips, point(px(15.0), px(5.0))), None);
    }
}
