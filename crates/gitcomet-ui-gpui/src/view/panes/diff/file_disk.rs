//! "File changed on disk" for the file on screen.
//!
//! The watcher only says that *something* in the worktree moved, so the pane
//! remembers what it read (a stat stamp plus a hash of the bytes) and, when the
//! repo reports a worktree change, re-checks the one path it is showing. A
//! difference raises an inline notice with Reload / Dismiss; the content is
//! never swapped under the user. View-owned, like the editor buffer: the store
//! never sees the bytes. Hashing and comparing run off the UI thread.

use super::*;
use crate::view::documents::disk::{DiskCheckOutcome, DiskIdentity, check_disk_identity};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) enum DiskSurface {
    Preview,
    Editor,
}

/// Why a check runs, which decides what a changed file leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) enum DiskCheckCause {
    /// The watcher (or window focus) reported a worktree write.
    WorktreeChanged,
    /// A GitComet-run command that writes the checkout finished or is still
    /// running.
    GitOperation,
    /// The surface was out of sight (editor ↔ preview, another file, another
    /// repo tab, a restored buffer) while the disk may have moved.
    CameIntoView,
}

impl DiskCheckCause {
    fn by_git_operation(self) -> bool {
        self == Self::GitOperation
    }

    /// A clean surface catches up without asking: GitComet's own command
    /// (the diff pane follows it too), or a change the user never saw the
    /// old version of.
    fn follows_when_clean(self) -> bool {
        matches!(self, Self::GitOperation | Self::CameIntoView)
    }
}

/// The repo revisions a surface was last read or checked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::view) struct FileDiskSeen {
    repo_id: RepoId,
    surface: DiskSurface,
    worktree_rev: u64,
    local_write_rev: u64,
}

/// `(repo, worktree_change_rev, local_worktree_write_rev)` of the active repo.
pub(in crate::view) type FileDiskRevs = (RepoId, u64, u64);

#[derive(Clone, Debug)]
pub(in crate::view) struct FileDiskNotice {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) abs_path: PathBuf,
    pub(in crate::view) surface: DiskSurface,
    /// The disk state the notice describes; Dismiss adopts it.
    seen: DiskIdentity,
    /// Attributed to a GitComet-run git command rather than another program.
    pub(in crate::view) by_git_operation: bool,
    pub(in crate::view) deleted: bool,
}

impl DiffRenderer {
    /// The working-tree file on screen: repo, repo-relative and absolute path.
    fn file_disk_target(&self) -> Option<(RepoId, PathBuf, PathBuf)> {
        if self.is_inline_submodule_diff_active() {
            return None;
        }
        let repo = self.active_repo()?;
        let Some(DiffTarget::WorkingTree { path, .. }) =
            self.bound_diff_state(repo).diff_target.as_ref()
        else {
            return None;
        };
        Some((repo.id, path.clone(), self.absolute_worktree_path(path)?))
    }

    /// The editor holds the file on screen, whether or not its read worked.
    fn file_editor_holds(&self, repo_id: RepoId, path: &Path) -> bool {
        self.is_file_editor_active()
            && !self.file_editor_loading()
            && self.file_editor_key().is_some()
            && self.file_editor_key() == self.document_identity(repo_id, path)
    }

    /// The surface showing bytes straight off the worktree, with the absolute
    /// path it reads. `None` for a commit blob, a load in flight, a failed
    /// read, a preview left loaded behind the diff view, a submodule summary —
    /// anything a disk check says nothing about.
    fn file_disk_surface(&self) -> Option<(RepoId, PathBuf, DiskSurface)> {
        let (repo_id, path, abs_path) = self.file_disk_target()?;
        if self.is_file_editor_active() {
            return (self.file_editor_holds(repo_id, &path) && self.file_editor_error().is_none())
                .then_some((repo_id, abs_path, DiskSurface::Editor));
        }
        // The preview also shows commit blobs and index copies through a temp
        // file; only a source that *is* the worktree path is checked.
        let preview_is_the_file = matches!(self.worktree_preview, Loadable::Ready(_))
            && self.worktree_preview_path.as_deref() == Some(abs_path.as_path())
            && self.worktree_preview_source_path.as_deref() == Some(abs_path.as_path())
            && self.is_file_preview_active();
        preview_is_the_file.then_some((repo_id, abs_path, DiskSurface::Preview))
    }

    /// The notice, when it names the surface on screen.
    pub(in crate::view) fn file_disk_notice_for_screen(&self) -> Option<&FileDiskNotice> {
        let notice = self.file_disk_notice.as_ref()?;
        let (repo_id, abs_path, surface) = self.file_disk_surface()?;
        (notice.repo_id == repo_id && notice.abs_path == abs_path && notice.surface == surface)
            .then_some(notice)
    }

    /// An editor notice is waiting for the user. Auto-save must not answer it
    /// by writing the buffer over the other program's version.
    pub(in crate::view) fn file_disk_notice_awaits_editor(&self) -> bool {
        self.file_disk_notice
            .as_ref()
            .is_some_and(|notice| notice.surface == DiskSurface::Editor)
    }

    /// Saving on purpose is choosing to keep the buffer: take the disk state
    /// the notice described as the one being overwritten.
    pub(in crate::view) fn answer_file_disk_notice_by_saving(&mut self) {
        if !self.file_disk_notice_awaits_editor() {
            return;
        }
        if let Some(notice) = self.file_disk_notice.take() {
            self.invalidate_file_disk_checks();
            if let Some(document) = self.editor_documents.active() {
                document.update_disk(|disk| disk.adopt(notice.seen));
            }
        }
    }

    /// Captured when a read *starts*, so a write that lands during the read
    /// still reads as news once it finishes.
    pub(in crate::view) fn current_file_disk_revs(&self) -> Option<FileDiskRevs> {
        self.active_repo().map(|repo| {
            (
                repo.id,
                repo.worktree_change_rev,
                repo.local_worktree_write_rev,
            )
        })
    }

    fn seat_file_disk_seen(&mut self, surface: DiskSurface, revs: Option<FileDiskRevs>) {
        self.file_disk_seen = revs.map(|(repo_id, worktree_rev, local_write_rev)| FileDiskSeen {
            repo_id,
            surface,
            worktree_rev,
            local_write_rev,
        });
    }

    /// A read of `surface` landed. Remember what it was read under and catch
    /// up at once if the worktree moved while it ran.
    pub(in crate::view) fn file_disk_read_landed(
        &mut self,
        surface: DiskSurface,
        revs: Option<FileDiskRevs>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.clear_file_disk_notice_for(surface);
        self.seat_file_disk_seen(surface, revs);
        self.sync_file_disk_check(false, cx);
    }

    fn clear_file_disk_notice_for(&mut self, surface: DiskSurface) {
        if self
            .file_disk_notice
            .as_ref()
            .is_some_and(|notice| notice.surface == surface)
        {
            self.file_disk_notice = None;
        }
    }

    /// Drop any check in flight: it compares against an identity that is
    /// about to change.
    fn invalidate_file_disk_checks(&mut self) {
        self.file_disk_check_seq = self.file_disk_check_seq.wrapping_add(1);
        self.file_disk_check_in_flight = None;
    }

    /// Called from `apply_state_snapshot` and when a read lands: check the open
    /// file when the repo reported a worktree change since the surface was
    /// read or last checked, or when the surface just came back into view.
    pub(in crate::view) fn sync_file_disk_check(
        &mut self,
        target_changed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if target_changed {
            self.file_disk_notice = None;
            self.file_disk_seen = None;
        }
        let Some((repo_id, abs_path, surface)) = self.file_disk_surface() else {
            if self.file_disk_notice.take().is_some() {
                cx.notify();
            }
            self.retry_failed_file_editor_read(cx);
            return;
        };
        if self.file_disk_notice.as_ref().is_some_and(|notice| {
            notice.repo_id != repo_id || notice.abs_path != abs_path || notice.surface != surface
        }) {
            self.file_disk_notice = None;
            cx.notify();
        }
        let Some((_, worktree_rev, local_write_rev)) = self.current_file_disk_revs() else {
            return;
        };
        let next = FileDiskSeen {
            repo_id,
            surface,
            worktree_rev,
            local_write_rev,
        };
        let Some(seen) = self.file_disk_seen.replace(next) else {
            // A surface nobody read under us: the editor still holding this
            // file from before another file or repo tab was shown.
            self.spawn_file_disk_check(DiskCheckCause::CameIntoView, cx);
            return;
        };
        if seen.repo_id != repo_id || seen.surface != surface {
            self.spawn_file_disk_check(DiskCheckCause::CameIntoView, cx);
            return;
        }
        let local_write_moved = seen.local_write_rev != local_write_rev;
        if seen.worktree_rev == worktree_rev && !local_write_moved {
            return;
        }
        // A slow command (pull, rebase) flushes the watcher before it
        // finishes, so "still running" counts as much as "just finished".
        let git_operation = local_write_moved
            || self
                .active_repo()
                .is_some_and(|repo| repo.git_operation_in_flight());
        self.spawn_file_disk_check(
            if git_operation {
                DiskCheckCause::GitOperation
            } else {
                DiskCheckCause::WorktreeChanged
            },
            cx,
        );
    }

    /// An editor whose read failed (the file was deleted, say) shows an error,
    /// not content, so there is nothing to protect: when the worktree moves,
    /// read again — once per move, not once per snapshot.
    fn retry_failed_file_editor_read(&mut self, cx: &mut gpui::Context<Self>) {
        let Some((repo_id, path, _)) = self.file_disk_target() else {
            return;
        };
        // A failed *save* leaves the unsaved text on screen under its error;
        // re-reading would replace it with the disk's.
        if self.file_editor_error().is_none()
            || self.file_editor_dirty()
            || !self.file_editor_holds(repo_id, &path)
        {
            return;
        }
        let Some(revs) = self.current_file_disk_revs() else {
            return;
        };
        let moved = self.file_disk_seen.is_some_and(|seen| {
            seen.repo_id == repo_id && (seen.worktree_rev, seen.local_write_rev) != (revs.1, revs.2)
        });
        if moved {
            self.seat_file_disk_seen(DiskSurface::Editor, Some(revs));
            self.reread_file_editor_from_disk(cx);
        }
    }

    /// Stat (and if needed read) the surface's file off-thread, then apply.
    pub(in crate::view) fn spawn_file_disk_check(
        &mut self,
        cause: DiskCheckCause,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((repo_id, abs_path, surface)) = self.file_disk_surface() else {
            return;
        };
        let (known, hash_limit) = match surface {
            DiskSurface::Editor => (
                self.file_editor_disk().clone(),
                super::preview::FILE_EDITOR_MAX_TEXT_BYTES as u64,
            ),
            DiskSurface::Preview => (
                self.worktree_preview_disk.clone(),
                rows::PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES as u64,
            ),
        };
        self.file_disk_check_seq = self.file_disk_check_seq.wrapping_add(1);
        self.file_disk_check_in_flight = Some(cause);
        let seq = self.file_disk_check_seq;
        cx.spawn(async move |view: WeakEntity<DiffRenderer>, cx| {
            let check = {
                let path = abs_path.clone();
                move || check_disk_identity(&path, &known, hash_limit)
            };
            let outcome = if crate::ui_runtime::current().uses_background_compute() {
                smol::unblock(check).await
            } else {
                check()
            };
            let _ = view.update(cx, |this, cx| {
                this.apply_file_disk_check(seq, (repo_id, abs_path, surface), cause, outcome, cx);
            });
        })
        .detach();
    }

    fn apply_file_disk_check(
        &mut self,
        seq: u64,
        checked: (RepoId, PathBuf, DiskSurface),
        cause: DiskCheckCause,
        outcome: DiskCheckOutcome,
        cx: &mut gpui::Context<Self>,
    ) {
        if seq != self.file_disk_check_seq {
            return;
        }
        self.file_disk_check_in_flight = None;
        if self.file_disk_surface().as_ref() != Some(&checked) {
            return;
        }
        let (repo_id, abs_path, surface) = checked;
        let (seen, deleted) = match outcome {
            DiskCheckOutcome::Unchanged => return,
            DiskCheckOutcome::Same { fresh, matched } => {
                match surface {
                    DiskSurface::Editor => {
                        if let Some(document) = self.editor_documents.active() {
                            document.update_disk(|disk| disk.confirm(fresh, matched));
                        }
                    }
                    DiskSurface::Preview => self.worktree_preview_disk.confirm(fresh, matched),
                }
                // The disk is back to bytes this surface holds; nothing left
                // to decide.
                if self
                    .file_disk_notice
                    .as_ref()
                    .is_some_and(|notice| notice.surface == surface)
                {
                    self.file_disk_notice = None;
                    cx.notify();
                }
                return;
            }
            DiskCheckOutcome::Changed { fresh, hash } => (DiskIdentity::loaded(fresh, hash), false),
            DiskCheckOutcome::Missing => (DiskIdentity::missing(), true),
        };
        let clean = match surface {
            DiskSurface::Editor => !self.file_editor_dirty(),
            DiskSurface::Preview => true,
        };
        if cause.follows_when_clean() && clean {
            self.file_disk_notice = None;
            match surface {
                DiskSurface::Editor => self.reload_file_editor_from_disk(cx),
                DiskSurface::Preview => self.reload_worktree_preview_keeping_scroll(cx),
            }
            return;
        }
        if self
            .file_disk_notice
            .as_ref()
            .is_some_and(|notice| notice.seen.same_disk_state(&seen))
        {
            return;
        }
        self.file_disk_notice = Some(FileDiskNotice {
            repo_id,
            abs_path,
            surface,
            seen,
            by_git_operation: cause.by_git_operation(),
            deleted,
        });
        cx.notify();
    }

    /// Reload: re-read the surface's file, dropping unsaved edits.
    pub(in crate::view) fn reload_file_from_disk_notice(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(notice) = self.file_disk_notice.take() else {
            return;
        };
        // The read about to start sees the latest disk; a check computed
        // against the old identity must not land on top of it.
        self.invalidate_file_disk_checks();
        match notice.surface {
            DiskSurface::Editor => self.reload_file_editor_from_disk(cx),
            DiskSurface::Preview => self.reload_worktree_preview_keeping_scroll(cx),
        }
        cx.notify();
    }

    /// Dismiss / Keep my edits: the disk state seen becomes the known one, so
    /// the same bytes never re-raise the notice; the next change does.
    pub(in crate::view) fn dismiss_file_disk_notice(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(notice) = self.file_disk_notice.take() else {
            return;
        };
        // A check in flight compared against the identity being replaced; run
        // it again against the new one rather than let it re-raise this.
        let in_flight = self.file_disk_check_in_flight;
        self.invalidate_file_disk_checks();
        match notice.surface {
            DiskSurface::Editor => {
                if let Some(document) = self.editor_documents.active() {
                    document.update_disk(|disk| disk.adopt(notice.seen));
                }
                // Keeping the edits accepts the disk as what the next save
                // replaces; the save's version check would refuse otherwise.
                self.refresh_file_editor_disk_version(cx);
                // Auto-save held off while the question was open.
                if self.auto_save_file_edits && self.file_editor_dirty() {
                    self.schedule_file_editor_autosave(cx);
                }
            }
            DiskSurface::Preview => self.worktree_preview_disk.adopt(notice.seen),
        }
        if let Some(cause) = in_flight {
            self.spawn_file_disk_check(cause, cx);
        }
        cx.notify();
    }

    /// Re-read the preview without sending the reader back to the top.
    ///
    /// The pixel offset, not a row: a uniform list records no per-row bounds,
    /// so its logical top is always row 0.
    pub(in crate::view) fn reload_worktree_preview_keeping_scroll(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let offset = self.worktree_preview_scroll.0.borrow().base_handle.offset();
        self.worktree_preview_restore_scroll_offset = Some(offset);
        self.worktree_preview = Loadable::NotLoaded;
        cx.notify();
    }
}
