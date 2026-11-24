use super::*;
use gitcomet_core::domain::{DiffPreviewTextSide, FileDiffImage, FileDiffText};

#[test]
fn a_hosted_pane_follows_its_own_patch_across_retargets() {
    use DiffPreviewTextSide::{New, Old};

    let (mut repos, ids, mut state, repo_id) = setup();
    let lifetime = state.repos[0].lifetime();
    let view = DiffViewId::next();
    let header = "diff --git a/file b/file\n";
    for (metadata, expected) in [
        ("new file mode 100644\n", Some(New)),
        ("deleted file mode 100755\n", Some(Old)),
        ("new file mode 120000\n", Some(New)),
        ("deleted file mode 160000\n", None),
        (
            "index 1111111..2222222 100644\n--- a/file\n+++ b/file\n@@ -0,0 +1 @@\n+text\n",
            None,
        ),
        (
            "deleted file mode 100644\ndiff --git a/file b/file\nnew file mode 120000\n",
            None,
        ),
    ] {
        let target = DiffTarget::commit_range(CommitId("base".into()), None, Some("file".into()));
        let open = reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Open {
                repo_id,
                lifetime,
                view,
                target: target.clone(),
                request: None,
            }),
        );
        assert_eq!(
            state.repos[0].diff_sessions[&view].present_file_side(),
            None
        );
        reduce(
            &mut repos,
            &ids,
            &mut state,
            Msg::DiffSession(DiffSessionMsg::Loaded {
                repo_id,
                lifetime,
                view,
                generation: session_effect(&open).generation,
                content: DiffSessionContent::Patch(Ok(Diff::from_unified(
                    target,
                    &format!("{header}{metadata}"),
                ))),
            }),
        );
        assert_eq!(
            state.repos[0].diff_sessions[&view].present_file_side(),
            expected,
            "{metadata}"
        );
    }
}

#[test]
fn a_truncated_deletion_section_can_be_the_first_half_of_a_type_change() {
    use crate::diff_session::{DiffSession, one_sided_patch};

    let target = DiffTarget::commit(CommitId("tip".into()), "file".into());
    let patch = Diff::from_unified(
        target.clone(),
        "diff --git a/file b/file\ndeleted file mode 100644\n@@ -1 +0,0 @@\n-old\n... diff truncated at 64 MiB\n",
    );
    let mut session = DiffSession::new(target);
    session.patch_side = one_sided_patch(&patch);
    session.diff = Loadable::Ready(Arc::new(patch));
    for (new, expected) in [
        (Some("target".into()), None),
        (None, Some(DiffPreviewTextSide::Old)),
    ] {
        session.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new(
            "file".into(),
            Some("old".into()),
            new,
        ))));
        assert_eq!(session.present_file_side(), expected);
    }
}

#[test]
fn untracked_linked_worktree_files_have_a_new_side_even_without_a_patch() {
    use crate::diff_session::DiffSession;

    let path = PathBuf::from("file");
    let target = worktree("file").in_worktree("/tmp/linked".into());
    let mut session = DiffSession::new(target.clone());
    session.diff = Loadable::Ready(Arc::new(Diff::from_unified(target, "")));
    assert_eq!(session.present_file_side(), None);
    session.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new(
        path.clone(),
        None,
        Some(String::new()),
    ))));
    assert_eq!(session.present_file_side(), Some(DiffPreviewTextSide::New));
    session.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText::new(
        path.clone(),
        Some(String::new()),
        Some(String::new()),
    ))));
    assert_eq!(session.present_file_side(), None);
    session.diff_file = Loadable::NotLoaded;
    session.diff_file_image = Loadable::Ready(Some(Arc::new(FileDiffImage {
        path,
        new: Some(vec![1]),
        ..Default::default()
    })));
    assert_eq!(session.present_file_side(), Some(DiffPreviewTextSide::New));
    session.content_preview = true;
    assert_eq!(session.present_file_side(), None);
}

#[test]
fn a_linked_worktrees_staged_file_and_gitlink_use_their_own_modes() {
    use crate::store::tests::worktree_redirect::{
        open_repo_and_wait, repo_with_linked_worktree, wait_until,
    };
    use gitcomet_git_gix::GixBackend;

    let (_dir, main, linked) = repo_with_linked_worktree();
    let head = GixBackend
        .open(&linked)
        .unwrap()
        .head_commit_id()
        .unwrap()
        .unwrap();
    run_git(
        &linked,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},module", head.as_ref()),
        ],
    );
    fs::write(linked.join("empty.txt"), "").unwrap();
    run_git(&linked, &["add", "empty.txt"]);
    let (store, _events) = AppStore::new_test(Arc::new(GixBackend));
    store.disable_repo_monitors_for_test();
    let repo_id = open_repo_and_wait(&store, &main);
    let lifetime = store.snapshot().repos[0].lifetime();
    let view = DiffViewId::next();
    for (path, expected) in [
        ("empty.txt", Some(DiffPreviewTextSide::New)),
        ("module", None),
    ] {
        let target =
            DiffTarget::working_tree(path.into(), DiffArea::Staged).in_worktree(linked.clone());
        store.dispatch(Msg::DiffSession(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: target.clone(),
            request: None,
        }));
        wait_until(path, || {
            store.snapshot().repos[0]
                .diff_sessions
                .get(&view)
                .is_some_and(|pane| {
                    pane.target == target && !pane.is_loading() && pane.diff.ready().is_some()
                })
        });
        assert_eq!(
            store.snapshot().repos[0].diff_sessions[&view].present_file_side(),
            expected,
            "{path}"
        );
    }
}
