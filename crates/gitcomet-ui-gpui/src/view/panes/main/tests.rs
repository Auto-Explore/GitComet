use super::*;

#[test]
fn notify_fingerprint_tracks_cherry_pick_message_readiness() {
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
    use gitcomet_state::model::{InteractiveCherryPickSetup, RepoState};
    use std::path::PathBuf;

    let mut state = AppState::test_default();
    state.active_repo = Some(RepoId(1));
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let without_setup = MainPaneView::notify_fingerprint_for(&state);

    state.repos[0].interactive_cherry_pick_setup = Some(InteractiveCherryPickSetup {
        entries: vec![InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Pick,
            commit_id: "1111111111111111111111111111111111111111".to_string(),
            summary: "subject".to_string(),
            message: "subject".to_string(),
            new_message: None,
        }],
        source_colors: vec![],
        full_messages: Loadable::Loading,
    });
    let loading = MainPaneView::notify_fingerprint_for(&state);
    assert_ne!(loading, without_setup);

    state.repos[0]
        .interactive_cherry_pick_setup
        .as_mut()
        .expect("setup")
        .full_messages = Loadable::Ready(());
    let ready = MainPaneView::notify_fingerprint_for(&state);
    assert_ne!(ready, loading);
}
