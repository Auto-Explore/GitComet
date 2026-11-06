//! The diff editor menu shows S, U and D beside its line entries. Pressed with
//! the menu closed, those letters did nothing; against a real repository each
//! must now move exactly the selected lines, and only from the diff itself.

use super::*;

const PATH: &str = "lines.txt";
const BASE: &str = "one\ntwo\nthree\nfour\nfive\n";
/// Two changes in one hunk, so a line action has to split the hunk.
const EDITED: &str = "one\nTWO\nthree\nfour\nFIVE\n";

#[derive(Clone, Copy, Debug)]
enum Select {
    /// A plain click on the row drawing this patch line: a caret, no
    /// selection and no outline.
    Caret(&'static str),
    /// The rows drawing these patch lines, as a shift-click selects them.
    Rows(&'static [&'static str]),
    /// All the text of the row drawing this patch line, as a drag selects it.
    Text(&'static str),
    /// Text selected from the start of the first line to the end of the
    /// second, then a right-click on each listed line's row, in that line's
    /// column, from the chosen focus, and the menu dismissed again.
    TextThenRightClicks(&'static str, &'static str, &'static [&'static str]),
    /// Right-clicks on these lines' rows from the chosen focus, with nothing
    /// selected first, and the menu dismissed again.
    RightClicks(&'static [&'static str]),
    /// F3 pressed this many times from the chosen focus. Opening the diff
    /// outlines its first change block and each F3 the next; nothing is
    /// selected.
    Navigated(usize),
    /// F3 once, then a click on the first line's row and a shift-click on
    /// the last's.
    NavigatedThenRows(&'static [&'static str]),
    /// F3 once, then a mouse press on this line's text, dragged to its end.
    NavigatedThenText(&'static str),
}

impl Select {
    fn navigations(self) -> usize {
        match self {
            Self::Navigated(count) => count,
            Self::NavigatedThenRows(_) | Self::NavigatedThenText(_) => 1,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Focus {
    DiffPanel,
    /// Nothing inside the window's view tree, so the key only reaches the
    /// diff through the window-wide fallback.
    Detached,
    /// The status file list, as after clicking a file there.
    StatusList,
    /// The commit message field: its own text input.
    CommitMessage,
}

struct Pressed {
    acted: bool,
    index: String,
    worktree: String,
}

/// The visible row drawing the patch line that reads `text`.
fn row_for(pane: &DiffRenderer, text: &str) -> usize {
    let src_ix = (0..pane.patch_diff_row_len())
        .find(|ix| {
            pane.patch_diff_row(*ix)
                .is_some_and(|line| line.text.as_ref() == text)
        })
        .unwrap_or_else(|| panic!("expected a patch line reading {text:?}"));
    (0..pane.diff_visible_len())
        .find(|row| pane.diff_src_ixs_for_visible_ix(*row).contains(&src_ix))
        .unwrap_or_else(|| panic!("expected a visible row for {text:?}"))
}

/// The column a patch line is drawn in.
fn line_region(mode: DiffViewMode, line: &str) -> DiffTextRegion {
    match (mode, line.starts_with('-')) {
        (DiffViewMode::Inline, _) => DiffTextRegion::Inline,
        (DiffViewMode::Split, true) => DiffTextRegion::SplitLeft,
        (DiffViewMode::Split, false) => DiffTextRegion::SplitRight,
    }
}

/// Selects text from the start of `first`'s row to the end of `last`'s, in
/// `first`'s column, as a drag does.
fn select_text(
    pane: &mut DiffRenderer,
    mode: DiffViewMode,
    first: &str,
    last: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<DiffRenderer>,
) {
    let region = line_region(mode, first);
    pane.diff_text_anchor = Some(DiffTextPos {
        source_visible_ix: row_for(pane, first),
        region,
        offset: 0,
    });
    pane.diff_text_head = Some(DiffTextPos {
        source_visible_ix: row_for(pane, last),
        region,
        offset: last.len() - 1,
    });
    pane.sync_diff_focus_to_text_selection();
    pane.diff_text_selection_owner.adopt(window, cx);
}

fn index_text(dir: &Path) -> String {
    String::from_utf8(git_output(dir, &["show", &format!(":{PATH}")])).unwrap()
}

/// Opens `lines.txt`'s whole-file diff for `area` in `mode`, selects, focuses
/// and presses `key`, then reports whether Git ran an action and what it left
/// behind.
fn press(
    cx: &mut gpui::TestAppContext,
    area: DiffArea,
    mode: DiffViewMode,
    select: Select,
    focus: Focus,
    key: &str,
) -> Pressed {
    press_in(cx, DiffContentMode::Full, area, mode, select, focus, key)
}

/// [`press`] with the diff shown as `content`: the whole file or its
/// collapsed hunks.
fn press_in(
    cx: &mut gpui::TestAppContext,
    content: DiffContentMode,
    area: DiffArea,
    mode: DiffViewMode,
    select: Select,
    focus: Focus,
    key: &str,
) -> Pressed {
    let case = format!("{content:?} {area:?} {mode:?} {select:?} {focus:?} {key:?}");
    let dir = tempfile::tempdir().unwrap();
    // Unwind cleanup closes the app before the directory declared above it.
    let mut scenario = ScenarioApp {
        app: cx.new_app(),
        store: None,
        finished: false,
    };
    restore_repository(dir.path(), clean_repository_template());
    std::fs::write(dir.path().join(PATH), BASE).unwrap();
    git(dir.path(), &["add", PATH]);
    commit(dir.path(), "lines");
    std::fs::write(dir.path().join(PATH), EDITED).unwrap();
    if area == DiffArea::Staged {
        git(dir.path(), &["add", PATH]);
    }

    let backend = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let status = backend
        .status(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let mut repo = opening_repo_state(REPO, dir.path());
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.worktree_status = Loadable::Ready(status.unstaged.clone());
    repo.staged_status = Loadable::Ready(status.staged.clone());
    repo.status = Loadable::Ready(Arc::new(status));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    scenario.store = Some(store.clone());
    store.disable_repo_monitors_for_test();
    let store_for_view = store.clone();
    let (view, cx) = scenario
        .app
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    crate::view::test_support::drain_store_worker(&view, cx);
    super::super::shortcuts::apply_state(cx, &view, app_state_with_repo(repo, REPO));
    store.insert_repo_for_test(REPO, backend);
    cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
    cx.update(|_, app| {
        view.update(app, |root, cx| {
            root.set_diff_view_mode(mode, cx);
            root.set_diff_content_mode(content, cx);
        });
    });

    let target = DiffTarget::working_tree(PATH.into(), area);
    store.dispatch(Msg::SelectDiff {
        repo_id: REPO,
        target: target.clone(),
    });
    // The whole-file view needs both the patch and the file's two versions.
    wait_for(cx, &view, &store, &case, |repo| {
        repo.diff_state.diff_target.as_ref() == Some(&target)
            && matches!(repo.diff_state.diff, Loadable::Ready(_))
            && matches!(repo.diff_state.diff_file, Loadable::Ready(Some(_)))
    });
    super::super::shortcuts::wait_until(cx, "the whole-file diff rows", |cx| {
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx);
            });
            let pane = view.read(app).diff_renderer.read(app);
            let rows_listed = match content {
                DiffContentMode::Full => pane.is_file_diff_view_active(),
                DiffContentMode::Collapsed => pane.is_collapsed_diff_projection_active(),
            };
            rows_listed
                && pane.diff_view == mode
                && pane.diff_visible_len() > 0
                && (0..pane.patch_diff_row_len()).any(|ix| {
                    pane.patch_diff_row(ix)
                        .is_some_and(|line| line.text.as_ref() == "+FIVE")
                })
        })
    });

    cx.update(|window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                match select {
                    Select::Caret(line) => {
                        let row = row_for(pane, line);
                        pane.handle_patch_row_click(row, DiffClickKind::Line, false);
                    }
                    Select::Navigated(_)
                    | Select::NavigatedThenRows(_)
                    | Select::NavigatedThenText(_) => {}
                    Select::Rows(lines) => {
                        let rows: Vec<usize> =
                            lines.iter().map(|line| row_for(pane, line)).collect();
                        let (first, last) =
                            (rows.iter().min().unwrap(), rows.iter().max().unwrap());
                        pane.diff_selection_anchor = Some(*first);
                        pane.diff_selection_range = Some((*first, *last));
                    }
                    Select::RightClicks(_) => {}
                    Select::Text(line) => select_text(pane, mode, line, line, window, cx),
                    Select::TextThenRightClicks(first, last, _) => {
                        select_text(pane, mode, first, last, window, cx)
                    }
                }
                cx.notify();
            });
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::bind_text_input_keys_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
        match focus {
            Focus::DiffPanel => {
                let handle = view
                    .read(app)
                    .diff_renderer
                    .read(app)
                    .diff_panel_focus_handle
                    .clone();
                window.focus(&handle, app);
            }
            Focus::Detached => {
                let handle = app.focus_handle();
                window.focus(&handle, app);
            }
            Focus::StatusList => {
                let handle = view
                    .read(app)
                    .details_pane
                    .read(app)
                    .status_section_focus_handle(StatusSection::CombinedUnstaged)
                    .clone();
                window.focus(&handle, app);
            }
            Focus::CommitMessage => {
                let handle = view
                    .read(app)
                    .details_pane
                    .read(app)
                    .commit_message_input
                    .read(app)
                    .focus_handle();
                window.focus(&handle, app);
            }
        }
    });
    // After the focus, as a real right-click lands wherever focus is. Each
    // menu opens on a later frame and is dismissed, as a click outside it
    // does, before the next: a menu left open would take the key itself.
    let clicks = match select {
        Select::TextThenRightClicks(_, _, clicks) | Select::RightClicks(clicks) => clicks,
        _ => &[],
    };
    for &line in clicks {
        cx.update(|window, app| {
            view.read(app)
                .diff_renderer
                .clone()
                .update(app, |pane, cx| {
                    pane.open_diff_editor_context_menu(
                        row_for(pane, line),
                        line_region(mode, line),
                        point(px(40.0), px(40.0)),
                        window,
                        cx,
                    );
                });
        });
        draw_and_drain_test_window(cx);
        cx.update(|_, app| {
            assert!(
                view.read(app)
                    .popover_host
                    .read(app)
                    .popover_kind_for_tests()
                    .is_some(),
                "{case}: the right-click on {line:?} opens a menu"
            );
        });
        cx.update(|window, app| {
            let host = view.read(app).popover_host.clone();
            host.update(app, |host, cx| {
                host.close_popover_and_restore_focus(window, cx)
            });
        });
        draw_and_drain_test_window(cx);
    }
    draw_and_drain_test_window(cx);
    crate::view::test_support::drain_store_worker(&view, cx);
    for _ in 0..select.navigations() {
        cx.simulate_keystrokes("f3");
        draw_and_drain_test_window(cx);
    }
    match select {
        Select::NavigatedThenRows(lines) => cx.update(|_, app| {
            view.read(app)
                .diff_renderer
                .clone()
                .update(app, |pane, cx| {
                    let first = row_for(pane, lines[0]);
                    let last = row_for(pane, lines[lines.len() - 1]);
                    pane.handle_patch_row_click(first, DiffClickKind::Line, false);
                    pane.handle_patch_row_click(last, DiffClickKind::Line, true);
                    cx.notify();
                });
        }),
        Select::NavigatedThenText(line) => {
            let row = cx.update(|_, app| row_for(view.read(app).diff_renderer.read(app), line));
            let click = wait_for_diff_text_click_position_for_offset_range(
                cx,
                &view,
                row,
                DiffTextRegion::Inline,
                0..1,
                "the line's first character",
            );
            cx.update(|_, app| {
                view.read(app)
                    .diff_renderer
                    .clone()
                    .update(app, |pane, cx| {
                        pane.begin_diff_text_selection(row, DiffTextRegion::Inline, click, cx);
                        pane.diff_text_head = Some(DiffTextPos {
                            source_visible_ix: row,
                            region: DiffTextRegion::Inline,
                            offset: line.len() - 1,
                        });
                        pane.diff_text_selecting = false;
                        pane.sync_diff_focus_to_text_selection();
                        cx.notify();
                    });
            });
        }
        _ => {}
    }
    draw_and_drain_test_window(cx);
    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        let outlined = (0..pane.diff_visible_len())
            .any(|row| pane.diff_focused_change_block_row(row).is_some());
        let diff_focused = pane.diff_panel_focus_handle.is_focused(window);
        if select.navigations() > 0 {
            assert_eq!(
                diff_focused,
                focus != Focus::CommitMessage,
                "{case}: F3 focuses the diff, except from a text field"
            );
        }
        let explicit = pane.diff_text_has_selection() || pane.diff_selection_range.is_some();
        let selected = match select {
            Select::Navigated(_) => !explicit && outlined,
            Select::Caret(_) => !explicit && !outlined,
            Select::NavigatedThenRows(_) => pane.diff_selection_range.is_some() && !outlined,
            Select::NavigatedThenText(_) => pane.diff_text_has_selection() && !outlined,
            Select::Rows(_) => pane.diff_selection_range.is_some(),
            Select::Text(_) => pane.diff_text_has_selection(),
            Select::TextThenRightClicks(..) => {
                pane.diff_text_has_selection() && pane.diff_selection_range.is_some()
            }
            Select::RightClicks(_) => pane.diff_selection_range.is_some(),
        };
        assert!(selected, "{case}: the selection must survive until the key");
        assert!(
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
                .is_none(),
            "{case}: the key must reach the diff, not a menu"
        );
    });
    let before = store.snapshot().repos[0].ops_rev;

    cx.simulate_keystrokes(key);
    draw_and_drain_test_window(cx);
    crate::view::test_support::drain_store_worker(&view, cx);
    let acted = store.snapshot().repos[0].ops_rev != before;
    if acted {
        wait_for(cx, &view, &store, &case, |repo| {
            repo.ops_rev >= before + 2 && repo.local_actions_in_flight == 0
        });
        let snapshot = store.snapshot();
        assert!(
            snapshot.repos[0].feedback.last_error.is_none(),
            "{case}: {:?}",
            snapshot.repos[0].feedback.last_error
        );
    }
    let pressed = Pressed {
        acted,
        index: index_text(dir.path()),
        worktree: std::fs::read_to_string(dir.path().join(PATH)).unwrap(),
    };

    // Release this case's windows before removing its temporary directory.
    store.dispatch(Msg::CloseRepo { repo_id: REPO });
    crate::view::test_support::drain_store_worker(&view, cx);
    drop(view);
    scenario.finish().expect("line key scenario teardown");
    pressed
}

#[gpui::test]
fn s_stages_only_the_selected_rows(cx: &mut gpui::TestAppContext) {
    for mode in [DiffViewMode::Inline, DiffViewMode::Split] {
        let pressed = press(
            cx,
            DiffArea::Unstaged,
            mode,
            Select::Rows(&["-two", "+TWO"]),
            Focus::DiffPanel,
            "s",
        );
        assert!(pressed.acted, "{mode:?}: S must stage the selected rows");
        assert_eq!(
            pressed.index, "one\nTWO\nthree\nfour\nfive\n",
            "{mode:?}: only the selected change reaches the index"
        );
        assert_eq!(
            pressed.worktree, EDITED,
            "{mode:?}: staging leaves the file"
        );
    }
}

#[gpui::test]
fn s_stages_only_the_lines_the_text_selection_covers(cx: &mut gpui::TestAppContext) {
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Inline,
        Select::Text("+TWO"),
        Focus::DiffPanel,
        "s",
    );
    assert!(pressed.acted, "S must stage the selected text's line");
    // The removal was not selected, so it stays behind as context.
    assert_eq!(pressed.index, "one\ntwo\nTWO\nthree\nfour\nfive\n");
    assert_eq!(pressed.worktree, EDITED);
}

#[gpui::test]
fn d_discards_only_the_selected_rows(cx: &mut gpui::TestAppContext) {
    for mode in [DiffViewMode::Inline, DiffViewMode::Split] {
        let pressed = press(
            cx,
            DiffArea::Unstaged,
            mode,
            Select::Rows(&["-five", "+FIVE"]),
            Focus::DiffPanel,
            "d",
        );
        assert!(pressed.acted, "{mode:?}: D must discard the selected rows");
        assert_eq!(
            pressed.worktree, "one\nTWO\nthree\nfour\nfive\n",
            "{mode:?}: only the selected change is reverted in the file"
        );
        assert_eq!(pressed.index, BASE, "{mode:?}: discarding leaves the index");
    }
}

#[gpui::test]
fn u_unstages_only_the_selected_rows(cx: &mut gpui::TestAppContext) {
    let pressed = press(
        cx,
        DiffArea::Staged,
        DiffViewMode::Inline,
        Select::Rows(&["-two", "+TWO"]),
        Focus::DiffPanel,
        "u",
    );
    assert!(pressed.acted, "U must unstage the selected rows");
    assert_eq!(pressed.index, "one\ntwo\nthree\nfour\nFIVE\n");
    assert_eq!(pressed.worktree, EDITED);
}

/// The letters are safe to press by accident: with only a caret, in the
/// other area's diff, or from outside the diff, they leave Git alone.
#[gpui::test]
fn line_keys_need_a_selection_in_the_focused_diff(cx: &mut gpui::TestAppContext) {
    let rows = Select::Rows(&["-five", "+FIVE"]);
    let caret = Select::Caret("+FIVE");
    for (area, select, focus, key) in [
        (DiffArea::Unstaged, caret, Focus::DiffPanel, "s"),
        (DiffArea::Unstaged, caret, Focus::DiffPanel, "d"),
        (DiffArea::Unstaged, rows, Focus::DiffPanel, "u"),
        (DiffArea::Unstaged, rows, Focus::DiffPanel, "shift-s"),
        (DiffArea::Unstaged, rows, Focus::Detached, "d"),
        (DiffArea::Staged, rows, Focus::DiffPanel, "s"),
        (DiffArea::Staged, rows, Focus::DiffPanel, "d"),
    ] {
        let pressed = press(cx, area, DiffViewMode::Inline, select, focus, key);
        let case = format!("{area:?} {select:?} {focus:?} {key}");
        assert!(!pressed.acted, "{case}: no Git action may run");
        let index = match area {
            DiffArea::Unstaged => BASE,
            DiffArea::Staged => EDITED,
        };
        assert_eq!(pressed.index, index, "{case}");
        assert_eq!(pressed.worktree, EDITED, "{case}");
    }
}

/// A right-click beside a text selection acts on the row it lands on, and the
/// key pressed once the menu is closed must act on that row too: discarding
/// the older text selection instead would revert lines the menu never showed.
#[gpui::test]
fn d_after_a_right_click_beside_the_text_discards_the_clicked_row(cx: &mut gpui::TestAppContext) {
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Inline,
        Select::TextThenRightClicks("+TWO", "+TWO", &["+FIVE"]),
        Focus::DiffPanel,
        "d",
    );
    assert!(pressed.acted, "D must discard the right-clicked row");
    assert!(
        pressed.worktree.contains("TWO") && !pressed.worktree.contains("FIVE"),
        "only the clicked line is reverted, not the text selected earlier: {:?}",
        pressed.worktree
    );
    assert_eq!(pressed.index, BASE);
}

/// A text selection in one Split column does not reach a right-click in the
/// other, even on a row it covers: that menu acts on the clicked row alone,
/// and so must the key.
#[gpui::test]
fn d_after_a_right_click_in_the_other_column_discards_the_clicked_row(
    cx: &mut gpui::TestAppContext,
) {
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Split,
        Select::TextThenRightClicks("-two", "-five", &["+TWO"]),
        Focus::DiffPanel,
        "d",
    );
    assert!(pressed.acted, "D must discard the right-clicked row");
    assert_eq!(
        pressed.worktree, "one\ntwo\nthree\nfour\nFIVE\n",
        "only the clicked row is reverted, not every row the old column's text covers"
    );
    assert_eq!(pressed.index, BASE);
}

/// A right-click inside the highlight acts on the selected text again, even
/// after an earlier right-click elsewhere selected a row.
#[gpui::test]
fn d_after_a_right_click_back_inside_the_text_discards_the_text(cx: &mut gpui::TestAppContext) {
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Inline,
        Select::TextThenRightClicks("+TWO", "+TWO", &["+FIVE", "+TWO"]),
        Focus::DiffPanel,
        "d",
    );
    assert!(pressed.acted, "D must discard the selected text's line");
    assert!(
        !pressed.worktree.contains("TWO") && pressed.worktree.contains("FIVE"),
        "the text's line is reverted, not the row the first right-click chose: {:?}",
        pressed.worktree
    );
    assert_eq!(pressed.index, BASE);
}

/// Right-clicking the diff while the file list holds focus hands the diff
/// focus, so dismissing the menu returns there and the keys act on the row.
#[gpui::test]
fn line_keys_act_after_a_right_click_from_the_file_list(cx: &mut gpui::TestAppContext) {
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Inline,
        Select::RightClicks(&["+FIVE"]),
        Focus::StatusList,
        "d",
    );
    assert!(pressed.acted, "D must discard the right-clicked row");
    assert!(
        pressed.worktree.contains("TWO") && !pressed.worktree.contains("FIVE"),
        "only the clicked line is reverted: {:?}",
        pressed.worktree
    );
    assert_eq!(pressed.index, BASE);
}

/// Collapsed hunks list the same file rows as the whole-file view, and the
/// keys move the same lines there.
#[gpui::test]
fn line_keys_act_on_collapsed_hunks(cx: &mut gpui::TestAppContext) {
    for (area, rows, key, index, worktree) in [
        (
            DiffArea::Unstaged,
            Select::Rows(&["-two", "+TWO"]),
            "s",
            "one\nTWO\nthree\nfour\nfive\n",
            EDITED,
        ),
        (
            DiffArea::Unstaged,
            Select::Rows(&["-five", "+FIVE"]),
            "d",
            BASE,
            "one\nTWO\nthree\nfour\nfive\n",
        ),
        (
            DiffArea::Staged,
            Select::Rows(&["-two", "+TWO"]),
            "u",
            "one\ntwo\nthree\nfour\nFIVE\n",
            EDITED,
        ),
    ] {
        let pressed = press_in(
            cx,
            DiffContentMode::Collapsed,
            area,
            DiffViewMode::Inline,
            rows,
            Focus::DiffPanel,
            key,
        );
        assert!(pressed.acted, "{area:?} {key}: the key must act");
        assert_eq!(pressed.index, index, "{area:?} {key}");
        assert_eq!(pressed.worktree, worktree, "{area:?} {key}");
    }
}

/// F2/F3 outline a change block without selecting it. S, U and D act on
/// that block, as nothing else is selected.
#[gpui::test]
fn line_keys_act_on_the_block_f3_outlined(cx: &mut gpui::TestAppContext) {
    let two_only = "one\nTWO\nthree\nfour\nfive\n";
    let five_only = "one\ntwo\nthree\nfour\nFIVE\n";
    for content in [DiffContentMode::Full, DiffContentMode::Collapsed] {
        for mode in [DiffViewMode::Inline, DiffViewMode::Split] {
            for (area, presses, key, index, worktree) in [
                (DiffArea::Unstaged, 0, "s", two_only, EDITED),
                (DiffArea::Unstaged, 1, "s", five_only, EDITED),
                (DiffArea::Unstaged, 1, "d", BASE, two_only),
                (DiffArea::Staged, 1, "u", two_only, EDITED),
            ] {
                let case = format!("{content:?} {mode:?} {area:?} F3x{presses} {key}");
                let pressed = press_in(
                    cx,
                    content,
                    area,
                    mode,
                    Select::Navigated(presses),
                    Focus::DiffPanel,
                    key,
                );
                assert!(
                    pressed.acted,
                    "{case}: the key must act on the outlined block"
                );
                assert_eq!(pressed.index, index, "{case}");
                assert_eq!(pressed.worktree, worktree, "{case}");
            }
        }
    }
}

/// F3 pressed outside the diff moves focus into it, so S then reaches the
/// block it outlined; from a text field focus stays and S is typed there.
#[gpui::test]
fn f3_from_outside_the_diff_focuses_it_for_the_line_keys(cx: &mut gpui::TestAppContext) {
    for focus in [Focus::StatusList, Focus::Detached] {
        let pressed = press(
            cx,
            DiffArea::Unstaged,
            DiffViewMode::Inline,
            Select::Navigated(1),
            focus,
            "s",
        );
        assert!(pressed.acted, "{focus:?}: S must stage the outlined block");
        assert_eq!(pressed.index, "one\ntwo\nthree\nfour\nFIVE\n", "{focus:?}");
        assert_eq!(pressed.worktree, EDITED, "{focus:?}");
    }
    let pressed = press(
        cx,
        DiffArea::Unstaged,
        DiffViewMode::Inline,
        Select::Navigated(1),
        Focus::CommitMessage,
        "s",
    );
    assert!(
        !pressed.acted,
        "S typed into the commit message stages nothing"
    );
    assert_eq!(pressed.index, BASE);
}

/// Opening the diff outlines its first block while focus stays in the file
/// list; S, U and D pressed there act on that block without F2/F3 first.
#[gpui::test]
fn line_keys_from_the_file_list_act_on_the_opened_block(cx: &mut gpui::TestAppContext) {
    let two_only = "one\nTWO\nthree\nfour\nfive\n";
    for (area, focus, key, index, worktree) in [
        (DiffArea::Unstaged, Focus::StatusList, "s", two_only, EDITED),
        (DiffArea::Unstaged, Focus::Detached, "s", two_only, EDITED),
        (
            DiffArea::Unstaged,
            Focus::StatusList,
            "d",
            BASE,
            "one\ntwo\nthree\nfour\nFIVE\n",
        ),
        (
            DiffArea::Staged,
            Focus::StatusList,
            "u",
            "one\ntwo\nthree\nfour\nFIVE\n",
            EDITED,
        ),
    ] {
        let case = format!("{area:?} {focus:?} {key}");
        let pressed = press(
            cx,
            area,
            DiffViewMode::Inline,
            Select::Navigated(0),
            focus,
            key,
        );
        assert!(
            pressed.acted,
            "{case}: the key must act on the outlined block"
        );
        assert_eq!(pressed.index, index, "{case}");
        assert_eq!(pressed.worktree, worktree, "{case}");
    }
}

/// A mouse selection made after F3 removes the outline and is what the keys
/// act on: F3 outlines the second block, the mouse selects in the first.
#[gpui::test]
fn a_mouse_selection_after_f3_replaces_the_outlined_block(cx: &mut gpui::TestAppContext) {
    for (select, index) in [
        (
            Select::NavigatedThenRows(&["-two", "+TWO"]),
            "one\nTWO\nthree\nfour\nfive\n",
        ),
        // Only the added line's text: the removal stays as context.
        (
            Select::NavigatedThenText("+TWO"),
            "one\ntwo\nTWO\nthree\nfour\nfive\n",
        ),
    ] {
        let pressed = press(
            cx,
            DiffArea::Unstaged,
            DiffViewMode::Inline,
            select,
            Focus::DiffPanel,
            "s",
        );
        assert!(
            pressed.acted,
            "{select:?}: S must stage the mouse selection"
        );
        assert_eq!(pressed.index, index, "{select:?}: not the outlined block");
        assert_eq!(pressed.worktree, EDITED, "{select:?}");
    }
}
