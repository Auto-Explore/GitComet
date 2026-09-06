use super::*;
use gitcomet_core::domain::{CommitId, LogCursor, LogPage, RepoSpec};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::model::AppState;
use gitcomet_state::store::AppStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// The linked-worktree rows live in this table, so the two revs behind them
/// have to move the fingerprint. Without them a finished scan -- or a row
/// being selected -- changed nothing the pane hashed, and the rows sat stale
/// until some unrelated rev happened to move. Most visual tests use uncached
/// mounts because GPUI does not replay debug bounds during cache reuse; focused
/// invalidation tests can opt into the production cache path.
#[test]
fn the_history_fingerprint_tracks_the_worktree_revs() {
    let mut state = AppState::default();
    state
        .repos
        .push(gitcomet_state::model::RepoState::new_opening(
            gitcomet_state::model::RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
    state.active_repo = Some(gitcomet_state::model::RepoId(1));

    let fingerprint = |state: &AppState| HistoryView::notify_fingerprint_for(state, false);
    let before = fingerprint(&state);

    // The revs stand in for the writes that bump them: those setters are
    // `pub(crate)` to `gitcomet-state`, and what is being asserted here is
    // that the fingerprint reads them at all.
    state.repos[0].worktree_dirty_rev += 1;
    let after_scan = fingerprint(&state);
    assert_ne!(
        before, after_scan,
        "a finished worktree scan must repaint the rows it feeds"
    );

    state.repos[0].history_state.worktree_selection_rev += 1;
    assert_ne!(
        after_scan,
        fingerprint(&state),
        "selecting a worktree row must repaint the row that shows it"
    );
}

struct BlockingBackend;

impl GitBackend for BlockingBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        loop {
            std::thread::park();
        }
    }
}

fn wait_until(
    cx: &mut gpui::VisualTestContext,
    description: &str,
    ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn set_history_view_state_for_tests(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    state: Arc<AppState>,
) {
    cx.update(|window, app| {
        // Cache replacements become visible during render. Mount the fixture
        // in the application too, so drawing the window actually draws history.
        let ui_model = view.read(app).ui_model.clone();
        ui_model.update(app, |model, cx| model.set_state(Arc::clone(&state), cx));
        let history_view = view.read(app).main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| {
            history.notify_fingerprint =
                HistoryView::notify_fingerprint_for(&state, history.history_show_tags);
            history.state = Arc::clone(&state);
            cx.notify();
        });
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn ensure_history_cache_for_tests(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    state: Arc<AppState>,
) {
    set_history_view_state_for_tests(cx, view, state);
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let history_view = main_pane.read(app).history_view.clone();
        history_view.update(app, |history, cx| history.ensure_history_cache(cx));
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn commit(id: &str, parents: &[&str], summary: &str) -> Commit {
    Commit {
        id: CommitId(id.into()),
        parent_ids: parents.iter().map(|p| CommitId((*p).into())).collect(),
        summary: summary.into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    }
}

/// Anchor placement is the part of the plan that depends on repo data: a
/// dirty worktree earns a row only when its HEAD is a commit currently on
/// screen.
fn worktree_anchors_for(
    commits: &[Commit],
    worktrees: &[(&str, &str)],
    dirty_paths: &[&str],
) -> Vec<usize> {
    let visible = HistoryVisibleIndices::all(commits.len());
    let mut visible_ix_by_commit: FxHashMap<&str, usize> = FxHashMap::default();
    for (visible_ix, commit_ix) in visible.iter().enumerate() {
        visible_ix_by_commit
            .entry(commits[commit_ix].id.as_ref())
            .or_insert(visible_ix);
    }
    dirty_paths
        .iter()
        .filter_map(|path| {
            let head = worktrees.iter().find(|(p, _)| p == path)?.1;
            visible_ix_by_commit.get(head).copied()
        })
        .collect()
}

fn all_columns_visible_drag_layout() -> HistoryColumnDragLayout {
    HistoryColumnDragLayout {
        show_graph: true,
        show_author: true,
        show_date: true,
        show_sha: true,
        branch_w: px(HISTORY_COL_BRANCH_PX),
        graph_w: px(HISTORY_COL_GRAPH_PX),
        author_w: px(HISTORY_COL_AUTHOR_PX),
        date_w: px(HISTORY_COL_DATE_PX),
        sha_w: px(HISTORY_COL_SHA_PX),
    }
}

fn branch(name: &str, target: &str) -> Branch {
    Branch {
        name: name.into(),
        target: CommitId(target.into()),
        upstream: None,
        divergence: None,
    }
}

fn remote_branch(remote: &str, name: &str, target: &str) -> RemoteBranch {
    RemoteBranch {
        remote: remote.into(),
        name: name.into(),
        target: CommitId(target.into()),
    }
}

fn log_page(commits: Vec<Commit>, next_cursor: Option<&str>) -> LogPage {
    LogPage {
        commits,
        next_cursor: next_cursor.map(|last_seen| LogCursor {
            last_seen: CommitId(last_seen.into()),
            resume_from: None,
            resume_token: None,
        }),
    }
}

/// The commit-id index the base cache carries agrees with the visible order it
/// was built from.
///
/// Its readers -- the worktree row anchors and the selected lane's colour --
/// look commits up during layout, and both used to scan the page instead. A
/// map that disagrees with `visible_indices` would anchor rows on the wrong
/// commits, so this pins the two together.

/// Branch attributed to each visible row, in row order.
fn lane_branch_labels(
    commits: Vec<Commit>,
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
    head_branch: Option<&str>,
) -> Vec<Option<String>> {
    let page = log_page(commits, None);
    let base_request = HistoryBaseCacheRequest {
        repo_id: RepoId(1),
        history_scope: LogScope::AllBranches,
        log_source: 0,
        history_author_filter: None,
        head_branch_rev: 0,
        detached_head_commit: None,
        head_branch_target: None,
        branches_rev: 0,
        remote_branches_rev: 0,
        stashes_rev: 0,
    };
    let base = build_history_base_cache(
        base_request.clone(),
        &page,
        AppTheme::gitcomet_dark(),
        head_branch,
        branches,
        remote_branches,
        &[],
    );
    let decorations = build_history_decoration_cache(
        HistoryDecorationCacheRequest {
            base_request,
            head_branch_rev: 0,
            detached_head_commit: None,
            branches_rev: 0,
            remote_branches_rev: 0,
            tags_rev: 0,
        },
        &page,
        &base,
        head_branch,
        branches,
        remote_branches,
        &[],
    );

    decorations
        .row_vms
        .iter()
        .map(|row| {
            row.lane_branch
                .and_then(|ix| decorations.branch_names.get(usize::from(ix)))
                .map(|name| name.to_string())
        })
        .collect()
}

mod base_cache;
mod columns;
mod interaction;
mod lane_attribution;
mod refresh;
mod reveal;
mod selection;
mod worktree_anchors;
