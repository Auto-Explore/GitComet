//! A pane tells its owner the shown file's change stops and the one its
//! cursor is on.
use super::*;
use gitcomet_core::domain::CommitId;
use gitcomet_extension_api::{DiffChangeStops, DiffLayout, DiffPaneEvent};

type Sent = std::rc::Rc<std::cell::RefCell<Vec<DiffChangeStops>>>;

/// A repository whose `a.rs` has three changes far enough apart to be
/// three blocks, whose `bin.dat` is binary, and whose `notes.md` has two
/// changes in one paragraph.
fn repository_with_three_changes() -> tempfile::TempDir {
    let dir = repository_with_edits_of(1, [0, 0]);
    let root = dir.path();
    std::fs::write(root.join("a.rs"), numbered("a", 40, None)).unwrap();
    std::fs::copy(root.join("a.rs"), root.join("b.rs")).unwrap();
    std::fs::write(root.join("bin.dat"), b"\x00\x01\xff\xfe\x00 one \x80\n").unwrap();
    std::fs::write(
        root.join("notes.md"),
        "# Notes\n\nalpha\nbeta\ngamma\ndelta\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "three"]);
    let edited: String = (0..40)
        .map(|n| match n {
            3 | 18 | 33 => format!("a edited {n}\n"),
            _ => format!("a {n}\n"),
        })
        .collect();
    std::fs::write(root.join("a.rs"), edited).unwrap();
    std::fs::write(root.join("bin.dat"), b"\x00\x01\xff\xfe\x00 two \x80\n").unwrap();
    // Two changes in the source, one changed paragraph once rendered.
    std::fs::write(
        root.join("notes.md"),
        "# Notes\n\nALPHA\nbeta\ngamma\nDELTA\n",
    )
    .unwrap();
    dir
}

fn unstaged(path: &str) -> DiffTarget {
    DiffTarget::working_tree(path.into(), DiffArea::Unstaged)
}

fn draw(cx: &mut gpui::VisualTestContext) {
    for _ in 0..3 {
        crate::view::test_support::redraw(cx);
        cx.run_until_parked();
    }
}

fn last(sent: &Sent) -> Option<(usize, Option<usize>)> {
    sent.borrow()
        .last()
        .map(|stops| (stops.count, stops.current))
}

fn step(cx: &mut gpui::VisualTestContext, pane: &DiffPane) {
    let renderer = hosted_renderer(cx, pane);
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            assert!(renderer.navigate_next_diff_change(cx));
            cx.notify();
        });
    });
    draw(cx);
}

fn stop_reports(sent: &Sent) -> DiffPaneOptions {
    let sent = sent.clone();
    DiffPaneOptions {
        layout: DiffLayout::Inline,
        on_event: Some(std::rc::Rc::new(move |event, _| {
            if let DiffPaneEvent::ChangeStops(stops) = event {
                sent.borrow_mut().push(stops);
            }
        })),
        ..Default::default()
    }
}

fn pane_with_stops(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    target: DiffTarget,
) -> (DiffPane, Sent) {
    let sent: Sent = Default::default();
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        let repository = host.active_repository(app).unwrap().unwrap();
        host.create_diff_pane(&repository, target, stop_reports(&sent), app)
            .unwrap()
    });
    (pane, sent)
}

#[gpui::test]
fn a_pane_reports_its_change_stops_and_the_one_its_cursor_is_on(
    test_cx: &mut gpui::TestAppContext,
) {
    let (_dir, store, view, cx) = open_repository_in(
        test_cx,
        Arc::new(TestBackend),
        repository_with_three_changes(),
    );
    let (pane, sent) = pane_with_stops(cx, &view, unstaged("a.rs"));
    settle(cx, &view, &store, "a.rs loads", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
    draw(cx);

    assert_eq!(last(&sent), Some((3, None)), "opening is not a stop");
    assert!(
        sent.borrow()
            .iter()
            .all(|stops| stops.target == unstaged("a.rs"))
    );
    for current in 0..3 {
        step(cx, &pane);
        assert_eq!(last(&sent), Some((3, Some(current))));
    }

    // The same stops in the other layout are nothing new.
    let before = sent.borrow().len();
    cx.update(|_, app| pane.set_layout(DiffLayout::Split, app));
    draw(cx);
    assert!(
        sent.borrow()[before..].iter().all(|stops| stops.count == 3),
        "{:?}",
        &sent.borrow()[before..]
    );

    // A binary file has nothing to step through.
    cx.update(|_, app| pane.set_target(unstaged("bin.dat"), app));
    settle(cx, &view, &store, "bin.dat loads", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    draw(cx);
    assert_eq!(
        sent.borrow()
            .last()
            .map(|stops| (&stops.target, stops.count)),
        Some((&unstaged("bin.dat"), 0))
    );

    // Rendered Markdown steps through its own changes.
    cx.update(|_, app| pane.set_target(unstaged("notes.md"), app));
    settle(cx, &view, &store, "notes.md loads", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    draw(cx);
    let source = sent.borrow().last().cloned().unwrap();
    assert_eq!(source.target, unstaged("notes.md"));
    let renderer = hosted_renderer(cx, &pane);
    cx.update(|_, app| {
        renderer.update(app, |renderer, cx| {
            let kind = crate::view::RenderedPreviewKind::Markdown;
            let mode = match renderer.rendered_preview_modes.get(kind) {
                crate::view::RenderedPreviewMode::Rendered => {
                    crate::view::RenderedPreviewMode::Source
                }
                crate::view::RenderedPreviewMode::Source => {
                    crate::view::RenderedPreviewMode::Rendered
                }
            };
            renderer.rendered_preview_modes.set(kind, mode);
            cx.notify();
        });
    });
    settle(cx, &view, &store, "the other Markdown mode", |_| {
        sent.borrow()
            .last()
            .is_some_and(|stops| stops.count != source.count)
    });
    let counts = [source.count, sent.borrow().last().unwrap().count];
    assert!(counts.contains(&2) && counts.contains(&1), "{counts:?}");
}

#[gpui::test]
fn a_failed_diff_load_does_not_report_an_empty_diff(test_cx: &mut gpui::TestAppContext) {
    let (_dir, store, view, cx) = open_repository(test_cx);
    let missing = DiffTarget::commit_range(
        CommitId("a".repeat(40).into()),
        Some(CommitId("b".repeat(40).into())),
        None,
    );
    let (pane, sent) = pane_with_stops(cx, &view, missing.clone());
    settle(cx, &view, &store, "the range's patch failed", |_| {
        store.snapshot().repos[0]
            .diff_sessions
            .values()
            .any(|session| {
                session.target == missing && matches!(session.diff_state.diff, Loadable::Error(_))
            })
    });
    let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
    draw(cx);
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());
}

#[gpui::test]
fn a_snapshot_pane_does_not_report_change_stops(test_cx: &mut gpui::TestAppContext) {
    let (_dir, _, view, cx) = open_repository(test_cx);
    let sent: Sent = Default::default();
    let pane = cx.update(|_, app| {
        let host = view.read(app).extension_window.as_ref().unwrap().host();
        host.create_snapshot_pane(
            DiffSnapshot::new("a.rs", "old\n", "new\n"),
            stop_reports(&sent),
            app,
        )
        .unwrap()
    });
    let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
    draw(cx);
    assert_eq!(rows_with(cx, &pane, "new"), 1);
    assert!(sent.borrow().is_empty(), "{:?}", sent.borrow());
}

#[gpui::test]
fn a_new_rename_source_reports_stops_even_when_the_rows_match(test_cx: &mut gpui::TestAppContext) {
    let dir = repository_with_three_changes();
    let base = CommitId(head(dir.path()).into());
    git(dir.path(), &["commit", "-qam", "edit"]);
    let tip = CommitId(head(dir.path()).into());
    let target = DiffTarget::commit_range(base, Some(tip), Some("a.rs".into()));
    let (_dir, store, view, cx) = open_repository_in(test_cx, Arc::new(TestBackend), dir);
    let (pane, sent) = pane_with_stops(cx, &view, target.clone());
    settle(cx, &view, &store, "the original comparison loads", |cx| {
        cx.update(|_, app| !pane.is_loading(app))
    });
    let (_, cx) = test_cx.add_window_view(|_, _| PaneHolder(pane.view()));
    draw(cx);
    assert_eq!(last(&sent), Some((3, None)));

    let rename = target.with_old_path(Some("b.rs".into()));
    cx.update(|_, app| pane.set_target(rename, app));
    settle(cx, &view, &store, "the rename source is reported", |_| {
        sent.borrow()
            .last()
            .is_some_and(|stops| stops.target.old_file_path() == Some(Path::new("b.rs")))
    });
    assert_eq!(last(&sent), Some((3, None)));
}
