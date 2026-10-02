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
    let lifetime = state.repos[0].lifetime();
    let (a, b) = (DiffViewId::next(), DiffViewId::next());
    let history_before = state.repos[0].diff_state.diff_target_rev;

    let open_a = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view: a,
            target: worktree("a.rs"),
        }),
    );
    let first_a = session_effect(&open_a).clone();
    assert!(
        matches!(
            state.repos[0].diff_sessions[&a].diff_file_image,
            Loadable::NotLoaded
        ),
        "a text pane must not activate the image viewer"
    );
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
            lifetime,
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
            lifetime,
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
        Msg::DiffSession(DiffSessionMsg::Close {
            repo_id,
            lifetime,
            view: b,
        }),
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
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
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
    let lifetime = state.repos[0].lifetime();
    let (live, pinned) = (DiffViewId::next(), DiffViewId::next());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
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
            lifetime,
            view: pinned,
            target: DiffTarget::commit(CommitId("abc".into()), PathBuf::from("a.rs")),
        }),
    );

    let encoding = gitcomet_core::text_format::TextEncoding::from_label("windows-1252");
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::SetEncoding {
            repo_id,
            lifetime,
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
            lifetime,
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
            lifetime,
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

    // Finish the live load before checking the eager refresh path.
    let generation = state.repos[0].diff_sessions[&live].generation;
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, live, lifetime, generation),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Loaded {
            repo_id,
            view: live,
            lifetime,
            generation,
            content: DiffSessionContent::Attributes(Ok(Default::default())),
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Loaded {
            repo_id,
            view: live,
            lifetime,
            generation,
            content: DiffSessionContent::FileText(Ok(None)),
        }),
    );

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
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
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
            lifetime,
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
            lifetime,
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
        Msg::DiffSession(DiffSessionMsg::CloseChanges {
            repo_id,
            lifetime,
            view,
        }),
    );
    assert!(second.cancellation.is_cancelled());
    assert!(state.repos[0].change_lists.is_empty());
}

#[test]
fn watcher_edits_do_not_cancel_a_session_load_or_reload_unrelated_files() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("a.rs"),
        }),
    );
    let load = session_effect(&effects).clone();
    for path in ["other.rs", "a.rs", "a.rs"] {
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: RepoExternalChange {
                    paths: crate::msg::ChangedPaths::known(vec![path.into()]),
                    ..RepoExternalChange::worktree()
                },
            },
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::DiffSession(_)))
        );
        assert!(!load.cancellation.is_cancelled());
        assert_eq!(
            state.repos[0].diff_sessions[&view].generation,
            load.generation
        );
        assert_eq!(
            state.repos[0].diff_sessions[&view].refresh_queued,
            path != "other.rs"
        );
    }
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, view, load.lifetime, load.generation),
    );
    assert!(effects.is_empty(), "wait for the file text too");
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Loaded {
            repo_id,
            view,
            lifetime: load.lifetime,
            generation: load.generation,
            content: DiffSessionContent::Attributes(Ok(Default::default())),
        }),
    );
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Loaded {
            repo_id,
            view,
            lifetime: load.lifetime,
            generation: load.generation,
            content: DiffSessionContent::FileText(Ok(None)),
        }),
    );
    let refreshed = session_effect(&effects);
    assert_eq!(refreshed.generation, load.generation + 1);
    let session = &state.repos[0].diff_sessions[&view];
    assert!(session.is_loading());
    assert!(
        matches!(session.diff, Loadable::Ready(_)),
        "the completed patch remains visible during the follow-up load"
    );
    assert!(!session.refresh_queued);
}

/// A tab switch cancels the repository's loads, sessions included. The
/// cancelled replies leave a session idle with its last content, not an
/// error, and the repository's next activation loads it again.
#[test]
fn a_cancelled_session_load_keeps_its_content_until_the_repository_is_active_again() {
    use crate::diff_session::ChangeSource;
    let (mut repos, ids, mut state, repo_id) = setup();
    state.repos[0].set_open(Loadable::Ready(()));
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("a.rs"),
        }),
    );
    let load = session_effect(&effects).clone();
    for content in [
        DiffSessionContent::Attributes(Ok(Default::default())),
        DiffSessionContent::FileText(Ok(None)),
    ] {
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Loaded {
                repo_id,
                view,
                lifetime: load.lifetime,
                generation: load.generation,
                content,
            }),
        );
    }
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, view, load.lifetime, load.generation),
    );
    assert!(!state.repos[0].diff_sessions[&view].is_loading());

    // An edit starts a reload; the tab is switched away while it runs.
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange {
                paths: crate::msg::ChangedPaths::known(vec!["a.rs".into()]),
                ..RepoExternalChange::worktree()
            },
        },
    );
    let reload = session_effect(&effects).clone();
    fn cancelled<T>() -> gitcomet_core::services::Result<T> {
        Err(gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Cancelled,
        ))
    }
    for content in [
        DiffSessionContent::Attributes(cancelled()),
        DiffSessionContent::Patch(cancelled()),
        DiffSessionContent::FileText(cancelled()),
    ] {
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Loaded {
                repo_id,
                view,
                lifetime: reload.lifetime,
                generation: reload.generation,
                content,
            }),
        );
        assert!(effects.is_empty(), "no retry into the same cancellation");
    }
    let session = &state.repos[0].diff_sessions[&view];
    assert!(!session.is_loading(), "every part was answered");
    assert!(
        matches!(session.diff, Loadable::Ready(_)),
        "the patch shown before the reload stays, not an error"
    );
    assert!(matches!(session.diff_file, Loadable::Ready(None)));
    assert!(session.refresh_queued);

    // A change list cancelled the same way, with nothing to retain.
    let list = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view: list,
            source: ChangeSource::Worktree {
                area: DiffArea::Unstaged,
                include_untracked: false,
            },
        }),
    );
    let list_load = session_effect(&effects).clone();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::ChangesLoaded {
            repo_id,
            view: list,
            lifetime: list_load.lifetime,
            generation: list_load.generation,
            result: cancelled(),
        }),
    );
    assert!(effects.is_empty());
    assert!(matches!(
        state.repos[0].change_lists[&list].files,
        Loadable::NotLoaded
    ));

    // Back on the tab, both load again.
    let effects = reduce(&mut repos, &ids, &mut state, Msg::SetActiveRepo { repo_id });
    let resumed: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::DiffSession(work) => Some((work.view, work.generation)),
            _ => None,
        })
        .collect();
    assert_eq!(
        resumed,
        vec![
            (view, reload.generation + 1),
            (list, list_load.generation + 1)
        ]
    );
    assert!(state.repos[0].diff_sessions[&view].is_loading());
    assert!(state.repos[0].change_lists[&list].is_loading());
}

#[test]
fn requested_blame_is_loaded_again_after_reload_and_encoding_change() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("a.rs"),
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::LoadBlame {
            repo_id,
            lifetime,
            view,
        }),
    );
    for event in [
        DiffSessionMsg::Reload {
            repo_id,
            lifetime,
            view,
        },
        DiffSessionMsg::SetEncoding {
            repo_id,
            lifetime,
            view,
            encoding: gitcomet_core::text_format::TextEncoding::from_label("windows-1252"),
        },
    ] {
        let effects = reduce(&mut repos, &ids, &mut state, Msg::DiffSession(event));
        assert!(effects.iter().any(|effect| matches!(effect,
            Effect::DiffSession(work) if matches!(work.work, DiffSessionWork::Blame { .. }))));
        assert!(matches!(
            state.repos[0].diff_sessions[&view].blame,
            Loadable::Loading
        ));
    }
}

#[test]
fn session_commands_from_a_closed_repository_cannot_reach_its_replacement() {
    use crate::diff_session::ChangeSource;
    let (mut repos, ids, mut state, repo_id) = setup();
    let stale_lifetime = state.repos[0].lifetime();
    let spec = state.repos[0].spec.clone();
    state.repos[0] = RepoState::new_opening(repo_id, spec);
    let lifetime = state.repos[0].lifetime();
    assert_ne!(lifetime, stale_lifetime);
    let view = DiffViewId::next();
    let source = ChangeSource::Commit(CommitId("head".into()));
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("new.rs"),
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view,
            source: source.clone(),
        }),
    );
    let orphan = DiffViewId::next();
    let lifetime = stale_lifetime;
    for event in [
        DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view: orphan,
            target: worktree("orphan.rs"),
        },
        DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view: orphan,
            source: source.clone(),
        },
        DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("old.rs"),
        },
        DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view,
            source: ChangeSource::Commit(CommitId("old".into())),
        },
        DiffSessionMsg::SetEncoding {
            repo_id,
            lifetime,
            view,
            encoding: gitcomet_core::text_format::TextEncoding::from_label("windows-1252"),
        },
        DiffSessionMsg::Reload {
            repo_id,
            lifetime,
            view,
        },
        DiffSessionMsg::LoadBlame {
            repo_id,
            lifetime,
            view,
        },
        DiffSessionMsg::Close {
            repo_id,
            lifetime,
            view,
        },
        DiffSessionMsg::CloseChanges {
            repo_id,
            lifetime,
            view,
        },
    ] {
        assert!(reduce(&mut repos, &ids, &mut state, Msg::DiffSession(event)).is_empty());
    }
    assert_eq!(state.repos[0].diff_sessions.len(), 1);
    assert_eq!(state.repos[0].change_lists.len(), 1);
    let session = &state.repos[0].diff_sessions[&view];
    assert_eq!(session.target, worktree("new.rs"));
    assert_eq!(session.generation, 1);
    assert_eq!(session.encoding, None);
    assert!(!session.cancellation.is_cancelled());
    assert_eq!(state.repos[0].change_lists[&view].source, source);
    assert_eq!(state.repos[0].change_lists[&view].generation, 1);
}

#[test]
fn working_tree_change_lists_coalesce_and_clear_the_base_on_source_change() {
    use crate::diff_session::ChangeSource;
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view,
            source: ChangeSource::Comparison {
                from: CommitId("main".into()),
                to: None,
                options: Default::default(),
            },
        }),
    );
    let first = session_effect(&effects).clone();
    for _ in 0..3 {
        let effects = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::RepoExternallyChanged {
                repo_id,
                change: RepoExternalChange::worktree(),
            },
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::DiffSession(_)))
        );
        assert!(!first.cancellation.is_cancelled());
    }
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::ChangesLoaded {
            repo_id,
            lifetime,
            view,
            generation: first.generation,
            result: Ok((Some(CommitId("fork".into())), Vec::new())),
        }),
    );
    let next = session_effect(&effects).generation;
    assert_eq!(next, first.generation + 1);
    assert!(matches!(
        state.repos[0].change_lists[&view].files,
        Loadable::Ready(_)
    ));
    assert!(state.repos[0].change_lists[&view].is_loading());
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::ChangesLoaded {
            repo_id,
            lifetime,
            view,
            generation: next,
            result: Ok((Some(CommitId("fork".into())), Vec::new())),
        }),
    );
    assert!(effects.is_empty(), "only one follow-up load");
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view,
            source: ChangeSource::Commit(CommitId("different".into())),
        }),
    );
    assert_eq!(state.repos[0].change_lists[&view].base, None);
}

#[test]
fn known_worktree_paths_include_rename_sources_and_leave_staged_and_pinned_targets_alone() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let renamed = DiffViewId::next();
    let staged = DiffViewId::next();
    let pinned = DiffViewId::next();
    let other = DiffViewId::next();
    for (view, target) in [
        (
            renamed,
            DiffTarget::commit_range(CommitId("base".into()), None, Some("new.rs".into()))
                .with_old_path(Some("old.rs".into())),
        ),
        (
            staged,
            DiffTarget::working_tree("old.rs".into(), DiffArea::Staged),
        ),
        (
            pinned,
            DiffTarget::commit(CommitId("head".into()), "old.rs".into()),
        ),
        (other, worktree("other.rs")),
    ] {
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Open {
                repo_id,
                lifetime,
                view,
                target,
            }),
        );
    }
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange {
                paths: crate::msg::ChangedPaths::known(vec!["old.rs".into()]),
                ..RepoExternalChange::worktree()
            },
        },
    );
    for view in [renamed, staged, pinned, other] {
        assert_eq!(
            state.repos[0].diff_sessions[&view].refresh_queued,
            view == renamed
        );
    }
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: RepoExternalChange::index(),
        },
    );
    for view in [renamed, staged, pinned, other] {
        assert_eq!(
            state.repos[0].diff_sessions[&view].refresh_queued,
            view != pinned
        );
    }
}

#[test]
fn session_encoding_is_part_of_its_diff_state_and_does_not_change_history() {
    use gitcomet_core::text_format::{TextEncoding, TextOverride};
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("a.rs"),
        }),
    );
    let value = TextOverride {
        encoding: Some(TextEncoding::UTF_16LE),
        tab_size: Some(8),
    };
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::SetTextOverride {
            repo_id,
            lifetime,
            view,
            path: "a.rs".into(),
            value,
        }),
    );
    let session = &state.repos[0].diff_sessions[&view];
    assert_eq!(
        session.diff_state.selected_encoding_override(),
        value.encoding
    );
    assert_eq!(
        session.diff_state.text_override_for(Path::new("a.rs")),
        Some(value)
    );
    assert!(state.repos[0].diff_state.text_override.is_none());
    assert!(state.repos[0].diff_state.diff_target.is_none());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("b.rs"),
        }),
    );
    let session = &state.repos[0].diff_sessions[&view];
    assert_eq!(session.encoding, None);
    assert_eq!(session.diff_state.selected_encoding_override(), None);
}

#[test]
fn clearing_a_session_cancels_work_and_reload_cannot_reopen_it() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let work = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: worktree("a.rs"),
        }),
    );
    let first = session_effect(&work).clone();
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Clear {
            repo_id,
            lifetime,
            view,
        }),
    );
    assert!(first.cancellation.is_cancelled());
    for event in [
        DiffSessionMsg::Reload {
            repo_id,
            lifetime,
            view,
        },
        DiffSessionMsg::LoadBlame {
            repo_id,
            lifetime,
            view,
        },
    ] {
        assert!(reduce(&mut repos, &ids, &mut state, Msg::DiffSession(event)).is_empty());
    }
    reduce(
        &mut repos,
        &ids,
        &mut state,
        patch_loaded(repo_id, view, lifetime, first.generation),
    );
    let session = &state.repos[0].diff_sessions[&view];
    assert!(session.diff_target.is_none());
    assert!(!session.is_loading());
    assert!(matches!(session.diff, Loadable::NotLoaded));
}

#[test]
fn session_editor_restores_its_previous_commit_without_changing_history() {
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let original = DiffTarget::commit(CommitId("abc123".into()), "a.rs".into());
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: original.clone(),
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::SetContentMode {
            repo_id,
            lifetime,
            view,
            preview: true,
            edit: false,
        }),
    );
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenEditor {
            repo_id,
            lifetime,
            view,
            path: "a.rs".into(),
        }),
    );
    let session = &state.repos[0].diff_sessions[&view];
    assert_eq!(session.diff_target.as_ref(), Some(&worktree("a.rs")));
    assert!(session.edit_mode);
    assert!(session.content_preview);
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::ExitEditor {
            repo_id,
            lifetime,
            view,
        }),
    );
    let session = &state.repos[0].diff_sessions[&view];
    assert_eq!(session.diff_target.as_ref(), Some(&original));
    assert!(!session.edit_mode);
    assert!(session.content_preview);
    assert!(session.edit_return_view.is_none());
    assert!(state.repos[0].diff_state.diff_target.is_none());
    assert!(!state.repos[0].diff_state.edit_mode);
}

/// A linked worktree's sessions and lists name it in their loads and reload
/// only on its own changes; the main worktree's reload only on the main
/// one's. History ignores a linked selection.
#[test]
fn linked_worktree_sessions_reload_on_their_own_worktree_and_leave_history_alone() {
    use crate::diff_session::ChangeSource;
    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let linked = PathBuf::from("/tmp/sessions-linked");
    let (main_view, linked_view, list_view) =
        (DiffViewId::next(), DiffViewId::next(), DiffViewId::next());
    let history_before = (
        state.repos[0].diff_state.diff_target_rev,
        state.repos[0].navigation.main_history.entries.len(),
    );
    let mut open = |view, target| {
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Open {
                repo_id,
                lifetime,
                view,
                target,
            }),
        )
    };
    open(main_view, worktree("a.rs"));
    let effects = open(linked_view, worktree("a.rs").in_worktree(linked.clone()));
    assert_eq!(
        session_effect(&effects).work.linked_path(),
        Some(linked.as_path())
    );
    let source = ChangeSource::linked_worktree(linked.clone(), DiffArea::Unstaged, true);
    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::DiffSession(DiffSessionMsg::OpenChanges {
            repo_id,
            lifetime,
            view: list_view,
            source: source.clone(),
        }),
    );
    assert_eq!(
        session_effect(&effects).work.linked_path(),
        Some(linked.as_path())
    );
    let change = gitcomet_core::domain::CommitFileChange::new(
        "a.rs".into(),
        gitcomet_core::domain::FileStatusKind::Modified,
    );
    assert_eq!(
        source.target_for(&change, None),
        worktree("a.rs").in_worktree(linked.clone()),
        "a listed file opens in the linked worktree"
    );

    let edit = || RepoExternalChange {
        paths: crate::msg::ChangedPaths::known(vec!["a.rs".into()]),
        ..RepoExternalChange::worktree()
    };
    let queued = |state: &AppState| {
        let repo = &state.repos[0];
        (
            repo.diff_sessions[&main_view].refresh_queued,
            repo.diff_sessions[&linked_view].refresh_queued,
            repo.change_lists[&list_view].refresh_queued,
        )
    };
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: edit(),
        },
    );
    assert_eq!(queued(&state), (true, false, false), "the main worktree's");
    reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::WorktreeExternallyChanged {
            repo_id,
            lifetime,
            // Another spelling of the same worktree.
            path: linked.join("."),
            change: edit(),
        },
    );
    assert_eq!(
        queued(&state),
        (true, true, true),
        "and then the linked one's"
    );

    let effects = reduce(
        &mut repos,
        &ids,
        &mut state,
        Msg::SelectDiff {
            repo_id,
            target: worktree("a.rs").in_worktree(linked),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.repos[0].diff_state.diff_target, None);
    assert_eq!(
        (
            state.repos[0].diff_state.diff_target_rev,
            state.repos[0].navigation.main_history.entries.len(),
        ),
        history_before
    );
}
