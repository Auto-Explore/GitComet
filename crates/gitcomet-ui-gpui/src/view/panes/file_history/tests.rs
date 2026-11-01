use super::*;
use crate::view::test_support::NoopBackend;
use gitcomet_core::domain::{Commit, DiffArea, LogPage};

const PATH: &str = "src/main.rs";
const COMMITS: usize = 20;

fn commit(ix: usize) -> Commit {
    Commit {
        id: CommitId(format!("{ix:0>40}").into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: format!("Commit {ix:03} touching the file").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

fn id(ix: usize) -> CommitId {
    commit(ix).id
}

struct Host {
    pane: Entity<FileHistoryPaneView>,
    escapes_seen: usize,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .w(px(360.0))
            .h(px(500.0))
            .flex()
            .flex_col()
            .on_key_down(
                cx.listener(|this, event: &gpui::KeyDownEvent, _window, _cx| {
                    if event.keystroke.key == "escape" {
                        this.escapes_seen += 1;
                    }
                }),
            )
            .child(self.pane.clone())
    }
}

struct Fixture {
    host: Entity<Host>,
    pane: Entity<FileHistoryPaneView>,
    model: Entity<AppUiModel>,
}

fn setup(cx: &mut gpui::TestAppContext) -> (Fixture, &mut gpui::VisualTestContext) {
    cx.update(crate::app::bind_text_input_keys_for_test);
    let (store, _events) = AppStore::new_test(Arc::new(NoopBackend));
    let store = Arc::new(store);
    let mut repo = RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/file-history-pane-test"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    });
    let mut built = None;
    let (host, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|cx| {
            AppUiModel::new_with_preferences(
                state,
                UiPreferences::default(),
                Arc::downgrade(&store),
                cx,
            )
        });
        let pane = cx.new(|cx| {
            FileHistoryPaneView::new(
                store,
                model.clone(),
                FileHistoryPaneInit {
                    theme: AppTheme::gitcomet_dark(),
                    root_view: WeakEntity::new_invalid(),
                },
                window,
                cx,
            )
        });
        built = Some((pane.clone(), model));
        Host {
            pane,
            escapes_seen: 0,
        }
    });
    let (pane, model) = built.unwrap();
    (Fixture { host, pane, model }, cx)
}

impl Fixture {
    fn repo(&self, cx: &mut gpui::VisualTestContext, f: impl FnOnce(&mut RepoState)) {
        self.model.update(cx, |model, cx| {
            let mut state = (*model.state).clone();
            f(&mut state.repos[0]);
            model.state = Arc::new(state);
            cx.notify();
        });
    }

    fn ready(&self, cx: &mut gpui::VisualTestContext, path: &str) {
        let path = std::path::PathBuf::from(path);
        self.repo(cx, |repo| {
            repo.history_state.file_history_path = Some(path);
            repo.history_state.file_history = Loadable::Ready(Arc::new(LogPage {
                commits: (0..COMMITS).map(commit).collect(),
                next_cursor: None,
            }));
        });
    }

    fn shown(&self, cx: &mut gpui::VisualTestContext, target: DiffTarget, content_preview: bool) {
        self.repo(cx, |repo| {
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.content_preview = content_preview;
        });
    }

    fn open(&self, cx: &mut gpui::VisualTestContext) {
        self.pane.update_in(cx, |pane, window, cx| {
            pane.open(RepoId(1), PATH.into(), window, cx)
        });
    }

    fn sent(&self, cx: &mut gpui::VisualTestContext) -> Vec<String> {
        self.pane.read_with(cx, |pane, _| pane.sent.clone())
    }

    fn selected(&self, cx: &mut gpui::VisualTestContext) -> Option<CommitId> {
        self.pane.read_with(cx, |pane, _| pane.selected.clone())
    }

    fn nav(&self, cx: &mut gpui::VisualTestContext, keys: PickerNavKeys) {
        self.pane
            .update(cx, |pane, cx| pane.on_filter_changed(keys, cx));
    }

    fn type_filter(&self, cx: &mut gpui::VisualTestContext, text: &str) {
        let input = self.pane.read_with(cx, |pane, _| pane.search_input.clone());
        input.update(cx, |input, cx| input.set_text(text, cx));
    }
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn show_changes(commit_ix: usize) -> String {
    format!(
        "ShowFileChangesAtCommit {{ repo_id: RepoId(1), commit_id: {:?}, path: {PATH:?} }}",
        id(commit_ix)
    )
}

fn down() -> PickerNavKeys {
    PickerNavKeys {
        arrow_down: true,
        ..Default::default()
    }
}

#[gpui::test]
fn open_requests_the_history_of_the_file(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    let sent = f.sent(cx);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].starts_with("LoadFileHistory") && sent[0].contains("limit: 200"));
}

#[gpui::test]
fn without_a_diff_the_first_row_is_selected_and_shown(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, PATH);
    assert_eq!(f.selected(cx), Some(id(0)));
    assert_eq!(f.sent(cx)[1..], [show_changes(0)]);
}

#[gpui::test]
fn a_page_for_another_file_selects_nothing(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, "other.rs");
    assert_eq!(f.selected(cx), None);
    assert_eq!(f.sent(cx).len(), 1);
}

#[gpui::test]
fn a_shown_commit_of_the_file_is_marked_without_a_message(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, DiffTarget::commit(id(5), PATH.into()), false);
    f.open(cx);
    f.ready(cx, PATH);
    assert_eq!(f.selected(cx), Some(id(5)));
    assert_eq!(f.sent(cx).len(), 1);
}

#[gpui::test]
fn the_start_selection_handles_file_mode_and_other_files(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, DiffTarget::commit(id(5), PATH.into()), true);
    f.open(cx);
    f.ready(cx, PATH);
    assert_eq!(f.selected(cx), Some(id(5)));
    assert_eq!(f.sent(cx)[1..], [show_changes(5)]);

    f.shown(cx, DiffTarget::commit(id(5), "other.rs".into()), false);
    f.open(cx);
    f.ready(cx, "other.rs");
    f.ready(cx, PATH);
    assert_eq!(f.selected(cx), Some(id(0)));
    assert_eq!(f.sent(cx).last(), Some(&show_changes(0)));
}

#[gpui::test]
fn a_working_tree_diff_stays_until_the_user_navigates(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(
        cx,
        DiffTarget::working_tree(PATH.into(), DiffArea::Unstaged),
        false,
    );
    f.open(cx);
    f.ready(cx, PATH);
    assert_eq!(f.selected(cx), None);
    assert_eq!(f.sent(cx).len(), 1);
    f.nav(cx, down());
    assert_eq!(f.selected(cx), Some(id(0)));
    assert_eq!(f.sent(cx)[1..], [show_changes(0)]);
}

#[gpui::test]
fn down_moves_to_the_next_row(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, PATH);
    f.nav(cx, down());
    assert_eq!(f.selected(cx), Some(id(1)));
    assert_eq!(f.sent(cx).last(), Some(&show_changes(1)));
}

#[gpui::test]
fn a_click_with_a_filter_picks_the_clicked_commit(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, DiffTarget::commit(id(0), PATH.into()), false);
    f.open(cx);
    f.ready(cx, PATH);
    f.type_filter(cx, "Commit 007");
    draw(cx);
    let row = ["picker_prompt_item_7", "picker_prompt_item_0"]
        .into_iter()
        .find_map(|selector| cx.debug_bounds(selector))
        .expect("the filtered row is rendered");
    cx.simulate_click(row.center(), gpui::Modifiers::none());
    assert_eq!(f.selected(cx), Some(id(7)));
    assert_eq!(f.sent(cx).last(), Some(&show_changes(7)));
}

#[gpui::test]
fn the_mode_switch_reopens_the_selection_as_a_file(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, PATH);
    f.pane
        .update(cx, |pane, cx| pane.set_mode(FileHistoryMode::File, cx));
    let last = f.sent(cx).pop().unwrap();
    assert!(last.starts_with("OpenFileAtCommit") && last.contains(&format!("{:?}", id(0))));
}

#[gpui::test]
fn selecting_the_shown_row_sends_nothing(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, DiffTarget::commit(id(3), PATH.into()), false);
    f.open(cx);
    f.ready(cx, PATH);
    f.pane.update(cx, |pane, cx| pane.select(id(3), cx));
    assert_eq!(f.sent(cx).len(), 1);
}

#[gpui::test]
fn escape_clears_the_filter_and_stays_in_the_pane(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, PATH);
    draw(cx);
    f.type_filter(cx, "Commit 007");
    cx.simulate_keystrokes("escape");
    draw(cx);
    let (text, focused) = f.pane.read_with(cx, |pane, app| {
        let input = pane.search_input.read(app);
        (input.text().to_string(), input.focus_handle())
    });
    assert_eq!(text, "");
    cx.update(|window, _| assert!(focused.is_focused(window)));
    assert_eq!(f.host.read_with(cx, |host, _| host.escapes_seen), 0);
    assert!(f.pane.read_with(cx, |pane, _| pane.target.is_some()));

    f.pane.update(cx, |pane, cx| pane.close(cx));
    assert!(f.pane.read_with(cx, |pane, _| pane.target.is_none()));
}

fn wt() -> DiffTarget {
    DiffTarget::working_tree(PATH.into(), DiffArea::Unstaged)
}

fn count(sent: &[String], msg: &str) -> usize {
    sent.iter().filter(|m| **m == msg).count()
}

/// Opens the tab on a working-tree diff, so no start selection happens.
fn opened(f: &Fixture, cx: &mut gpui::VisualTestContext) {
    f.shown(cx, wt(), false);
    f.open(cx);
    f.ready(cx, PATH);
}

#[gpui::test]
fn an_older_result_that_arrives_last_is_replaced(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.pane.update(cx, |pane, cx| pane.select(id(1), cx));
    f.pane.update(cx, |pane, cx| pane.select(id(2), cx));
    f.shown(cx, DiffTarget::commit(id(2), PATH.into()), false);
    f.shown(cx, DiffTarget::commit(id(1), PATH.into()), false);
    assert_eq!(count(&f.sent(cx), &show_changes(2)), 2);
    assert_eq!(f.selected(cx), Some(id(2)));
}

#[gpui::test]
fn results_in_request_order_need_no_new_request(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.pane.update(cx, |pane, cx| pane.select(id(1), cx));
    f.pane.update(cx, |pane, cx| pane.select(id(2), cx));
    f.shown(cx, DiffTarget::commit(id(1), PATH.into()), false);
    f.shown(cx, DiffTarget::commit(id(2), PATH.into()), false);
    assert_eq!(count(&f.sent(cx), &show_changes(2)), 1);
    assert_eq!(f.selected(cx), Some(id(2)));
}

#[gpui::test]
fn going_back_to_a_shown_row_needs_no_request(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.pane.update(cx, |pane, cx| pane.select(id(1), cx));
    f.shown(cx, DiffTarget::commit(id(1), PATH.into()), false);
    f.pane.update(cx, |pane, cx| pane.select(id(1), cx));
    f.shown(cx, DiffTarget::commit(id(4), PATH.into()), false);
    f.shown(cx, DiffTarget::commit(id(1), PATH.into()), false);
    let sent = f.sent(cx);
    assert_eq!(count(&sent, &show_changes(1)), 1);
    assert_eq!(f.selected(cx), Some(id(1)));
}

#[gpui::test]
fn a_renamed_path_keeps_the_mark_of_a_requested_commit(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.pane.update(cx, |pane, cx| pane.select(id(3), cx));
    f.shown(cx, DiffTarget::commit(id(3), "old/name.rs".into()), false);
    assert_eq!(f.selected(cx), Some(id(3)));
}

#[gpui::test]
fn history_navigation_moves_the_mark_and_the_mode(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.shown(cx, DiffTarget::commit(id(6), PATH.into()), true);
    assert_eq!(f.selected(cx), Some(id(6)));
    assert_eq!(
        f.pane.read_with(cx, |pane, _| pane.mode),
        FileHistoryMode::File
    );
}

#[gpui::test]
fn another_file_in_the_main_diff_clears_the_mark(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.pane.update(cx, |pane, cx| pane.select(id(3), cx));
    f.shown(cx, DiffTarget::commit(id(3), PATH.into()), false);
    f.shown(cx, DiffTarget::commit(id(5), "other.rs".into()), false);
    assert_eq!(f.selected(cx), None);
    assert!(f.pane.read_with(cx, |pane, _| pane.target.is_some()));
}

#[gpui::test]
fn a_cleared_history_is_loaded_once_more(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.repo(cx, |repo| {
        repo.history_state.file_history_path = None;
        repo.history_state.file_history = Loadable::NotLoaded;
    });
    f.repo(cx, |_| {});
    let loads = f
        .sent(cx)
        .iter()
        .filter(|m| m.starts_with("LoadFileHistory"))
        .count();
    assert_eq!(loads, 2);
}

#[gpui::test]
fn a_stale_snapshot_after_open_does_not_load_twice(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, "other.rs");
    f.repo(cx, |_| {});
    let loads = f
        .sent(cx)
        .iter()
        .filter(|m| m.starts_with("LoadFileHistory"))
        .count();
    assert_eq!(loads, 1);
}

const LONG: usize = 200;

/// A page of `count` commits for `PATH`; `more` marks a first page whose rest
/// is still on its way.
fn ready_page(
    f: &Fixture,
    cx: &mut gpui::VisualTestContext,
    count: usize,
    next_cursor: Option<gitcomet_core::domain::LogCursor>,
) {
    f.repo(cx, |repo| {
        repo.history_state.file_history_path = Some(PATH.into());
        repo.history_state.file_history = Loadable::Ready(Arc::new(LogPage {
            commits: (0..count).map(commit).collect(),
            next_cursor,
        }));
    });
}

fn filtered(f: &Fixture, cx: &mut gpui::VisualTestContext, query: &str) -> usize {
    f.type_filter(cx, query);
    f.pane.read_with(cx, |pane, _| pane.rows().filtered_len())
}

/// The first page is 200 commits: only the rows in view may be built.
#[gpui::test]
fn a_long_history_renders_only_the_rows_in_view(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, wt(), false);
    f.open(cx);
    ready_page(&f, cx, LONG, None);
    draw(cx);
    assert!(cx.debug_bounds("picker_prompt_item_0").is_some());
    assert!(cx.debug_bounds("picker_prompt_item_150").is_none());
}

/// Arrowing up from nothing selects the last row, far outside the window.
#[gpui::test]
fn arrowing_to_the_last_row_scrolls_it_into_view(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, wt(), false);
    f.open(cx);
    ready_page(&f, cx, LONG, None);
    draw(cx);
    assert!(cx.debug_bounds("picker_prompt_item_199").is_none());
    f.nav(
        cx,
        PickerNavKeys {
            arrow_up: true,
            ..Default::default()
        },
    );
    draw(cx);
    draw(cx);
    assert_eq!(f.selected(cx), Some(id(LONG - 1)));
    assert!(cx.debug_bounds("picker_prompt_item_199").is_some());
}

#[gpui::test]
fn a_page_for_another_file_shows_no_rows(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, "other.rs");
    draw(cx);
    assert!(cx.debug_bounds("picker_prompt_item_0").is_none());
}

#[gpui::test]
fn the_filter_matches_summary_author_and_sha(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, wt(), false);
    f.open(cx);
    f.ready(cx, PATH);
    assert_eq!(filtered(&f, cx, "Commit 007"), 1);
    assert_eq!(filtered(&f, cx, "Alice"), COMMITS);
    assert_eq!(filtered(&f, cx, "00000000"), COMMITS);
    assert_eq!(filtered(&f, cx, "nobody"), 0);
}

/// The first page is on screen before the rest is fetched, so a search would
/// miss older commits; the pane says so until the page is complete.
#[gpui::test]
fn the_footer_reports_older_commits_still_loading(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    let pending = Some(gitcomet_core::domain::LogCursor {
        last_seen: id(COMMITS - 1),
        resume_from: None,
        resume_token: None,
    });
    ready_page(&f, cx, COMMITS, pending);
    draw(cx);
    assert!(cx.debug_bounds("file_history_loading_older").is_some());
    assert!(cx.debug_bounds("picker_prompt_item_0").is_some());

    ready_page(&f, cx, COMMITS, None);
    draw(cx);
    assert!(cx.debug_bounds("file_history_loading_older").is_none());
}

#[gpui::test]
fn rows_show_summary_monospace_sha_author_and_date(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, wt(), false);
    f.open(cx);
    f.ready(cx, PATH);
    f.pane.update(cx, |pane, cx| {
        let first = pane.rows();
        assert_eq!(
            first.items[0].debug_display_text(),
            "Commit 000 touching the file"
        );
        assert_eq!(
            first.items[0].debug_secondary_part_font_families()[1],
            Some(crate::view::UI_MONOSPACE_FONT_FAMILY)
        );
        assert!(
            first.items[0]
                .debug_secondary_text()
                .contains("00000000  •  Alice  •  ")
        );
        assert!(first.items[0].debug_secondary_text().ends_with("ago"));
        pane.set_relative_dates(false, cx);
        pane.set_date_settings(DateTimeFormat::YmdHm, Timezone::Utc, false, cx);
        assert!(
            pane.rows().items[0]
                .debug_secondary_text()
                .contains("1970-01-01")
        );
    });
}

#[gpui::test]
fn home_end_and_page_keys_select_and_show_the_expected_row(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(cx, wt(), false);
    f.open(cx);
    ready_page(&f, cx, LONG, None);
    draw(cx);
    let page_rows = f.pane.update(cx, |pane, cx| {
        let row_height = components::picker_row_height(UiScale::current(cx), true);
        (f32::from(pane.list_height) / f32::from(row_height))
            .floor()
            .max(1.0) as usize
    });
    let key = |name: &str| PickerNavKeys {
        end: name == "end",
        home: name == "home",
        page_down: name == "page_down",
        page_up: name == "page_up",
        ..Default::default()
    };
    for (name, row) in [
        ("end", LONG - 1),
        ("home", 0),
        ("page_down", page_rows),
        ("page_up", 0),
    ] {
        f.nav(cx, key(name));
        assert_eq!(f.selected(cx), Some(id(row)), "{name}");
        assert_eq!(f.sent(cx).last(), Some(&show_changes(row)), "{name}");
    }
}

#[gpui::test]
fn down_then_up_ends_on_the_row_that_was_asked_for_last(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.shown(cx, DiffTarget::commit(id(0), PATH.into()), false);
    f.pane.update(cx, |pane, cx| pane.select(id(1), cx));
    f.pane.update(cx, |pane, cx| pane.select(id(0), cx));
    f.shown(cx, DiffTarget::commit(id(1), PATH.into()), false);
    assert_eq!(count(&f.sent(cx), &show_changes(0)), 1);
    assert_eq!(f.selected(cx), Some(id(0)));
}

#[gpui::test]
fn a_commit_shown_for_another_file_is_requested_for_the_tab_file(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    opened(&f, cx);
    f.shown(cx, DiffTarget::commit(id(3), "other.rs".into()), false);
    f.pane.update(cx, |pane, cx| pane.select(id(3), cx));
    assert_eq!(count(&f.sent(cx), &show_changes(3)), 1);
}

#[gpui::test]
fn real_arrow_keys_in_the_filter_move_the_selection(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.shown(
        cx,
        DiffTarget::working_tree(PATH.into(), gitcomet_core::domain::DiffArea::Unstaged),
        false,
    );
    f.open(cx);
    f.ready(cx, PATH);
    draw(cx);
    assert_eq!(f.selected(cx), None);
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(id(0)));
    cx.simulate_keystrokes("down down up");
    assert_eq!(f.selected(cx), Some(id(1)));
}

#[gpui::test]
fn a_click_on_a_row_gives_the_arrow_keys_back_to_the_filter(cx: &mut gpui::TestAppContext) {
    let (f, cx) = setup(cx);
    f.open(cx);
    f.ready(cx, PATH);
    draw(cx);
    cx.update(|window, app| window.blur(app));
    let bounds = cx
        .debug_bounds("picker_prompt_item_1")
        .expect("row 1 is drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    assert_eq!(f.selected(cx), Some(id(1)));
    cx.simulate_keystrokes("down");
    assert_eq!(f.selected(cx), Some(id(2)));
}
