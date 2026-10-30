use super::*;
use gitcomet_extension_api::{DiffLayout, DiffLineRange, DiffSelectionAction};

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
