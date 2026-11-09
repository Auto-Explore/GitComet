use super::*;
use gitcomet_extension_api::{
    Extension, ExtensionId, HistoryAnnotator, HistoryRowAnnotation, HostedAction, HostedMenuItem,
    Registrar, Registry, RowMark, SlotSignal,
};
use gitcomet_state::model::ComparisonMark;
use std::cell::Cell;
use std::rc::Rc;

/// Leads every row with a mark whose menu counts how often it was asked for.
struct LeadingMark(Rc<Cell<usize>>);

impl Extension for LeadingMark {
    fn id(&self) -> ExtensionId {
        ExtensionId::new("com.example.leading").unwrap()
    }

    fn register(&self, r: &mut Registrar) {
        let asked = self.0.clone();
        r.history_annotator(
            "leading",
            HistoryAnnotator::new(SlotSignal::default(), |_, _, _| {
                HistoryRowAnnotation::default()
                    .with_leading(RowMark::new(gpui::green()).with_label("Run"))
            })
            .with_mark_menu(move |_, _, _, _| {
                asked.set(asked.get() + 1);
                vec![HostedMenuItem::action(HostedAction::new("Stop", |_| {}))]
            }),
        );
    }
}

fn mark(id: &str, label: &str) -> ComparisonMark {
    ComparisonMark {
        commit_id: CommitId(id.into()),
        label: label.into(),
    }
}

/// Mounts a short log with `mark` as the comparison mark, in a window `width`
/// wide when given.
fn mount(
    cx: &mut gpui::TestAppContext,
    mark: Option<ComparisonMark>,
    width: Option<Pixels>,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    if let Some(width) = width {
        cx.simulate_resize(size(width, px(900.0)));
    }
    let page = Arc::new(log_page(
        (0..4)
            .map(|ix| commit(&format!("c{ix:02}"), &[], &format!("commit {ix:02}")))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-comparison-mark"),
        },
    );
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    repo.navigation.comparison_mark = mark;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    });
    set_history_view_state_for_tests(cx, &view, state.clone());
    ensure_history_cache_for_tests(cx, &view, state);
    wait_until(cx, "first history row", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });
    (view, cx)
}

/// Where along row 0's message cell the pointer finds the comparison mark's
/// chip, and the tooltip it shows there.
fn find_chip(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<(Vec<Pixels>, Option<SharedString>)> {
    let row = cx.debug_bounds("history_row_0").unwrap();
    let message = cx
        .debug_bounds("history_message_header_cell")
        .expect("the message column");
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let mut on_chip = Vec::new();
    let mut tooltip = None;
    let mut x = message.left();
    while x < message.left() + px(160.0) {
        cx.simulate_mouse_move(point(x, row.center().y), None, gpui::Modifiers::default());
        cx.run_until_parked();
        if cx.update(|_, app| history.read(app).row_hover(app))
            == Some((0, HistoryRowHoverArea::ComparisonMark))
        {
            on_chip.push(x);
            tooltip = crate::view::test_support::tooltip_text(cx, view);
        }
        x += px(2.0);
    }
    (!on_chip.is_empty()).then_some((on_chip, tooltip))
}

#[gpui::test]
fn the_marked_commit_shows_a_comparison_mark_chip(cx: &mut gpui::TestAppContext) {
    let (view, cx) = mount(cx, Some(mark("c00", "main")), None);
    let (_, tooltip) = find_chip(cx, &view).expect("the marked row shows the chip");
    assert_eq!(
        tooltip.as_deref(),
        Some(
            "main is marked for comparison. Right-click another commit, branch or tag \
             and choose \"Compare with main\"."
        )
    );
}

#[gpui::test]
fn an_unmarked_commit_shows_no_comparison_mark_chip(cx: &mut gpui::TestAppContext) {
    let (view, cx) = mount(cx, Some(mark("c02", "c02")), None);
    assert_eq!(find_chip(cx, &view), None);
}

/// An extension's leading mark follows the chip rather than sharing its
/// place, so a right-click on the chip is not taken for the mark's.
#[gpui::test]
fn leading_extension_marks_start_after_the_chip(cx: &mut gpui::TestAppContext) {
    let asked = Rc::new(Cell::new(0));
    cx.update(|cx| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(LeadingMark(asked.clone()))]).unwrap(),
            cx,
        )
    });
    let (view, cx) = mount(cx, Some(mark("c00", "main")), None);
    let (on_chip, _) = find_chip(cx, &view).expect("the marked row shows the chip");
    let row = cx.debug_bounds("history_row_0").unwrap();
    let right_click = |cx: &mut gpui::VisualTestContext, x: Pixels| {
        let position = point(x, row.center().y);
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(position, gpui::MouseButton::Right, Default::default());
        cx.simulate_mouse_up(position, gpui::MouseButton::Right, Default::default());
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        cx.update(|_window, app| {
            let popover_host = view.read(app).popover_host.clone();
            popover_host.update(app, |host, cx| host.close_popover(cx));
        });
        cx.run_until_parked();
    };
    for x in [on_chip[on_chip.len() / 2], *on_chip.last().unwrap()] {
        right_click(cx, x);
    }
    assert_eq!(
        asked.get(),
        0,
        "a right-click on the chip opened the mark's menu"
    );
    let chip_right = *on_chip.last().unwrap();
    let mut x = chip_right + px(2.0);
    while asked.get() == 0 && x < chip_right + px(80.0) {
        right_click(cx, x);
        x += px(2.0);
    }
    assert_eq!(asked.get(), 1, "the leading mark sits right after the chip");
}

#[test]
fn marking_a_commit_repaints_the_history() {
    let state = |mark: Option<ComparisonMark>| {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/history-comparison-mark"),
            },
        );
        repo.navigation.comparison_mark = mark;
        AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..AppState::test_default()
        }
    };
    let fingerprint = |mark| HistoryView::notify_fingerprint_for(&state(mark), true);
    assert_ne!(fingerprint(None), fingerprint(Some(mark("c00", "c00"))));
    assert_ne!(
        fingerprint(Some(mark("c00", "c00"))),
        fingerprint(Some(mark("c01", "c01")))
    );
    // A branch and a tag on one tip mark the same commit under two labels.
    assert_ne!(
        fingerprint(Some(mark("c00", "main"))),
        fingerprint(Some(mark("c00", "v1")))
    );
    assert_eq!(
        fingerprint(Some(mark("c00", "main"))),
        fingerprint(Some(mark("c00", "main")))
    );
}

/// A message cell too narrow to keep half for the subject shows only the
/// icon, which still explains itself on hover; narrower still, no chip.
#[gpui::test]
fn a_narrow_message_cell_shows_the_chip_as_its_icon(cx: &mut gpui::TestAppContext) {
    let chip = |cx: &mut gpui::TestAppContext, width| {
        let mut cx = cx.clone();
        let (view, cx) = mount(&mut cx, Some(mark("c00", "main")), Some(px(width)));
        find_chip(cx, &view)
            .map(|(on_chip, tooltip)| (*on_chip.last().unwrap() - on_chip[0], tooltip.is_some()))
    };
    let (labelled, _) = chip(cx, 1400.0).expect("a wide cell shows the labelled chip");
    let (icon, tooltip) = chip(cx, 1100.0).expect("a narrow cell keeps the icon");
    assert!(labelled > px(60.0), "labelled chip spans {labelled:?}");
    assert!(icon < px(30.0), "icon-only chip spans {icon:?}");
    assert!(tooltip);
    assert_eq!(chip(cx, 900.0), None);
}
