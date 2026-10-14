use super::DocumentsView;
use super::buffers::{DocumentBuffer, DocumentOwner};
use crate::view::panes::DiffRenderer;
use gitcomet_core::filesystem::{DiskVersion, OperationId, PathChange};
use gitcomet_state::model::RepoId;
use gpui::{App, EntityId, WeakEntity};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default)]
pub(super) struct DocumentPresenters {
    repositories: BTreeMap<EntityId, WeakEntity<DiffRenderer>>,
    documents: Vec<WeakEntity<DocumentsView>>,
}

impl DocumentOwner {
    pub fn register_repository(&self, presenter: WeakEntity<DiffRenderer>) {
        self.presenters
            .borrow_mut()
            .repositories
            .insert(presenter.entity_id(), presenter);
    }

    pub fn register_documents(&self, presenter: WeakEntity<DocumentsView>) {
        self.presenters.borrow_mut().documents.push(presenter);
    }

    fn repositories(&self) -> Vec<WeakEntity<DiffRenderer>> {
        let mut presenters = self.presenters.borrow_mut();
        presenters
            .repositories
            .retain(|_, presenter| presenter.upgrade().is_some());
        presenters.repositories.values().cloned().collect()
    }

    fn documents(&self) -> Vec<WeakEntity<DocumentsView>> {
        let mut presenters = self.presenters.borrow_mut();
        presenters
            .documents
            .retain(|presenter| presenter.upgrade().is_some());
        presenters.documents.clone()
    }

    pub(in crate::view) fn adopt(&self, document: DocumentBuffer, cx: &mut App) {
        document.detach(cx);
        if !document.read().dirty() && document.read().saving().is_none() {
            self.release_clean(document.id());
            return;
        }
        if let Some(documents) = self.documents().first().cloned() {
            cx.defer(move |cx| {
                let _ = documents.update(cx, |documents, cx| documents.adopt(document, cx));
            });
        }
    }

    pub fn flush(&self, repository: Option<RepoId>, cx: &mut App) -> bool {
        for presenter in self.repositories() {
            let _ = presenter.update(cx, |pane, cx| {
                if repository.is_none_or(|repository| {
                    pane.editor_documents
                        .active()
                        .is_some_and(|document| document.read().repository() == Some(repository))
                }) {
                    pane.flush_file_editor_buffer(cx);
                }
            });
        }
        !self.saves_drained(repository)
    }

    pub fn resolve(
        &self,
        repository: Option<RepoId>,
        paths: Option<&[PathBuf]>,
        save: bool,
        cx: &mut App,
    ) -> bool {
        let mut resolved = true;
        for presenter in self.repositories() {
            if let Ok(saved) = presenter.update(cx, |pane, cx| {
                if let Some(paths) = paths {
                    pane.filesystem_resolve_unsaved(paths, save, cx);
                    true
                } else if save {
                    if let Some(repo) = repository {
                        pane.save_file_edits_for_repo(repo, cx)
                    } else {
                        pane.save_all_file_edits(cx)
                    }
                } else {
                    if let Some(repo) = repository {
                        pane.discard_file_edits_for_repo(repo, cx);
                    } else {
                        pane.discard_all_file_edits(cx);
                    }
                    true
                }
            }) {
                resolved &= saved;
            }
        }
        for documents in self
            .documents()
            .into_iter()
            .filter(|_| repository.is_none())
        {
            let _ = documents.update(cx, |documents, cx| {
                if let Some(paths) = paths {
                    documents.resolve_presented(paths, save, cx);
                } else if save {
                    documents.save_all(cx);
                } else {
                    documents.discard_all(cx);
                }
            });
        }
        resolved
    }

    pub fn operations_drained(&self, cx: &App) -> bool {
        self.repositories().iter().all(|presenter| {
            presenter
                .upgrade()
                .is_none_or(|presenter| presenter.read(cx).filesystem_saves_drained())
        })
    }

    pub fn finish_saves(&self, state: &gitcomet_state::model::AppState, cx: &mut App) {
        let mut completed = std::collections::BTreeSet::new();
        for presenter in self.repositories() {
            if let Ok(receipts) =
                presenter.update(cx, |pane, cx| pane.process_file_editor_saves(state, cx))
            {
                completed.extend(receipts);
            }
        }
        for documents in self.documents() {
            if let Ok(receipts) = documents.update(cx, |documents, cx| {
                documents.process_presented_saves(state, cx)
            }) {
                completed.extend(receipts);
            }
        }
        if !completed.is_empty()
            && let Some(store) = self.store.upgrade()
        {
            store.dispatch(gitcomet_state::msg::Msg::AcknowledgeFilesystemResults(
                completed.into_iter().collect(),
            ));
        }
    }

    pub fn pause_presenters(&self, operation: OperationId, cx: &mut App) {
        for presenter in self.repositories() {
            let _ = presenter.update(cx, |pane, cx| pane.filesystem_pause(operation, cx));
        }
        for documents in self.documents() {
            let _ = documents.update(cx, |documents, cx| documents.pause_presented(operation, cx));
        }
    }

    pub fn finish_presenters(
        &self,
        operation: OperationId,
        changes: &[PathChange],
        versions: &BTreeMap<PathBuf, DiskVersion>,
        cx: &mut App,
    ) {
        for documents in self.documents() {
            let _ = documents.update(cx, |documents, cx| {
                documents.finish_presented(operation, changes, versions, cx)
            });
        }
        for presenter in self.repositories() {
            let _ = presenter.update(cx, |pane, cx| {
                pane.filesystem_finish(operation, changes, versions, cx)
            });
        }
    }
}
