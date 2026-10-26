use super::*;
use gitcomet_extension_api::{
    DiffInlineCard, DiffInset, DiffLayout, DiffLineRange, DiffSelectionAction,
};

#[path = "hosted_context_capture.rs"]
mod capture;

#[gpui::test]
fn context_actions_target_the_clicked_side_or_its_selection_and_reject_stale_menus(
    cx: &mut gpui::TestAppContext,
) {
    let (_dir, store, view, cx) = open_repository(cx);
    let acted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = acted.clone();
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        host.create_snapshot_pane(
            DiffSnapshot::new(
                "notes.txt",
                "one\ntwo\nthree\nfour\n",
                "one\nTWO\nthree\nfour\n",
            ),
            DiffPaneOptions {
                layout: DiffLayout::Split,
                context_actions: vec![DiffSelectionAction::new("Inspect", move |range, _| {
                    record.borrow_mut().push(range)
                })],
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settle(cx, &view, &store, "snapshot", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let renderer = hosted_renderer(cx, &pane);
    let actions = cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.ensure_diff_visible_indices();
            let row = |line| {
                (0..renderer.diff_visible_len())
                    .find(|row| renderer.hosted_line_numbers(*row).1 == Some(line))
                    .unwrap()
            };
            let (two, three, four) = (row(2), row(3), row(4));
            renderer.select_hosted_row(two, false);
            renderer.select_hosted_row(three, true);
            [
                renderer.hosted_context_actions(three, DiffTextRegion::SplitRight, cx)[0].clone(),
                renderer.hosted_context_actions(four, DiffTextRegion::SplitRight, cx)[0].clone(),
                renderer.hosted_context_actions(two, DiffTextRegion::SplitLeft, cx)[0].clone(),
            ]
        })
    });
    for action in &actions {
        cx.update(|_, app| action.invoke(app));
        cx.run_until_parked();
    }
    assert_eq!(
        *acted.borrow(),
        [
            DiffLineRange {
                side: DiffLineSide::New,
                start: 2,
                end: 3
            },
            DiffLineRange {
                side: DiffLineSide::New,
                start: 4,
                end: 4
            },
            DiffLineRange {
                side: DiffLineSide::Old,
                start: 2,
                end: 2
            },
        ]
    );
    assert_eq!(
        cx.update(|_, app| pane.text_for_range(
            DiffLineRange {
                side: DiffLineSide::Old,
                start: 2,
                end: 2
            },
            app
        )),
        Some("two".into())
    );
    cx.update(|_, app| {
        pane.set_context_actions(Vec::new(), app);
        actions[0].invoke(app);
    });
    cx.run_until_parked();
    assert_eq!(acted.borrow().len(), 3);
    cx.update(|_, app| {
        pane.set_snapshot(
            DiffSnapshot::new("blank.txt", "\n\nfoo\n", "\n\nbar\n"),
            app,
        )
    });
    settle(cx, &view, &store, "blank-line snapshot", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    assert_eq!(
        cx.update(|_, app| pane.text_for_range(
            DiffLineRange {
                side: DiffLineSide::Old,
                start: 1,
                end: 3
            },
            app
        )),
        Some("\n\nfoo".into())
    );
    assert_eq!(
        cx.update(|_, app| pane.text_for_range(
            DiffLineRange {
                side: DiffLineSide::Old,
                start: 2,
                end: 100
            },
            app
        )),
        None
    );
}

#[gpui::test]
fn inline_structural_selection_uses_the_old_text_it_draws(cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(cx);
    let acted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = acted.clone();
    let pane = cx.update(|_, app| {
        view.read(app)
            .extension_window
            .as_ref()
            .unwrap()
            .host()
            .create_snapshot_pane(
                DiffSnapshot::new(
                    "code.rs",
                    "pub fn first() {}\npub fn second() {}\n",
                    "fn first() {}\nfn second() {}\n",
                ),
                DiffPaneOptions {
                    layout: DiffLayout::Inline,
                    engine_override: Some(gitcomet_core::diff_engine::DiffEngine::Difftastic),
                    context_actions: vec![DiffSelectionAction::new("Inspect", move |range, _| {
                        record.borrow_mut().push(range)
                    })],
                    ..Default::default()
                },
                app,
            )
            .unwrap()
    });
    settle(cx, &view, &store, "structural snapshot", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let renderer = hosted_renderer(cx, &pane);
    let action = cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.ensure_diff_visible_indices();
            let old_rows: Vec<_> = (0..renderer.diff_visible_len())
                .filter(|row| renderer.hosted_line_numbers(*row).0.is_some())
                .collect();
            assert_eq!(old_rows.len(), 2);
            assert_eq!(
                renderer.hosted_line_numbers(old_rows[0]),
                (Some(1), Some(1))
            );
            renderer.select_hosted_row(old_rows[0], false);
            renderer.select_hosted_row(old_rows[1], true);
            renderer.hosted_context_actions(old_rows[1], DiffTextRegion::Inline, cx)[0].clone()
        })
    });
    cx.update(|_, app| action.invoke(app));
    cx.run_until_parked();
    assert_eq!(
        *acted.borrow(),
        [DiffLineRange {
            side: DiffLineSide::Old,
            start: 1,
            end: 2
        }]
    );
}

#[gpui::test]
fn inline_cards_wrap_with_the_pane_and_keep_notes_and_source_rows(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (_dir, store, view, app_cx) = open_repository(cx);
    let pane = app_cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        host.create_snapshot_pane(
            DiffSnapshot::new("notes.txt", "one\ntwo\nthree\n", "one\nTWO\nthree\n"),
            DiffPaneOptions {
                layout: DiffLayout::Inline,
                content_width: Some(700.0),
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settle(app_cx, &view, &store, "card snapshot", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let range = DiffLineRange {
        side: DiffLineSide::New,
        start: 2,
        end: 2,
    };
    let card = DiffInlineCard::new(
        "answer",
        range,
        "Explanation",
        "An answer long enough to wrap across several rows when the pane is narrow. ".repeat(6),
    )
    .pending(true);
    app_cx.update(|_, app| {
        pane.set_insets(
            vec![DiffInset::new(
                DiffLineSide::New,
                2,
                ["A review note".into()],
            )],
            app,
        );
        pane.set_inline_cards(vec![card.clone()], app);
    });
    let renderer = hosted_renderer(app_cx, &pane);
    let mounted = pane.view();
    let (_, cx) = cx.add_window_view(move |_, _| PaneHolder(mounted));
    let count = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            renderer.update(app, |renderer, cx| {
                renderer.prepare_hosted_cards(window, cx);
                renderer.diff_list_len()
            })
        })
    };
    let wide = count(cx);
    cx.update(|_, app| pane.set_content_width(Some(180.0), app));
    let narrow = count(cx);
    assert!(narrow > wide, "the card wraps when its viewport narrows");
    assert_eq!(
        rows_with(cx, &pane, "answer long"),
        0,
        "source search ignores cards"
    );
    assert_eq!(
        cx.update(|_, app| pane.text_for_range(range, app)),
        Some("TWO".into())
    );
    cx.update(|_, app| pane.set_inline_cards(Vec::new(), app));
    assert_eq!(
        count(cx),
        5,
        "four source rows plus the existing review note remain"
    );
    cx.update(|_, app| {
        pane.set_inline_cards(vec![card], app);
        pane.set_snapshot(DiffSnapshot::new("other.txt", "old\n", "new\n"), app);
    });
    cx.run_until_parked();
    assert_eq!(
        count(cx),
        2,
        "a replacement source drops its transient cards"
    );
}

#[gpui::test]
fn inline_card_actions_stay_on_their_side_and_streaming_preserves_the_source_scroll(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    use gitcomet_extension_api::{DiffScrollAnchor, HostedAction};
    let (_dir, store, view, app_cx) = open_repository(cx);
    let pane = app_cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        host.create_snapshot_pane(
            DiffSnapshot::new(
                "notes.txt",
                numbered("line", 100, None),
                numbered("line", 100, Some(1)),
            ),
            DiffPaneOptions {
                layout: DiffLayout::Split,
                ..Default::default()
            },
            app,
        )
        .unwrap()
    });
    settle(app_cx, &view, &store, "card snapshot", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let clicked = std::rc::Rc::new(std::cell::Cell::new(0));
    let record = clicked.clone();
    let range = DiffLineRange {
        side: DiffLineSide::New,
        start: 2,
        end: 2,
    };
    let mut card = DiffInlineCard::new("answer", range, "Explanation", "Waiting for response…")
        .pending(true)
        .with_action(HostedAction::new("Stop", move |_| {
            record.set(record.get() + 1)
        }));
    app_cx.update(|_, app| pane.set_inline_cards(vec![card.clone()], app));
    let entity = pane
        .view()
        .downcast::<crate::view::hosted::diff_pane::DiffPaneView>()
        .unwrap();
    let id = app_cx.update(|_, app| entity.read(app).view_id());
    let renderer = hosted_renderer(app_cx, &pane);
    app_cx.update(|_, app| {
        app.set_global(crate::appearance::Appearance {
            density: crate::appearance::UiDensity::Comfortable,
            ..Default::default()
        })
    });
    let mounted = pane.view();
    let (_, cx) = cx.add_window_view(move |_, _| PaneHolder(mounted));
    let draw = |cx: &mut gpui::VisualTestContext| {
        for _ in 0..3 {
            cx.update(|window, app| {
                window.activate();
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
    };
    draw(cx);
    let action = selector(format!("hosted_diff_{id}_card_answer_action_0"));
    let bounds = cx.debug_bounds(action).unwrap();
    let row_bounds = cx
        .debug_bounds(selector(format!("hosted_diff_{id}_card_answer_row_1")))
        .unwrap();
    assert!(
        bounds.top() >= row_bounds.top() && bounds.bottom() <= row_bounds.bottom(),
        "Comfortable-density controls fit their card row"
    );
    let width = cx.update(|window, _| window.viewport_size().width);
    assert!(
        bounds.origin.x >= width / 2.0,
        "the new-side card renders on the right"
    );
    click_with(cx, action, Default::default());
    cx.run_until_parked();
    assert_eq!(clicked.get(), 1);
    draw(cx);
    let key = |cx: &mut gpui::VisualTestContext, key: &str| {
        cx.simulate_keystrokes(key);
        cx.update(|window, app| {
            window.dispatch_event(
                gpui::PlatformInput::KeyUp(gpui::KeyUpEvent {
                    keystroke: gpui::Keystroke::parse(key).unwrap(),
                }),
                app,
            )
        });
        cx.run_until_parked();
    };
    key(cx, "enter");
    assert_eq!(clicked.get(), 2);
    key(cx, "space");
    assert_eq!(clicked.get(), 3);
    key(cx, "ctrl-enter");
    key(cx, "shift-space");
    assert_eq!(
        clicked.get(),
        3,
        "modified keys do not activate a card action"
    );
    cx.update(|_, app| {
        pane.restore_scroll_anchor(
            DiffScrollAnchor {
                side: DiffLineSide::New,
                line: 40,
            },
            app,
        )
    });
    draw(cx);
    let before = cx.update(|_, app| pane.scroll_anchor(app)).unwrap();
    assert_eq!(before.line, 40);
    card.text = "The streamed explanation continues with more words. "
        .repeat(20)
        .into();
    cx.update(|_, app| pane.set_inline_cards(vec![card.clone()], app));
    draw(cx);
    assert_eq!(cx.update(|_, app| pane.scroll_anchor(app)), Some(before));

    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer.diff_scroll_sync = DiffScrollSync::None;
            renderer
                .diff_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(gpui::point(px(0.0), px(0.0)));
            cx.notify();
        })
    });
    draw(cx);
    card.text = "M".repeat(1600).into();
    cx.update(|_, app| pane.set_inline_cards(vec![card], app));
    draw(cx);
    cx.update(|_, app| {
        let renderer = renderer.read(app);
        assert_eq!(
            renderer.diff_scroll.0.borrow().base_handle.offset().y,
            px(0.0)
        );
        let height = renderer.theme.editor_row_height(100);
        let right_top = (-renderer
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
            / height)
            .round() as usize;
        assert_eq!(
            right_top,
            renderer.diff_list_index(39),
            "the unsynchronised right viewport keeps source line 40"
        );
    });
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            renderer
                .diff_split_right_scroll
                .0
                .borrow()
                .base_handle
                .set_offset(gpui::point(px(0.0), px(0.0)));
            cx.notify();
        })
    });
    draw(cx);
    let body_bounds = cx
        .debug_bounds(selector(format!("hosted_diff_{id}_card_answer_row_2")))
        .unwrap();
    cx.update(|_, app| {
        let renderer = renderer.read(app);
        assert!(renderer.diff_split_right_scroll.is_scrollable());
        assert!(
            renderer
                .hosted_card_painted_widths()
                .iter()
                .all(|width| *width <= body_bounds.size.width),
            "wrapped glyphs fit inside the list beside its visible scrollbar"
        );
    });
}
