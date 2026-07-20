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
/// ref column doesn't shout.
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
            border: with_alpha(theme.colors.accent, 0.35),
            bg: with_alpha(theme.colors.accent, 0.12),
            text: theme.colors.accent,
        },
        HistoryChipStyleKind::Head => HistoryChipVisual {
            border: with_alpha(theme.colors.accent, 0.90),
            bg: with_alpha(theme.colors.accent, 0.90),
            text: theme.colors.accent_text,
        },
        HistoryChipStyleKind::Branch { selected } => HistoryChipVisual {
            border: if selected {
                with_alpha(theme.colors.accent, 0.45)
            } else {
                with_alpha(theme.colors.border, 0.90)
            },
            bg: theme.colors.surface_bg_elevated,
            text: if selected {
                selected_branch_label_color(theme)
            } else {
                theme.colors.text_muted
            },
        },
    }
}

fn history_chip_style_kind(
    kind: &HistoryRefListItemKind,
    selected_branch_entry_text: Option<&SharedString>,
    item_text: &str,
) -> HistoryChipStyleKind {
    match kind {
        HistoryRefListItemKind::Tag { .. } => HistoryChipStyleKind::Tag,
        HistoryRefListItemKind::AttachedHead { .. } | HistoryRefListItemKind::DetachedHead => {
            HistoryChipStyleKind::Head
        }
        HistoryRefListItemKind::LocalBranch { .. }
        | HistoryRefListItemKind::RemoteBranch { .. } => HistoryChipStyleKind::Branch {
            selected: selected_branch_entry_text
                .is_some_and(|selected| selected.as_ref() == item_text),
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
    selected_branch_entry_text: Option<SharedString>,
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
                window.paint_quad(fill(bounds, theme.colors.hover));
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
                                graph_bounds,
                                window,
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
                                theme.colors.text_muted,
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
                                        selected_branch_entry_text.as_ref(),
                                        item.text.as_ref(),
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
                                theme.colors.text_muted,
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

            let node_color = graph_row
                .lanes_now
                .get(usize::from(graph_row.node_col))
                .map(|lane| history_graph::lane_color(theme, lane.color_ix))
                .unwrap_or(theme.colors.text_muted);

            let mut summary_text_left = summary_bounds.left() + cell_pad_x;
            if show_graph_color_marker {
                // A rounded lane-color pill with clear air before the text so it
                // reads as a deliberate marker, not a stray glyph.
                let marker_w = scaled_px(3.0);
                let marker_h = scaled_px(14.0);
                let y = bounds.top() + (bounds.size.height - marker_h) * 0.5;
                window.paint_quad(
                    fill(
                        Bounds::new(point(summary_bounds.left(), y), size(marker_w, marker_h)),
                        node_color,
                    )
                    .corner_radii(marker_w * 0.5),
                );
                summary_text_left = summary_bounds.left() + marker_w + scaled_px(6.0);
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
                    theme.colors.text,
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
                    theme.colors.text_muted,
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
                    theme.colors.text_muted,
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
                    theme.colors.text_muted,
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

            window.on_mouse_event({
                let view = view.clone();
                let commit_id = commit_id.clone();
                let ref_items = Arc::clone(&ref_items);
                move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || ref_items.is_empty()
                        || !branch_bounds.contains(&event.position)
                    {
                        return;
                    }

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
                }
            });

            window.on_mouse_event({
                let view = view.clone();
                let commit_id = commit_id.clone();
                move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Right
                        || !bounds.contains(&event.position)
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

    #[test]
    fn history_chip_style_kind_marks_selected_branch() {
        let selected: SharedString = "feat/new_gui".into();
        let kind = HistoryRefListItemKind::LocalBranch {
            name: "feat/new_gui".to_string(),
        };
        assert!(matches!(
            history_chip_style_kind(&kind, Some(&selected), "feat/new_gui"),
            HistoryChipStyleKind::Branch { selected: true }
        ));
        assert!(matches!(
            history_chip_style_kind(&kind, Some(&selected), "feat/other"),
            HistoryChipStyleKind::Branch { selected: false }
        ));
        assert!(matches!(
            history_chip_style_kind(&kind, None, "feat/new_gui"),
            HistoryChipStyleKind::Branch { selected: false }
        ));
    }

    #[test]
    fn history_chip_style_kind_maps_head_and_tags() {
        assert!(matches!(
            history_chip_style_kind(
                &HistoryRefListItemKind::AttachedHead {
                    branch: "main".to_string()
                },
                None,
                "HEAD → main"
            ),
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_chip_style_kind(&HistoryRefListItemKind::DetachedHead, None, "HEAD"),
            HistoryChipStyleKind::Head
        ));
        assert!(matches!(
            history_chip_style_kind(
                &HistoryRefListItemKind::Tag {
                    name: "v1.0".to_string()
                },
                None,
                "v1.0"
            ),
            HistoryChipStyleKind::Tag
        ));
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
