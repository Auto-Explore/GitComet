//! Diff sessions: independent targets, generations, cancellation, and
//! teardown, apart from History's selected diff.

use super::*;
use crate::diff_session::{DiffSessionContent, DiffSessionMsg, DiffSessionWork, DiffViewId};
use gitcomet_core::domain::{Diff, DiffArea, DiffTarget};

fn setup() -> (
    FxHashMap<RepoId, Arc<dyn GitRepository>>,
    AtomicU64,
    AppState,
    RepoId,
) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/sessions")));
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/sessions"),
        },
    ));
    (repos, AtomicU64::new(1), state, repo_id)
}

fn worktree(path: &str) -> DiffTarget {
    DiffTarget::working_tree(PathBuf::from(path), DiffArea::Unstaged)
}

fn session_effect(effects: &[Effect]) -> &crate::diff_session::DiffSessionEffect {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::DiffSession(work) => Some(work),
            _ => None,
        })
        .expect("a session load")
}

fn patch_loaded(repo_id: RepoId, view: DiffViewId, lifetime: u64, generation: u64) -> Msg {
    Msg::DiffSession(DiffSessionMsg::Loaded {
        repo_id,
        view,
        lifetime,
        generation,
        content: DiffSessionContent::Patch(Ok(Diff::from_unified(
            worktree("a.rs"),
            "diff --git a/a.rs b/a.rs\n",
        ))),
    })
}

#[test]
fn two_sessions_load_retarget_and_close_without_touching_each_other_or_history() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let (a, b) = (DiffViewId::next(), DiffViewId::next());
    let history_before = state.repos[0].diff_state.diff_target_rev;
    let lifetime = state.repos[0].lifetime();

    let open_a = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view: a,
            target: worktree("a.rs"),
        }),
    );
    let first_a = session_effect(&open_a).clone();
    assert!(matches!(
        first_a.work,
        DiffSessionWork::Content {
            patch: true,
            file_text: true,
            image: false,
            ..
        }
    ));
    let open_b = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view: b,
            target: worktree("b.rs"),
        }),
    );
    let first_b = session_effect(&open_b).clone();

    // Retargeting A cancels A's work only.
    let retarget = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view: a,
            target: worktree("c.rs"),
        }),
    );
    let second_a = session_effect(&retarget).clone();
    assert!(first_a.cancellation.is_cancelled());
    assert!(!second_a.cancellation.is_cancelled());
    assert!(!first_b.cancellation.is_cancelled());
    assert_ne!(first_a.generation, second_a.generation);

    // A reply for A's old generation is dropped; B's lands.
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, a, lifetime, first_a.generation),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, b, lifetime, first_b.generation),
    );
    let sessions = &state.repos[0].diff_sessions;
    assert!(matches!(sessions[&a].diff, Loadable::Loading));
    assert!(matches!(sessions[&b].diff, Loadable::Ready(_)));

    // Closing B cancels its work and forgets it; A is unaffected.
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Close { repo_id, view: b }),
    );
    assert!(first_b.cancellation.is_cancelled());
    assert!(!state.repos[0].diff_sessions.contains_key(&b));
    assert!(state.repos[0].diff_sessions.contains_key(&a));
    assert!(!second_a.cancellation.is_cancelled());

    assert_eq!(
        state.repos[0].diff_state.diff_target_rev, history_before,
        "History's selected diff is untouched"
    );
    assert!(state.repos[0].diff_state.diff_target.is_none());
}

#[test]
fn replies_from_a_previous_repository_lifetime_are_dropped() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let view = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view,
            target: worktree("a.rs"),
        }),
    );
    let work = session_effect(&effects).clone();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(
            repo_id,
            view,
            work.lifetime.wrapping_add(1),
            work.generation,
        ),
    );
    assert!(matches!(
        state.repos[0].diff_sessions[&view].diff,
        Loadable::Loading
    ));
}

#[test]
fn encoding_blame_and_worktree_edits_reload_the_right_sessions() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let (live, pinned) = (DiffViewId::next(), DiffViewId::next());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view: live,
            target: worktree("a.rs"),
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            view: pinned,
            target: DiffTarget::commit(CommitId("abc".into()), Some(PathBuf::from("a.rs"))),
        }),
    );

    let encoding = gitcomet_core::text_format::TextEncoding::from_label("windows-1252");
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::SetEncoding {
            repo_id,
            view: live,
            encoding,
        }),
    );
    assert!(matches!(
        &session_effect(&effects).work,
        DiffSessionWork::Content { encoding: sent, .. } if *sent == encoding
    ));
    let unchanged = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::SetEncoding {
            repo_id,
            view: live,
            encoding,
        }),
    );
    assert!(unchanged.is_empty(), "the same encoding does not reload");

    let blame = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::LoadBlame {
            repo_id,
            view: pinned,
        }),
    );
    assert!(matches!(
        &session_effect(&blame).work,
        DiffSessionWork::Blame { source: gitcomet_core::domain::BlameSource::Revision(Some(rev)), .. }
            if rev == "abc"
    ));
    assert!(matches!(
        state.repos[0].diff_sessions[&pinned].blame,
        Loadable::Loading
    ));

    // A worktree edit reloads the live session, not the commit one.
    let before: Vec<u64> = [live, pinned]
        .iter()
        .map(|view| state.repos[0].diff_sessions[view].generation)
        .collect();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange::worktree(),
        },
    );
    let reloaded: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DiffSession(work) => Some(work.view),
            _ => None,
        })
        .collect();
    assert_eq!(reloaded, vec![live]);
    assert_ne!(state.repos[0].diff_sessions[&live].generation, before[0]);
    assert_eq!(state.repos[0].diff_sessions[&pinned].generation, before[1]);
}

#[test]
fn change_lists_load_by_generation_and_give_each_file_its_target() {
    use crate::diff_session::ChangeSource;
    use gitcomet_core::domain::{CommitFileChange, FileStatusKind};

    let (mut repos, ids, mut state, repo_id) = setup();
    let view = DiffViewId::next();
    let lifetime = state.repos[0].lifetime();
    let source = ChangeSource::Comparison {
        from: CommitId("main".into()),
        to: Some(CommitId("feature".into())),
        options: gitcomet_core::services::ComparisonOptions::merge_base(),
    };
    let first = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            view,
            source: source.clone(),
        }),
    );
    let first = session_effect(&first).clone();
    assert!(matches!(&first.work, DiffSessionWork::Changes { source: sent } if *sent == source));
    let reopened = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            view,
            source: source.clone(),
        }),
    );
    let second = session_effect(&reopened).clone();
    assert!(first.cancellation.is_cancelled());

    let renamed = CommitFileChange::new(PathBuf::from("new.rs"), FileStatusKind::Renamed)
        .with_old_path(Some(PathBuf::from("old.rs")));
    for generation in [first.generation, second.generation] {
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::ChangesLoaded {
                repo_id,
                view,
                lifetime,
                generation,
                result: Ok((Some(CommitId("fork".into())), vec![renamed.clone()])),
            }),
        );
    }
    let list = &state.repos[0].change_lists[&view];
    assert!(matches!(&list.files, Loadable::Ready(files) if files.len() == 1));
    assert_eq!(list.rev, 3, "open, reopen, and one accepted load");
    let target = list.source.target_for(&renamed, list.base.as_ref());
    assert_eq!(
        target,
        DiffTarget::commit_range(
            CommitId("fork".into()),
            Some(CommitId("feature".into())),
            Some(PathBuf::from("new.rs"))
        )
    );
    assert_eq!(target.old_file_path(), Some(std::path::Path::new("old.rs")));

    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::CloseChanges { repo_id, view }),
    );
    assert!(second.cancellation.is_cancelled());
    assert!(state.repos[0].change_lists.is_empty());
}
