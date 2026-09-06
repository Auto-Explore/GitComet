//! The Files tab following the history selection, end to end through `reduce`.

use super::*;
use crate::model::{FileBrowserSettings, SidebarMode};
use gitcomet_core::domain::{FileEntry, FileEntryKind, FileSource};
use std::sync::atomic::AtomicU64;

type Repos = FxHashMap<RepoId, Arc<dyn GitRepository>>;

fn commit(n: u8) -> CommitId {
    CommitId(format!("{n:0>40}").into())
}

fn dir_entry(path: &str) -> FileEntry {
    FileEntry {
        name: path.to_string(),
        path: Arc::new(PathBuf::from(path)),
        kind: FileEntryKind::Directory,
        depth: 0,
    }
}

/// Sources of the file-browser loads in `effects`, in order.
fn file_browser_loads(effects: &[Effect]) -> Vec<FileSource> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::LoadFileBrowser { source, .. } => Some(source.clone()),
            _ => None,
        })
        .collect()
}

/// An open repo with active browsing and a loaded, partly expanded live tree.
fn ready_state(sidebar_mode: SidebarMode) -> (Repos, AtomicU64, AppState, RepoId) {
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.sidebar_mode = sidebar_mode;
    state.repos[0].set_open(Loadable::Ready(()));
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
    state.repos[0].file_browser.active = true;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![dir_entry("src")]));
    state.repos[0]
        .file_browser
        .expanded_dirs
        .insert(Arc::new(PathBuf::from("src")));
    (FxHashMap::default(), AtomicU64::new(1), state, repo_id)
}

fn select(
    repos: &mut Repos,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    repo_id: RepoId,
    commit_id: CommitId,
) -> Vec<Effect> {
    reduce(
        repos,
        id_alloc,
        state,
        Msg::SelectCommit { repo_id, commit_id },
    )
}

/// Deliver the listing the executor would send back, releasing the lane.
fn deliver_listing(
    repos: &mut Repos,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    repo_id: RepoId,
    source: FileSource,
) -> Vec<Effect> {
    reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
            repo_id,
            source,
            result: Ok(vec![dir_entry("src")]),
        }),
    )
}

#[test]
fn file_browsing_follows_only_after_it_is_started() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    state.repos[0].file_browser = crate::model::FileBrowserState::default();
    assert!(!state.repos[0].file_browser.active);

    let a = commit(1);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: a.clone(),
        },
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(a.clone())]
    );
    assert!(state.repos[0].file_browser.active);
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a),
    );

    let b = commit(2);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert_eq!(file_browser_loads(&effects), vec![FileSource::Commit(b)]);
}

#[test]
fn selecting_a_commit_browses_it_when_the_files_tab_shows() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let a = commit(1);

    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());

    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(a.clone())]
    );
    let repo = &state.repos[0];
    assert_eq!(repo.browsing_commit(), Some(&a));
    assert!(
        repo.file_browser
            .expanded_dirs
            .contains(&Arc::new(PathBuf::from("src"))),
        "following must not collapse the tree"
    );
    assert!(
        matches!(repo.file_browser.entries, Loadable::Ready(_)),
        "the rows stay up until the commit's listing lands"
    );
    assert!(
        repo.navigation.browse_history.is_empty(),
        "following is not a manual browse"
    );
    assert_eq!(state.sidebar_mode, SidebarMode::Files);
}

#[test]
fn the_setting_off_leaves_the_browse_point_alone() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    state.file_browser_settings.follow_selected_commit = false;

    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, commit(1));

    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);
}

#[test]
fn a_hidden_files_tab_catches_up_when_shown() {
    // The commit walk is only worth running while someone can see the tree.
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Branches);
    let a = commit(1);
    let b = commit(2);

    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, a);
    assert!(file_browser_loads(&effects).is_empty());
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetSidebarMode {
            mode: SidebarMode::Files,
        },
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(b.clone())],
        "exactly one walk, already for the selected commit"
    );
    assert_eq!(state.repos[0].browsing_commit(), Some(&b));
}

#[test]
fn the_working_tree_row_goes_live() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let a = commit(1);
    select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ClearCommitSelection { repo_id },
    );

    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::WorkingDirectory]
    );
    assert_eq!(state.repos[0].browsing_commit(), None);
    assert!(state.repos[0].file_browser.active);

    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
    );
    let b = commit(2);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert_eq!(file_browser_loads(&effects), vec![FileSource::Commit(b)]);
}

#[test]
fn exiting_file_browsing_stays_live_until_explicitly_started_again() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let a = commit(1);
    let b = commit(2);
    select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResetBrowseToLive { repo_id },
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::WorkingDirectory]
    );
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
    );

    // Unrelated traffic must not drag the tree back to the selected commit.
    let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::DismissBannerError);
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);

    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);
    assert!(!state.repos[0].file_browser.active);

    for mode in [SidebarMode::Branches, SidebarMode::Files] {
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SetSidebarMode { mode },
        );
        assert!(file_browser_loads(&effects).is_empty());
    }
    for enabled in [false, true] {
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SetFileBrowserSettings(FileBrowserSettings {
                follow_selected_commit: enabled,
            }),
        );
        assert!(file_browser_loads(&effects).is_empty());
    }
    assert!(!state.repos[0].file_browser.active);

    let other = RepoId(2);
    let mut other_repo = RepoState::new_opening(
        other,
        RepoSpec {
            workdir: PathBuf::from("/tmp/other"),
        },
    );
    other_repo.set_open(Loadable::Ready(()));
    other_repo.file_browser.entries = Loadable::Ready(Arc::new(Vec::new()));
    state.repos.push(other_repo);
    for repo_id in [other, repo_id] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SetActiveRepo { repo_id },
        );
    }
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert!(file_browser_loads(&effects).is_empty());
    assert!(!state.repos[0].file_browser.active);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: b.clone(),
        },
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(b.clone())]
    );
    assert!(state.repos[0].file_browser.active);
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(b),
    );
    let c = commit(3);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, c.clone());
    assert_eq!(file_browser_loads(&effects), vec![FileSource::Commit(c)]);
}

#[test]
fn exiting_file_browsing_while_live_updates_the_mode_without_reloading() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let rev = state.repos[0].file_browser.file_browser_rev;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResetBrowseToLive { repo_id },
    );
    assert!(file_browser_loads(&effects).is_empty());
    assert!(!state.repos[0].file_browser.active);
    assert_ne!(state.repos[0].file_browser.file_browser_rev, rev);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, commit(1));
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), None);
}

#[test]
fn a_pending_commit_listing_cannot_resume_file_browsing_after_exit() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let a = commit(1);
    select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    select(&mut repos, &id_alloc, &mut state, repo_id, commit(2));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResetBrowseToLive { repo_id },
    );
    assert!(file_browser_loads(&effects).is_empty());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadFileBrowser {
            repo_id,
            source: FileSource::Commit(a.clone()),
        },
    );
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(
        state.repos[0].file_browser.source,
        FileSource::WorkingDirectory
    );
    select(&mut repos, &id_alloc, &mut state, repo_id, commit(3));
    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a),
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::WorkingDirectory]
    );
    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
    );
    assert!(file_browser_loads(&effects).is_empty());
    assert!(!state.repos[0].file_browser.active);
    assert_eq!(state.repos[0].browsing_commit(), None);
    assert!(!state.repos[0].file_browser.stale);
}

#[test]
fn a_manual_browse_elsewhere_sticks_until_the_selection_moves() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let a = commit(1);
    let b = commit(2);
    let c = commit(3);
    select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a),
    );

    // A SHA link browses a commit that is not the selected row.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: c.clone(),
        },
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(c.clone())]
    );
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(c.clone()),
    );

    let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::DismissBannerError);
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), Some(&c));

    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, b.clone());
    assert_eq!(file_browser_loads(&effects), vec![FileSource::Commit(b)]);
}

#[test]
fn a_manual_browse_from_a_hidden_tab_is_not_undone_by_the_first_sync() {
    // Selecting with the tab hidden records nothing; the SHA-link browse that
    // then shows the tab must win over the selection it never followed.
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Branches);
    let a = commit(1);
    let c = commit(3);
    select(&mut repos, &id_alloc, &mut state, repo_id, a);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::BrowseRepositoryAtCommit {
            repo_id,
            commit_id: c.clone(),
        },
    );

    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(c.clone())]
    );
    assert_eq!(state.sidebar_mode, SidebarMode::Files);
    assert_eq!(state.repos[0].browsing_commit(), Some(&c));
}

#[test]
fn a_burst_of_selections_walks_once_and_queues_the_last() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let commits: Vec<CommitId> = (1..=5).map(commit).collect();

    let mut loads = Vec::new();
    for id in &commits {
        loads.extend(file_browser_loads(&select(
            &mut repos,
            &id_alloc,
            &mut state,
            repo_id,
            id.clone(),
        )));
    }
    assert_eq!(
        loads,
        vec![FileSource::Commit(commits[0].clone())],
        "the lane holds one walk; the rest coalesce into one pending request"
    );
    assert_eq!(state.repos[0].browsing_commit(), Some(&commits[4]));

    // The first walk's reply is for a source nobody wants; it releases the lane
    // and the queued request carries the last selection.
    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(commits[0].clone()),
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(commits[4].clone())]
    );
    assert!(
        state.repos[0].file_browser.stale,
        "the old rows are still the ones on screen"
    );

    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(commits[4].clone()),
    );
    assert!(file_browser_loads(&effects).is_empty());
    assert!(!state.repos[0].file_browser.stale);
}

#[test]
fn turning_the_setting_on_syncs_immediately() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    state.file_browser_settings.follow_selected_commit = false;
    let a = commit(1);
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    assert!(file_browser_loads(&effects).is_empty());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserSettings(FileBrowserSettings {
            follow_selected_commit: true,
        }),
    );

    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(a.clone())]
    );
    assert_eq!(state.repos[0].browsing_commit(), Some(&a));

    // Turning it off keeps whatever is being browsed.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetFileBrowserSettings(FileBrowserSettings {
            follow_selected_commit: false,
        }),
    );
    assert!(file_browser_loads(&effects).is_empty());
    assert_eq!(state.repos[0].browsing_commit(), Some(&a));
}

#[test]
fn activating_a_repo_tab_enters_it_live_with_one_walk() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let other = RepoId(2);
    state.repos.push(RepoState::new_opening(
        other,
        RepoSpec {
            workdir: PathBuf::from("/tmp/other"),
        },
    ));
    state.repos[1].set_open(Loadable::Ready(()));
    state.repos[1].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));

    let a = commit(1);
    select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(a.clone()),
    );
    assert_eq!(state.repos[0].browsing_commit(), Some(&a));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: other },
    );
    // Coming back clears the selection, so the tab enters at its live tip and
    // the activation's own listing request already names the live tree.
    let mut effects = crate::store::reducer::SetActiveRepoEffects::new();
    crate::store::reducer::fill_set_active_repo_inline(&repos, &mut state, repo_id, &mut effects);

    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::WorkingDirectory]
    );
    assert_eq!(state.repos[0].browsing_commit(), None);

    // Selecting before the activation's walk finishes must share its lane.
    let effects = select(&mut repos, &id_alloc, &mut state, repo_id, a.clone());
    assert!(file_browser_loads(&effects).is_empty());
    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::WorkingDirectory,
    );
    assert_eq!(file_browser_loads(&effects), vec![FileSource::Commit(a)]);
}

#[test]
fn following_reopens_the_file_at_the_latest_selection_and_closes_it_when_missing() {
    let (mut repos, id_alloc, mut state, repo_id) = ready_state(SidebarMode::Files);
    let path = PathBuf::from("src/lib.rs");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenFileContent {
            repo_id,
            source: FileSource::WorkingDirectory,
            path: path.clone(),
        },
    );
    let first = commit(1);
    let latest = commit(2);
    select(&mut repos, &id_alloc, &mut state, repo_id, first.clone());
    select(&mut repos, &id_alloc, &mut state, repo_id, latest.clone());
    let effects = deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(first),
    );
    assert_eq!(
        file_browser_loads(&effects),
        vec![FileSource::Commit(latest.clone())]
    );
    assert!(state.repos[0].file_browser.pending_reopen.is_some());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
            repo_id,
            source: FileSource::Commit(latest.clone()),
            result: Ok(vec![
                dir_entry("src"),
                FileEntry {
                    name: "lib.rs".to_string(),
                    path: Arc::new(path.clone()),
                    kind: FileEntryKind::File,
                    depth: 1,
                },
            ]),
        }),
    );
    assert_eq!(
        state.repos[0].diff_state.diff_target,
        Some(DiffTarget::Commit {
            commit_id: latest,
            path: Some(path),
        })
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadSelectedDiff { .. }))
    );

    let missing = commit(3);
    select(&mut repos, &id_alloc, &mut state, repo_id, missing.clone());
    deliver_listing(
        &mut repos,
        &id_alloc,
        &mut state,
        repo_id,
        FileSource::Commit(missing),
    );
    assert!(state.repos[0].diff_state.diff_target.is_none());
    assert!(!state.repos[0].diff_state.content_preview);
    assert!(
        state.repos[0]
            .file_browser
            .expanded_dirs
            .contains(&Arc::new(PathBuf::from("src")))
    );
}
