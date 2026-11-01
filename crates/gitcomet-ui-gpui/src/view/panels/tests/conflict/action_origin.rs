use super::*;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictRegionResolution, ConflictSession};
use gitcomet_core::merge::MergeSource;
use gitcomet_state::model::{ConflictFileLoadMode, RepoId};

const REPO: RepoId = RepoId(176);
const PATH: &str = "origin.txt";
const BASE: &str = "base one\nmiddle\nbase two\n";
const OURS: &str = "ours one\nmiddle\nours two\n";
const THEIRS: &str = "theirs one\nmiddle\ntheirs two\n";
const CURRENT: &str = "<<<<<<< ours\nours one\n=======\ntheirs one\n>>>>>>> theirs\nmiddle\n<<<<<<< ours\nours two\n=======\ntheirs two\n>>>>>>> theirs\n";

fn open_resolver(
    cx: &mut gpui::TestAppContext,
) -> (
    AppStore,
    Entity<GitCometView>,
    Entity<DiffRenderer>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    crate::view::test_support::drain_store_worker(&view, cx);
    let mut repo = opening_repo_state(REPO, &std::env::temp_dir());
    set_test_conflict_status(&mut repo, PATH, gitcomet_core::domain::DiffArea::Unstaged);
    set_test_conflict_file(&mut repo, PATH, BASE, OURS, THEIRS, CURRENT);
    repo.conflict_state.conflict_file_load_mode = ConflictFileLoadMode::Full;
    repo.conflict_state.conflict_session = Some(ConflictSession::from_stage_inputs_with_current(
        PATH.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(BASE.into()),
        ConflictPayload::Text(OURS.into()),
        ConflictPayload::Text(THEIRS.into()),
        Some(ConflictPayload::Text(CURRENT.into())),
    ));
    super::super::shortcuts::apply_state(cx, &view, app_state_with_repo(repo, REPO));
    draw_and_drain_test_window(cx);
    let renderer = cx.update(|_, app| {
        let renderer = view.read(app).diff_renderer.clone();
        assert_eq!(
            renderer
                .read(app)
                .conflict_resolver
                .conflict_region_indices
                .len(),
            2
        );
        assert!(renderer.read(app).conflict_resolver.region_origin.is_some());
        renderer
    });
    (store, view, renderer, cx)
}

#[gpui::test]
fn a_pick_from_the_provisional_pane_keeps_its_side_after_the_full_load(
    cx: &mut gpui::TestAppContext,
) {
    let (store, view, renderer, cx) = open_resolver(cx);
    let mut full = store.snapshot().as_ref().clone();
    let mut provisional = full.clone();
    let conflict = &mut provisional.repos[0].conflict_state;
    let Loadable::Ready(Some(file)) = &mut conflict.conflict_file else {
        panic!("loaded fixture");
    };
    file.base = None;
    file.ours = None;
    file.theirs = None;
    conflict.conflict_file_load_mode = ConflictFileLoadMode::CurrentOnly;
    conflict.conflict_session = Some(ConflictSession::from_merged_text(
        PATH.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        CURRENT,
    ));
    conflict.conflict_rev += 1;
    full.repos[0].conflict_state.conflict_rev = conflict.conflict_rev + 1;
    super::super::shortcuts::apply_state(cx, &view, Arc::new(provisional));
    cx.update(|_, app| {
        assert!(
            renderer
                .read(app)
                .conflict_resolver
                .loaded_file
                .as_ref()
                .unwrap()
                .base
                .is_none()
        );
        store.replace_snapshot_for_test(Arc::new(full));
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(0, conflict_resolver::ConflictChoice::Ours, cx);
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[0]
            .resolution,
        ConflictRegionResolution::Sources(MergeSource::B.into())
    );
    super::super::shortcuts::apply_state(cx, &view, snapshot);
    cx.update(|_, app| {
        let pane = renderer.read(app);
        assert!(
            pane.conflict_resolver_input
                .read(app)
                .text()
                .starts_with("ours one\n"),
            "{}",
            pane.conflict_resolver_input.read(app).text()
        );
    });
}

#[gpui::test]
fn preparing_output_for_save_does_not_resend_an_untouched_regions_stale_decision(
    cx: &mut gpui::TestAppContext,
) {
    let (store, view, renderer, cx) = open_resolver(cx);
    let mut newer = store.snapshot().as_ref().clone();
    let session = newer.repos[0]
        .conflict_state
        .conflict_session
        .as_mut()
        .unwrap();
    assert!(session.replace_region_selection(1, MergeSource::C.into()));
    newer.repos[0].conflict_state.conflict_rev += 1;
    cx.update(|_, app| {
        store.replace_snapshot_for_test(Arc::new(newer));
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(0, conflict_resolver::ConflictChoice::Ours, cx);
            let text = pane.current_conflict_resolved_output_text(cx);
            assert_eq!(
                pane.conflict_resolver_save_contents_from_text(text.clone()),
                text
            );
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    let snapshot = store.snapshot();
    let session = snapshot.repos[0]
        .conflict_state
        .conflict_session
        .as_ref()
        .unwrap();
    assert_eq!(
        session.regions[0].resolution,
        ConflictRegionResolution::Sources(MergeSource::B.into())
    );
    assert_eq!(
        session.regions[1].resolution,
        ConflictRegionResolution::Sources(MergeSource::C.into())
    );
}

#[gpui::test]
fn mixed_local_chunks_survive_an_unrelated_store_revision(cx: &mut gpui::TestAppContext) {
    let (store, view, renderer, cx) = open_resolver(cx);
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(1, conflict_resolver::ConflictChoice::Ours, cx);
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            let output = pane.current_conflict_resolved_output_text(cx);
            let range = &pane.conflict_resolved_output_block_map.ranges()[1];
            let line = output[..range.start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            pane.conflict_resolver_apply_pick_target(
                ResolverPickTarget::Chunk {
                    conflict_ix: 1,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix: Some(line),
                },
                cx,
            );
            assert_eq!(
                pane.conflict_resolver.conflict_region_indices,
                vec![0, 1, 1]
            );
        });
    });
    let mut newer = store.snapshot().as_ref().clone();
    assert!(
        newer.repos[0]
            .conflict_state
            .conflict_session
            .as_mut()
            .unwrap()
            .replace_region_selection(1, MergeSource::C.into())
    );
    newer.repos[0].conflict_state.conflict_rev += 1;
    let expected = cx.update(|_, app| {
        store.replace_snapshot_for_test(Arc::new(newer));
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(0, conflict_resolver::ConflictChoice::Ours, cx);
            let text = pane.current_conflict_resolved_output_text(cx);
            assert!(text.contains("ours two\n"));
            assert!(text.contains("theirs two\n"));
            pane.conflict_resolver_save_contents_from_text(text)
        })
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.repos[0]
            .conflict_state
            .conflict_session
            .as_ref()
            .unwrap()
            .regions[1]
            .resolution,
        ConflictRegionResolution::Sources(MergeSource::C.into())
    );
    super::super::shortcuts::apply_state(cx, &view, snapshot);
    cx.update(|_, app| {
        assert_eq!(
            renderer.read(app).conflict_resolver_input.read(app).text(),
            expected
        );
    });
}

#[gpui::test]
fn removing_a_local_chunk_resumes_the_deferred_full_snapshot(cx: &mut gpui::TestAppContext) {
    let (store, view, renderer, cx) = open_resolver(cx);
    let mut full = store.snapshot().as_ref().clone();
    let mut provisional = full.clone();
    let conflict = &mut provisional.repos[0].conflict_state;
    let Loadable::Ready(Some(file)) = &mut conflict.conflict_file else {
        panic!("loaded fixture");
    };
    file.base = None;
    file.ours = None;
    file.theirs = None;
    conflict.conflict_file_load_mode = ConflictFileLoadMode::CurrentOnly;
    conflict.conflict_session = Some(ConflictSession::from_merged_text(
        PATH.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        ConflictPayload::Absent,
        CURRENT,
    ));
    conflict.conflict_rev += 1;
    full.repos[0].conflict_state.conflict_rev = conflict.conflict_rev + 2;
    assert!(
        full.repos[0]
            .conflict_state
            .conflict_session
            .as_mut()
            .unwrap()
            .replace_region_selection(1, MergeSource::B.into())
    );
    super::super::shortcuts::apply_state(cx, &view, Arc::new(provisional));
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(1, conflict_resolver::ConflictChoice::Ours, cx);
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            let output = pane.current_conflict_resolved_output_text(cx);
            let line = output[..pane.conflict_resolved_output_block_map.ranges()[1].start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            pane.conflict_resolver_apply_pick_target(
                ResolverPickTarget::Chunk {
                    conflict_ix: 1,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix: Some(line),
                },
                cx,
            );
            assert_eq!(
                pane.conflict_resolver.conflict_region_indices,
                vec![0, 1, 1]
            );
        });
    });
    super::super::shortcuts::apply_state(cx, &view, Arc::new(full));
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            assert!(
                pane.conflict_resolver
                    .loaded_file
                    .as_ref()
                    .unwrap()
                    .base
                    .is_none()
            );
            assert!(pane.conflict_resolver.refresh_deferred);
            pane.conflict_resolver_apply_pick_target(
                ResolverPickTarget::Chunk {
                    conflict_ix: 1,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix: None,
                },
                cx,
            );
            assert!(
                pane.conflict_resolver
                    .loaded_file
                    .as_ref()
                    .unwrap()
                    .base
                    .is_some()
            );
            assert!(!pane.conflict_resolver.refresh_deferred);
            assert_eq!(pane.conflict_resolver.conflict_region_indices, vec![0, 1]);
            assert!(
                pane.conflict_resolver_input
                    .read(cx)
                    .text()
                    .ends_with("ours two\n")
            );
        });
    });
}

#[gpui::test]
fn manual_edits_in_duplicate_chunks_survive_a_full_rebuild(cx: &mut gpui::TestAppContext) {
    let (store, view, renderer, cx) = open_resolver(cx);
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(1, conflict_resolver::ConflictChoice::Ours, cx);
        });
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            let output = pane.current_conflict_resolved_output_text(cx);
            let line = output[..pane.conflict_resolved_output_block_map.ranges()[1].start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count();
            pane.conflict_resolver_apply_pick_target(
                ResolverPickTarget::Chunk {
                    conflict_ix: 1,
                    choice: conflict_resolver::ConflictChoice::Theirs,
                    output_line_ix: Some(line),
                },
                cx,
            );
            assert_eq!(
                pane.conflict_resolver.conflict_region_indices,
                vec![0, 1, 1]
            );
            pane.conflict_resolver_pick_at(1, conflict_resolver::ConflictChoice::empty(), cx);
            pane.conflict_resolver_pick_at(1, conflict_resolver::ConflictChoice::Theirs, cx);
            assert!(
                pane.conflict_resolver
                    .marker_segments
                    .iter()
                    .filter_map(|segment| match segment {
                        conflict_resolver::ConflictSegment::Block(block) => Some(block),
                        conflict_resolver::ConflictSegment::Text(_) => None,
                    })
                    .skip(1)
                    .all(|block| block.resolved
                        && block.choice == conflict_resolver::ConflictChoice::Theirs)
            );
            let ranges = pane.conflict_resolved_output_block_map.ranges().to_vec();
            pane.conflict_resolver_input.update(cx, |input, cx| {
                input.replace_utf8_range(ranges[2].clone(), "manual second chunk\n", cx);
                input.replace_utf8_range(ranges[1].clone(), "manual first chunk\n", cx);
            });
        });
    });
    cx.run_until_parked();
    let expected = cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            let text = pane.current_conflict_resolved_output_text(cx);
            assert!(text.ends_with("manual first chunk\nmanual second chunk\n"));
            pane.conflict_resolver_save_contents_from_text(text)
        })
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    let mut reloaded = store.snapshot().as_ref().clone();
    let conflict = &mut reloaded.repos[0].conflict_state;
    let Loadable::Ready(Some(file)) = &mut conflict.conflict_file else {
        panic!("loaded fixture");
    };
    file.current = Some("external worktree text\n".into());
    conflict.conflict_session.as_mut().unwrap().current =
        Some(ConflictPayload::Text("external worktree text\n".into()));
    conflict.conflict_rev += 1;
    super::super::shortcuts::apply_state(cx, &view, Arc::new(reloaded));
    cx.update(|_, app| {
        assert_eq!(
            renderer.read(app).conflict_resolver_input.read(app).text(),
            expected
        );
    });
}
