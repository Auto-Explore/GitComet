use super::*;
use gitcomet_core::domain::{Commit, CommitFileChange, CommitId, CommitParentIds, FileStatusKind};
use gitcomet_state::model::{CommitMultiSelection, Loadable, RangeSelection, RepoId};

fn sha(ix: usize) -> String {
    format!("{ix:040}")
}

fn log_commit(ix: usize) -> Commit {
    Commit {
        id: CommitId(sha(ix).into()),
        parent_ids: CommitParentIds::new(),
        summary: format!("commit {ix}").into(),
        author: "Alice".into(),
        time: std::time::SystemTime::UNIX_EPOCH,
    }
}

/// What state the comparison's changed-file list is in. `Loaded(n)` is the
/// common case; the other two exist because their render paths differ in ways
/// that are easy to regress into each other.
enum Files {
    Loading,
    Failed(&'static str),
    Loaded(usize),
}

impl Files {
    fn into_loadable(self) -> Loadable<Arc<Vec<CommitFileChange>>> {
        match self {
            Files::Loading => Loadable::Loading,
            Files::Failed(message) => Loadable::Error(message.to_string()),
            Files::Loaded(count) => Loadable::Ready(Arc::new(
                (0..count)
                    .map(|ix| CommitFileChange {
                        path: std::path::PathBuf::from(format!("src/file_{ix}.rs")),
                        kind: FileStatusKind::Modified,
                        is_submodule: false,
                        additions: Some(1),
                        deletions: Some(0),
                    })
                    .collect(),
            )),
        }
    }
}

/// Put the details pane into an active comparison and draw it. `commit_count`
/// commits go into the log (newest first, so index 0 is the tip); `selected` is
/// how many of them are also multi-selected, which is what decides whether the
/// comparison presents itself as a merged multi-selection or as two named
/// points. `files` is the state of the changed-file section below the cards.
fn draw_comparison(
    cx: &mut gpui::TestAppContext,
    repo_id: RepoId,
    commit_count: usize,
    selected: usize,
    files: Files,
) -> &mut gpui::VisualTestContext {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-range-compare"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: (0..commit_count).map(log_commit).collect(),
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.multi_selection = CommitMultiSelection {
                commits: (0..selected).map(|ix| CommitId(sha(ix).into())).collect(),
                ..Default::default()
            };
            // Oldest is the base, newest the tip — the same ordering the
            // reducer produces for either flow.
            repo.history_state.range_selection = Some(RangeSelection {
                from: CommitId(sha(commit_count.saturating_sub(1)).into()),
                to: Some(CommitId(sha(0).into())),
                from_label: "base".into(),
                to_label: "tip".into(),
            });
            repo.history_state.range_files = files.into_loadable();

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx
}

/// A range comparison started via "mark + compare" (or a branch/tag/worktree
/// compare) leaves `multi_selection` empty — the endpoints live only in
/// `range_selection`. The compared-commit preview cards must still render,
/// resolved from those endpoints by looking each SHA up in the log.
///
/// Regression guard: the cards were built with a virtualized `uniform_list`
/// inside a fixed-height, non-scrolling container, which painted nothing and
/// left only the "Viewing diff between N commits" subheader visible.
#[gpui::test]
fn range_comparison_renders_endpoint_commit_cards(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(77), 2, 0, Files::Loaded(1));

    assert!(
        cx.debug_bounds("commit_multi_row_0").is_some(),
        "expected the tip commit's preview card to render for a range comparison"
    );
    assert!(
        cx.debug_bounds("commit_multi_row_1").is_some(),
        "expected the base commit's preview card to render for a range comparison"
    );
}

/// Every plain history click leaves one commit in `multi_selection`, so a
/// comparison started from a context menu afterwards finds a stale selection
/// sitting there. The cards must come from the comparison's own endpoints, not
/// from that leftover — showing it would name a commit that is not part of the
/// comparison at all.
#[gpui::test]
fn a_leftover_single_selection_does_not_supply_the_cards(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(78), 3, 1, Files::Loaded(1));

    assert!(
        cx.debug_bounds("commit_multi_row_0").is_some(),
        "expected the comparison's tip card"
    );
    assert!(
        cx.debug_bounds("commit_multi_row_1").is_some(),
        "expected both endpoints as cards, not the single leftover selection"
    );
}

/// A multi-selection comparison renders one card per selected commit, so a
/// large selection would otherwise grow an unbounded column and push the
/// changed-file list — the part the user came for — off the pane. The card
/// area is capped at half the comparison body, so the two lists split it
/// evenly and the cards scroll beyond that.
#[gpui::test]
fn a_large_multi_selection_splits_the_body_evenly_with_the_file_list(
    cx: &mut gpui::TestAppContext,
) {
    // Far more commits than can fit in half the body at ~44px per card. The
    // `debug_bounds` selectors below are literals, so they track these.
    const REPO_ID: RepoId = RepoId(79);
    const SELECTED: usize = 60;
    let cx = draw_comparison(cx, REPO_ID, SELECTED, SELECTED, Files::Loaded(3));

    let body = cx
        .debug_bounds("range_comparison_body")
        .expect("the comparison body should render");
    let cards = cx
        .debug_bounds("range_comparison_cards")
        .expect("the card area should render");
    let files = cx
        .debug_bounds("range_comparison_files")
        .expect("the changed-file section should render");
    let row = cx
        .debug_bounds("commit_multi_row_0")
        .expect("the first card should render");

    // Guard the comparisons below against a collapsed (zero-height) layout,
    // which would satisfy them vacuously.
    assert!(
        cards.size.height >= row.size.height,
        "the card area should be at least one card tall"
    );

    // Derive the uncapped height from a real card rather than the layout
    // constant, so this stays honest if the card height changes.
    let natural_height = row.size.height * SELECTED as f32;
    assert!(
        natural_height > cards.size.height,
        "the selection should overflow the card area (natural {natural_height:?}, actual {:?})",
        cards.size.height
    );

    // The cap is half the *body*, not half the window — so it holds whatever
    // height the splitter leaves the pane at.
    assert!(
        cards.size.height <= body.size.height * 0.5 + px(1.0),
        "the card area should be capped at half the body (got {:?} of {:?})",
        cards.size.height,
        body.size.height
    );
    // And the file list keeps essentially the other half. Not exactly half: the
    // subheader line and the body's two gaps come out of the body before either
    // list, so the shortfall is that chrome rather than anything the cards took.
    // A cap that stopped working would put this far below 40%.
    assert!(
        files.size.height >= body.size.height * 0.4,
        "the file list should keep essentially the other half (files {:?} of body {:?})",
        files.size.height,
        body.size.height
    );

    // Capped means virtualized: the far end of the selection is scrolled out.
    assert!(
        cx.debug_bounds("commit_multi_row_59").is_none(),
        "the last card should be out of view rather than stretching the pane"
    );

    // The point of the cap: the changed-file list still has room below.
    assert!(
        cx.debug_bounds("range_file_79_0").is_some(),
        "the changed-file list must stay visible under a large selection"
    );
}

/// A two-endpoint comparison is nowhere near the cap, so its cards size to
/// their content and leave the rest of the body to the file list.
#[gpui::test]
fn a_small_comparison_sizes_its_cards_to_content(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(83), 2, 0, Files::Loaded(3));

    let body = cx
        .debug_bounds("range_comparison_body")
        .expect("the comparison body should render");
    let cards = cx
        .debug_bounds("range_comparison_cards")
        .expect("the card area should render");
    let row = cx
        .debug_bounds("commit_multi_row_0")
        .expect("the first card should render");

    assert!(
        cards.size.height <= row.size.height * 2.0 + px(1.0),
        "two endpoints should occupy two rows, not the whole cap"
    );
    assert!(
        cards.size.height < body.size.height * 0.5,
        "a small comparison should stay well under the cap"
    );
}

/// A failed file listing must not render as "No files." — that is exactly what
/// a genuinely identical pair of commits looks like, so the user would read a
/// broken comparison as a successful, empty one.
#[gpui::test]
fn a_failed_file_listing_shows_the_error_not_an_empty_comparison(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(80), 2, 0, Files::Failed("object not found"));

    assert!(
        cx.debug_bounds("range_files_error").is_some(),
        "a failed listing should render its error"
    );
    assert!(
        cx.debug_bounds("range_files_empty").is_none(),
        "a failed listing must not be presented as an empty comparison"
    );
    // And it must not claim a count it never got.
    assert!(cx.debug_bounds("range_files_label_pending").is_some());
    assert!(cx.debug_bounds("range_files_label_count").is_none());
}

/// While the listing loads there is no count to state, so the section header
/// must not assert one — "0 changed" is both definite and usually wrong.
#[gpui::test]
fn a_loading_file_listing_does_not_claim_a_count(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(81), 2, 0, Files::Loading);

    assert!(
        cx.debug_bounds("range_files_loading").is_some(),
        "a loading listing should say so"
    );
    assert!(
        cx.debug_bounds("range_files_label_pending").is_some(),
        "the header must not state a count before there is one"
    );
    assert!(cx.debug_bounds("range_files_label_count").is_none());
    assert!(cx.debug_bounds("range_files_empty").is_none());
}

/// A genuinely empty comparison still reads as empty, and does state its count.
#[gpui::test]
fn an_empty_comparison_reads_as_empty(cx: &mut gpui::TestAppContext) {
    let cx = draw_comparison(cx, RepoId(82), 2, 0, Files::Loaded(0));

    assert!(cx.debug_bounds("range_files_empty").is_some());
    assert!(cx.debug_bounds("range_files_error").is_none());
    assert!(cx.debug_bounds("range_files_label_count").is_some());
}
