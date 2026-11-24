use super::*;

#[gpui::test]
fn pr530_focus_changes_persist_only_when_flushed(cx: &mut gpui::TestAppContext) {
    let first = cx.add_window(|_, _| gpui::Empty);
    let second = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));
    for window in [first, second] {
        cx.update(|cx| {
            sync_window(
                cx,
                window.window_id(),
                None,
                vec![PathBuf::from(format!("/repo/{:?}", window.window_id()))],
                None,
            )
        });
    }
    let writes = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&writes);
    cx.update(|cx| {
        // Synchronous test sink makes every queued write observable.
        cx.global::<WorkspaceManager>().state.borrow_mut().writer =
            Some(Arc::new(WorkspaceWriter {
                pending: Mutex::new(None),
                wake: Condvar::new(),
                writing: Mutex::new(()),
                worker_running: AtomicBool::new(false),
                sink: Box::new(move |workspaces| lock(&recorded).push(workspaces.to_vec())),
            }));
        for window in [first, second, first, second] {
            mark_window_active(cx, window.window_id());
        }
    });
    assert!(
        lock(&writes).is_empty(),
        "focus alone must not rewrite session.json"
    );
    cx.update(flush_to_disk);
    let writes = lock(&writes);
    assert_eq!(writes.len(), 1);
    let latest = writes[0]
        .iter()
        .max_by_key(|w| w.last_activation_order)
        .unwrap();
    assert_eq!(Some(latest.id), cx.update(|cx| active_workspace_id(cx)));
}

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

#[gpui::test]
fn customized_workspace_survives_losing_its_last_repository(cx: &mut gpui::TestAppContext) {
    let window = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));
    let id = cx.update(|cx| {
        sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
            .expect("durable workspace")
    });
    cx.update(|cx| assert!(set_workspace_name(cx, id, "  Client work ")));

    let kept = cx.update(|cx| sync_window(cx, window.window_id(), Some(id), Vec::new(), None));

    assert_eq!(kept, Some(id), "the window keeps its customized workspace");
    let workspace = cx
        .update(|cx| workspace_for_window(cx, window.window_id()))
        .expect("still mapped to the window");
    assert_eq!(workspace.custom_name.as_deref(), Some("Client work"));
    assert!(workspace.repositories.is_empty());
    assert!(workspace.restore_on_launch);
}

#[gpui::test]
fn clearing_the_last_customization_of_an_empty_workspace_removes_it(cx: &mut gpui::TestAppContext) {
    let window = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));
    let id = cx.update(|cx| {
        sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
            .expect("durable workspace")
    });
    cx.update(|cx| {
        assert!(set_workspace_theme_mode(cx, id, Some("tokyo_night".into())));
        assert!(
            !set_workspace_theme_mode(cx, id, Some("tokyo_night".into())),
            "an unchanged value reports no change"
        );
    });
    cx.update(|cx| sync_window(cx, window.window_id(), Some(id), Vec::new(), None));
    assert!(cx.update(|cx| workspace(cx, id)).is_some());

    cx.update(|cx| assert!(set_workspace_theme_mode(cx, id, None)));

    assert!(cx.update(|cx| workspace(cx, id)).is_none());
    assert!(
        cx.update(|cx| workspace_for_window(cx, window.window_id()))
            .is_none()
    );
}

#[gpui::test]
fn focusing_a_window_stamps_its_workspace_open_time(cx: &mut gpui::TestAppContext) {
    let window = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));
    let id = cx.update(|cx| {
        sync_window(cx, window.window_id(), None, vec![path("/repos/a")], None)
            .expect("durable workspace")
    });
    assert_eq!(
        cx.update(|cx| workspace(cx, id)).unwrap().last_opened_at,
        None
    );

    cx.update(|cx| mark_window_active(cx, window.window_id()));

    assert!(
        cx.update(|cx| workspace(cx, id))
            .unwrap()
            .last_opened_at
            .is_some()
    );
}

#[gpui::test]
fn reads_do_not_notify_global_observers(cx: &mut gpui::TestAppContext) {
    use std::cell::Cell;
    use std::rc::Rc;

    let window = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));
    let workspace_id = cx.update(|cx| {
        sync_window(
            cx,
            window.window_id(),
            None,
            vec![path("/repos/a")],
            Some(path("/repos/a")),
        )
        .expect("durable group")
    });
    cx.run_until_parked();

    let notifications = Rc::new(Cell::new(0usize));
    let counter = Rc::clone(&notifications);
    let _subscription = cx.update(|cx| {
        cx.observe_global::<WorkspaceManager>(move |_cx| {
            counter.set(counter.get() + 1);
        })
    });

    cx.update(|cx| {
        for _ in 0..8 {
            let _ = workspaces(cx);
            let _ = workspace(cx, workspace_id);
            let _ = workspace_for_window(cx, window.window_id());
        }
    });
    cx.run_until_parked();
    assert_eq!(notifications.get(), 0, "reads must not lease the global");

    cx.update(|cx| {
        assert!(set_workspace_color(
            cx,
            workspace_id,
            Some(session::WorkspaceColor::Blue)
        ));
    });
    cx.run_until_parked();
    assert_eq!(
        notifications.get(),
        1,
        "a mutation still notifies observers"
    );
}

#[gpui::test]
fn live_windows_keep_independent_repository_workspaces(cx: &mut gpui::TestAppContext) {
    let first = cx.add_window(|_, _| gpui::Empty);
    let second = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));

    let first_id = cx.update(|cx| {
        sync_window(
            cx,
            first.window_id(),
            None,
            vec![path("/repos/a"), path("/repos/b")],
            Some(path("/repos/b")),
        )
        .expect("first durable group")
    });
    let second_id = cx.update(|cx| {
        sync_window(
            cx,
            second.window_id(),
            None,
            vec![path("/repos/f")],
            Some(path("/repos/f")),
        )
        .expect("second durable group")
    });

    assert_ne!(first_id, second_id);
    let workspaces = cx.update(|cx| workspaces(cx));
    assert_eq!(workspaces.len(), 2);
    assert_eq!(
        workspaces
            .iter()
            .find(|workspace| workspace.id == first_id)
            .expect("first group")
            .repositories,
        vec![path("/repos/a"), path("/repos/b")]
    );
    assert_eq!(
        workspaces
            .iter()
            .find(|workspace| workspace.id == second_id)
            .expect("second group")
            .repositories,
        vec![path("/repos/f")]
    );
}

#[gpui::test]
fn close_hides_a_workspace_from_launch_but_keeps_it_recoverable(cx: &mut gpui::TestAppContext) {
    let window = cx.add_window(|_, _| gpui::Empty);
    let mut saved = Workspace::new(vec![path("/repos/a")]);
    saved.id = WorkspaceId::from_u128(1);
    let saved_id = saved.id;
    cx.update(|cx| initialize_for_test(cx, vec![saved]));
    let placement = PortableWindowPlacement {
        normal_frame: Some(SavedWindowFrame {
            x: 40,
            y: 60,
            width: 1200,
            height: 800,
        }),
        ..Default::default()
    };

    cx.update(|cx| {
        let id = sync_window(
            cx,
            window.window_id(),
            Some(saved_id),
            vec![path("/repos/a")],
            Some(path("/repos/a")),
        );
        assert_eq!(id, Some(saved_id));
        record_window_placement(cx, window.window_id(), placement.clone());
        mark_window_closed(cx, window.window_id());
    });

    let saved = cx
        .update(|cx| workspace(cx, saved_id))
        .expect("recoverable group");
    assert!(!saved.restore_on_launch);
    assert_eq!(saved.repositories, vec![path("/repos/a")]);
    assert_eq!(saved.placement, placement);
}

#[gpui::test]
fn workspace_color_can_be_set_and_restored_to_the_theme_default(cx: &mut gpui::TestAppContext) {
    let mut saved = Workspace::new(vec![path("/repos/a")]);
    saved.id = WorkspaceId::from_u128(1);
    let saved_id = saved.id;
    cx.update(|cx| initialize_for_test(cx, vec![saved]));

    assert!(
        cx.update(|cx| { set_workspace_color(cx, saved_id, Some(session::WorkspaceColor::Blue)) })
    );
    assert_eq!(
        cx.update(|cx| workspace(cx, saved_id).and_then(|workspace| workspace.color)),
        Some(session::WorkspaceColor::Blue)
    );
    assert!(cx.update(|cx| set_workspace_color(cx, saved_id, None)));
    assert_eq!(
        cx.update(|cx| workspace(cx, saved_id).and_then(|workspace| workspace.color)),
        None
    );
    assert!(
        !cx.update(|cx| set_workspace_color(cx, saved_id, None)),
        "selecting the current color should be a no-op"
    );
}

#[gpui::test]
fn empty_ephemeral_window_is_not_saved_and_empty_durable_window_is_deleted(
    cx: &mut gpui::TestAppContext,
) {
    let window = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));

    assert_eq!(
        cx.update(|cx| sync_window(cx, window.window_id(), None, Vec::new(), None)),
        None
    );
    assert!(cx.update(|cx| workspaces(cx)).is_empty());

    let workspace_id = cx
        .update(|cx| {
            sync_window(
                cx,
                window.window_id(),
                None,
                vec![path("/repos/a")],
                Some(path("/repos/a")),
            )
        })
        .expect("durable group after first repository");
    assert!(cx.update(|cx| workspace(cx, workspace_id)).is_some());

    assert_eq!(
        cx.update(|cx| {
            sync_window(cx, window.window_id(), Some(workspace_id), Vec::new(), None)
        }),
        None
    );
    assert!(cx.update(|cx| workspaces(cx)).is_empty());
}

#[gpui::test]
fn review_regression_lifecycle_first_workspace_replays_the_empty_windows_focus(
    cx: &mut gpui::TestAppContext,
) {
    let first = cx.add_window(|_, _| gpui::Empty);
    let second = cx.add_window(|_, _| gpui::Empty);
    cx.update(|cx| initialize_for_test(cx, Vec::new()));

    let first_workspace = cx
        .update(|cx| {
            sync_window(
                cx,
                first.window_id(),
                None,
                vec![path("/repos/first")],
                Some(path("/repos/first")),
            )
        })
        .expect("first group");
    cx.update(|cx| mark_window_active(cx, first.window_id()));

    // The second window receives focus while it is still ephemeral, then
    // becomes durable only when its first repository is added.
    cx.update(|cx| mark_window_active(cx, second.window_id()));
    let second_workspace = cx
        .update(|cx| {
            sync_window(
                cx,
                second.window_id(),
                None,
                vec![path("/repos/second")],
                Some(path("/repos/second")),
            )
        })
        .expect("second group");

    // Clicking back must make the first window newest. If the provisional
    // focus was forgotten, active_workspace still says "first" and suppresses
    // this activation update.
    cx.update(|cx| mark_window_active(cx, first.window_id()));
    let workspaces = cx.update(|cx| workspaces(cx));
    let activation = |id| {
        workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .expect("saved group")
            .last_activation_order
    };
    assert!(
        activation(first_workspace) > activation(second_workspace),
        "the last clicked window must be restored frontmost"
    );
}

#[test]
fn placement_rebases_relative_position_and_clamps_to_a_smaller_display() {
    let restored = rebase_window_frame(
        SavedWindowFrame {
            x: 960,
            y: 540,
            width: 1600,
            height: 1000,
        },
        Some(SavedWindowFrame {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
        }),
        SavedWindowFrame {
            x: 0,
            y: 24,
            width: 1366,
            height: 744,
        },
        800,
        600,
    );

    assert_eq!(restored.width, 1366);
    assert_eq!(restored.height, 744);
    assert_eq!(restored.x, 0);
    assert_eq!(restored.y, 24);
}

#[test]
fn placement_without_old_display_metadata_is_clamped_on_screen() {
    let restored = rebase_window_frame(
        SavedWindowFrame {
            x: -9000,
            y: 9000,
            width: 900,
            height: 700,
        },
        None,
        SavedWindowFrame {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        },
        800,
        600,
    );

    assert_eq!(restored.x, -1920);
    assert_eq!(restored.y, 340);
    assert_eq!(restored.width, 900);
    assert_eq!(restored.height, 700);
}

/// Each write the gated sink recorded: its thread and the paths written.
type WrittenRx = std::sync::mpsc::Receiver<(std::thread::ThreadId, Vec<PathBuf>)>;

/// A writer whose sink records each write and holds the first one until
/// `release` is sent, so tests can queue behind an in-progress write.
fn gated_writer() -> (Arc<WorkspaceWriter>, std::sync::mpsc::Sender<()>, WrittenRx) {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = Mutex::new(Some(release_rx));
    let (written_tx, written_rx) = std::sync::mpsc::channel();
    let written_tx = Mutex::new(written_tx);
    let writer = WorkspaceWriter::spawn(Box::new(move |workspaces: &[Workspace]| {
        if let Some(release) = lock(&release_rx).take() {
            let _ = release.recv();
        }
        let repositories = workspaces
            .iter()
            .flat_map(|workspace| workspace.repositories.clone())
            .collect();
        let _ = lock(&written_tx).send((std::thread::current().id(), repositories));
    }));
    (writer, release_tx, written_rx)
}

fn snapshot(repo: &str) -> Vec<Workspace> {
    vec![Workspace::new(vec![path(repo)])]
}

/// Session writes fsync, so they run off the UI thread; while one is in
/// progress, later snapshots collapse into the newest.
#[test]
fn workspace_writes_leave_the_calling_thread_and_keep_the_newest() {
    let (writer, release, written) = gated_writer();
    let timeout = std::time::Duration::from_secs(5);

    writer.enqueue(snapshot("/repos/a"));
    // Let the worker take `a` and block in the sink before queueing more.
    std::thread::sleep(std::time::Duration::from_millis(50));
    writer.enqueue(snapshot("/repos/b"));
    writer.enqueue(snapshot("/repos/c"));
    release.send(()).unwrap();

    let first = written.recv_timeout(timeout).expect("first write");
    let second = written.recv_timeout(timeout).expect("second write");
    assert_eq!(first.1, vec![path("/repos/a")]);
    assert_eq!(second.1, vec![path("/repos/c")], "`b` was superseded");
    assert_ne!(first.0, std::thread::current().id());
    assert!(
        written
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
}

/// The quit flush writes synchronously and is the last write: a snapshot
/// queued before it may land first, never after.
#[test]
fn quit_flush_is_the_last_workspace_write() {
    let (writer, release, written) = gated_writer();

    writer.enqueue(snapshot("/repos/a"));
    std::thread::sleep(std::time::Duration::from_millis(50));
    writer.enqueue(snapshot("/repos/stale"));
    let flusher = {
        let writer = Arc::clone(&writer);
        std::thread::spawn(move || writer.write_now(&snapshot("/repos/final")))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    release.send(()).unwrap();
    flusher.join().unwrap();

    let writes: Vec<_> = std::iter::from_fn(|| {
        written
            .recv_timeout(std::time::Duration::from_millis(200))
            .ok()
    })
    .map(|(_, repositories)| repositories)
    .collect();
    assert_eq!(writes.first(), Some(&vec![path("/repos/a")]));
    assert_eq!(
        writes.last(),
        Some(&vec![path("/repos/final")]),
        "nothing may land after the quit flush: {writes:?}"
    );
}

/// Main-thread cost of one focus change: the old synchronous session
/// write versus queueing it. Run with `--ignored --nocapture`.
#[test]
#[ignore = "timing probe"]
fn timing_workspace_persist_on_the_calling_thread() {
    let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("tempdir on disk");
    let session_path = dir.path().join("session.json");
    let workspaces: Vec<Workspace> = (0..6)
        .map(|index| {
            Workspace::new(
                (0..4)
                    .map(|repo| path(&format!("/home/user/src/project-{index}/repo-{repo}")))
                    .collect(),
            )
        })
        .collect();
    let median = |mut samples: Vec<std::time::Duration>| {
        samples.sort();
        samples[samples.len() / 2]
    };

    let sync: Vec<_> = (0..40)
        .map(|_| {
            let started = std::time::Instant::now();
            session::persist_workspaces_to_path(&workspaces, &session_path).unwrap();
            started.elapsed()
        })
        .collect();
    let target = session_path.clone();
    let writer = WorkspaceWriter::spawn(Box::new(move |workspaces: &[Workspace]| {
        let _ = session::persist_workspaces_to_path(workspaces, &target);
    }));
    let queued: Vec<_> = (0..40)
        .map(|_| {
            let started = std::time::Instant::now();
            writer.enqueue(workspaces.clone());
            let elapsed = started.elapsed();
            std::thread::sleep(std::time::Duration::from_millis(5));
            elapsed
        })
        .collect();
    println!(
        "timing workspace_persist sync_median={:?} queued_median={:?}",
        median(sync),
        median(queued)
    );
}
