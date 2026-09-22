//! "File changed on disk": the editor and the read-only preview notice an
//! external write to the file they show, and never swap content unasked.

use super::*;
use crate::view::panes::main::DiskSurface;

fn unique_workdir(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{label}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A repo showing `file_rel` from the working tree as a full-content view:
/// the editor when `edit`, the read-only preview otherwise.
fn file_state(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &Path,
    file_rel: &Path,
    edit: bool,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, workdir);
    repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.to_path_buf(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    repo.diff_state.content_preview = true;
    repo.diff_state.edit_mode = edit;
    app_state_with_repo(repo, repo_id)
}

struct Harness {
    view: gpui::Entity<super::super::GitCometView>,
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    state: Arc<AppState>,
    /// What the store currently says: editor on or off, and the revisions.
    edit: std::cell::Cell<bool>,
    revs: std::cell::Cell<(u64, u64)>,
}

impl Harness {
    fn file(&self) -> std::path::PathBuf {
        self.workdir.join(&self.file_rel)
    }

    fn open<'a>(
        cx: &'a mut gpui::TestAppContext,
        label: &str,
        repo_id: u64,
        contents: &str,
        edit: bool,
    ) -> (Self, &'a mut gpui::VisualTestContext) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) = cx.add_window_view(|window, cx| {
            super::super::GitCometView::new(store, events, None, window, cx)
        });
        let repo_id = gitcomet_state::model::RepoId(repo_id);
        let workdir = unique_workdir(label);
        let file_rel = std::path::PathBuf::from("main.rs");
        std::fs::write(workdir.join(&file_rel), contents).expect("write fixture");
        let state = file_state(repo_id, &workdir, &file_rel, edit);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                push_test_state(this, Arc::clone(&state), cx);
                this.main_pane.update(cx, |pane, cx| {
                    if edit {
                        pane.ensure_file_editor_loaded(cx);
                    } else {
                        pane.ensure_selected_file_preview_loaded(cx);
                    }
                });
            });
        });
        cx.run_until_parked();
        (
            Self {
                view,
                workdir,
                file_rel,
                state,
                edit: std::cell::Cell::new(edit),
                revs: std::cell::Cell::new((0, 0)),
            },
            cx,
        )
    }

    /// The opening state with the current mode and revisions applied.
    fn current(&self) -> Arc<AppState> {
        let mut next = AppState::clone(&self.state);
        let (worktree, local) = self.revs.get();
        next.repos[0].worktree_change_rev = worktree;
        next.repos[0].local_worktree_write_rev = local;
        next.repos[0].diff_state.edit_mode = self.edit.get();
        Arc::new(next)
    }

    /// Publish a snapshot whose revisions moved, the way the reducer moves
    /// them for a watcher flush (`worktree`) or a finished command (`local`),
    /// and let the check land.
    fn bump(&self, cx: &mut gpui::VisualTestContext, worktree: u64, local: u64) {
        self.revs.set((worktree, local));
        let next = self.current();
        cx.update(|_window, app| {
            self.view
                .update(app, |this, cx| push_test_state(this, next, cx));
        });
        cx.run_until_parked();
    }

    /// Publish the same repo with the editor switched on or off, revisions
    /// untouched: the Edit toggle, as far as the pane can tell.
    fn show(&self, cx: &mut gpui::VisualTestContext, edit: bool) {
        self.edit.set(edit);
        let next = self.current();
        cx.update(|_window, app| {
            self.view.update(app, |this, cx| {
                push_test_state(this, next, cx);
                this.main_pane.update(cx, |pane, cx| {
                    if edit {
                        pane.ensure_file_editor_loaded(cx);
                    } else {
                        pane.ensure_selected_file_preview_loaded(cx);
                    }
                });
            });
        });
        cx.run_until_parked();
    }

    fn preview_text(&self, cx: &mut gpui::VisualTestContext) -> String {
        self.with_pane(cx, |pane, _| pane.worktree_preview_text.to_string())
    }

    fn with_pane<R>(
        &self,
        cx: &mut gpui::VisualTestContext,
        f: impl FnOnce(&MainPaneView, &gpui::App) -> R,
    ) -> R {
        cx.update(|_window, app| {
            let pane = self.view.read(app).main_pane.read(app);
            f(pane, app)
        })
    }

    fn update_pane(
        &self,
        cx: &mut gpui::VisualTestContext,
        f: impl FnOnce(&mut MainPaneView, &mut gpui::Context<MainPaneView>),
    ) {
        cx.update(|_window, app| {
            self.view.update(app, |this, cx| {
                this.main_pane.update(cx, f);
            });
        });
        cx.run_until_parked();
    }

    fn editor_text(&self, cx: &mut gpui::VisualTestContext) -> String {
        self.with_pane(cx, |pane, app| {
            pane.file_editor_input.read(app).text().to_string()
        })
    }

    fn notice(&self, cx: &mut gpui::VisualTestContext) -> Option<(DiskSurface, bool, bool)> {
        self.with_pane(cx, |pane, _| {
            pane.file_disk_notice
                .as_ref()
                .map(|notice| (notice.surface, notice.by_git_operation, notice.deleted))
        })
    }

    fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
    }
}

#[gpui::test]
async fn external_write_to_the_open_editor_file_raises_the_notice_and_reload_keeps_the_caret(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_editor", 960, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input
            .update(cx, |input, cx| input.set_cursor_offset(5, cx));
    });

    std::fs::write(h.file(), "fn main() { changed(); }\n").expect("external write");
    h.bump(cx, 1, 0);

    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, false)));
    assert_eq!(
        h.editor_text(cx),
        "fn main() {}\n",
        "the buffer must not be swapped under the user"
    );
    assert!(h.with_pane(cx, |pane, _| !pane.file_editor_is_dirty()));

    // The strip is on screen with both buttons; Reload is the way in.
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("file_disk_notice").is_some());
    assert!(cx.debug_bounds("file_disk_notice_dismiss").is_some());
    let reload = cx
        .debug_bounds("file_disk_notice_reload")
        .expect("reload button is painted");
    simulate_counted_click(cx, reload.center(), 1);
    cx.run_until_parked();

    assert_eq!(h.editor_text(cx), "fn main() { changed(); }\n");
    assert_eq!(h.notice(cx), None);
    assert!(h.with_pane(cx, |pane, app| {
        pane.file_editor_input.read(app).cursor_offset() == 5 && !pane.file_editor_is_dirty()
    }));
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("file_disk_notice").is_none());

    // Same bytes, another flush: nothing to say.
    h.bump(cx, 2, 0);
    assert_eq!(h.notice(cx), None);
    h.cleanup();
}

#[gpui::test]
async fn dirty_buffer_keeps_its_edits_on_dismiss_until_the_disk_moves_again(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_dirty", 961, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input.update(cx, |input, cx| {
            input.replace_utf8_range(0..0, "// header\n", cx);
        });
    });
    assert!(h.with_pane(cx, |pane, _| pane.file_editor_is_dirty()));

    std::fs::write(h.file(), "fn main() { changed(); }\n").expect("external write");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, false)));
    assert_eq!(h.editor_text(cx), "// header\nfn main() {}\n");

    h.update_pane(cx, |pane, cx| pane.dismiss_file_disk_notice(cx));
    assert_eq!(h.notice(cx), None);
    assert_eq!(h.editor_text(cx), "// header\nfn main() {}\n");
    assert!(h.with_pane(cx, |pane, _| pane.file_editor_is_dirty()));

    // The dismissed disk state is now the known one.
    h.bump(cx, 2, 0);
    assert_eq!(h.notice(cx), None, "the same bytes must not nag");

    std::fs::write(h.file(), "fn main() { changed_again(); }\n").expect("external write");
    h.bump(cx, 3, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, false)));

    // Reload from a dirty buffer drops the edits.
    h.update_pane(cx, |pane, cx| pane.reload_file_from_disk_notice(cx));
    assert_eq!(h.editor_text(cx), "fn main() { changed_again(); }\n");
    assert!(h.with_pane(cx, |pane, _| !pane.file_editor_is_dirty()));
    assert_eq!(h.notice(cx), None);
    h.cleanup();
}

#[gpui::test]
async fn own_save_does_not_raise_the_notice_before_or_after_the_write_lands(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_own_save", 962, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input.update(cx, |input, cx| {
            input.replace_utf8_range(0..0, "// header\n", cx);
            input.set_cursor_offset(3, cx);
        });
    });
    let model_id_before = h.with_pane(cx, |pane, app| {
        pane.file_editor_input.read(app).text_snapshot().model_id()
    });
    h.update_pane(cx, |pane, cx| pane.save_file_editor_buffer(cx));
    assert!(h.with_pane(cx, |pane, _| !pane.file_editor_is_dirty()));

    // A flush for some other file arrives before the write lands: the disk
    // still holds the old bytes, which the buffer knows about.
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), None, "pre-write disk bytes are our own");

    // The write lands, as the save effect would do it.
    std::fs::write(h.file(), "// header\nfn main() {}\n").expect("save lands");
    h.bump(cx, 2, 1);
    assert_eq!(
        h.notice(cx),
        None,
        "our own bytes are not an external change"
    );
    assert_eq!(h.editor_text(cx), "// header\nfn main() {}\n");
    assert!(
        h.with_pane(cx, |pane, app| {
            let input = pane.file_editor_input.read(app);
            input.cursor_offset() == 3 && input.text_snapshot().model_id() == model_id_before
        }),
        "the buffer must not be re-seated"
    );
    h.cleanup();
}

#[gpui::test]
async fn unrelated_file_change_does_not_raise_the_notice(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_unrelated", 963, "fn main() {}\n", true);
    std::fs::write(h.workdir.join("other.rs"), "other\n").expect("write other");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), None);
    assert_eq!(h.editor_text(cx), "fn main() {}\n");
    h.cleanup();
}

#[gpui::test]
async fn git_operation_change_on_a_clean_editor_reloads_silently(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_git_op_clean", 964, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input
            .update(cx, |input, cx| input.set_cursor_offset(5, cx));
    });

    // A checkout finished: the watcher flush and the completion both moved.
    std::fs::write(h.file(), "fn main() { checked_out(); }\n").expect("checkout wrote");
    h.bump(cx, 1, 1);
    assert_eq!(
        h.notice(cx),
        None,
        "a clean view follows GitComet's own command"
    );
    assert_eq!(h.editor_text(cx), "fn main() { checked_out(); }\n");
    assert!(h.with_pane(
        cx,
        |pane, app| pane.file_editor_input.read(app).cursor_offset() == 5
    ));

    // A slow command: the flush arrives while it is still running.
    let mut running = AppState::clone(&h.current());
    running.repos[0].worktree_change_rev = 2;
    running.repos[0].local_worktree_write_rev = 1;
    running.repos[0].local_actions_in_flight = 1;
    std::fs::write(h.file(), "fn main() { pulled(); }\n").expect("pull wrote");
    cx.update(|_window, app| {
        h.view
            .update(app, |this, cx| push_test_state(this, Arc::new(running), cx));
    });
    cx.run_until_parked();
    assert_eq!(h.notice(cx), None, "a running command owns the change");
    assert_eq!(h.editor_text(cx), "fn main() { pulled(); }\n");
    h.cleanup();
}

#[gpui::test]
async fn git_operation_change_on_a_dirty_editor_is_attributed_to_git(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_git_op_dirty", 965, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input.update(cx, |input, cx| {
            input.replace_utf8_range(0..0, "// header\n", cx);
        });
    });
    std::fs::write(h.file(), "fn main() { checked_out(); }\n").expect("checkout wrote");
    h.bump(cx, 1, 1);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, true, false)));
    assert_eq!(h.editor_text(cx), "// header\nfn main() {}\n");
    assert!(h.with_pane(cx, |pane, _| pane.file_editor_is_dirty()));
    h.cleanup();
}

#[gpui::test]
async fn deleting_the_open_file_raises_a_deleted_notice_once(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_deleted", 966, "fn main() {}\n", true);
    std::fs::remove_file(h.file()).expect("delete");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, true)));
    assert_eq!(h.editor_text(cx), "fn main() {}\n");

    h.update_pane(cx, |pane, cx| pane.dismiss_file_disk_notice(cx));
    h.bump(cx, 2, 0);
    assert_eq!(h.notice(cx), None, "still gone is not news");

    std::fs::write(h.file(), "fn main() { back(); }\n").expect("recreate");
    h.bump(cx, 3, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, false)));
    h.cleanup();
}

#[gpui::test]
async fn preview_external_write_raises_the_notice_and_reload_keeps_the_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let old: String = (0..400).map(|ix| format!("line {ix}\n")).collect();
    let (h, cx) = Harness::open(cx, "disk_notice_preview", 967, &old, false);
    assert_eq!(
        h.with_pane(cx, |pane, _| pane.worktree_preview_text.to_string()),
        old
    );

    // Read a way down the file.
    draw_and_drain_test_window(cx);
    h.update_pane(cx, |pane, _| {
        set_uniform_list_offset(&pane.worktree_preview_scroll, point(px(0.0), px(-600.0)));
    });
    draw_and_drain_test_window(cx);
    let offset_before = h.with_pane(cx, |pane, _| {
        uniform_list_offset(&pane.worktree_preview_scroll)
    });
    assert!(offset_before.y < px(0.0), "the preview must have scrolled");

    let new: String = (0..400).map(|ix| format!("line {ix} changed\n")).collect();
    std::fs::write(h.file(), &new).expect("external write");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Preview, false, false)));
    assert_eq!(
        h.with_pane(cx, |pane, _| pane.worktree_preview_text.to_string()),
        old,
        "the preview must not be swapped under the reader"
    );

    draw_and_drain_test_window(cx);
    let reload = cx
        .debug_bounds("file_disk_notice_reload")
        .expect("reload button is painted");
    simulate_counted_click(cx, reload.center(), 1);
    // The reload happens on the next render, and its read after that.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    assert_eq!(h.notice(cx), None);
    assert_eq!(
        h.with_pane(cx, |pane, _| pane.worktree_preview_text.to_string()),
        new
    );
    let offset_after = h.with_pane(cx, |pane, _| {
        uniform_list_offset(&pane.worktree_preview_scroll)
    });
    assert_eq!(
        offset_after, offset_before,
        "Reload must keep the reader's place"
    );
    h.cleanup();
}

#[gpui::test]
async fn preview_follows_a_git_operation_silently(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_preview_git_op", 968, "alpha\n", false);
    std::fs::write(h.file(), "beta\n").expect("checkout wrote");
    h.bump(cx, 1, 1);
    // The reload is picked up by the next render.
    draw_and_drain_test_window(cx);
    assert_eq!(h.notice(cx), None);
    assert_eq!(
        h.with_pane(cx, |pane, _| pane.worktree_preview_text.to_string()),
        "beta\n"
    );
    h.cleanup();
}

#[gpui::test]
async fn external_write_back_of_the_loaded_version_after_a_save_raises_the_notice(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_write_back", 969, "fn main() {}\n", true);
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input.update(cx, |input, cx| {
            input.replace_utf8_range(0..0, "// header\n", cx);
        });
    });
    h.update_pane(cx, |pane, cx| pane.save_file_editor_buffer(cx));
    std::fs::write(h.file(), "// header\nfn main() {}\n").expect("save lands");
    h.bump(cx, 1, 1);
    assert_eq!(h.notice(cx), None);

    // Another editor, still holding the old text, saves it back.
    std::fs::write(h.file(), "fn main() {}\n").expect("stale write-back");
    h.bump(cx, 2, 1);
    assert_eq!(
        h.notice(cx),
        Some((DiskSurface::Editor, false, false)),
        "the pre-save bytes stopped being ours once the save landed"
    );
    assert_eq!(h.editor_text(cx), "// header\nfn main() {}\n");
    h.cleanup();
}

#[gpui::test]
async fn returning_to_the_editor_catches_up_a_clean_buffer(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(
        cx,
        "disk_notice_back_to_editor",
        970,
        "fn main() {}\n",
        true,
    );
    h.update_pane(cx, |pane, cx| {
        pane.file_editor_input
            .update(cx, |input, cx| input.set_cursor_offset(5, cx));
    });

    // Leave for the read-only view; the editor keeps its buffer behind it.
    h.show(cx, false);
    assert_eq!(h.preview_text(cx), "fn main() {}\n");

    // The file moves while the editor is out of sight, and no flush arrives
    // (a sandbox, or the watcher missed it).
    std::fs::write(h.file(), "fn main() { moved(); }\n").expect("external write");
    h.show(cx, true);

    assert_eq!(
        h.editor_text(cx),
        "fn main() { moved(); }\n",
        "a clean buffer coming back into view must not show what disk no longer has"
    );
    assert_eq!(h.notice(cx), None);
    assert!(h.with_pane(
        cx,
        |pane, app| pane.file_editor_input.read(app).cursor_offset() == 5
    ));
    h.cleanup();
}

#[gpui::test]
async fn returning_to_the_preview_catches_it_up(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_back_to_preview", 971, "alpha\n", false);
    assert_eq!(h.preview_text(cx), "alpha\n");

    h.show(cx, true);
    assert_eq!(h.editor_text(cx), "alpha\n");
    std::fs::write(h.file(), "beta\n").expect("external write");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, false)));
    h.update_pane(cx, |pane, cx| pane.reload_file_from_disk_notice(cx));
    assert_eq!(h.editor_text(cx), "beta\n");

    // Back to the read-only view, which still holds what it read first.
    h.show(cx, false);
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    assert_eq!(h.preview_text(cx), "beta\n");
    assert_eq!(h.notice(cx), None);
    h.cleanup();
}

#[gpui::test]
async fn a_failed_read_retries_when_the_file_comes_back(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (h, cx) = Harness::open(cx, "disk_notice_retry", 972, "fn main() {}\n", true);
    std::fs::remove_file(h.file()).expect("delete");
    h.bump(cx, 1, 0);
    assert_eq!(h.notice(cx), Some((DiskSurface::Editor, false, true)));

    h.update_pane(cx, |pane, cx| pane.reload_file_from_disk_notice(cx));
    assert!(
        h.with_pane(cx, |pane, _| pane.file_editor_error.is_some()),
        "reloading a deleted file shows why there is nothing to edit"
    );

    std::fs::write(h.file(), "fn main() { back(); }\n").expect("recreate");
    h.bump(cx, 2, 0);
    assert!(h.with_pane(cx, |pane, _| pane.file_editor_error.is_none()));
    assert_eq!(h.editor_text(cx), "fn main() { back(); }\n");
    assert_eq!(h.notice(cx), None);
    h.cleanup();
}
