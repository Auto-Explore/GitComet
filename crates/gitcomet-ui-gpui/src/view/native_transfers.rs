use super::*;
use gitcomet_core::filesystem::{
    Cancellation, Operation, OperationId, OutboundReceipt, Request, TransferIntent,
};
use std::collections::BTreeMap;

#[derive(Default)]
struct NativeTransfers {
    clipboard: std::collections::BTreeSet<u64>,
    preparing: BTreeMap<u64, Cancellation>,
    ready: BTreeMap<u64, (OutboundReceipt, bool)>,
}
impl gpui::Global for NativeTransfers {}

pub(super) fn active(cx: &gpui::App) -> bool {
    if cx.has_pending_native_file_transfers() {
        return true;
    }
    cx.try_global::<NativeTransfers>().is_some_and(|t| {
        !t.preparing.is_empty() || t.ready.keys().any(|id| !t.clipboard.contains(id))
    })
}

pub(super) fn cancel_preparing(cx: &mut gpui::App) {
    cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
        for cancellation in transfers.preparing.values() {
            cancellation.cancel();
        }
    });
}

pub(super) fn mark_handled(paths: &[std::path::PathBuf], cx: &mut gpui::App) -> bool {
    cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
        let Some((_, handled)) = transfers
            .ready
            .iter_mut()
            .filter(|(id, _)| !transfers.clipboard.contains(id))
            .map(|(_, transfer)| transfer)
            .find(|(receipt, _)| {
                !paths.is_empty()
                    && paths
                        .iter()
                        .all(|p| receipt.paths.iter().any(|source| p.starts_with(source)))
            })
        else {
            return false;
        };
        *handled = true;
        true
    })
}

impl GitCometView {
    #[cfg(target_os = "windows")]
    pub(in crate::view) fn prepare_native_cut(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        let id = OperationId::allocate();
        let cancellation = Cancellation::default();
        let clipboard_before = crate::clipboard::snapshot(cx);
        cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
            transfers.preparing.insert(id.0, cancellation.clone());
        });
        cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                gitcomet_core::filesystem::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .prepare_outbound(id, paths, &cancellation)
            })
            .await;
            let pending = cx.update(|cx| {
                cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
                    transfers.preparing.remove(&id.0)
                })
            });
            if pending.is_none_or(|c| c.is_cancelled()) {
                return;
            }
            let _ = view.update(cx, |this, cx| {
                if crate::clipboard::snapshot(cx) != clipboard_before {
                    return;
                }
                match result {
                    Ok(receipt) => {
                        crate::clipboard::write_files_owned(
                            cx,
                            receipt.paths.clone(),
                            TransferIntent::Move,
                            id.0,
                        );
                        cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
                            transfers.clipboard.insert(id.0);
                            transfers.ready.insert(id.0, (receipt, false));
                        });
                    }
                    Err(error) => {
                        this.push_toast(components::ToastKind::Error, error.to_string(), cx)
                    }
                }
            });
        })
        .detach();
    }

    pub(in crate::view) fn prepare_native_drag(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        intent: gpui::FileTransferOperation,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Task<Option<gpui::ExternalDragPayload>> {
        let id = OperationId::allocate();
        let cancellation = Cancellation::default();
        cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
            transfers.preparing.insert(id.0, cancellation.clone());
        });
        cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                gitcomet_core::filesystem::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .prepare_outbound(id, paths, &cancellation)
            })
            .await;
            let pending = cx.update(|cx| {
                cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
                    transfers.preparing.remove(&id.0)
                })
            });
            if pending.is_none_or(|c| c.is_cancelled()) {
                return None;
            }
            view.update(cx, |this, cx| match result {
                Ok(receipt) => {
                    let payload = gpui::FileDragPaths::new(
                        receipt
                            .paths
                            .iter()
                            .cloned()
                            .zip(receipt.directories.iter().copied()),
                    )
                    .with_transfer(intent, id.0);
                    cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
                        transfers.ready.insert(id.0, (receipt, false));
                    });
                    Some(gpui::ExternalDragPayload::Files(payload))
                }
                Err(error) => {
                    this.push_toast(components::ToastKind::Error, error.to_string(), cx);
                    None
                }
            })
            .ok()
            .flatten()
        })
    }

    pub(super) fn process_native_transfers(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        for completion in cx.take_file_transfer_completions() {
            let receipt = cx.update_default_global::<NativeTransfers, _>(|transfers, _| {
                transfers.clipboard.remove(&completion.files.ownership);
                transfers.ready.remove(&completion.files.ownership)
            });
            let Some((receipt, handled)) = receipt else {
                continue;
            };
            let intent = completion.operation.map(|operation| {
                if operation == gpui::FileTransferOperation::Move {
                    TransferIntent::Move
                } else {
                    TransferIntent::Copy
                }
            });
            self.submit_filesystem_operation(
                Request::new(Operation::CompleteOutbound {
                    receipt,
                    intent,
                    source_removed: completion.source_removed || handled,
                }),
                Some(completion.files.ownership),
                window,
                cx,
            );
        }
    }
}
