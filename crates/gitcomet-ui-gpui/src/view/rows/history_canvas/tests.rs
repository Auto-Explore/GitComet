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
fn history_canvas_graph_origin_tracks_one_x_device_pixel_rounding(cx: &mut gpui::TestAppContext) {
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
fn chip_label_clipping_omits_the_ellipsis_and_reclaims_its_width(cx: &mut gpui::TestAppContext) {
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
            clipped.text.chars().count() > ellipsized.text.trim_start_matches('…').chars().count(),
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
        history_branch_chip_style_kind(&chip, Some(&selected(BranchSection::Remote, "fork/main"))),
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
                let selected = selected_branch_for_history_row(Some(&selected), RepoId(7), true);
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
                    let visual = history_chip_visual(theme, row, HistoryChipStyleKind::Tag, open);
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
