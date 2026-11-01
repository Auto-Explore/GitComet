use super::buffers::{BufferId, DocumentBuffer, DocumentOwner, SaveOrigin, StashedFileEdit};
use super::disk::PendingWrite;
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity, OperationId, PathChange, Request};
use gitcomet_state::model::RepoId;
use gpui::App;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub(in crate::view) struct RepositoryDocuments {
    owner: DocumentOwner,
    active: Option<BufferId>,
    buffers: BTreeMap<BufferId, DocumentBuffer>,
    pauses: BTreeSet<OperationId>,
}

impl RepositoryDocuments {
    pub fn new(owner: DocumentOwner) -> Self {
        Self {
            owner,
            active: None,
            buffers: BTreeMap::new(),
            pauses: BTreeSet::new(),
        }
    }

    pub fn save(
        &self,
        document: &DocumentBuffer,
        origin: SaveOrigin,
        request: Request,
        fingerprint: u64,
        write: PendingWrite,
        cx: &mut App,
    ) -> bool {
        self.owner
            .save(document, origin, request, fingerprint, write, cx)
    }

    #[cfg(test)]
    pub fn retain_for_tests(
        &mut self,
        identity: DocumentIdentity,
        repository: RepoId,
        text: gpui::SharedString,
        cx: &mut App,
    ) {
        let document = self.owner.create(identity, Some(repository));
        document.seat_text(1, Some(2), cx);
        document.retain_text(text, 0, 1);
        self.buffers.insert(document.id(), document);
    }

    pub fn active(&self) -> Option<&DocumentBuffer> {
        self.active.and_then(|id| self.buffers.get(&id))
    }

    pub fn release(&mut self, cx: &mut App) {
        self.active = None;
        for (_, document) in std::mem::take(&mut self.buffers) {
            self.owner.adopt(document, cx);
        }
    }

    pub fn adopt(&self, document: DocumentBuffer, cx: &mut App) {
        self.owner.adopt(document, cx);
    }

    pub fn activate(&mut self, identity: DocumentIdentity, repository: RepoId, _cx: &mut App) {
        if let Some(previous) = self.active() {
            previous.invalidate_read();
        }
        let buffer = self
            .find(&identity)
            .unwrap_or_else(|| self.owner.create(identity, Some(repository)));
        for operation in &self.pauses {
            buffer.pause(*operation);
        }
        self.active = Some(buffer.id());
        self.buffers.insert(buffer.id(), buffer);
        self.release_inactive_clean();
    }

    pub fn find(&self, identity: &DocumentIdentity) -> Option<DocumentBuffer> {
        self.buffers
            .values()
            .find(|buffer| buffer.read().identity() == identity)
            .cloned()
    }

    pub fn all(&self) -> Vec<DocumentBuffer> {
        self.buffers.values().cloned().collect()
    }

    pub fn stashed(&self, identity: &DocumentIdentity) -> Option<StashedFileEdit> {
        self.find(identity)?.stashed()
    }

    pub fn stashes(&self) -> Vec<(DocumentIdentity, StashedFileEdit)> {
        self.buffers
            .values()
            .filter_map(|buffer| Some((buffer.read().identity().clone(), buffer.stashed()?)))
            .collect()
    }

    pub fn pending(&self) -> Vec<DocumentBuffer> {
        self.buffers
            .values()
            .filter(|buffer| buffer.read().saving().is_some())
            .cloned()
            .collect()
    }

    pub fn paused(&self) -> bool {
        !self.pauses.is_empty()
    }

    #[cfg(test)]
    pub fn paused_by(&self, operation: OperationId) -> bool {
        self.pauses.contains(&operation)
    }

    #[cfg(test)]
    pub fn deactivate(&mut self) {
        if let Some(document) = self.active() {
            document.invalidate_read();
        }
        self.active = None;
    }

    pub fn pause(&mut self, operation: OperationId, _cx: &mut App) {
        self.pauses.insert(operation);
        for buffer in self.buffers.values() {
            buffer.pause(operation);
        }
    }

    pub fn resume(&mut self, operation: OperationId, _cx: &mut App) {
        self.pauses.remove(&operation);
        for buffer in self.buffers.values() {
            buffer.resume(operation);
        }
    }

    pub fn discard(&mut self, identity: &DocumentIdentity, cx: &mut App) {
        if let Some(buffer) = self.find(identity) {
            buffer.discard(cx);
        }
        self.release_inactive_clean();
    }

    pub fn detach(&mut self, buffer: &DocumentBuffer, cx: &mut App) {
        self.buffers.remove(&buffer.id());
        if self.active == Some(buffer.id()) {
            self.active = None;
        }
        buffer.detach(cx);
        self.owner.release_clean(buffer.id());
    }

    pub fn release_inactive_clean(&mut self) {
        self.buffers.retain(|id, buffer| {
            let state = buffer.read();
            let keep = self.active == Some(*id) || state.dirty() || state.saving().is_some();
            if !keep {
                self.owner.release_clean(*id);
            }
            keep
        });
    }

    pub fn retarget(
        &mut self,
        changes: &[PathChange],
        versions: &BTreeMap<PathBuf, DiskVersion>,
        cx: &mut App,
    ) -> Vec<DocumentBuffer> {
        let mut buffers = self.all();
        buffers.sort_by_key(|buffer| {
            let state = buffer.read();
            (
                Some(buffer.id()) == self.active,
                state.dirty(),
                state.identity().retarget(changes) != *state.identity(),
            )
        });
        let mut by_path = BTreeMap::new();
        let mut detached = Vec::new();
        for buffer in buffers {
            buffer.retarget(changes, versions, cx);
            let path = buffer.read().identity().clone();
            if let Some(displaced) = by_path.insert(path, buffer) {
                self.detach(&displaced, cx);
                if displaced.read().dirty() {
                    detached.push(displaced);
                } else {
                    self.owner.release_clean(displaced.id());
                }
            }
        }
        detached
    }
}
