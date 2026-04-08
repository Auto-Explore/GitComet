use super::*;
use gpui::{
    Bounds, ContentMask, CursorStyle, DispatchPhase, HitboxBehavior, MouseButton, fill, point, px,
    size,
};
use rustc_hash::FxHasher;
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
    max_width: Pixels,
    color: gpui::Rgba,
    font_family: Option<&'static str>,
) -> gpui::ShapedLine {
    use std::hash::{Hash, Hasher};

    let key = {
        let mut hasher = FxHasher::default();
        text.as_ref().hash(&mut hasher);
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
    let mut runs = vec![style.to_run(text.len())];
    let mut wrapper = window.text_system().line_wrapper(style.font(), font_size);
    let truncated = wrapper.truncate_line(text.clone(), max_width.max(px(0.0)), "…", &mut runs);
    let shaped = window
        .text_system()
        .shape_line(truncated, font_size, &runs, None);

    HISTORY_TEXT_LAYOUT_CACHE.with(|cache| {
        cache.borrow_mut().put(key, shaped.clone());
    });

    shaped
}

fn shape_truncated_line_with_highlights(
    window: &mut Window,
    base_style: &gpui::TextStyle,
    font_size: Pixels,
    text: &SharedString,
    max_width: Pixels,
    color: gpui::Rgba,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
    font_family: Option<&'static str>,
) -> gpui::ShapedLine {
    let mut style = base_style.clone();
    style.color = color.into();
    if let Some(family) = font_family {
        style.font_family = family.into();
    }

    let mut runs = compute_highlight_runs(text.as_ref(), &style, highlights);
    let mut wrapper = window.text_system().line_wrapper(style.font(), font_size);
    let truncated = wrapper.truncate_line(text.clone(), max_width.max(px(0.0)), "…", &mut runs);
    window
        .text_system()
        .shape_line(truncated, font_size, &runs, None)
}

fn compute_highlight_runs(
    text: &str,
    default_style: &gpui::TextStyle,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn layout_chip_bounds(
    branch_bounds: Bounds<Pixels>,
    row_bounds: Bounds<Pixels>,
    chip_height: Pixels,
    gap: Pixels,
    chip_widths: &[Pixels],
) -> Vec<Bounds<Pixels>> {
    let y = row_bounds.top() + (row_bounds.size.height - chip_height).max(px(0.0)) * 0.5;
    let mut x = branch_bounds.left();
    let mut out = Vec::with_capacity(chip_widths.len());
    for w in chip_widths {
        let w = (*w).max(px(0.0));
        if x + w > branch_bounds.right() {
            break;
        }
        out.push(Bounds::new(point(x, y), size(w, chip_height)));
        x += w + gap;
        if x >= branch_bounds.right() {
            break;
        }
    }
    out
}

fn hit_test_any(bounds: &[Bounds<Pixels>], p: gpui::Point<Pixels>) -> bool {
    bounds.iter().any(|b| b.contains(&p))
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
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    show_graph_color_marker: bool,
    is_stash_node: bool,
    connect_from_top_col: Option<usize>,
    graph_rows: Arc<[history_graph::GraphRow]>,
    graph_row_ix: usize,
    tag_names: Arc<[SharedString]>,
    branches_text: SharedString,
    branch_highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    author: SharedString,
    summary: SharedString,
    when: SharedString,
    short_sha: SharedString,
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
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);

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
            let cell_pad_x = px(HISTORY_COL_HANDLE_PX / 2.0);

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
            let graph_bounds = Bounds::new(
                point(x, bounds.top()),
                size(col_graph.max(px(0.0)), bounds.size.height),
            );
            x += col_graph;

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

            let chip_height = px(HISTORY_TAG_CHIP_HEIGHT_PX);
            let chip_pad_x = px(HISTORY_TAG_CHIP_PADDING_X_PX);
            let chip_gap = px(HISTORY_TAG_CHIP_GAP_PX);

            let branch_content_bounds = Bounds::new(
                point(branch_bounds.left() + cell_pad_x, branch_bounds.top()),
                size(
                    (branch_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                    branch_bounds.size.height,
                ),
            );

            let mut tag_chip_bounds: Vec<Bounds<Pixels>> = Vec::with_capacity(tag_names.len());
            if !tag_names.is_empty() || !branches_text.as_ref().trim().is_empty() {
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: branch_content_bounds,
                    }),
                    |window| {
                        let mut x = branch_content_bounds.left();
                        let mut chip_widths: Vec<Pixels> = Vec::with_capacity(tag_names.len());
                        let mut chip_texts: Vec<gpui::ShapedLine> =
                            Vec::with_capacity(tag_names.len());

                        for name in tag_names.iter() {
                            let remaining = (branch_content_bounds.right() - x).max(px(0.0));
                            if remaining <= chip_pad_x * 2.0 {
                                break;
                            }

                            let shaped = shape_truncated_line_cached(
                                window,
                                &base_style,
                                xs_font,
                                name,
                                (remaining - chip_pad_x * 2.0).max(px(0.0)),
                                theme.colors.accent,
                                None,
                            );

                            let chip_w = (shaped.width + chip_pad_x * 2.0).min(remaining);
                            chip_widths.push(chip_w);
                            chip_texts.push(shaped);

                            x += chip_w + chip_gap;
                            if x >= branch_content_bounds.right() {
                                break;
                            }
                        }

                        tag_chip_bounds = layout_chip_bounds(
                            branch_content_bounds,
                            bounds,
                            chip_height,
                            chip_gap,
                            &chip_widths,
                        );

                        for (shaped, chip_bounds) in chip_texts.iter().zip(tag_chip_bounds.iter()) {
                            let border = with_alpha(theme.colors.accent, 0.35);
                            let bg = with_alpha(theme.colors.accent, 0.12);
                            let radius = px(theme.radii.pill);

                            window.paint_quad(fill(*chip_bounds, border).corner_radii(radius));
                            let inner = Bounds::new(
                                point(chip_bounds.left() + px(1.0), chip_bounds.top() + px(1.0)),
                                size(
                                    (chip_bounds.size.width - px(2.0)).max(px(0.0)),
                                    (chip_bounds.size.height - px(2.0)).max(px(0.0)),
                                ),
                            );
                            window.paint_quad(
                                fill(inner, bg).corner_radii((radius - px(1.0)).max(px(0.0))),
                            );

                            let text_y = chip_bounds.top()
                                + (chip_bounds.size.height - xs_line_height).max(px(0.0)) * 0.5;
                            let _ = shaped.paint(
                                point(chip_bounds.left() + chip_pad_x, text_y),
                                xs_line_height,
                                window,
                                cx,
                            );
                        }

                        let x = if let Some(last) = tag_chip_bounds.last() {
                            (last.right() + chip_gap).min(branch_content_bounds.right())
                        } else {
                            branch_content_bounds.left()
                        };

                        if !branches_text.as_ref().trim().is_empty()
                            && x < branch_content_bounds.right()
                        {
                            let remaining = (branch_content_bounds.right() - x).max(px(0.0));
                            let shaped = if branch_highlights.is_empty() {
                                shape_truncated_line_cached(
                                    window,
                                    &base_style,
                                    xs_font,
                                    &branches_text,
                                    remaining,
                                    theme.colors.text_muted,
                                    None,
                                )
                            } else {
                                shape_truncated_line_with_highlights(
                                    window,
                                    &base_style,
                                    xs_font,
                                    &branches_text,
                                    remaining,
                                    theme.colors.text_muted,
                                    branch_highlights.as_ref(),
                                    None,
                                )
                            };
                            let _ = shaped.paint(
                                point(x, center_y(xs_line_height)),
                                xs_line_height,
                                window,
                                cx,
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

            if show_graph_color_marker {
                let marker_w = px(2.0);
                let marker_h = px(12.0);
                let y = bounds.top() + (bounds.size.height - marker_h) * 0.5;
                window.paint_quad(
                    fill(
                        Bounds::new(point(summary_bounds.left(), y), size(marker_w, marker_h)),
                        node_color,
                    )
                    .corner_radii(px(2.0)),
                );
            }

            let summary_text_bounds = Bounds::new(
                point(summary_bounds.left() + cell_pad_x, bounds.top()),
                size(
                    (summary_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                    bounds.size.height,
                ),
            );
            if !summary.as_ref().is_empty() {
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    sm_font,
                    &summary,
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
                            window,
                            cx,
                        );
                    },
                );
            }

            if show_author && !author.as_ref().is_empty() {
                let author_text_bounds = Bounds::new(
                    point(author_bounds.left() + cell_pad_x, author_bounds.top()),
                    size(
                        (author_bounds.size.width - cell_pad_x * 2.0).max(px(0.0)),
                        author_bounds.size.height,
                    ),
                );
                let shaped = shape_truncated_line_cached(
                    window,
                    &base_style,
                    xs_font,
                    &author,
                    author_text_bounds.size.width.max(px(0.0)),
                    theme.colors.text_muted,
                    None,
                );
                let origin_x =
                    (author_text_bounds.right() - shaped.width).max(author_text_bounds.left());
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: author_text_bounds,
                    }),
                    |window| {
                        let _ = shaped.paint(
                            point(origin_x, center_y(xs_line_height)),
                            xs_line_height,
                            window,
                            cx,
                        );
                    },
                );
            }

            if show_date && !when.as_ref().is_empty() {
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
                    &when,
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
                            window,
                            cx,
                        );
                    },
                );
            }

            if show_sha && !short_sha.as_ref().is_empty() {
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
                    &short_sha,
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
                            window,
                            cx,
                        );
                    },
                );
            }

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

                    let is_tag = hit_test_any(&tag_chip_bounds, event.position);
                    view.update(cx, |this, cx| {
                        this.store.dispatch(Msg::SelectCommit {
                            repo_id,
                            commit_id: commit_id.clone(),
                        });
                        let context_menu_invoker: SharedString =
                            format!("history_commit_menu_{}_{}", repo_id.0, commit_id.as_ref())
                                .into();
                        this.activate_context_menu_invoker(context_menu_invoker, cx);
                        let kind = if is_tag {
                            PopoverKind::TagMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
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
    .h(px(24.0))
    .w_full()
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_chip_bounds_never_overflows_branch_column() {
        let row = Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(24.0)));
        let branch = Bounds::new(point(px(10.0), px(0.0)), size(px(100.0), px(24.0)));
        let chip_height = px(18.0);
        let gap = px(4.0);
        let chip_widths = vec![px(40.0), px(40.0), px(40.0)];

        let chips = layout_chip_bounds(branch, row, chip_height, gap, &chip_widths);
        assert!(!chips.is_empty());
        for b in chips {
            assert!(b.left() >= branch.left());
            assert!(b.right() <= branch.right());
            assert!(b.top() >= row.top());
            assert!(b.bottom() <= row.bottom());
        }
    }

    #[test]
    fn hit_test_any_detects_points_inside_chips() {
        let chips = vec![
            Bounds::new(point(px(0.0), px(0.0)), size(px(10.0), px(10.0))),
            Bounds::new(point(px(20.0), px(0.0)), size(px(10.0), px(10.0))),
        ];
        assert!(hit_test_any(&chips, point(px(5.0), px(5.0))));
        assert!(hit_test_any(&chips, point(px(25.0), px(5.0))));
        assert!(!hit_test_any(&chips, point(px(15.0), px(5.0))));
    }
}
