//! Themed dialogs for file operations: collisions, permanent deletes, and
//! "Apply to all remaining".

use super::*;
use crate::view::test_support::{TestBackend, popover_kind, sync_store_snapshot};
use gitcomet_core::filesystem::TransferIntent;
use gitcomet_state::store::AppStore;
use gpui::{Modifiers, MouseButton};
use std::sync::Arc;

fn open_window(
    cx: &mut gpui::TestAppContext,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (root, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        // Escape for dialogs is bound with the text-input keys.
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
        window.activate();
    });
    (root, cx)
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

/// Feeds store results to the view and renders until `done` holds.
fn pump_until(
    root: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    mut done: impl FnMut(&GitCometView, &gpui::App) -> bool,
) -> bool {
    for _ in 0..400 {
        cx.update(|_, app| root.update(app, |root, cx| sync_store_snapshot(root, cx)));
        cx.run_until_parked();
        draw(cx);
        if cx.update(|_, app| done(root.read(app), app)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

fn conflict_prompt(
    root: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
) -> Option<FilesystemConflictPrompt> {
    cx.update(|_, app| match popover_kind(root.read(app), app) {
        Some(PopoverKind::FilesystemConflict(prompt)) => Some(prompt),
        _ => None,
    })
}

fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    draw(cx);
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} must be drawn"))
        .center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

/// `a.txt` and `b.txt` beside a `dest` folder that already holds both.
fn colliding_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let destination = workdir.join("dest");
    std::fs::create_dir(&destination).unwrap();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(workdir.join(name), format!("new {name}")).unwrap();
        std::fs::write(destination.join(name), format!("old {name}")).unwrap();
    }
    (directory, workdir, destination)
}

#[gpui::test]
fn keep_both_with_apply_to_all_resolves_every_collision_with_one_dialog(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = crate::test_support::lock_visual_test();
    let (root, cx) = open_window(cx);
    let (_directory, workdir, destination) = colliding_fixture();
    let request = Request::new(Operation::Transfer {
        sources: vec![workdir.join("a.txt"), workdir.join("b.txt")],
        destination: destination.clone(),
        intent: TransferIntent::Copy,
    });
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_operation(request, None, window, cx)
        })
    });

    assert!(
        pump_until(&root, cx, |root, app| matches!(
            popover_kind(root, app),
            Some(PopoverKind::FilesystemConflict(_))
        )),
        "a collision opens the themed dialog"
    );
    let first = conflict_prompt(&root, cx).unwrap();
    assert_eq!(first.destination, destination.join("a.txt"));
    assert_eq!(first.remaining, 1, "the other collision is offered too");
    assert!(cx.debug_bounds("filesystem_conflict_destination").is_some());
    assert!(
        cx.debug_bounds("filesystem_conflict_merge").is_none(),
        "files cannot merge"
    );

    click(cx, "filesystem_conflict_apply_all");
    click(cx, "filesystem_conflict_keep_both");

    let mut prompts = BTreeSet::from([first.prompt_id]);
    let finished = pump_until(&root, cx, |root, app| {
        if let Some(PopoverKind::FilesystemConflict(prompt)) = popover_kind(root, app) {
            prompts.insert(prompt.prompt_id);
        }
        !root.file_operations.has_pending() && destination.join("b copy.txt").exists()
    });
    assert!(finished, "both collisions resolve");
    assert_eq!(prompts.len(), 1, "one dialog answered both collisions");
    for name in ["a", "b"] {
        assert_eq!(
            std::fs::read_to_string(destination.join(format!("{name} copy.txt"))).unwrap(),
            format!("new {name}.txt")
        );
        assert_eq!(
            std::fs::read_to_string(destination.join(format!("{name}.txt"))).unwrap(),
            format!("old {name}.txt"),
            "keep both leaves the existing item alone"
        );
    }
    assert!(cx.update(|_, app| popover_kind(root.read(app), app).is_none()));
}

#[gpui::test]
fn without_apply_to_all_each_collision_is_asked_about(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (root, cx) = open_window(cx);
    let (_directory, workdir, destination) = colliding_fixture();
    let request = Request::new(Operation::Transfer {
        sources: vec![workdir.join("a.txt"), workdir.join("b.txt")],
        destination: destination.clone(),
        intent: TransferIntent::Copy,
    });
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_operation(request, None, window, cx)
        })
    });
    assert!(pump_until(&root, cx, |root, app| matches!(
        popover_kind(root, app),
        Some(PopoverKind::FilesystemConflict(_))
    )));
    let first = conflict_prompt(&root, cx).unwrap();
    click(cx, "filesystem_conflict_skip");

    assert!(
        pump_until(&root, cx, |root, app| matches!(
            popover_kind(root, app),
            Some(PopoverKind::FilesystemConflict(prompt)) if prompt.prompt_id != first.prompt_id
        )),
        "the second collision gets its own dialog"
    );
    let second = conflict_prompt(&root, cx).unwrap();
    assert_eq!(second.destination, destination.join("b.txt"));
    assert_eq!(second.remaining, 0);
    assert!(
        cx.debug_bounds("filesystem_conflict_apply_all").is_none(),
        "nothing remains to apply to"
    );
    click(cx, "filesystem_conflict_replace");
    assert!(pump_until(&root, cx, |root, _| !root
        .file_operations
        .has_pending()));
    assert_eq!(
        std::fs::read_to_string(destination.join("a.txt")).unwrap(),
        "old a.txt",
        "skipped"
    );
    assert_eq!(
        std::fs::read_to_string(destination.join("b.txt")).unwrap(),
        "new b.txt",
        "replaced"
    );
    assert!(!destination.join("a copy.txt").exists());
}

#[gpui::test]
fn escape_cancels_every_queued_collision_and_releases_the_drop(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (root, cx) = open_window(cx);
    let (_directory, workdir, destination) = colliding_fixture();
    let request = Request::new(Operation::Transfer {
        sources: vec![workdir.join("a.txt"), workdir.join("b.txt")],
        destination: destination.clone(),
        intent: TransferIntent::Move,
    });
    let completions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let completed = completions.clone();
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_drop(
                request,
                gpui::FileDropTransfer {
                    operation: gpui::FileTransferOperation::Move,
                    source_owns_move: false,
                    completion: gpui::FilePaste::new(move |operation| {
                        completed.lock().unwrap().push(operation)
                    }),
                },
                TransferIntent::Move,
                window,
                cx,
            )
        })
    });
    assert!(pump_until(&root, cx, |root, app| matches!(
        popover_kind(root, app),
        Some(PopoverKind::FilesystemConflict(_))
    )));
    assert!(
        completions.lock().unwrap().is_empty(),
        "the drop waits for the decision"
    );

    cx.simulate_keystrokes("escape");
    assert!(
        pump_until(&root, cx, |root, app| popover_kind(root, app).is_none()
            && !root.file_operations.has_pending()),
        "Escape closes the dialog and cancels the rest"
    );
    assert_eq!(*completions.lock().unwrap(), vec![None]);
    for name in ["a.txt", "b.txt"] {
        assert!(workdir.join(name).exists(), "a cancelled move keeps {name}");
        assert_eq!(
            std::fs::read_to_string(destination.join(name)).unwrap(),
            format!("old {name}")
        );
    }
}

#[gpui::test]
fn delete_permanently_waits_for_the_themed_dialog(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (root, cx) = open_window(cx);
    let directory = tempfile::tempdir().unwrap();
    let workdir =
        gitcomet_core::path_utils::canonicalize_or_original(directory.path().to_path_buf());
    let kept = workdir.join("kept.txt");
    let deleted = workdir.join("deleted.txt");
    std::fs::write(&kept, "kept").unwrap();
    std::fs::write(&deleted, "deleted").unwrap();

    for (path, confirm) in [(&kept, false), (&deleted, true)] {
        let request = Request::new(Operation::DeletePermanently {
            sources: vec![path.clone()],
            confirmed: false,
        });
        cx.update(|window, app| {
            root.update(app, |root, cx| {
                root.submit_filesystem_operation(request, None, window, cx)
            })
        });
        assert!(pump_until(&root, cx, |root, app| matches!(
            popover_kind(root, app),
            Some(PopoverKind::DeletePermanentlyConfirm(_))
        )));
        let names = cx.update(|_, app| match popover_kind(root.read(app), app) {
            Some(PopoverKind::DeletePermanentlyConfirm(prompt)) => prompt.names,
            _ => Vec::new(),
        });
        assert_eq!(names, vec![path_label(path)]);
        assert!(path.exists(), "nothing is deleted before the answer");
        click(
            cx,
            if confirm {
                "delete_permanently_confirm"
            } else {
                "delete_permanently_cancel"
            },
        );
        assert!(pump_until(&root, cx, |root, app| popover_kind(root, app)
            .is_none()
            && !root.file_operations.has_pending()));
        assert_eq!(path.exists(), !confirm);
    }
}

#[gpui::test]
fn a_displaced_dialog_comes_back_when_the_screen_is_free(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (root, cx) = open_window(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("item.txt");
    std::fs::write(&path, "item").unwrap();
    let request = Request::new(Operation::DeletePermanently {
        sources: vec![path.clone()],
        confirmed: false,
    });
    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.submit_filesystem_operation(request, None, window, cx)
        })
    });
    assert!(pump_until(&root, cx, |root, app| matches!(
        popover_kind(root, app),
        Some(PopoverKind::DeletePermanentlyConfirm(_))
    )));

    cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.open_popover_centered(PopoverKind::CloneRepo, window, cx)
        })
    });
    draw(cx);
    assert_eq!(
        cx.update(|_, app| popover_kind(root.read(app), app)),
        Some(PopoverKind::CloneRepo)
    );
    assert!(path.exists(), "displacing the dialog is not an answer");

    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.popover_host
                .update(cx, |host, cx| host.close_popover(cx))
        })
    });
    assert!(
        pump_until(&root, cx, |root, app| matches!(
            popover_kind(root, app),
            Some(PopoverKind::DeletePermanentlyConfirm(_))
        )),
        "the delete question is asked again"
    );
    click(cx, "delete_permanently_confirm");
    assert!(pump_until(&root, cx, |root, _| !root
        .file_operations
        .has_pending()));
    assert!(!path.exists());
}
