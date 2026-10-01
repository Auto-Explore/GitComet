use super::*;
use gitcomet_core::filesystem::{
    Conflict, ConflictChoice, ConflictResolution, ItemOutcome, Operation, OperationId, Request,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// The file-operation dialog on screen. The popover carries display data only;
/// the request it answers stays here.
enum OpenFilesystemDialog {
    Conflict {
        prompt_id: u64,
        request: Request,
        conflict: Conflict,
        ownership: Option<u64>,
    },
    Delete {
        prompt_id: u64,
        request: Request,
        ownership: Option<u64>,
    },
    UnsavedEdits {
        prompt_id: u64,
        request: Request,
        removals: Vec<PathBuf>,
        ownership: Option<u64>,
    },
}

impl OpenFilesystemDialog {
    fn prompt_id(&self) -> u64 {
        match self {
            Self::Conflict { prompt_id, .. }
            | Self::Delete { prompt_id, .. }
            | Self::UnsavedEdits { prompt_id, .. } => *prompt_id,
        }
    }

    fn request(&self) -> &Request {
        match self {
            Self::Conflict { request, .. }
            | Self::Delete { request, .. }
            | Self::UnsavedEdits { request, .. } => request,
        }
    }
}

#[derive(Default)]
pub(super) struct FileOperationsUi {
    requests: BTreeMap<OperationId, (Request, Option<u64>)>,
    pastes: BTreeMap<OperationId, crate::clipboard::PasteReceipt>,
    conflicts: VecDeque<(Request, Conflict, Option<u64>)>,
    /// Requests blocked on a dialog or on editor saves.
    confirmations: BTreeMap<OperationId, gitcomet_core::filesystem::Cancellation>,
    pending_delete: VecDeque<(Request, Option<u64>)>,
    pending_unsaved: VecDeque<(Request, Vec<PathBuf>, Option<u64>)>,
    open_dialog: Option<OpenFilesystemDialog>,
    next_prompt_id: u64,
    /// "Apply to all remaining" per logical operation, with the destinations it
    /// already answered: a destination that conflicts again is asked about.
    sticky: BTreeMap<OperationId, (ConflictChoice, BTreeSet<PathBuf>)>,
}

impl FileOperationsUi {
    pub(super) fn has_pending(&self) -> bool {
        !self.requests.is_empty()
            || self.open_dialog.is_some()
            || !self.conflicts.is_empty()
            || !self.confirmations.is_empty()
            || !self.pastes.is_empty()
            || !self.pending_delete.is_empty()
            || !self.pending_unsaved.is_empty()
    }

    /// A dialog is waiting for the screen to be free.
    fn has_queued_dialog(&self) -> bool {
        self.open_dialog.is_none()
            && (!self.pending_delete.is_empty()
                || !self.pending_unsaved.is_empty()
                || !self.conflicts.is_empty())
    }

    fn allocate_prompt_id(&mut self) -> u64 {
        self.next_prompt_id = self.next_prompt_id.wrapping_add(1);
        self.next_prompt_id
    }

    fn take_open_dialog(&mut self, prompt_id: u64) -> Option<OpenFilesystemDialog> {
        if self
            .open_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.prompt_id() == prompt_id)
        {
            self.open_dialog.take()
        } else {
            None
        }
    }

    /// The "Apply to all remaining" answer for this collision, used once per
    /// destination. Merge only covers collisions that can merge.
    fn sticky_choice(&mut self, request: &Request, conflict: &Conflict) -> Option<ConflictChoice> {
        let (choice, answered) = self.sticky.get_mut(&request.logical_id)?;
        let applies = *choice != ConflictChoice::Merge || conflict.can_merge;
        (applies && answered.insert(conflict.destination.clone())).then_some(*choice)
    }

    fn retire_finished_sticky_choices(&mut self) {
        if self.sticky.is_empty() {
            return;
        }
        let live: BTreeSet<OperationId> = self
            .requests
            .values()
            .map(|(request, _)| request.logical_id)
            .chain(
                self.conflicts
                    .iter()
                    .map(|(request, _, _)| request.logical_id),
            )
            .chain(
                self.pending_delete
                    .iter()
                    .map(|(request, _)| request.logical_id),
            )
            .chain(
                self.pending_unsaved
                    .iter()
                    .map(|(request, _, _)| request.logical_id),
            )
            .chain(
                self.open_dialog
                    .as_ref()
                    .map(|dialog| dialog.request().logical_id),
            )
            .collect();
        self.sticky.retain(|id, _| live.contains(id));
    }
}

fn path_label(path: &Path) -> SharedString {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
        .into()
}

impl GitCometView {
    pub(in crate::view) fn submit_filesystem_drop(
        &mut self,
        request: Request,
        transfer: gpui::FileDropTransfer,
        completion_intent: gitcomet_core::filesystem::TransferIntent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_operations.pastes.insert(
            request.logical_id,
            crate::clipboard::PasteReceipt::from_drop(
                request.operation.sources().to_vec(),
                transfer,
                completion_intent,
            ),
        );
        self.submit_filesystem_operation(request, None, window, cx);
    }

    pub(in crate::view) fn cancel_filesystem_operations(&mut self, cx: &mut gpui::Context<Self>) {
        let operations = &mut self.file_operations;
        for (request, _) in operations.requests.values() {
            request.cancellation.cancel();
        }
        for cancellation in operations.confirmations.values() {
            cancellation.cancel();
        }
        for (request, _, _) in &operations.conflicts {
            request.cancellation.cancel();
        }
        operations.conflicts.clear();
        operations.sticky.clear();
        // Dialog-blocked requests have no task of their own to release them.
        let blocked: Vec<OperationId> = operations
            .pending_delete
            .drain(..)
            .map(|(request, _)| request.id)
            .chain(
                operations
                    .pending_unsaved
                    .drain(..)
                    .map(|(request, _, _)| request.id),
            )
            .chain(
                operations
                    .open_dialog
                    .take()
                    .map(|dialog| dialog.request().id),
            )
            .collect();
        for id in blocked {
            operations.confirmations.remove(&id);
        }
        self.popover_host.update(cx, |host, cx| {
            if host.open_filesystem_prompt_id().is_some() {
                host.close_popover(cx);
            }
        });
        self.finish_filesystem_pastes(cx);
        cx.notify();
    }

    pub(crate) fn notify_filesystem_paths_changed(
        &mut self,
        changes: Vec<gitcomet_core::filesystem::PathChange>,
        undo: bool,
        redo: bool,
        _cx: &mut gpui::Context<Self>,
    ) {
        self.store.dispatch(Msg::FilesystemPathsChanged(changes));
        self.store
            .dispatch(Msg::FilesystemJournalUpdated { undo, redo });
    }
    pub(in crate::view) fn submit_filesystem_operation(
        &mut self,
        request: Request,
        ownership: Option<u64>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if request.cancellation.is_cancelled() {
            self.finish_filesystem_pastes(cx);
            return;
        }
        // Decisions are queued; render opens their dialogs one at a time.
        if let Operation::DeletePermanently {
            confirmed: false, ..
        } = request.operation
        {
            self.file_operations
                .confirmations
                .insert(request.id, request.cancellation.clone());
            self.file_operations
                .pending_delete
                .push_back((request, ownership));
            cx.notify();
            return;
        }
        let mut removals = if request.operation.removes_sources() || request.native_source_move {
            request.operation.sources().to_vec()
        } else {
            vec![]
        };
        removals.extend(
            request
                .resolutions
                .iter()
                .filter(|(_, decision)| decision.choice == ConflictChoice::Replace)
                .map(|(path, _)| path.clone()),
        );
        if crate::app::filesystem_has_unsaved_buffers(&removals, cx) {
            self.file_operations
                .confirmations
                .insert(request.id, request.cancellation.clone());
            self.file_operations
                .pending_unsaved
                .push_back((request, removals, ownership));
            cx.notify();
            return;
        }
        self.enqueue_filesystem_operation(request, ownership, cx);
    }

    fn wait_for_filesystem_saves(
        &mut self,
        request: Request,
        removals: Vec<std::path::PathBuf>,
        ownership: Option<u64>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_operations
            .confirmations
            .insert(request.id, request.cancellation.clone());
        cx.spawn_in(window, async move |view, cx| {
            // Dispatch is asynchronous. Wait for the store to accept saves,
            // then for all windows' command counters to drain.
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            for _ in 0..300 {
                if request.cancellation.is_cancelled() {
                    let _ = view.update(cx, |this, cx| {
                        this.file_operations.confirmations.remove(&request.id);
                        this.finish_filesystem_pastes(cx);
                        cx.notify();
                    });
                    return;
                }
                let ready = cx
                    .update(|_, cx| crate::app::filesystem_saves_drained(cx))
                    .unwrap_or(false);
                if ready {
                    let _ = view.update_in(cx, |this, _window, cx| {
                        this.file_operations.confirmations.remove(&request.id);
                        if !crate::app::filesystem_has_unsaved_buffers(&removals, cx) {
                            this.enqueue_filesystem_operation(request, ownership, cx);
                        } else {
                            this.push_toast(
                                components::ToastKind::Error,
                                "Save did not complete. Files were preserved.".into(),
                                cx,
                            );
                        }
                        this.finish_filesystem_pastes(cx);
                    });
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let _ = view.update(cx, |this, cx| {
                this.file_operations.confirmations.remove(&request.id);
                this.finish_filesystem_pastes(cx);
                this.push_toast(components::ToastKind::Error, "Still waiting for editor saves. The file operation was cancelled; files were preserved.".into(), cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn enqueue_filesystem_operation(
        &mut self,
        request: Request,
        ownership: Option<u64>,
        cx: &mut gpui::Context<Self>,
    ) {
        if ownership.is_some()
            && request.id == request.logical_id
            && let Operation::Transfer {
                sources, intent, ..
            } = &request.operation
            && let Some(paste) = crate::clipboard::capture_paste(cx, sources, *intent)
        {
            self.file_operations
                .pastes
                .insert(request.logical_id, paste);
        }
        crate::app::pause_filesystem_editors(request.id, cx);
        self.file_operations
            .requests
            .insert(request.id, (request.clone(), ownership));
        self.bottom_status_bar.update(cx, |_, cx| cx.notify());
        if crate::app::filesystem_saves_drained(cx) {
            self.store.dispatch(Msg::FilesystemRequest(request));
        } else {
            // Stores in different windows submit on different threads. Pause
            // first, then await acknowledged saves before submitting the move;
            // sharing a worker alone cannot establish that ordering.
            let store = self.store.clone();
            cx.spawn(async move |_view, cx| {
                loop {
                    if request.cancellation.is_cancelled()
                        || cx.update(|cx| crate::app::filesystem_saves_drained(cx))
                    {
                        store.dispatch(Msg::FilesystemRequest(request));
                        return;
                    }
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn process_filesystem_results(&mut self, cx: &mut gpui::Context<Self>) {
        let completed: Vec<_> = self
            .state
            .filesystem
            .completed
            .iter()
            .filter(|r| self.file_operations.requests.contains_key(&r.id))
            .cloned()
            .collect();
        for result in completed {
            let Some((request, ownership)) = self.file_operations.requests.remove(&result.id)
            else {
                continue;
            };
            self.bottom_status_bar.update(cx, |_, cx| cx.notify());
            crate::app::finish_filesystem_editors(
                result.id,
                &result.changes,
                &result.moved_versions,
                result.undo_available,
                result.redo_available,
                cx,
            );
            self.store
                .dispatch(Msg::AcknowledgeFilesystemResults(vec![result.id]));
            let successful: Vec<_> = result
                .items
                .iter()
                .filter(|item| matches!(item.outcome, ItemOutcome::Completed))
                .map(|item| item.source.clone())
                .collect();
            let moved: Vec<_> = result
                .changes
                .iter()
                .filter_map(|change| change.old.clone())
                .collect();
            if let Some(paste) = self.file_operations.pastes.get_mut(&request.logical_id) {
                if matches!(
                    request.operation,
                    Operation::Transfer {
                        intent: gitcomet_core::filesystem::TransferIntent::Move,
                        ..
                    }
                ) {
                    paste.completed(&moved);
                } else {
                    paste.completed(&successful);
                }
            }
            for item in result.items {
                match item.outcome {
                    ItemOutcome::Completed => {}
                    ItemOutcome::Skipped => {}
                    ItemOutcome::Conflict(conflict) if !request.cancellation.is_cancelled() => self
                        .file_operations
                        .conflicts
                        .push_back((request.clone(), conflict, ownership)),
                    ItemOutcome::Conflict(_) => {}
                    ItemOutcome::Failed(message) => {
                        self.push_toast(components::ToastKind::Error, message, cx)
                    }
                    ItemOutcome::Cancelled => {}
                }
            }
            if matches!(
                request.operation,
                Operation::Transfer {
                    intent: gitcomet_core::filesystem::TransferIntent::Move,
                    ..
                } | Operation::CompleteOutbound {
                    intent: Some(gitcomet_core::filesystem::TransferIntent::Move),
                    ..
                }
            ) && let Some(ownership) = ownership
            {
                crate::clipboard::complete_file_move(cx, ownership, &moved);
            }
            cx.notify();
        }
        self.file_operations.retire_finished_sticky_choices();
        self.finish_filesystem_pastes(cx);
    }

    fn finish_filesystem_pastes(&mut self, cx: &mut gpui::Context<Self>) {
        if self.file_operations.open_dialog.is_some()
            || !self.file_operations.confirmations.is_empty()
        {
            return;
        }
        let finished: Vec<_> = self
            .file_operations
            .pastes
            .keys()
            .filter(|id| {
                !self
                    .file_operations
                    .requests
                    .values()
                    .any(|(request, _)| request.logical_id == **id)
                    && !self
                        .file_operations
                        .conflicts
                        .iter()
                        .any(|(request, _, _)| request.logical_id == **id)
            })
            .copied()
            .collect();
        for id in finished {
            if let Some(paste) = self.file_operations.pastes.remove(&id) {
                paste.finish(cx);
            }
        }
    }

    /// Opens the next queued file-operation dialog. Collisions covered by
    /// "Apply to all remaining" are answered here without one.
    pub(super) fn open_pending_filesystem_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_operations.open_dialog.is_some()
            || self.popover_host.read(cx).has_open_popover()
        {
            return;
        }
        let focus_return = window.focused(cx);
        loop {
            if let Some((request, removals, ownership)) =
                self.file_operations.pending_unsaved.pop_front()
            {
                if request.cancellation.is_cancelled() {
                    self.file_operations.confirmations.remove(&request.id);
                    continue;
                }
                let files = crate::app::filesystem_unsaved_buffer_paths(&removals, cx)
                    .iter()
                    .map(|path| path_label(path))
                    .collect();
                let prompt_id = self.file_operations.allocate_prompt_id();
                self.file_operations.open_dialog = Some(OpenFilesystemDialog::UnsavedEdits {
                    prompt_id,
                    request,
                    removals,
                    ownership,
                });
                let kind =
                    PopoverKind::FilesystemUnsavedEditsConfirm(FilesystemUnsavedEditsPrompt {
                        prompt_id,
                        files,
                    });
                self.open_filesystem_dialog(kind, focus_return, window, cx);
                return;
            }
            if let Some((request, ownership)) = self.file_operations.pending_delete.pop_front() {
                if request.cancellation.is_cancelled() {
                    self.file_operations.confirmations.remove(&request.id);
                    continue;
                }
                let names = request
                    .operation
                    .sources()
                    .iter()
                    .map(|path| path_label(path))
                    .collect();
                let prompt_id = self.file_operations.allocate_prompt_id();
                self.file_operations.open_dialog = Some(OpenFilesystemDialog::Delete {
                    prompt_id,
                    request,
                    ownership,
                });
                let kind = PopoverKind::DeletePermanentlyConfirm(DeletePermanentlyPrompt {
                    prompt_id,
                    names,
                });
                self.open_filesystem_dialog(kind, focus_return, window, cx);
                return;
            }
            // Editor saves for a Replace are still landing.
            if !self.file_operations.confirmations.is_empty() {
                return;
            }
            let Some((request, conflict, ownership)) = self.file_operations.conflicts.pop_front()
            else {
                break;
            };
            if request.cancellation.is_cancelled() {
                continue;
            }
            if let Some(choice) = self.file_operations.sticky_choice(&request, &conflict) {
                self.resubmit_resolved_conflict(request, conflict, ownership, choice, window, cx);
                continue;
            }
            let remaining = self
                .file_operations
                .conflicts
                .iter()
                .filter(|(queued, _, _)| queued.logical_id == request.logical_id)
                .count();
            self.file_operations
                .confirmations
                .insert(request.id, request.cancellation.clone());
            let prompt_id = self.file_operations.allocate_prompt_id();
            let prompt = FilesystemConflictPrompt {
                prompt_id,
                destination: conflict.destination.clone(),
                can_merge: conflict.can_merge,
                remaining,
                in_directory_merge: conflict.continuation.is_some(),
            };
            self.file_operations.open_dialog = Some(OpenFilesystemDialog::Conflict {
                prompt_id,
                request,
                conflict,
                ownership,
            });
            let kind = PopoverKind::FilesystemConflict(prompt);
            self.open_filesystem_dialog(kind, focus_return, window, cx);
            return;
        }
        self.finish_filesystem_pastes(cx);
    }

    fn open_filesystem_dialog(
        &mut self,
        kind: PopoverKind,
        focus_return: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let request = PopoverRequest::from(kind);
        let request = match focus_return {
            Some(focus) => request.returning_focus_to(focus),
            None => request,
        };
        self.open_popover_centered(request, window, cx);
    }

    /// Resubmits one collision with its answer. Source and destination come
    /// from the conflict itself, so explorer changes meanwhile are safe.
    fn resubmit_resolved_conflict(
        &mut self,
        mut request: Request,
        conflict: Conflict,
        ownership: Option<u64>,
        choice: ConflictChoice,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if choice == ConflictChoice::Skip && conflict.continuation.is_none() {
            return;
        }
        let intent = match request.operation {
            Operation::Transfer { intent, .. } => intent,
            Operation::Rename { .. } => gitcomet_core::filesystem::TransferIntent::Move,
            _ => gitcomet_core::filesystem::TransferIntent::Copy,
        };
        if let Some(continuation) = conflict.continuation {
            request = *continuation;
        } else {
            request.operation = if matches!(request.operation, Operation::Rename { .. }) {
                Operation::Rename {
                    source: conflict.source,
                    name: conflict.destination.file_name().unwrap().to_owned(),
                }
            } else {
                Operation::Transfer {
                    sources: vec![conflict.source],
                    destination: conflict.destination.parent().unwrap().to_path_buf(),
                    intent,
                }
            };
        }
        request.id = OperationId::allocate();
        request.resolutions.insert(
            conflict.destination,
            ConflictResolution {
                expected: conflict.version,
                choice,
            },
        );
        self.submit_filesystem_operation(request, ownership, window, cx);
    }

    pub(in crate::view) fn resolve_filesystem_conflict(
        &mut self,
        prompt_id: u64,
        choice: ConflictChoice,
        apply_to_all: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(OpenFilesystemDialog::Conflict {
            request,
            conflict,
            ownership,
            ..
        }) = self.file_operations.take_open_dialog(prompt_id)
        else {
            return;
        };
        self.file_operations.confirmations.remove(&request.id);
        if choice == ConflictChoice::Cancel || request.cancellation.is_cancelled() {
            self.file_operations.conflicts.clear();
            self.file_operations.sticky.clear();
        } else {
            if apply_to_all {
                self.file_operations.sticky.insert(
                    request.logical_id,
                    (choice, BTreeSet::from([conflict.destination.clone()])),
                );
            }
            self.resubmit_resolved_conflict(request, conflict, ownership, choice, window, cx);
        }
        self.finish_filesystem_pastes(cx);
        cx.notify();
    }

    pub(in crate::view) fn resolve_delete_permanently(
        &mut self,
        prompt_id: u64,
        confirmed: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(OpenFilesystemDialog::Delete {
            mut request,
            ownership,
            ..
        }) = self.file_operations.take_open_dialog(prompt_id)
        else {
            return;
        };
        self.file_operations.confirmations.remove(&request.id);
        if !confirmed || request.cancellation.is_cancelled() {
            self.finish_filesystem_pastes(cx);
        } else {
            if let Operation::DeletePermanently { confirmed, .. } = &mut request.operation {
                *confirmed = true;
            }
            self.submit_filesystem_operation(request, ownership, window, cx);
        }
        cx.notify();
    }

    pub(in crate::view) fn resolve_filesystem_unsaved_edits(
        &mut self,
        prompt_id: u64,
        choice: FilesystemUnsavedEditsChoice,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(OpenFilesystemDialog::UnsavedEdits {
            request,
            removals,
            ownership,
            ..
        }) = self.file_operations.take_open_dialog(prompt_id)
        else {
            return;
        };
        self.file_operations.confirmations.remove(&request.id);
        if choice == FilesystemUnsavedEditsChoice::Cancel || request.cancellation.is_cancelled() {
            self.finish_filesystem_pastes(cx);
            cx.notify();
            return;
        }
        let save = choice == FilesystemUnsavedEditsChoice::Save;
        crate::app::resolve_filesystem_buffers(&removals, save, cx);
        if save {
            // Saves precede the operation in the single filesystem executor;
            // do not allow it to run if a save fails.
            self.wait_for_filesystem_saves(request, removals, ownership, window, cx);
        } else {
            self.enqueue_filesystem_operation(request, ownership, cx);
        }
        cx.notify();
    }

    /// Esc, the scrim, or any other close without a button counts as Cancel.
    pub(in crate::view) fn filesystem_dialog_dismissed(
        &mut self,
        prompt_id: u64,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(dialog) = self.file_operations.take_open_dialog(prompt_id) else {
            return;
        };
        self.file_operations
            .confirmations
            .remove(&dialog.request().id);
        if matches!(dialog, OpenFilesystemDialog::Conflict { .. }) {
            self.file_operations.conflicts.clear();
            self.file_operations.sticky.clear();
        }
        self.finish_filesystem_pastes(cx);
        cx.notify();
    }

    /// Another popover replaced the dialog; ask again once the screen is free.
    pub(in crate::view) fn filesystem_dialog_displaced(
        &mut self,
        prompt_id: u64,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(dialog) = self.file_operations.take_open_dialog(prompt_id) else {
            return;
        };
        let operations = &mut self.file_operations;
        match dialog {
            OpenFilesystemDialog::Conflict {
                request,
                conflict,
                ownership,
                ..
            } => {
                operations.confirmations.remove(&request.id);
                operations
                    .conflicts
                    .push_front((request, conflict, ownership));
            }
            OpenFilesystemDialog::Delete {
                request, ownership, ..
            } => operations.pending_delete.push_front((request, ownership)),
            OpenFilesystemDialog::UnsavedEdits {
                request,
                removals,
                ownership,
                ..
            } => operations
                .pending_unsaved
                .push_front((request, removals, ownership)),
        }
        cx.notify();
    }

    pub(in crate::view) fn notify_if_filesystem_dialog_queued(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_operations.has_queued_dialog() {
            cx.notify();
        }
    }
}
