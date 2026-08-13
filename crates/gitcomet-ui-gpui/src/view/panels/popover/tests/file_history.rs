//! The file-history picker, which is the newly-windowed list with a real bound
//! on it: the history load caps at 200 commits, which is some sixteen viewports.

use super::*;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state, push_test_state};

const COMMIT_COUNT: usize = 200;

/// A named change to one of the inputs the rows are built from, and the label
/// the assertion reports it under.
type RowsInputBump = (&'static str, fn(&mut PopoverHost));

fn commit(ix: usize) -> gitcomet_core::domain::Commit {
    gitcomet_core::domain::Commit {
        // Commit ids are content hashes, so a distinct one per row is what a
        // real page looks like — and what the cache signature reads.
        id: CommitId(format!("{ix:0>40}").into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: format!("Commit {ix:03} touching the file").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

fn repo_with_file_history(repo_id: RepoId, path: &std::path::Path) -> RepoState {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_file_history_{}", std::process::id()));
    let mut repo = opening_repo_state(repo_id, &workdir);
    repo.history_state.file_history_path = Some(path.to_path_buf());
    repo.history_state.file_history = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: (0..COMMIT_COUNT).map(commit).collect(),
            next_cursor: None,
        }
        .into(),
    );
    repo
}

/// Seeds a repository with a page of [`COMMIT_COUNT`] commits and opens the
/// file-history popover over it.
fn open_file_history(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> gpui::Entity<PopoverHost> {
    let repo_id = RepoId(1);
    let path = std::path::PathBuf::from("src/main.rs");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_file_history(repo_id, &path);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::FileHistory {
                        repo_id,
                        path: path.clone(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    cx.update(|_window, app| view.read(app).popover_host.clone())
}

/// Boilerplate every test below opens with. Expands to statements rather than a
/// block so that rebinding `cx` to the visual context reaches the test body.
macro_rules! file_history_picker {
    ($cx:ident, $host:ident) => {
        let (store, events) = AppStore::new(Arc::new(TestBackend));
        let (view, $cx) =
            $cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let $host = open_file_history(&view, $cx);
    };
}

/// Windowing is no longer opted into picker by picker, so this list gets it by
/// being long: 200 commits in a 340px viewport is some sixteen viewports of
/// content, and every frame — including the ones a hover between rows causes —
/// used to build an element for each.
#[gpui::test]
fn a_long_file_history_renders_only_the_rows_in_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let matched = cx.update(|_window, app| {
        super::super::file_history::cached(popover_host.read(app), "")
            .layout
            .item_indices
            .len()
    });
    assert_eq!(matched, COMMIT_COUNT);

    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_some(),
        "the first row is in view"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_150").is_none(),
        "a row 150 places down must not be built until it is scrolled to"
    );
}

/// Arrowing up from nothing selects the last row, far outside the window. There
/// is no element there for `ScrollHandle::scroll_to_item` to find, so this only
/// works if the picker scrolls by its row geometry.
#[gpui::test]
fn arrowing_to_the_last_file_history_row_scrolls_it_into_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    // `debug_bounds` takes a `&'static str`, so the last row is named by literal.
    const LAST_ROW: &str = "picker_prompt_item_199";
    assert!(
        cx.debug_bounds(LAST_ROW).is_none(),
        "the row this arrows to must start outside the window"
    );

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert_eq!(
        cx.update(|_window, app| popover_host.read(app).file_history_selected_index),
        Some(COMMIT_COUNT - 1),
        "arrowing up from nothing selects the last row"
    );
    assert!(
        cx.debug_bounds(LAST_ROW).is_some(),
        "the selected row has to be scrolled into the window, not left unbuilt"
    );
}

/// The windowed list stands spacers in for the rows it does not render, sized
/// from the geometry alone. If that arithmetic drifted from what rows really
/// paint at, scrolling would drift with it.
#[gpui::test]
fn file_history_row_geometry_matches_the_height_rows_paint_at(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let geometry = cx.update(|_window, app| {
        let rows = super::super::file_history::cached(popover_host.read(app), "");
        components::PickerPromptGeometry::new(&rows.items, &rows.layout, 100u32)
    });
    let first = cx
        .debug_bounds("picker_prompt_item_0")
        .expect("expected the first row to render");
    let second = cx
        .debug_bounds("picker_prompt_item_1")
        .expect("expected the second row to render");

    assert_eq!(
        first.size.height,
        geometry.row_height(0),
        "a painted row must be exactly as tall as the geometry says"
    );
    assert_eq!(
        second.origin.y - first.origin.y,
        geometry.row_top(1) - geometry.row_top(0),
        "the stride between rows must match the geometry"
    );
}

/// `PopoverHost` is an uncached overlay view, so a hover moving between rows
/// re-renders the whole picker. Rebuilding 200 rows on each of those frames is
/// what the cache is here to avoid — and every input the rows read has to be in
/// its signature, or the list goes stale with nothing on screen to say so.
#[gpui::test]
fn file_history_rows_are_reused_until_their_data_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    file_history_picker!(cx, popover_host);

    let rows = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| super::super::file_history::cached(popover_host.read(app), ""))
    };

    let first = rows(cx);
    assert!(
        std::rc::Rc::ptr_eq(&first, &rows(cx)),
        "an unchanged page must hand back the very same rows"
    );

    let bumps: [RowsInputBump; 3] = [
        // Same number of commits, different ones: a signature that only counted
        // the rows would call this unchanged and keep showing the old page.
        ("the commits", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].history_state.file_history = Loadable::Ready(
                gitcomet_core::domain::LogPage {
                    commits: (1_000..1_000 + COMMIT_COUNT).map(commit).collect(),
                    next_cursor: None,
                }
                .into(),
            );
            host.state = Arc::new(state);
        }),
        ("the file", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].history_state.file_history_path =
                Some(std::path::PathBuf::from("src/other.rs"));
            host.state = Arc::new(state);
        }),
        ("the commit being viewed", |host| {
            let mut state = (*host.state).clone();
            state.repos[0].diff_state.diff_target = Some(DiffTarget::Commit {
                commit_id: commit(1).id,
                path: Some(std::path::PathBuf::from("src/main.rs")),
            });
            host.state = Arc::new(state);
        }),
    ];

    let mut previous = rows(cx);
    for (label, bump) in bumps {
        cx.update(|_window, app| {
            popover_host.update(app, |host, _cx| bump(host));
        });
        let rebuilt = rows(cx);
        assert!(
            !std::rc::Rc::ptr_eq(&previous, &rebuilt),
            "changing {label} must rebuild the rows"
        );
        previous = rebuilt;
    }
}
