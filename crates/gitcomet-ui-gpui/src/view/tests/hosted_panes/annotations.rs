use super::*;

#[gpui::test]
fn pane_contributions_annotate_act_and_inset_without_touching_file_lines(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::{
        DiffAnnotation, DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange,
        DiffSelectionAction,
    };
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());

    let gutter_clicks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let acted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (on_gutter, on_action) = (
        std::rc::Rc::clone(&gutter_clicks),
        std::rc::Rc::clone(&acted),
    );
    let pane = app_cx.update(|_window, app| {
        host.create_snapshot_pane(
            DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n"),
            DiffPaneOptions {
                on_gutter_click: Some(std::rc::Rc::new(move |side, line, _| {
                    on_gutter.borrow_mut().push((side, line))
                })),
                selection_actions: vec![DiffSelectionAction::new("Comment", move |range, _| {
                    on_action.borrow_mut().push(range)
                })],
                // Rows are counted inline, whatever the diff setting says.
                layout: gitcomet_extension_api::DiffLayout::Inline,
                ..DiffPaneOptions::default()
            },
            app,
        )
        .unwrap()
    });
    app_cx.run_until_parked();
    let red = gpui::red();
    app_cx.update(|_window, app| {
        pane.set_insets(
            vec![DiffInset::new(
                DiffLineSide::New,
                2,
                ["note one".into(), "note two".into()],
            )],
            app,
        );
        pane.set_annotations(
            DiffAnnotations::new().with(
                DiffLineSide::New,
                3,
                DiffAnnotation::new(red).with_label("flag"),
            ),
            app,
        );
        pane.set_legend(vec![DiffLegendItem::new("Flagged", red)], app);
    });
    let pane_view = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap_or_else(|_| panic!("a hosted diff pane"));
    let id = app_cx.update(|_window, app| pane_view.read(app).view_id());

    let view_any = pane.view();
    let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    let marker_builds = cx.update(|_, app| pane_view.read(app).marker_builds(app));
    assert!(marker_builds > 0, "the shared renderer placed the markers");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }
    assert!(cx.update(|_, app| pane_view.read(app).shares_renderer_rows(app)));
    // Rows: one, two (removed), TWO, two inset rows, three.
    for part in ["legend", "markers", "row_3", "row_4", "row_5"] {
        assert!(
            cx.debug_bounds(selector(format!("hosted_diff_{id}_{part}")))
                .is_some(),
            "{part} is drawn"
        );
    }
    assert_eq!(
        cx.update(|_window, app| pane_view.read(app).marker_builds(app)),
        marker_builds,
        "unchanged frames reuse the shared marker projection"
    );
    assert_eq!(rows_with(cx, &pane, "note"), 0, "insets are not file lines");
    assert_eq!(rows_with(cx, &pane, "three"), 1);
    rows_with(cx, &pane, "");

    let selection =
        |cx: &mut gpui::VisualTestContext| cx.update(|_window, app| pane.selection(app));
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_0")),
        gpui::Modifiers::default(),
    );
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_2")),
        gpui::Modifiers::shift(),
    );
    cx.run_until_parked();
    let expected = DiffLineRange {
        side: DiffLineSide::New,
        start: 1,
        end: 2,
    };
    assert_eq!(selection(cx), Some(expected));
    assert_eq!(
        cx.update(|_window, app| pane.selected_text(app)).as_deref(),
        Some("one\nTWO")
    );

    // Clicking an inset or a gutter leaves the selection alone.
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_row_3")),
        gpui::Modifiers::default(),
    );
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_gutter_5")),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert_eq!(selection(cx), Some(expected));
    assert_eq!(*gutter_clicks.borrow(), vec![(DiffLineSide::New, 3)]);

    for _ in 0..2 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    click_with(
        cx,
        selector(format!("hosted_diff_{id}_action_0")),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert_eq!(*acted.borrow(), vec![expected]);
}

type Clicks = std::rc::Rc<std::cell::RefCell<Vec<(&'static str, DiffLineSide, u32)>>>;

/// A snapshot pane with a clickable note under new line 2 and an annotation
/// on new line 3, drawn by the same renderer as repository diffs.
fn clickable_pane(
    host: &gitcomet_extension_api::WindowHost,
    policy: DiffPanePolicy,
    clicks: &Clicks,
    app: &mut App,
) -> (
    DiffPane,
    gpui::Entity<crate::view::hosted::diff_pane::DiffPaneView>,
) {
    use crate::view::hosted::diff_pane::DiffPaneView;
    use gitcomet_extension_api::{DiffAnnotation, DiffAnnotations, DiffInset, HostedAction};
    let record = |kind: &'static str| {
        let clicks = clicks.clone();
        Some(std::rc::Rc::new(move |side, line, _: &mut App| {
            clicks.borrow_mut().push((kind, side, line))
        }) as gitcomet_extension_api::DiffGutterAction)
    };
    let options = DiffPaneOptions {
        policy,
        on_gutter_click: record("gutter"),
        on_annotation_click: record("annotation"),
        // Rows are counted inline, whatever the diff setting says.
        layout: gitcomet_extension_api::DiffLayout::Inline,
        ..DiffPaneOptions::default()
    };
    let snapshot = DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n");
    let pane = host.create_snapshot_pane(snapshot, options, app).unwrap();
    let entity = pane.view().downcast::<DiffPaneView>().unwrap();
    let noted = clicks.clone();
    pane.set_insets(
        vec![
            DiffInset::new(DiffLineSide::New, 2, ["note".into()])
                .with_action(HostedAction::new("Open note", move |_| {
                    noted.borrow_mut().push(("inset", DiffLineSide::New, 2))
                })),
        ],
        app,
    );
    pane.set_annotations(
        DiffAnnotations::new().with(
            DiffLineSide::New,
            3,
            DiffAnnotation::new(gpui::red()).with_label("flag"),
        ),
        app,
    );
    (pane, entity)
}

/// A note's click runs its action and a click on an annotation runs the
/// annotation action, in the shared renderer and in the pane's own rows;
/// other lines' gutters still run the gutter action. A read-only pane runs
/// no line action, but its notes stay clickable.
#[gpui::test]
fn insets_and_annotations_run_their_actions(cx: &mut gpui::TestAppContext) {
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let (new2, new3) = ((DiffLineSide::New, 2), (DiffLineSide::New, 3));

    for policy in [DiffPanePolicy::default(), DiffPanePolicy::read_only()] {
        let clicks = Clicks::default();
        let (pane, entity) = cx.update(|app| clickable_pane(&host, policy, &clicks, app));
        cx.run_until_parked();
        let view_any = pane.view();
        let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
        for _ in 0..3 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
        let (id, shared) = cx.update(|_, app| {
            let pane = entity.read(app);
            (pane.view_id(), pane.shares_renderer_rows(app))
        });
        assert!(shared);
        // Rows: one, two (removed), TWO, the note, three.
        let (note, two, three) = (3, 2, 4);
        let click = |cx: &mut gpui::VisualTestContext, part: String| {
            click_with(
                cx,
                selector(format!("hosted_diff_{id}_{part}")),
                Default::default(),
            );
            cx.run_until_parked();
        };
        click(cx, format!("row_{note}"));
        click(cx, format!("gutter_{two}"));
        // The shared lane draws the annotation.
        click(cx, format!("gutter_{three}"));
        let expected: Vec<_> = if policy.line_action {
            [("inset", new2), ("gutter", new2)]
                .into_iter()
                .chain(std::iter::once(("annotation", new3)))
                .map(|(kind, (side, line))| (kind, side, line))
                .collect()
        } else {
            vec![("inset", new2.0, new2.1)]
        };
        assert_eq!(
            *clicks.borrow(),
            expected,
            "line actions: {}",
            policy.line_action
        );
    }
}

/// Where `text`'s row starts its text in `region`: its row and left edge.
fn text_start(
    cx: &mut gpui::VisualTestContext,
    pane: &DiffPane,
    region: DiffTextRegion,
    text: &str,
) -> (usize, Pixels) {
    let renderer = hosted_renderer(cx, pane);
    cx.update(|_, app| {
        renderer
            .read(app)
            .diff_text_hitboxes
            .iter()
            .find(|((_, at), hit)| *at == region && hit.painted_text.contains(text))
            .map(|((row, _), hit)| (*row, hit.bounds.left()))
            .unwrap_or_else(|| panic!("{text} is painted in {region:?}"))
    })
}

/// An annotation's people give its pane a column before the line numbers,
/// on each side of a split, as wide as its largest group: two circles and a
/// "+k" draw there inside its padding, a click there runs the annotation
/// action, and the text, and a drag selecting it, start past it. Without
/// people the lane stays over the line numbers and nothing moves.
#[gpui::test]
fn people_annotations_draw_in_a_column_before_the_rows(cx: &mut gpui::TestAppContext) {
    use gitcomet_extension_api::{DiffAnnotation, DiffAnnotations, DiffLayout, RowGlyph};
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    install_example(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, app_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    publish(app_cx, &view, Arc::new(AppState::test_default()));
    let host =
        app_cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    let annotate = |glyph: RowGlyph| {
        DiffAnnotations::new().with(
            DiffLineSide::New,
            3,
            DiffAnnotation::new(gpui::red())
                .with_label("comments")
                .with_glyph(glyph),
        )
    };
    // With a colour-only annotation on line 1 beside them.
    let people = |names: &[&str]| {
        DiffAnnotations::new()
            .with(
                DiffLineSide::New,
                3,
                DiffAnnotation::new(gpui::red())
                    .with_label("comments")
                    .with_people(names.iter().copied()),
            )
            .with(DiffLineSide::New, 1, DiffAnnotation::new(gpui::blue()))
    };
    // The glyph sits inside the column's padding on both sides.
    let fits = |glyph: Bounds<Pixels>, column: Bounds<Pixels>, what: &str| {
        let padding = px(4.0) - px(0.5);
        assert!(
            glyph.left() - column.left() >= padding && column.right() - glyph.right() >= padding,
            "{what}: {glyph:?} in {column:?}"
        );
    };
    let draw = |cx: &mut gpui::VisualTestContext| {
        for _ in 0..3 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
    };

    for (layout, regions) in [
        (DiffLayout::Inline, &[DiffTextRegion::Inline][..]),
        (
            DiffLayout::Split,
            &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight][..],
        ),
    ] {
        let clicks = Clicks::default();
        let record = clicks.clone();
        let pane = cx.update(|app| {
            host.create_snapshot_pane(
                DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n"),
                DiffPaneOptions {
                    on_annotation_click: Some(std::rc::Rc::new(move |side, line, _| {
                        record.borrow_mut().push(("annotation", side, line))
                    })),
                    layout,
                    ..DiffPaneOptions::default()
                },
                app,
            )
            .unwrap()
        });
        cx.update(|app| pane.set_annotations(annotate(RowGlyph::Text("C".into())), app));
        cx.run_until_parked();
        let view_any = pane.view();
        let (_holder, cx) = cx.add_window_view(move |_, _| PaneHolder(view_any));
        draw(cx);
        let id = cx.update(|_, app| {
            pane.view()
                .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
                .unwrap()
                .read(app)
                .view_id()
        });
        let starts = |cx: &mut gpui::VisualTestContext| {
            regions
                .iter()
                .map(|region| text_start(cx, &pane, *region, "three"))
                .collect::<Vec<_>>()
        };
        let before = starts(cx);
        let row = before.last().unwrap().0;
        let gutter = selector(format!("hosted_diff_{id}_gutter_{row}"));
        let glyph = selector(format!("hosted_diff_{id}_gutter_glyph_{row}"));
        assert_eq!(
            cx.debug_bounds(gutter).unwrap().size.width,
            px(12.0),
            "{layout:?}: without people the lane overlays the line numbers"
        );

        cx.update(|_, app| pane.set_annotations(people(&["Ada Lovelace"]), app));
        draw(cx);
        let after = starts(cx);
        let column = cx.debug_bounds(gutter).unwrap();
        for ((row, start), (row_after, moved)) in before.iter().zip(&after) {
            assert_eq!(row, row_after);
            assert!(
                (*moved - *start - column.size.width).abs() < px(0.01),
                "{layout:?}: each side's text starts past its column: {start:?} to {moved:?}"
            );
        }
        assert!(after.last().unwrap().1 >= column.right());
        let one = cx.debug_bounds(glyph).unwrap();
        assert!(
            (one.size.width - px(16.0)).abs() < px(0.01),
            "one circle: {one:?}"
        );
        fits(one, column, "one person");
        // A colour fills the lane's 12px at the column's start, not the
        // whole column; a click anywhere in the column is still its.
        let fill = cx
            .debug_bounds(selector(format!("hosted_diff_{id}_gutter_fill_0")))
            .expect("line 1's colour");
        let first = cx
            .debug_bounds(selector(format!("hosted_diff_{id}_gutter_0")))
            .unwrap();
        assert!((fill.size.width - px(12.0)).abs() < px(0.01), "{fill:?}");
        assert_eq!(fill.left(), first.left());
        assert!((first.size.width - column.size.width).abs() < px(0.01));

        cx.update(|_, app| pane.set_annotations(people(&["Ada Lovelace", "Grace Hopper"]), app));
        draw(cx);
        fits(
            cx.debug_bounds(glyph).unwrap(),
            cx.debug_bounds(gutter).unwrap(),
            "two people",
        );
        let two = cx.debug_bounds(glyph).unwrap().size.width;
        assert!(
            (two - px(16.0 * 1.6)).abs() <= px(0.5),
            "two circles overlapping by two fifths: {two:?}"
        );

        let three = ["Ada Lovelace", "Grace Hopper", "Alan Turing"];
        cx.update(|_, app| pane.set_annotations(people(&three), app));
        draw(cx);
        let glyph_bounds = cx.debug_bounds(glyph).unwrap();
        assert!(
            glyph_bounds.size.width > two + px(4.0),
            "a third person draws as +1 after the circles: {glyph_bounds:?}"
        );
        fits(
            glyph_bounds,
            cx.debug_bounds(gutter).unwrap(),
            "three people",
        );

        // Fourteen widen the column to fit "+12", whose text fits its room
        // in the editor font and in the UI font alike.
        let fourteen: Vec<String> = (0..14).map(|n| format!("Person {n}")).collect();
        let names: Vec<&str> = fourteen.iter().map(String::as_str).collect();
        cx.update(|_, app| pane.set_annotations(people(&names), app));
        draw(cx);
        let wide = cx.debug_bounds(gutter).unwrap();
        let glyph_bounds = cx.debug_bounds(glyph).unwrap();
        assert!(wide.size.width > column.size.width, "{wide:?}");
        fits(glyph_bounds, wide, "fourteen people");
        let room = glyph_bounds.size.width - two - px(2.0);
        let editor_font =
            cx.update(|_, app| gitcomet_ui_kit::font_preferences::current_editor_font_family(app));
        for family in [Some(editor_font), None] {
            let ink = cx.update(|window, _| {
                let mut font = window.text_style().font();
                if let Some(family) = family.clone() {
                    font.family = family.into();
                }
                let run = gpui::TextRun {
                    len: "+12".len(),
                    font,
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: None,
                };
                window
                    .text_system()
                    .shape_line("+12".into(), px(10.0), &[run])
                    .width
            });
            assert!(
                ink <= room,
                "+12 is {ink:?} in {family:?}, its room {room:?}"
            );
        }

        click_with(cx, gutter, Default::default());
        cx.run_until_parked();
        assert_eq!(
            *clicks.borrow(),
            vec![("annotation", DiffLineSide::New, 3)],
            "{layout:?}"
        );

        // A drag from the second line's text to the third's selects those
        // lines on the side it is made in.
        for region in regions {
            let (side, second) = match region {
                DiffTextRegion::SplitLeft => (DiffLineSide::Old, "two"),
                _ => (DiffLineSide::New, "TWO"),
            };
            let renderer = hosted_renderer(cx, &pane);
            let (from, to) = cx.update(|_, app| {
                let renderer = renderer.read(app);
                let hit = |text: &str| {
                    renderer
                        .diff_text_hitboxes
                        .iter()
                        .find(|((_, at), hit)| at == region && hit.painted_text.contains(text))
                        .map(|(_, hit)| hit.bounds)
                        .unwrap()
                };
                let (second, third) = (hit(second), hit("three"));
                (
                    point(second.left() + px(1.0), second.center().y),
                    third.center(),
                )
            });
            cx.simulate_mouse_move(from, None, Default::default());
            cx.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
            cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Default::default());
            cx.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
            cx.run_until_parked();
            assert_eq!(
                cx.update(|_, app| pane.selection(app)),
                Some(gitcomet_extension_api::DiffLineRange {
                    side,
                    start: 2,
                    end: 3
                }),
                "{region:?}"
            );
            assert_eq!(
                cx.update(|_, app| pane.selected_text(app))
                    .as_deref()
                    .map(|text| text.lines().next().unwrap_or("").to_string()),
                Some(second.to_string()),
                "{region:?}"
            );
        }

        // Without people the column goes and the text moves back.
        cx.update(|_, app| pane.set_annotations(annotate(RowGlyph::Text("C".into())), app));
        draw(cx);
        for ((_, start), (_, back)) in before.iter().zip(&starts(cx)) {
            assert!(
                (*back - *start).abs() < px(0.01),
                "{layout:?}: {start:?} {back:?}"
            );
        }
    }
}
