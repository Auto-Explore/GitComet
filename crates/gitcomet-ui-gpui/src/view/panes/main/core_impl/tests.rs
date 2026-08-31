#[allow(unused_imports)]
use super::*;

#[test]
fn notify_fingerprint_tracks_cherry_pick_message_readiness() {
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::services::{InteractiveRebaseAction, InteractiveRebaseEntry};
    use gitcomet_state::model::{InteractiveCherryPickSetup, RepoState};
    use std::path::PathBuf;

    let mut state = AppState::default();
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

#[test]
fn should_request_blame_retries_failure_only_when_forced() {
    use gitcomet_state::model::Loadable;
    // A new/changed target always loads, regardless of state or force.
    assert!(should_request_blame(
        false,
        &Loadable::<()>::Ready(()),
        false
    ));
    assert!(should_request_blame(
        false,
        &Loadable::<()>::Error("x".into()),
        false
    ));
    // Same target, healthy or in flight: never reload (even when forced), so a
    // toggle-on doesn't re-blame an already-loaded file.
    assert!(!should_request_blame(
        true,
        &Loadable::<()>::Ready(()),
        true
    ));
    assert!(!should_request_blame(true, &Loadable::<()>::Loading, true));
    // Same target, not yet loaded: load.
    assert!(should_request_blame(
        true,
        &Loadable::<()>::NotLoaded,
        false
    ));
    // Same target, failed: never retry from the per-frame Render path
    // (force=false), but retry on an explicit toggle (force=true).
    assert!(!should_request_blame(
        true,
        &Loadable::<()>::Error("e".into()),
        false
    ));
    assert!(should_request_blame(
        true,
        &Loadable::<()>::Error("e".into()),
        true
    ));
}

#[test]
fn clamp_raw_scroll_y_uses_gpui_negative_offset_range() {
    assert_eq!(clamp_raw_scroll_y(px(-180.0), px(120.0)), px(-120.0));
    assert_eq!(clamp_raw_scroll_y(px(180.0), px(120.0)), px(0.0));
    assert_eq!(clamp_raw_scroll_y(px(-40.0), px(120.0)), px(-40.0));
}

#[test]
fn synced_scroll_offsets_keep_longer_pane_as_master_after_shorter_clamps() {
    let targets = compute_synced_scroll_offsets(
        [px(-100.0), px(-500.0)],
        [px(100.0), px(500.0)],
        [px(-90.0), px(-90.0)],
        1,
    );

    assert_eq!(targets, [px(-100.0), px(-500.0)]);
}

#[test]
fn synced_scroll_offsets_follow_shorter_pane_when_user_scrolled_it() {
    let targets = compute_synced_scroll_offsets(
        [px(-100.0), px(-320.0)],
        [px(100.0), px(500.0)],
        [px(-80.0), px(-320.0)],
        1,
    );

    assert_eq!(targets, [px(-100.0), px(-100.0)]);
}

#[test]
fn synced_scroll_offsets_support_four_panes_when_output_is_scrolled() {
    let targets = compute_synced_scroll_offsets(
        [px(-100.0), px(-100.0), px(-100.0), px(-320.0)],
        [px(100.0), px(100.0), px(100.0), px(500.0)],
        [px(-100.0), px(-100.0), px(-100.0), px(-80.0)],
        3,
    );

    assert_eq!(targets, [px(-100.0), px(-100.0), px(-100.0), px(-320.0)]);
}

#[test]
fn synced_scroll_offsets_hold_steady_when_nothing_changed() {
    // A clamped follower (shorter pane, offset -100) sits alongside a master
    // scrolled further (-320). Nothing moved since the last sync (offsets ==
    // last_synced), so even though the offsets are unequal the follower must
    // stay put — re-driving it onto the widest handle here is the idle-frame
    // snap-back the horizontal output sync used to produce.
    let steady = [px(-100.0), px(-320.0)];
    let targets = compute_synced_scroll_offsets(steady, [px(100.0), px(500.0)], steady, 1);

    assert_eq!(targets, steady);
}

#[test]
fn synced_scroll_offsets_do_not_promote_a_follower_clamped_during_paint() {
    let steady = [px(-100.0), px(-500.0)];
    let targets = compute_synced_scroll_offsets(
        steady,
        [px(100.0), px(500.0)],
        // The previous render requested -120 for the shorter follower;
        // GPUI painted it at its current -100 maximum afterward.
        [px(-120.0), px(-500.0)],
        1,
    );

    assert_eq!(targets, steady);
}

#[test]
fn explicit_wheel_master_wins_when_multiple_handles_changed() {
    let targets = compute_synced_scroll_offsets_with_master(
        [px(0.0), px(-100.0)],
        [px(500.0), px(500.0)],
        [px(-100.0), px(0.0)],
        0,
        Some(1),
    );

    assert_eq!(targets, [px(-100.0), px(-100.0)]);
}

#[test]
fn explicit_wheel_master_at_top_pulls_stale_follower_to_top() {
    let targets = compute_synced_scroll_offsets_with_master(
        [px(0.0), px(-100.0)],
        [px(500.0), px(500.0)],
        [px(0.0), px(-100.0)],
        1,
        Some(0),
    );

    assert_eq!(targets, [px(0.0), px(0.0)]);
}

#[test]
fn revealed_whitespace_wrap_ranges_follow_rendered_tab_markers() {
    let hidden = diff_wrap_byte_ranges_for_text("a    b", Some("a\tb"), 4, false)
        .into_iter()
        .map(rows::DiffWrapByteRange::range)
        .collect::<Vec<_>>();
    assert_eq!(hidden, vec![0..4, 4..6]);

    let revealed = diff_wrap_byte_ranges_for_text("a    b", Some("a\tb"), 4, true)
        .into_iter()
        .map(rows::DiffWrapByteRange::range)
        .collect::<Vec<_>>();
    assert_eq!(revealed, vec![0..6]);
}
