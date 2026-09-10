use super::*;
use gitcomet_core::filesystem::{
    Conflict, ConflictChoice, ConflictResolution, ItemOutcome, Operation, OperationId, Request,
};
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub(super) struct FileOperationsUi {
    requests: BTreeMap<OperationId, (Request, Option<u64>)>,
    pastes: BTreeMap<OperationId, crate::clipboard::PasteReceipt>,
    conflicts: VecDeque<(Request, Conflict, Option<u64>)>,
    prompting: bool,
    confirmations: BTreeMap<OperationId, gitcomet_core::filesystem::Cancellation>,
}

impl FileOperationsUi {
    pub(super) fn has_pending(&self) -> bool {
        !self.requests.is_empty()
            || self.prompting
            || !self.conflicts.is_empty()
            || !self.confirmations.is_empty()
            || !self.pastes.is_empty()
    }
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
        for (request, _) in self.file_operations.requests.values() {
            request.cancellation.cancel();
        }
        for cancellation in self.file_operations.confirmations.values() {
            cancellation.cancel();
        }
        for (request, _, _) in &self.file_operations.conflicts {
            request.cancellation.cancel();
        }
        self.file_operations.conflicts.clear();
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
        mut request: Request,
        ownership: Option<u64>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if request.cancellation.is_cancelled() {
            return;
        }
        if let Operation::DeletePermanently {
            confirmed: false, ..
        } = request.operation
        {
            self.file_operations
                .confirmations
                .insert(request.id, request.cancellation.clone());
            let answer = window.prompt(
                gpui::PromptLevel::Warning,
                "Delete permanently?",
                Some("These items will be permanently removed. This action cannot be undone."),
                &[
                    gpui::PromptButton::cancel("Cancel"),
                    gpui::PromptButton::new("Delete permanently"),
                ],
                cx,
            );
            cx.spawn_in(window, async move |view, cx| {
                let choice = answer.await.ok();
                let _ = view.update(cx, |this, _| {
                    this.file_operations.confirmations.remove(&request.id);
                });
                if choice == Some(1) && !request.cancellation.is_cancelled() {
                    if let Operation::DeletePermanently { confirmed, .. } = &mut request.operation {
                        *confirmed = true;
                    }
                    let _ = view.update_in(cx, |this, window, cx| {
                        this.submit_filesystem_operation(request, ownership, window, cx)
                    });
                }
            })
            .detach();
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
            let answer = window.prompt(
                gpui::PromptLevel::Warning,
                "These items contain unsaved edits",
                Some("Save the edits before continuing, or discard the unsaved buffers."),
                &[
                    gpui::PromptButton::cancel("Cancel"),
                    gpui::PromptButton::new("Save"),
                    gpui::PromptButton::new("Discard"),
                ],
                cx,
            );
            cx.spawn_in(window, async move |view, cx| {
                let choice = answer.await.ok();
                let _ = view.update(cx, |this, _| {
                    this.file_operations.confirmations.remove(&request.id);
                });
                if request.cancellation.is_cancelled() {
                    return;
                }
                let Some(choice @ (1 | 2)) = choice else {
                    return;
                };
                let _ = view.update_in(cx, |this, window, cx| {
                    crate::app::resolve_filesystem_buffers(&removals, choice == 1, cx);
                    if choice == 1 {
                        // Saves precede the operation in the single filesystem
                        // executor; do not allow it to run if a save fails.
                        this.wait_for_filesystem_saves(request, removals, ownership, window, cx);
                    } else {
                        this.enqueue_filesystem_operation(request, ownership, cx);
                    }
                });
            })
            .detach();
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
                    let _ = view.update(cx, |this, cx| { this.file_operations.confirmations.remove(&request.id); cx.notify(); });
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
                    });
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let _ = view.update(cx, |this, cx| {
                this.file_operations.confirmations.remove(&request.id);
                this.push_toast(components::ToastKind::Error, "Still waiting for editor saves. The file operation was cancelled; files were preserved.".into(), cx);
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

    pub(super) fn process_filesystem_results(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
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
                    ItemOutcome::Conflict(conflict) => self.file_operations.conflicts.push_back((
                        request.clone(),
                        conflict,
                        ownership,
                    )),
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
        }
        if self.file_operations.prompting || !self.file_operations.confirmations.is_empty() {
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
        let Some((mut request, conflict, ownership)) = self.file_operations.conflicts.pop_front()
        else {
            return;
        };
        self.file_operations.prompting = true;
        self.file_operations
            .confirmations
            .insert(request.id, request.cancellation.clone());
        let mut choices = vec![
            ConflictChoice::Cancel,
            ConflictChoice::KeepBoth,
            ConflictChoice::Replace,
            ConflictChoice::Skip,
        ];
        let mut buttons = vec![
            gpui::PromptButton::cancel("Cancel"),
            gpui::PromptButton::new("Keep Both"),
            gpui::PromptButton::new("Replace"),
            gpui::PromptButton::new("Skip"),
        ];
        if conflict.can_merge {
            choices.push(ConflictChoice::Merge);
            buttons.push(gpui::PromptButton::new("Merge"));
        }
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            "An item with this name already exists",
            Some(&conflict.destination.display().to_string()),
            &buttons,
            cx,
        );
        cx.spawn_in(window, async move |view, cx| {
            let choice = answer
                .await
                .ok()
                .and_then(|i| choices.get(i).copied())
                .unwrap_or(ConflictChoice::Cancel);
            let _ = view.update_in(cx, |this, window, cx| {
                this.file_operations.prompting = false;
                this.file_operations.confirmations.remove(&request.id);
                if choice == ConflictChoice::Cancel || request.cancellation.is_cancelled() {
                    this.file_operations.conflicts.clear();
                } else if choice != ConflictChoice::Skip || conflict.continuation.is_some() {
                    // Freeze source and destination from this specific conflict;
                    // changing explorer focus while the prompt is open is safe.
                    let intent = match request.operation {
                        Operation::Transfer { intent, .. } => intent,
                        Operation::Rename { .. } => gitcomet_core::filesystem::TransferIntent::Move,
                        _ => gitcomet_core::filesystem::TransferIntent::Copy,
                    };
                    if let Some(continuation) = conflict.continuation {
                        request = *continuation;
                    } else {
                        request.operation = if matches!(request.operation, Operation::Rename { .. })
                        {
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
                    this.submit_filesystem_operation(request, ownership, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
