use super::*;
use gitcomet_core::domain::{CommitId, RepoSpec};
use gitcomet_state::model::{AppState, AutoFetchStatus, Loadable, RepoState};

fn open_repository() -> AppState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: std::path::PathBuf::from("/tmp/gitcomet-menu-models"),
        },
    );
    repo.open = Loadable::Ready(());
    AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        ..AppState::test_default()
    }
}

fn failed_auto_fetch(state: &mut AppState, message: &str) {
    state.remote_settings.auto_fetch.enabled = true;
    state.repos[0].auto_fetch.status = AutoFetchStatus::Failed {
        at: SystemTime::UNIX_EPOCH,
        error: Error::new(ErrorKind::Backend(message.to_string())),
    };
}

/// An open menu repaints when its model changes, so what a menu must follow
/// is pinned on its model: a pending lease in Push, and the text of the last
/// automatic fetch failure in Pull.
#[gpui::test]
fn picker_models_follow_the_state_they_show(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut model_changes = |kind: PopoverKind, before: AppState, after: AppState| {
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.popover_host.update(cx, |host, cx| {
                    host.state = Arc::new(before);
                    let shown = host.context_menu_model(&kind, cx);
                    host.state = Arc::new(after);
                    shown != host.context_menu_model(&kind, cx)
                })
            })
        })
    };

    let mut leased = open_repository();
    leased.repos[0].pending.force_push_lease = Some(gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: CommitId("1111111111111111111111111111111111111111".into()),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    });
    assert!(model_changes(
        PopoverKind::PushPicker,
        open_repository(),
        leased
    ));

    let (mut refused, mut timed_out) = (open_repository(), open_repository());
    failed_auto_fetch(&mut refused, "connection refused");
    failed_auto_fetch(&mut timed_out, "timed out");
    assert!(model_changes(
        PopoverKind::PullPicker,
        refused.clone(),
        timed_out
    ));
    assert!(!model_changes(
        PopoverKind::PullPicker,
        refused.clone(),
        refused
    ));
}
