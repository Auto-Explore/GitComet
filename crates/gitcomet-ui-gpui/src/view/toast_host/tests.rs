use super::*;

#[test]
fn errors_stay_until_closed_and_other_toasts_expire() {
    assert_eq!(toast_ttl(components::ToastKind::Error), None);
    assert!(toast_ttl(components::ToastKind::Warning).is_some());
    assert!(toast_ttl(components::ToastKind::Success).is_some());
}

#[gpui::test]
fn expiring_toasts_fade_out_then_leave(cx: &mut gpui::TestAppContext) {
    let host = cx.update(|app| {
        app.new(|cx| {
            let mut host = ToastHost::new(gpui::WeakEntity::new_invalid());
            // Only the live runtime gives toasts a TTL.
            crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                host.push_toast(components::ToastKind::Success, "Saved".into(), cx)
            });
            host
        })
    });
    let leaving = |cx: &mut gpui::TestAppContext| {
        cx.update(|app| {
            host.read(app)
                .toasts
                .iter()
                .map(|toast| toast.leaving)
                .collect::<Vec<_>>()
        })
    };
    let shown = toast_fade_in_duration()
        + toast_ttl(components::ToastKind::Success).expect("success toasts expire");

    cx.executor()
        .advance_clock(shown - Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(leaving(cx), [false], "shown through its fade-in and TTL");
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(leaving(cx), [true], "then fades out");
    cx.executor().advance_clock(toast_fade_out_duration());
    cx.run_until_parked();
    assert!(leaving(cx).is_empty(), "and is removed");
}
use crate::theme::with_alpha;
use gitcomet_state::model::{CloneProgressMeter, CloneProgressStage};
use palette::IntoColor;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

fn clone_op(
    dest: Arc<PathBuf>,
    status: CloneOpStatus,
    stage: CloneProgressStage,
    percent: u8,
    seq: u64,
) -> CloneOpState {
    CloneOpState {
        url: Arc::<str>::from("file:///tmp/repo.git"),
        dest,
        status,
        progress: CloneProgressMeter { stage, percent },
        seq,
        output_tail: VecDeque::new(),
    }
}

fn submodule_add_progress(url: &str, path: &str) -> SubmoduleAddProgressState {
    SubmoduleAddProgressState {
        url: url.to_string(),
        path: PathBuf::from(path),
    }
}

#[test]
fn apply_clone_progress_sync_tracks_running_progress_and_deduplicates_same_seq() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let op = clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Running,
        CloneProgressStage::Loading,
        32,
        7,
    );
    let mut progress = None;
    let mut last_seq = 0;
    let mut tracked_dest = None;

    let first =
        apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, Some(&op));
    assert_eq!(
        first,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: None,
        }
    );
    assert_eq!(progress.as_ref(), Some(&op));
    assert_eq!(last_seq, 7);
    assert_eq!(tracked_dest.as_ref(), Some(&dest));

    let second =
        apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, Some(&op));
    assert_eq!(
        second,
        CloneProgressSyncAction {
            progress_changed: false,
            notice: None,
        }
    );
    assert_eq!(progress.as_ref(), Some(&op));
    assert_eq!(last_seq, 7);
}

#[test]
fn apply_clone_progress_sync_resets_for_restarted_clone_even_at_same_path() {
    let first_dest = Arc::new(PathBuf::from("/tmp/repo"));
    let second_dest = Arc::new(PathBuf::from("/tmp/repo"));
    let first = clone_op(
        Arc::clone(&first_dest),
        CloneOpStatus::Running,
        CloneProgressStage::RemoteObjects,
        91,
        9,
    );
    let restarted = clone_op(
        Arc::clone(&second_dest),
        CloneOpStatus::Running,
        CloneProgressStage::Loading,
        4,
        1,
    );
    let mut progress = Some(first.clone());
    let mut last_seq = first.seq;
    let mut tracked_dest = Some(first_dest);

    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&restarted),
    );

    assert_eq!(
        action,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: None,
        }
    );
    assert_eq!(progress.as_ref(), Some(&restarted));
    assert_eq!(last_seq, 1);
    assert_eq!(tracked_dest.as_ref(), Some(&second_dest));
}

#[test]
fn apply_clone_progress_sync_emits_success_notice_once_and_clears_progress() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let finished = clone_op(
        Arc::clone(&dest),
        CloneOpStatus::FinishedOk,
        CloneProgressStage::RemoteObjects,
        100,
        8,
    );
    let mut progress = Some(clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Running,
        CloneProgressStage::RemoteObjects,
        84,
        7,
    ));
    let mut last_seq = 7;
    let mut tracked_dest = Some(dest);

    let first = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );
    assert_eq!(
        first,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: Some((
                components::ToastKind::Success,
                "Clone finished: /tmp/repo".to_string(),
            )),
        }
    );
    assert!(progress.is_none());
    assert_eq!(last_seq, 8);
    assert!(tracked_dest.is_none());

    let second = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );
    assert_eq!(
        second,
        CloneProgressSyncAction {
            progress_changed: false,
            notice: None,
        }
    );
}

#[test]
fn clone_authentication_wait_clears_progress_without_a_notice_and_allows_fast_retry() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let waiting = clone_op(
        Arc::clone(&dest),
        CloneOpStatus::WaitingForAuthentication,
        CloneProgressStage::Loading,
        0,
        1,
    );
    let mut progress = Some(clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Running,
        CloneProgressStage::Loading,
        0,
        0,
    ));
    let mut last_seq = 0;
    let mut tracked_dest = Some(dest);

    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&waiting),
    );
    assert!(action.progress_changed);
    assert!(action.notice.is_none());
    assert!(progress.is_none());
    assert!(tracked_dest.is_none());
    assert_eq!(last_seq, 0);

    // The retry completes before the UI observes its Running snapshot,
    // and reuses the challenge's sequence number.
    let finished = clone_op(
        Arc::new(PathBuf::from("/tmp/repo")),
        CloneOpStatus::FinishedOk,
        CloneProgressStage::RemoteObjects,
        100,
        waiting.seq,
    );
    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );
    assert_eq!(
        action.notice,
        Some((
            components::ToastKind::Success,
            "Clone finished: /tmp/repo".to_string(),
        ))
    );
    let repeated = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );
    assert!(repeated.notice.is_none());
}

#[test]
fn clone_retry_completion_is_reported_when_waiting_and_running_snapshots_are_skipped() {
    for status in [
        CloneOpStatus::FinishedOk,
        CloneOpStatus::FinishedErr("Clone failed: network disconnected".into()),
    ] {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let mut progress = Some(clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Running,
            CloneProgressStage::Loading,
            0,
            1,
        ));
        let mut last_seq = 1;
        let mut tracked_dest = Some(dest);
        let expected_kind = if matches!(&status, CloneOpStatus::FinishedOk) {
            components::ToastKind::Success
        } else {
            components::ToastKind::Error
        };
        let finished = clone_op(
            Arc::new(PathBuf::from("/tmp/repo")),
            status,
            CloneProgressStage::Loading,
            0,
            1,
        );

        let action = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );
        assert!(action.progress_changed);
        assert_eq!(action.notice.map(|(kind, _)| kind), Some(expected_kind));
        assert!(progress.is_none());
        let repeated = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );
        assert!(repeated.notice.is_none());
    }
}

#[test]
fn cancelling_clone_authentication_is_quiet_but_terminal_retry_failure_is_reported() {
    let waiting = clone_op(
        Arc::new(PathBuf::from("/tmp/repo")),
        CloneOpStatus::WaitingForAuthentication,
        CloneProgressStage::Loading,
        0,
        1,
    );
    let mut progress = None;
    let mut last_seq = 0;
    let mut tracked_dest = None;

    for next in [Some(&waiting), Some(&waiting), None] {
        let action =
            apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, next);
        assert!(action.notice.is_none());
        assert!(progress.is_none());
    }

    let failed = clone_op(
        Arc::new(PathBuf::from("/tmp/repo")),
        CloneOpStatus::FinishedErr("Clone failed: network disconnected".into()),
        CloneProgressStage::Loading,
        0,
        1,
    );
    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&failed),
    );
    assert_eq!(
        action.notice,
        Some((
            components::ToastKind::Error,
            "Clone failed: network disconnected".into(),
        ))
    );
}

#[test]
fn apply_clone_progress_sync_emits_cancel_notice_and_clears_progress() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let finished = clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Cancelled,
        CloneProgressStage::Loading,
        12,
        5,
    );
    let mut progress = Some(clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Cancelling,
        CloneProgressStage::Loading,
        12,
        4,
    ));
    let mut last_seq = 4;
    let mut tracked_dest = Some(dest);

    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );

    assert_eq!(
        action,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: Some((
                components::ToastKind::Warning,
                "Clone aborted: /tmp/repo".to_string(),
            )),
        }
    );
    assert!(progress.is_none());
    assert_eq!(last_seq, 5);
    assert!(tracked_dest.is_none());
}

#[test]
fn apply_clone_progress_sync_emits_error_notice_and_clears_progress() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let finished = clone_op(
        Arc::clone(&dest),
        CloneOpStatus::FinishedErr("Clone failed: permission denied".to_string()),
        CloneProgressStage::RemoteObjects,
        57,
        11,
    );
    let mut progress = Some(clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Running,
        CloneProgressStage::RemoteObjects,
        57,
        10,
    ));
    let mut last_seq = 10;
    let mut tracked_dest = Some(dest);

    let action = apply_clone_progress_sync(
        &mut progress,
        &mut last_seq,
        &mut tracked_dest,
        Some(&finished),
    );

    assert_eq!(
        action,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: Some((
                components::ToastKind::Error,
                "Clone failed: permission denied".to_string(),
            )),
        }
    );
    assert!(progress.is_none());
    assert_eq!(last_seq, 11);
    assert!(tracked_dest.is_none());
}

#[test]
fn apply_clone_progress_sync_clears_progress_when_clone_disappears() {
    let dest = Arc::new(PathBuf::from("/tmp/repo"));
    let mut progress = Some(clone_op(
        Arc::clone(&dest),
        CloneOpStatus::Running,
        CloneProgressStage::Loading,
        43,
        3,
    ));
    let mut last_seq = 3;
    let mut tracked_dest = Some(dest);

    let action = apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, None);

    assert_eq!(
        action,
        CloneProgressSyncAction {
            progress_changed: true,
            notice: None,
        }
    );
    assert!(progress.is_none());
    assert_eq!(last_seq, 0);
    assert!(tracked_dest.is_none());
}

#[test]
fn apply_submodule_add_progress_sync_deduplicates_equal_snapshots() {
    let mut progress = vec![submodule_add_progress(
        "https://example.com/sub.git",
        "mods/sub",
    )];

    assert!(!apply_submodule_add_progress_sync(
        &mut progress,
        &[submodule_add_progress(
            "https://example.com/sub.git",
            "mods/sub",
        )],
    ));
    assert_eq!(
        progress,
        vec![submodule_add_progress(
            "https://example.com/sub.git",
            "mods/sub"
        )]
    );
}

#[test]
fn apply_submodule_add_progress_sync_replaces_and_clears_entries() {
    let mut progress = vec![submodule_add_progress(
        "https://example.com/one.git",
        "mods/one",
    )];

    assert!(apply_submodule_add_progress_sync(
        &mut progress,
        &[
            submodule_add_progress("https://example.com/two.git", "mods/two"),
            submodule_add_progress("https://example.com/three.git", "mods/three"),
        ],
    ));
    assert_eq!(
        progress,
        vec![
            submodule_add_progress("https://example.com/two.git", "mods/two"),
            submodule_add_progress("https://example.com/three.git", "mods/three"),
        ]
    );

    assert!(apply_submodule_add_progress_sync(&mut progress, &[]));
    assert!(progress.is_empty());
}

#[test]
fn submodule_progress_masks_url_credentials() {
    let label = submodule_add_progress_url_label("https://alice:secret@example.com/org/repo.git");
    assert_eq!(label, "https://alice:***@example.com/org/repo.git");
    assert!(!label.contains("secret"));
}

#[gpui::test]
fn toast_inputs_follow_their_window_theme(cx: &mut gpui::TestAppContext) {
    let light = AppTheme::gitcomet_light();
    let dark = AppTheme::gitcomet_dark();

    let (host, cx) = cx.add_window_view(|_, cx| {
        let mut host = ToastHost::new(gpui::WeakEntity::new_invalid());
        host.push_survey_toast(
            "survey-id",
            "Survey",
            "Help shape GitComet by taking a short user survey.",
            "https://example.com",
            "Open Survey",
            "Later",
            60,
            cx,
        );
        host
    });
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    let toast_input = cx.update(|_, app| {
        let host = host.read(app);
        assert_eq!(host.toasts.len(), 1);
        let ToastBody::Text { input, .. } = &host.toasts[0].body else {
            panic!("a survey toast is text");
        };
        input.clone()
    });

    for theme in [light, dark] {
        cx.update(|_, app| crate::theme::set_window_theme(app, window_id, theme));
        crate::test_support::refresh_and_draw(cx);
        cx.update(|_, app| {
            assert_eq!(
                toast_input.read(app).debug_text_color(),
                theme.colors.foreground.primary.into_color()
            );
        });
    }
}

#[test]
fn clone_progress_shell_uses_a_subtle_accent_border() {
    let dark = AppTheme::gitcomet_dark();
    let light = AppTheme::gitcomet_light();

    assert_eq!(
        clone_progress_shell_border_color(dark),
        with_alpha(dark.colors.accent.foreground, 0.36)
    );
    assert_eq!(
        clone_progress_shell_border_color(light),
        with_alpha(light.colors.accent.foreground, 0.28)
    );
}

#[test]
fn toast_stack_anchor_is_bottom_left() {
    assert_eq!(toast_viewport_corner(), ToastViewportCorner::BottomLeft);
}
