use super::*;
use gitcomet_state::msg::CommitSelectMode;
use gpui::{
    Bounds, ContentMask, CursorStyle, DispatchPhase, HitboxBehavior, MouseButton, TruncateFrom,
    fill, point, px, size,
};
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::cell::RefCell;

const HISTORY_TAG_CHIP_HEIGHT_PX: f32 = 18.0;
const HISTORY_TAG_CHIP_PADDING_X_PX: f32 = 6.0;
const HISTORY_TAG_CHIP_GAP_PX: f32 = 4.0;

const HISTORY_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 8_192;

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
        color.r.to_bits().hash(&mut hasher);
        color.g.to_bits().hash(&mut hasher);
        color.b.to_bits().hash(&mut hasher);
        color.a.to_bits().hash(&mut hasher);
        matches!(truncate_from, TruncateFrom::Start).hash(&mut hasher);
        hasher.finish()
    };

    if let Some(shaped) =
        HISTORY_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned())
    {
        return shaped;
    }

    let mut style = base_style.clone();
    style.color = color.into();
    if let Some(family) = font_family {
        style.font_family = family.into();
    }
    let runs = vec![style.to_run(text.len())];
    let mut wrapper = window.text_system().line_wrapper(style.font(), font_size);
    let (truncated, runs) = wrapper.truncate_line(
        text.clone(),
        max_width.max(px(0.0)),
        "…",
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
}

fn history_chip_visual(theme: AppTheme, kind: HistoryChipStyleKind) -> HistoryChipVisual {
    match kind {
        HistoryChipStyleKind::Tag => HistoryChipVisual {
            border: with_alpha(theme.colors.accent.foreground, 0.35),
            bg: with_alpha(theme.colors.accent.foreground, 0.12),
            text: theme.colors.accent.foreground,
        },
        // The HEAD chip carries no selection state: a ring around a pill that is
        // already a solid accent fill reads as a rendering artifact, not as a
        // selection. Selecting the checked-out branch is left unmarked here.
        HistoryChipStyleKind::Head => HistoryChipVisual {
            border: with_alpha(theme.colors.accent.foreground, 0.90),
            bg: with_alpha(theme.colors.accent.foreground, 0.90),
            text: theme.colors.accent.on_solid,
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
        },
        HistoryChipStyleKind::Branch { selected: false } => HistoryChipVisual {
            border: with_alpha(theme.colors.stroke.default, 0.90),
            bg: theme.colors.surface.raised,
            text: theme.colors.foreground.secondary,
        },
    }
}

/// Whether one rendered ref is the branch selected in the sidebar. Comparison
/// is on ref identity, not the chip's label: a local and a remote branch of the
/// same name are different refs that can share a row, and label matching cannot
/// tell them apart. The checked-out branch matches through its `HEAD → name`
/// chip, which is the only ref item it gets.
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
/// the column splits in two: the commit, everything it descends from and its
/// direct children go to the theme's emphasis foreground, and everything else
/// drops to muted -- so the selected commit's line of history reads as a
/// continuous run.
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

fn history_chip_style_kind(
    kind: &HistoryRefListItemKind,
    selected_branch: Option<&SelectedHistoryBranch>,
) -> HistoryChipStyleKind {
    match kind {
        HistoryRefListItemKind::Tag { .. } => HistoryChipStyleKind::Tag,
        HistoryRefListItemKind::AttachedHead { .. } | HistoryRefListItemKind::DetachedHead => {
            HistoryChipStyleKind::Head
        }
        HistoryRefListItemKind::LocalBranch { .. }
        | HistoryRefListItemKind::RemoteBranch { .. } => HistoryChipStyleKind::Branch {
            selected: history_ref_is_selected_branch(kind, selected_branch),
        },
    }
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
    ref_items: Arc<[HistoryRefListItem]>,
    selected_branch: Option<SelectedHistoryBranch>,
    selected_lane_color_ix: Option<history_graph::LaneColorIx>,
    lane_branch_name: Option<SharedString>,
    author: HistoryTextVm,
    summary: HistoryTextVm,
    when: HistoryTextVm,
    short_sha: HistoryTextVm,
) -> AnyElement {
    super::canvas::keyed_canvas(
        ("history_commit_row_canvas", row_id),
        move |bounds, window, _cx| {
            let pad = window.rem_size() * 0.5;
            let inner = Bounds::new(
                point(bounds.left() + pad, bounds.top()),
                size(
                    (bounds.size.width - pad * 2.0).max(px(0.0)),
                    bounds.size.height,
                ),
            );
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            (inner, pad, hitbox)
        },
        move |bounds, (inner, _pad, hitbox), window, cx| {
            let Some(graph_row) = graph_rows.get(graph_row_ix) else {
                return;
            };
            if hitbox.is_hovered(window) {
                window.paint_quad(fill(bounds, theme.colors.interaction.hover_background));
            }
            // Purple highlight on the commit currently being browsed historically.
            if view
                .read(cx)
                .active_repo()
                .is_some_and(|repo| repo.browsing_commit() == Some(&commit_id))
            {
                window.paint_quad(fill(
                    bounds,
                    crate::theme::with_alpha(crate::theme::historical_outline(theme.is_dark), 0.22),
                ));
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

            let mut x = inner.left();
            let branch_bounds = Bounds::new(
                point(x, bounds.top()),
                size(col_branch.max(px(0.0)), bounds.size.height),
            );
            x += col_branch;
            let graph_w = if show_graph {
                col_graph.max(px(0.0))
            } else {
                px(0.0)
            };
            let graph_bounds =
                Bounds::new(point(x, bounds.top()), size(graph_w, bounds.size.height));
            x += graph_w;

            let mut right_x = inner.right();
            let sha_bounds = if show_sha {
                right_x -= col_sha;
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(col_sha.max(px(0.0)), bounds.size.height),
                )
            } else {
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(px(0.0), bounds.size.height),
                )
            };
            let date_bounds = if show_date {
                right_x -= col_date;
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(col_date.max(px(0.0)), bounds.size.height),
                )
            } else {
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(px(0.0), bounds.size.height),
                )
            };
            let author_bounds = if show_author {
                right_x -= col_author;
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(col_author.max(px(0.0)), bounds.size.height),
                )
            } else {
                Bounds::new(
                    point(right_x, bounds.top()),
                    size(px(0.0), bounds.size.height),
                )
            };

            let summary_right = right_x.max(x);
            let summary_bounds = Bounds::new(
                point(x, bounds.top()),
                size((summary_right - x).max(px(0.0)), bounds.size.height),
            );

            // Everything coloured from this row's lane -- the node, the
            // message border, the fade wash and the hover badge -- washes with
            // that lane, so a row never shows two different strengths of the
            // same colour.
            let related_to_selection =
                selected_lane_color_ix.map(|selected| selected == graph_row.node_color_ix);
            let node_color = super::history_graph_paint::lane_wash_color(
                theme,
                graph_row.node_color_ix,
                selected_lane_color_ix,
            );

            // A lane-coloured wash across the right of the graph column, fading
            // into the border on the message cell so a commit's dot and its
            // message read as one unit. Painted before the lanes so the strokes
            // stay crisp on top of it, and a single gradient quad either way.
            //
            // `graph_bounds.right()` is exactly `summary_bounds.left()`, so the
            // gradient ends where the border begins.
            if show_graph && show_graph_color_marker && graph_bounds.size.width > px(0.0) {
                let fade_w = graph_bounds
                    .size
                    .width
                    .min(scaled_px(HISTORY_GRAPH_FADE_WIDTH_PX));
                window.paint_quad(fill(
                    Bounds::new(
                        point(graph_bounds.right() - fade_w, graph_bounds.top()),
                        size(fade_w, graph_bounds.size.height),
                    ),
                    gpui::linear_gradient(
                        90.0,
                        gpui::linear_color_stop(with_alpha(node_color, 0.0), 0.0),
                        gpui::linear_color_stop(
                            with_alpha(node_color, HISTORY_GRAPH_FADE_ALPHA),
                            1.0,
                        ),
                    ),
                ));
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
                                connect_from_top_col,
                                is_stash_node,
                                selected_lane_color_ix,
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
            let branch_ref_count = ref_items
                .iter()
                .filter(|item| !matches!(item.kind, HistoryRefListItemKind::Tag { .. }))
                .count();
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
                            Ref(&'a HistoryRefListItem),
                        }
                        // HEAD first (the strongest signal), then tags, then
                        // plain branches; overflow beyond the column collapses
                        // into a "+N" chip resolved by the refs hover menu.
                        let head_entries = ref_items
                            .iter()
                            .filter(|item| {
                                matches!(
                                    item.kind,
                                    HistoryRefListItemKind::AttachedHead { .. }
                                        | HistoryRefListItemKind::DetachedHead
                                )
                            })
                            .map(ChipEntry::Ref);
                        let branch_entries = ref_items
                            .iter()
                            .filter(|item| {
                                matches!(
                                    item.kind,
                                    HistoryRefListItemKind::LocalBranch { .. }
                                        | HistoryRefListItemKind::RemoteBranch { .. }
                                )
                            })
                            .map(ChipEntry::Ref);
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
                            let max_text_w =
                                branch_content_bounds.right() - x - reserve - chip_pad_x * 2.0;
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

                            let (shaped, style_kind, is_tag) = match &entry {
                                ChipEntry::Tag(name) => (
                                    shape_truncated_line_cached(
                                        window,
                                        &base_style,
                                        xxs_font,
                                        name.shared(),
                                        name.text_hash(),
                                        max_text_w,
                                        history_chip_visual(theme, HistoryChipStyleKind::Tag).text,
                                        None,
                                    ),
                                    HistoryChipStyleKind::Tag,
                                    true,
                                ),
                                ChipEntry::Ref(item) => {
                                    let style_kind = history_chip_style_kind(
                                        &item.kind,
                                        selected_branch.as_ref(),
                                    );
                                    (
                                        // Truncate from the start so the leaf
                                        // segment ("…/feature_name") stays visible.
                                        shape_truncated_line_cached_from(
                                            window,
                                            &base_style,
                                            xxs_font,
                                            item.text.shared(),
                                            item.text.text_hash(),
                                            max_text_w,
                                            history_chip_visual(theme, style_kind).text,
                                            None,
                                            TruncateFrom::Start,
                                        ),
                                        style_kind,
                                        false,
                                    )
                                }
                            };

                            let chip_w = shaped.width + chip_pad_x * 2.0;
                            let chip_bounds =
                                Bounds::new(point(x, chip_y), size(chip_w, chip_height));
                            let visual = history_chip_visual(theme, style_kind);
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
                            );
                            if is_tag {
                                tag_chip_bounds.push(chip_bounds);
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

                    let tag_name = hit_test_index(&tag_chip_bounds, event.position)
                        .and_then(|ix| tag_names.get(ix))
                        .map(|tag| tag.as_ref().to_string());
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
                        let context_menu_invoker: SharedString =
                            format!("history_commit_menu_{}_{}", repo_id.0, commit_id.as_ref())
                                .into();
                        this.activate_context_menu_invoker(context_menu_invoker, cx);
                        let kind = if let Some(name) = tag_name {
                            PopoverKind::TagRefMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                                name,
                            }
                        } else {
                            PopoverKind::CommitMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                            }
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

    fn selected(section: BranchSection, name: &str) -> SelectedHistoryBranch {
        SelectedHistoryBranch {
            section,
            name: name.into(),
        }
    }

    #[test]
    fn history_chip_style_kind_marks_the_selected_branch_by_identity() {
        let local = HistoryRefListItemKind::LocalBranch {
            name: "feat/new_gui".to_string(),
        };
        assert!(matches!(
            history_chip_style_kind(
                &local,
                Some(&selected(BranchSection::Local, "feat/new_gui"))
            ),
            HistoryChipStyleKind::Branch { selected: true }
        ));
        assert!(matches!(
            history_chip_style_kind(&local, Some(&selected(BranchSection::Local, "feat/other"))),
            HistoryChipStyleKind::Branch { selected: false }
        ));
        assert!(matches!(
            history_chip_style_kind(&local, None),
            HistoryChipStyleKind::Branch { selected: false }
        ));
    }

    #[test]
    fn history_chip_style_kind_keeps_local_and_remote_branches_apart() {
        let local = HistoryRefListItemKind::LocalBranch {
            name: "main".to_string(),
        };
        let remote = HistoryRefListItemKind::RemoteBranch {
            name: "origin/main".to_string(),
        };

        assert!(matches!(
            history_chip_style_kind(
                &remote,
                Some(&selected(BranchSection::Remote, "origin/main"))
            ),
            HistoryChipStyleKind::Branch { selected: true }
        ));
        // Selecting the local branch must not light up its remote twin.
        assert!(matches!(
            history_chip_style_kind(&remote, Some(&selected(BranchSection::Local, "main"))),
            HistoryChipStyleKind::Branch { selected: false }
        ));
        assert!(matches!(
            history_chip_style_kind(
                &local,
                Some(&selected(BranchSection::Remote, "origin/main"))
            ),
            HistoryChipStyleKind::Branch { selected: false }
        ));
    }

    #[test]
    fn history_chip_style_kind_leaves_the_head_chip_unmarked_when_selected() {
        // Selecting the checked-out branch must not restyle its HEAD pill: a
        // ring around an already-solid accent fill reads as an artifact.
        let head = HistoryRefListItemKind::AttachedHead {
            branch: "main".to_string(),
        };
        assert!(matches!(
            history_chip_style_kind(&head, Some(&selected(BranchSection::Local, "main"))),
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_chip_style_kind(&head, Some(&selected(BranchSection::Local, "other"))),
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_chip_style_kind(&HistoryRefListItemKind::DetachedHead, None),
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_chip_style_kind(
                &HistoryRefListItemKind::Tag {
                    name: "v1.0".to_string()
                },
                None
            ),
            HistoryChipStyleKind::Tag
        ));
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
        assert_eq!((on_chain.r, on_chain.g, on_chain.b), (1.0, 1.0, 1.0));

        let light = AppTheme::gitcomet_light();
        let on_chain = history_summary_color(light, false, Some(true));
        assert_eq!((on_chain.r, on_chain.g, on_chain.b), (0.0, 0.0, 0.0));

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
                (plain.r, plain.g, plain.b, plain.a),
                (tip.r, tip.g, tip.b, tip.a),
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
            let rgba = |c: gpui::Rgba| (c.r, c.g, c.b, c.a);
            let sel = history_chip_visual(theme, HistoryChipStyleKind::Branch { selected: true });
            let plain =
                history_chip_visual(theme, HistoryChipStyleKind::Branch { selected: false });
            let head = history_chip_visual(theme, HistoryChipStyleKind::Head);

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
