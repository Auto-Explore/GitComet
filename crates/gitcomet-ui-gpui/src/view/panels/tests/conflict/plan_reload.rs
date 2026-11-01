use super::*;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::merge::{MergeBlockId, MergeSource, OrderedSelection};
use gitcomet_state::model::{ConflictFileLoadMode, RepoId};

const REPO: RepoId = RepoId(177);
const PATH: &str = "plan.txt";
const BASE: &str = "base\nshared\nsame\n";
const OURS: &str = "ours\nshared\nlocal\n";
const THEIRS: &str = "theirs\nshared\nsame\n";
const CURRENT: &str = "<<<<<<< ours\nours\n=======\ntheirs\n>>>>>>> theirs\nshared\nlocal\n";

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
    let renderer = cx.update(|_, app| view.read(app).diff_renderer.clone());
    (store, view, renderer, cx)
}

fn click(cx: &mut gpui::VisualTestContext, id: &'static str) {
    draw_and_drain_test_window(cx);
    let bounds = cx.debug_bounds(id).expect("rendered plan choice button");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn block_selection(session: &ConflictSession, id: MergeBlockId) -> OrderedSelection {
    session
        .merge_plan
        .as_ref()
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.id == id)
        .unwrap()
        .selection
        .clone()
}

fn choose_from_toolbar(cx: &mut gpui::TestAppContext, pending: bool, automatic: bool) {
    let (store, view, renderer, cx) = open_resolver(cx);
    let block_id = cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            let index = pane
                .conflict_resolver
                .nav_targets
                .iter()
                .position(|target| target.is_delta && target.original_conflict != automatic)
                .unwrap();
            pane.conflict_jump_to_nav_target(index, cx);
            if automatic {
                assert_eq!(pane.conflict_resolver.active_conflict, None);
            }
            let conflict_resolver::ConflictNavTargetId::PlanBlock(id) =
                pane.conflict_resolver.nav_targets[index].id
            else {
                panic!("plan target");
            };
            id
        })
    });
    let mut loading = store.snapshot().as_ref().clone();
    let loaded_file = loading.repos[0].conflict_state.conflict_file.clone();
    if pending {
        let conflict = &mut loading.repos[0].conflict_state;
        conflict.conflict_file = Loadable::Loading;
        conflict.session_pending_restore = conflict.conflict_session.take();
        conflict.conflict_rev += 1;
        super::super::shortcuts::apply_state(cx, &view, Arc::new(loading));
    }
    click(
        cx,
        if automatic {
            "conflict_pick_theirs"
        } else {
            "conflict_pick_ours"
        },
    );
    if !automatic {
        cx.update(|_, app| {
            assert_eq!(
                renderer.read(app).conflict_resolver_input.read(app).text(),
                OURS
            );
        });
    }
    crate::view::test_support::drain_store_worker(&view, cx);
    let snapshot = store.snapshot();
    let conflict = &snapshot.repos[0].conflict_state;
    let session = if pending {
        conflict.session_pending_restore.as_ref().unwrap()
    } else {
        conflict.conflict_session.as_ref().unwrap()
    };
    let expected = if automatic {
        OrderedSelection::from_sources([MergeSource::B, MergeSource::C])
    } else {
        MergeSource::B.into()
    };
    assert_eq!(block_selection(session, block_id), expected);
    assert_eq!(snapshot.repos[0].feedback.last_error, None);
    let mut ready = snapshot.as_ref().clone();
    let conflict = &mut ready.repos[0].conflict_state;
    conflict.conflict_file = loaded_file;
    if pending {
        conflict.conflict_session = conflict.session_pending_restore.take();
    }
    conflict.conflict_rev += 1;
    super::super::shortcuts::apply_state(cx, &view, Arc::new(ready));
    cx.update(|_, app| {
        let output = renderer.read(app).conflict_resolver_input.read(app).text();
        if automatic {
            assert!(output.ends_with("shared\nlocal\nsame\n"), "{output:?}");
        } else {
            assert_eq!(output, OURS);
        }
    });
}

#[gpui::test]
fn the_plan_toolbar_applies_a_choice_to_a_ready_session(cx: &mut gpui::TestAppContext) {
    choose_from_toolbar(cx, false, false);
}

#[gpui::test]
fn the_plan_toolbar_applies_a_choice_during_reload(cx: &mut gpui::TestAppContext) {
    choose_from_toolbar(cx, true, false);
}

#[gpui::test]
fn the_plan_toolbar_applies_an_automatic_delta_choice_during_reload(cx: &mut gpui::TestAppContext) {
    choose_from_toolbar(cx, true, true);
}

#[gpui::test]
fn a_stale_plan_choice_is_refused_and_reconciled_after_loading_finishes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, view, renderer, cx) = open_resolver(cx);
    let mut loading = store.snapshot().as_ref().clone();
    let conflict = &mut loading.repos[0].conflict_state;
    let Loadable::Ready(Some(mut file)) = conflict.conflict_file.clone() else {
        panic!("loaded fixture");
    };
    let changed_base = "base\nshared\nexternal\n";
    file.base = Some(changed_base.into());
    file.base_bytes = Some(Arc::from(changed_base.as_bytes()));
    let replacement = ConflictSession::from_stage_inputs_with_current(
        PATH.into(),
        gitcomet_core::domain::FileConflictKind::BothModified,
        ConflictPayload::Text(changed_base.into()),
        ConflictPayload::Text(OURS.into()),
        ConflictPayload::Text(THEIRS.into()),
        Some(ConflictPayload::Text(CURRENT.into())),
    );
    let before = replacement.merge_plan.clone();
    conflict.conflict_file = Loadable::Loading;
    conflict.conflict_session = None;
    conflict.session_pending_restore = Some(replacement);
    conflict.conflict_rev += 1;
    store.replace_snapshot_for_test(Arc::new(loading));
    cx.update(|_, app| {
        renderer.update(app, |pane, cx| {
            pane.conflict_resolver_pick_at(0, conflict_resolver::ConflictChoice::Ours, cx);
        });
        assert_eq!(
            renderer.read(app).conflict_resolver_input.read(app).text(),
            OURS
        );
    });
    crate::view::test_support::drain_store_worker(&view, cx);
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.repos[0]
            .conflict_state
            .session_pending_restore
            .as_ref()
            .unwrap()
            .merge_plan,
        before
    );
    assert!(
        snapshot.repos[0]
            .feedback
            .last_error
            .as_ref()
            .unwrap()
            .contains("Review the refreshed conflict")
    );
    let mut ready = snapshot.as_ref().clone();
    let conflict = &mut ready.repos[0].conflict_state;
    conflict.conflict_file = Loadable::Ready(Some(file));
    conflict.conflict_session = conflict.session_pending_restore.take();
    conflict.conflict_rev += 1;
    super::super::shortcuts::apply_state(cx, &view, Arc::new(ready));
    cx.update(|_, app| {
        assert_eq!(
            renderer.read(app).conflict_resolver_input.read(app).text(),
            "<Merge Conflict>\nshared\n<Merge Conflict>\n"
        );
    });
}
