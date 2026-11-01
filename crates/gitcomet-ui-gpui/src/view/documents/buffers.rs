use super::disk::{DiskIdentity, PendingWrite};
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity, OperationId, PathChange, Request};
use gitcomet_core::text_format::{FormatSource, LineEndingStats, SideTextFormat};
use gitcomet_state::diff_session::DiffViewId;
use gitcomet_state::model::RepoId;
use gitcomet_state::{msg::Msg, store::AppStore};
use gpui::{App, AppContext, Entity, SharedString};
use std::cell::{Cell, Ref, RefCell};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::view) struct BufferId(u64);

#[derive(Clone, Copy, Debug)]
pub(in crate::view) enum SaveOrigin {
    Detached,
    Repository {
        repo_id: RepoId,
        lifetime: u64,
        session: Option<DiffViewId>,
    },
}

#[derive(Clone, Debug)]
enum BufferRead {
    Empty,
    Loading {
        generation: u64,
        previous_error: Option<SharedString>,
        blocks_editing: bool,
    },
    Ready,
    Failed(SharedString),
}

#[derive(Clone, Debug)]
pub(in crate::view) struct BufferSave {
    pub operation: OperationId,
    pub destination: PathBuf,
    pub fingerprint: u64,
    disposition: SaveDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SaveDisposition {
    KeepEdits,
    Discarded,
}

impl BufferSave {
    pub fn new(operation: OperationId, destination: PathBuf, fingerprint: u64) -> Self {
        Self {
            operation,
            destination,
            fingerprint,
            disposition: SaveDisposition::KeepEdits,
        }
    }
}

#[derive(Debug)]
pub(in crate::view) struct BufferState {
    identity: DocumentIdentity,
    repository: Option<RepoId>,
    version: Option<DiskVersion>,
    text_format: Option<SideTextFormat>,
    source_text_format: Option<SideTextFormat>,
    disk: DiskIdentity,
    retained: Option<RetainedText>,
    first_dirty_line: Option<u32>,
    save_error: Option<SharedString>,
    write_error: Option<SharedString>,
    saved_fingerprint: Option<u64>,
    current_fingerprint: Option<u64>,
    read: BufferRead,
    dirty: bool,
    saving: Option<BufferSave>,
    failed_destination: Option<PathBuf>,
    pauses: BTreeSet<OperationId>,
    generation: u64,
}

impl BufferState {
    fn new(identity: DocumentIdentity, repository: Option<RepoId>) -> Self {
        Self {
            identity,
            repository,
            version: None,
            text_format: None,
            source_text_format: None,
            disk: DiskIdentity::default(),
            retained: None,
            first_dirty_line: None,
            save_error: None,
            write_error: None,
            saved_fingerprint: None,
            current_fingerprint: None,
            read: BufferRead::Empty,
            dirty: false,
            saving: None,
            failed_destination: None,
            pauses: BTreeSet::new(),
            generation: 0,
        }
    }

    pub fn identity(&self) -> &DocumentIdentity {
        &self.identity
    }
    pub fn repository(&self) -> Option<RepoId> {
        self.repository
    }
    pub fn version(&self) -> &Option<DiskVersion> {
        &self.version
    }
    pub fn text_format(&self) -> Option<SideTextFormat> {
        self.text_format
    }
    pub fn source_text_format(&self) -> Option<SideTextFormat> {
        self.source_text_format
    }
    pub fn disk(&self) -> &DiskIdentity {
        &self.disk
    }
    pub fn first_dirty_line(&self) -> Option<u32> {
        self.first_dirty_line
    }
    pub fn save_error(&self) -> &Option<SharedString> {
        &self.save_error
    }
    pub fn saved_fingerprint(&self) -> Option<u64> {
        self.saved_fingerprint
    }
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    pub fn saving(&self) -> &Option<BufferSave> {
        &self.saving
    }
    pub fn failed_destination(&self) -> &Option<PathBuf> {
        &self.failed_destination
    }
    pub fn pauses(&self) -> &BTreeSet<OperationId> {
        &self.pauses
    }

    fn begin_read(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.read = BufferRead::Loading {
            generation: self.generation,
            previous_error: self.error().cloned(),
            blocks_editing: true,
        };
        self.generation
    }

    pub fn accepts_read(&self, generation: u64) -> bool {
        self.generation == generation
            && matches!(self.read, BufferRead::Loading { generation: current, .. } if current == generation)
    }

    #[cfg(test)]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn loading(&self) -> bool {
        matches!(
            self.read,
            BufferRead::Loading {
                blocks_editing: true,
                ..
            }
        )
    }

    pub fn error(&self) -> Option<&SharedString> {
        if self.write_error.is_some() {
            return self.write_error.as_ref();
        }
        match &self.read {
            BufferRead::Failed(error) => Some(error),
            BufferRead::Loading { previous_error, .. } => previous_error.as_ref(),
            BufferRead::Empty | BufferRead::Ready => None,
        }
    }

    fn unsaved(&self) -> bool {
        if self.repository.is_some() {
            self.dirty && !self.loading()
        } else {
            self.dirty || self.saving.is_some()
        }
    }

    fn visible_change(&self) -> (DocumentIdentity, Option<RepoId>, bool, bool) {
        (
            self.identity.clone(),
            self.repository,
            self.dirty,
            self.saving.is_some(),
        )
    }
}

#[derive(Clone, Debug)]
struct RetainedText {
    pub text: SharedString,
    pub cursor: usize,
    pub fingerprint: u64,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct StashedFileEdit {
    pub document: DocumentBuffer,
    pub text: SharedString,
    pub cursor: usize,
    pub text_fingerprint: u64,
}

impl StashedFileEdit {
    pub fn is_dirty(&self) -> bool {
        self.document.read().dirty
    }
}

#[derive(Debug)]
pub(in crate::view) struct DocumentChanges {
    revision: u64,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct DocumentBuffer {
    id: BufferId,
    state: Rc<RefCell<BufferState>>,
    changes: Entity<DocumentChanges>,
}

impl DocumentBuffer {
    pub fn id(&self) -> BufferId {
        self.id
    }

    pub fn read(&self) -> Ref<'_, BufferState> {
        self.state.borrow()
    }

    pub fn update_disk(&self, update: impl FnOnce(&mut DiskIdentity)) {
        update(&mut self.state.borrow_mut().disk);
    }

    fn notify(&self, cx: &mut App) {
        self.changes.update(cx, |changes, cx| {
            changes.revision = changes.revision.wrapping_add(1);
            cx.notify();
        });
    }

    pub fn begin_read(&self, blocks_editing: bool, cx: &mut App) -> u64 {
        self.update(cx, |state| {
            let generation = state.begin_read();
            if let BufferRead::Loading {
                blocks_editing: blocks,
                ..
            } = &mut state.read
            {
                *blocks = blocks_editing;
            }
            generation
        })
    }

    pub fn invalidate_read(&self) {
        let mut state = self.state.borrow_mut();
        state.generation = state.generation.wrapping_add(1);
    }

    pub fn finish_read(
        &self,
        generation: u64,
        version: DiskVersion,
        format: Option<SideTextFormat>,
        disk: DiskIdentity,
        cx: &mut App,
    ) -> bool {
        self.update(cx, |state| {
            if !state.accepts_read(generation) {
                return false;
            }
            state.version = Some(version);
            state.text_format = format;
            state.source_text_format = format;
            state.disk = disk;
            state.read = BufferRead::Ready;
            state.write_error = None;
            state.failed_destination = None;
            true
        })
    }

    pub fn fail_read(&self, generation: u64, error: SharedString, cx: &mut App) {
        self.update(cx, |state| {
            if state.accepts_read(generation) {
                state.read = BufferRead::Failed(error);
            }
        });
    }

    pub fn edit(&self, fingerprint: u64, edited_row: Option<u32>, cx: &mut App) {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.loading() {
                return;
            }
            state.current_fingerprint = Some(fingerprint);
            let dirty = state.saved_fingerprint != Some(fingerprint);
            let changed = state.dirty != dirty;
            state.dirty = dirty;
            state.first_dirty_line = if dirty {
                edited_row.map_or(state.first_dirty_line, |row| {
                    Some(
                        state
                            .first_dirty_line
                            .map_or(row, |current| current.min(row)),
                    )
                })
            } else {
                None
            };
            changed
        };
        if changed {
            self.notify(cx);
        }
    }

    pub fn seat_text(&self, fingerprint: u64, baseline: Option<u64>, cx: &mut App) {
        self.update(cx, |state| {
            state.current_fingerprint = Some(fingerprint);
            state.saved_fingerprint = Some(baseline.unwrap_or(fingerprint));
            state.dirty = state.saved_fingerprint != Some(fingerprint);
            state.first_dirty_line = None;
        });
    }

    pub fn capture_fingerprint(&self, fingerprint: u64, cx: &mut App) {
        if self.read().current_fingerprint.is_some() {
            self.edit(fingerprint, None, cx);
        }
    }

    pub fn retain_text(&self, text: SharedString, cursor: usize, fingerprint: u64) {
        let mut state = self.state.borrow_mut();
        state.retained = (state.current_fingerprint.is_some()
            && (state.dirty || state.saving.is_some()))
        .then_some(RetainedText {
            text,
            cursor,
            fingerprint,
        });
    }

    pub fn stashed(&self) -> Option<StashedFileEdit> {
        let state = self.read();
        let retained = state.retained.as_ref()?;
        Some(StashedFileEdit {
            document: self.clone(),
            text: retained.text.clone(),
            cursor: retained.cursor,
            text_fingerprint: retained.fingerprint,
        })
    }

    pub fn begin_save(&self, save: BufferSave, write: PendingWrite, cx: &mut App) -> bool {
        self.update(cx, |state| {
            if state.loading()
                || state.current_fingerprint.is_none()
                || !state.pauses.is_empty()
                || state.saving.is_some()
            {
                return false;
            }
            state.save_error = None;
            state.disk.note_pending_write(write);
            state.saving = Some(save);
            true
        })
    }

    pub fn finish_save(
        &self,
        operation: OperationId,
        result: Result<Option<DiskVersion>, SharedString>,
        cx: &mut App,
    ) -> bool {
        self.update(cx, |state| {
            if !state
                .saving
                .as_ref()
                .is_some_and(|save| save.operation == operation)
            {
                return false;
            }
            let save = state.saving.take().expect("matching save");
            if save.disposition == SaveDisposition::Discarded {
                return true;
            }
            match result {
                Ok(version) => {
                    state.identity = DocumentIdentity(save.destination);
                    if version.is_some() {
                        state.version = version;
                    }
                    state.saved_fingerprint = Some(save.fingerprint);
                    state.dirty = state
                        .current_fingerprint
                        .is_some_and(|fingerprint| fingerprint != save.fingerprint);
                    if !state.dirty {
                        state.first_dirty_line = None;
                        state.retained = None;
                    }
                    state.write_error = None;
                    state.failed_destination = None;
                }
                Err(error) => {
                    state.failed_destination = Some(save.destination);
                    state.write_error = Some(error);
                }
            }
            true
        })
    }

    pub fn discard(&self, cx: &mut App) {
        self.update(cx, |state| {
            state.retained = None;
            state.dirty = false;
            state.first_dirty_line = None;
            state.save_error = None;
            state.write_error = None;
            state.saved_fingerprint = None;
            state.current_fingerprint = None;
            if let Some(save) = &mut state.saving {
                save.disposition = SaveDisposition::Discarded;
            }
        });
    }

    pub fn prepare_reload(&self, cx: &mut App) {
        self.discard(cx);
        self.begin_read(true, cx);
    }

    pub fn set_disk_version(&self, version: Option<DiskVersion>, cx: &mut App) {
        self.update(cx, |state| state.version = version);
    }

    pub fn change_text_format(&self, format: SideTextFormat, cx: &mut App) {
        self.update(cx, |state| {
            state.text_format = Some(format);
            state.dirty = true;
            state.saved_fingerprint = None;
            state.save_error = None;
        });
    }

    pub fn follow_format_source(&self, source: FormatSource, cx: &mut App) {
        self.update(cx, |state| {
            if let Some(format) = &mut state.text_format {
                format.source = source;
            }
        });
    }

    pub fn set_line_endings(&self, line_endings: LineEndingStats, cx: &mut App) {
        self.update(cx, |state| {
            if let Some(format) = &mut state.text_format {
                format.line_endings = line_endings;
            }
        });
    }

    pub fn set_save_error(&self, error: Option<SharedString>, cx: &mut App) {
        self.update(cx, |state| state.save_error = error);
    }

    pub fn report_write_error(&self, error: SharedString, cx: &mut App) {
        self.update(cx, |state| state.write_error = Some(error));
    }

    pub fn dismiss_error(&self, cx: &mut App) {
        self.update(cx, |state| {
            state.save_error = None;
            state.write_error = None;
            if matches!(state.read, BufferRead::Failed(_)) {
                state.read = BufferRead::Ready;
            }
        });
    }

    pub fn clear_retained(&self) {
        self.state.borrow_mut().retained = None;
    }

    pub fn attach(&self, repository: RepoId, cx: &mut App) {
        self.update(cx, |state| state.repository = Some(repository));
    }

    pub fn pause(&self, operation: OperationId) {
        self.state.borrow_mut().pauses.insert(operation);
    }

    pub fn resume(&self, operation: OperationId) -> bool {
        let mut state = self.state.borrow_mut();
        state.pauses.remove(&operation) && state.pauses.is_empty() && state.loading()
    }

    pub fn retarget(
        &self,
        changes: &[PathChange],
        versions: &BTreeMap<PathBuf, DiskVersion>,
        cx: &mut App,
    ) {
        self.update(cx, |state| {
            state.identity = state.identity.retarget(changes);
            if let Some(version) = &state.version
                && let Some(next) = versions
                    .get(&state.identity.0)
                    .filter(|next| next.same_contents(version))
            {
                state.version = Some(next.clone());
            }
            if let Some(save) = &mut state.saving {
                save.destination = DocumentIdentity(save.destination.clone())
                    .retarget(changes)
                    .0;
            }
        });
    }

    pub fn detach(&self, cx: &mut App) {
        self.update(cx, |state| state.repository = None);
    }

    fn update<R>(&self, cx: &mut App, update: impl FnOnce(&mut BufferState) -> R) -> R {
        let (result, changed) = {
            let mut state = self.state.borrow_mut();
            let previous = state.visible_change();
            let result = update(&mut state);
            (result, previous != state.visible_change())
        };
        if changed {
            self.notify(cx);
        }
        result
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DocumentOwner {
    buffers: Rc<RefCell<BTreeMap<BufferId, DocumentBuffer>>>,
    next_id: Rc<Cell<u64>>,
    changes: Entity<DocumentChanges>,
    pub(super) presenters: Rc<RefCell<super::presenters::DocumentPresenters>>,
    pub(super) store: std::sync::Weak<AppStore>,
}

impl DocumentOwner {
    pub fn new(store: std::sync::Weak<AppStore>, cx: &mut App) -> Self {
        Self {
            buffers: Rc::new(RefCell::new(BTreeMap::new())),
            next_id: Rc::new(Cell::new(0)),
            changes: cx.new(|_| DocumentChanges { revision: 0 }),
            presenters: Rc::default(),
            store,
        }
    }

    /// The store's filesystem engine. Without a store nothing is saved, so
    /// reads share the process engine.
    pub(super) fn filesystem(
        &self,
    ) -> &'static std::sync::Mutex<gitcomet_core::filesystem::Filesystem> {
        self.store
            .upgrade()
            .map_or_else(gitcomet_core::filesystem::global, |store| {
                store.filesystem()
            })
    }

    pub(in crate::view) fn save(
        &self,
        document: &DocumentBuffer,
        origin: SaveOrigin,
        request: Request,
        fingerprint: u64,
        write: PendingWrite,
        cx: &mut App,
    ) -> bool {
        let Some(store) = self.store.upgrade() else {
            return false;
        };
        let gitcomet_core::filesystem::Operation::Save { path, worktree, .. } = &request.operation
        else {
            return false;
        };
        let admitted = match origin {
            SaveOrigin::Detached => document.read().repository.is_none() && worktree.is_none(),
            SaveOrigin::Repository {
                repo_id,
                lifetime,
                session,
            } => {
                let live = store.snapshot();
                let Some(repository) = live
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
                else {
                    return false;
                };
                let buffer = document.read();
                repository.has_working_tree()
                    && !repository.bare_mode_conversion.is_busy()
                    && buffer.repository == Some(repo_id)
                    && path == &buffer.identity.0
                    && path.starts_with(&repository.spec.workdir)
                    && worktree.as_ref() == Some(&repository.spec.workdir)
                    && session.is_none_or(|session| {
                        repository
                            .diff_sessions
                            .get(&session)
                            .is_some_and(|session| session.worktree().is_none())
                    })
            }
        };
        if !admitted {
            return false;
        }
        if !document.begin_save(
            BufferSave::new(request.id, path.clone(), fingerprint),
            write,
            cx,
        ) {
            return false;
        }
        store.dispatch(Msg::FilesystemRequest(request));
        true
    }

    pub(in crate::view) fn create(
        &self,
        identity: DocumentIdentity,
        repository: Option<RepoId>,
    ) -> DocumentBuffer {
        let next = self
            .next_id
            .get()
            .checked_add(1)
            .expect("buffer IDs exhausted");
        self.next_id.set(next);
        let buffer = DocumentBuffer {
            id: BufferId(next),
            state: Rc::new(RefCell::new(BufferState::new(identity, repository))),
            changes: self.changes.clone(),
        };
        self.buffers.borrow_mut().insert(buffer.id, buffer.clone());
        buffer
    }

    pub(in crate::view) fn changes(&self) -> &Entity<DocumentChanges> {
        &self.changes
    }

    pub fn revision(&self, cx: &App) -> u64 {
        self.changes.read(cx).revision
    }

    pub(in crate::view) fn release_clean(&self, id: BufferId) {
        let mut buffers = self.buffers.borrow_mut();
        if buffers.get(&id).is_some_and(|buffer| {
            let state = buffer.read();
            !state.dirty && state.saving.is_none()
        }) {
            buffers.remove(&id);
        }
    }

    #[cfg(test)]
    pub fn unsaved_paths(&self, repository: Option<RepoId>) -> Vec<PathBuf> {
        self.buffers
            .borrow()
            .values()
            .filter_map(|buffer| {
                let state = buffer.read();
                ((state.dirty || state.saving.is_some())
                    && repository.is_none_or(|repository| state.repository == Some(repository)))
                .then(|| state.identity.0.clone())
            })
            .collect()
    }

    pub fn dirty_paths(&self, repository: RepoId) -> Vec<PathBuf> {
        self.buffers
            .borrow()
            .values()
            .filter_map(|buffer| {
                let state = buffer.read();
                (state.repository == Some(repository) && state.dirty && !state.loading())
                    .then(|| state.identity.0.clone())
            })
            .collect()
    }

    pub fn unsaved_documents(&self) -> Vec<(DocumentIdentity, Option<RepoId>)> {
        self.buffers
            .borrow()
            .values()
            .filter_map(|buffer| {
                let state = buffer.read();
                state
                    .unsaved()
                    .then(|| (state.identity.clone(), state.repository))
            })
            .collect()
    }

    pub fn has_unsaved_within(&self, paths: &[PathBuf]) -> bool {
        self.buffers.borrow().values().any(|buffer| {
            let state = buffer.read();
            state.unsaved() && paths.iter().any(|path| state.identity.0.starts_with(path))
        })
    }

    pub fn saves_drained(&self, repository: Option<RepoId>) -> bool {
        self.buffers.borrow().values().all(|buffer| {
            let state = buffer.read();
            repository.is_some_and(|repository| state.repository != Some(repository))
                || state.saving.is_none()
        })
    }

    pub fn pending_paths(&self, repository: RepoId) -> Vec<PathBuf> {
        self.buffers
            .borrow()
            .values()
            .filter_map(|buffer| {
                let state = buffer.read();
                (state.repository == Some(repository) && state.saving.is_some())
                    .then(|| state.identity.0.clone())
            })
            .collect()
    }

    #[cfg(test)]
    pub fn detach_repository(&self, repository: RepoId, cx: &mut App) {
        for buffer in self.buffers.borrow().values() {
            buffer.update(cx, |state| {
                if state.repository == Some(repository) {
                    state.repository = None;
                }
            });
        }
    }

    #[cfg(test)]
    pub(in crate::view) fn at_path(&self, path: &Path) -> Vec<DocumentBuffer> {
        self.buffers
            .borrow()
            .values()
            .filter(|buffer| buffer.read().identity.0 == path)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn repository_document_save_refuses_bare_and_conversion_after_origin_was_captured(
        cx: &mut gpui::TestAppContext,
    ) {
        use gitcomet_core::domain::{RepoSpec, RepositoryMode};
        use gitcomet_core::filesystem::Operation;
        use gitcomet_state::model::{AppState, BareModeConversionState, Loadable, RepoState};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("edited.txt");
        std::fs::write(&path, "original").unwrap();
        let (store, _events) =
            AppStore::new_test(std::sync::Arc::new(crate::view::test_support::TestBackend));
        let store = std::sync::Arc::new(store);
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: directory.path().to_path_buf(),
            },
        );
        repo.open = Loadable::Ready(());
        repo.repository_mode = Some(RepositoryMode::WorkingTree);
        let origin = SaveOrigin::Repository {
            repo_id: repo.id,
            lifetime: repo.lifetime(),
            session: None,
        };
        let base = AppState {
            active_repo: Some(repo.id),
            repos: vec![repo],
            ..AppState::test_default()
        };
        store.replace_snapshot_for_test(std::sync::Arc::new(base.clone()));
        let pane = crate::view::pane_store::PaneStore::from(store.clone());
        assert!(
            pane.document_save_origin(RepoId(1), base.repos[0].lifetime())
                .is_some()
        );
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Arc::downgrade(&store), cx);
            let document = owner.create(DocumentIdentity(path.clone()), Some(RepoId(1)));
            document.seat_text(10, None, cx);
            document.edit(20, None, cx);
            document.retain_text("edited".into(), 3, 20);
            for (mode, conversion) in [
                (RepositoryMode::Bare, BareModeConversionState::Idle),
                (
                    RepositoryMode::WorkingTree,
                    BareModeConversionState::Converting,
                ),
            ] {
                let mut state = base.clone();
                state.repos[0].repository_mode = Some(mode);
                state.repos[0].bare_mode_conversion = conversion;
                store.replace_snapshot_for_test(std::sync::Arc::new(state));
                assert!(
                    pane.document_save_origin(RepoId(1), base.repos[0].lifetime())
                        .is_none()
                );
                let request = Request::new(Operation::Save {
                    path: path.clone(),
                    worktree: Some(directory.path().to_path_buf()),
                    contents: std::sync::Arc::from(b"edited".as_slice()),
                    expected: None,
                    overwrite: true,
                });
                assert!(!owner.save(&document, origin, request, 20, "edited".into(), cx));
                assert!(document.read().saving().is_none());
                assert!(document.read().dirty());
                assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
            }
        });
    }

    fn start_pending_save(document: &DocumentBuffer, cx: &mut App) {
        document.seat_text(10, None, cx);
        document.edit(20, None, cx);
        document.retain_text("edited".into(), 3, 20);
        let destination = document.read().identity.0.clone();
        assert!(document.begin_save(
            BufferSave::new(OperationId(1), destination, 20),
            "edited".into(),
            cx,
        ));
    }

    #[gpui::test]
    fn discarded_pending_save_does_not_resurrect_an_inactive_buffer(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let document = owner.create(DocumentIdentity(PathBuf::from("a.txt")), Some(RepoId(1)));
            start_pending_save(&document, cx);
            document.discard(cx);
            assert!(!owner.saves_drained(None));
            assert!(document.finish_save(OperationId(1), Ok(None), cx));
            assert!(!document.read().dirty);
            assert!(document.stashed().is_none());
            owner.release_clean(document.id());
            assert!(owner.at_path(Path::new("a.txt")).is_empty());
        });
    }

    #[gpui::test]
    fn pending_save_completion_preserves_a_newer_discard_read(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let document = owner.create(DocumentIdentity(PathBuf::from("a.txt")), Some(RepoId(1)));
            start_pending_save(&document, cx);
            document.discard(cx);
            let generation = document.begin_read(true, cx);
            assert!(document.finish_save(OperationId(1), Ok(None), cx));
            assert!(document.read().accepts_read(generation));
            document.edit(0, None, cx);
            assert!(
                !document.read().dirty,
                "a loading placeholder is not an edit"
            );
            assert!(document.read().loading());
        });
    }

    #[gpui::test]
    fn filesystem_dirty_query_keeps_pending_write_drain_separate(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let path = PathBuf::from("a.txt");
            let document = owner.create(DocumentIdentity(path.clone()), Some(RepoId(1)));
            start_pending_save(&document, cx);
            document.discard(cx);
            assert!(!owner.has_unsaved_within(std::slice::from_ref(&path)));
            assert!(!owner.saves_drained(None));
            document.detach(cx);
            assert!(owner.has_unsaved_within(std::slice::from_ref(&path)));
            assert!(!owner.saves_drained(None));
        });
    }

    #[gpui::test]
    fn retained_buffers_keep_distinct_ids_at_the_same_path(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let first = owner.create(DocumentIdentity(PathBuf::from("same.txt")), Some(RepoId(1)));
            let second = owner.create(DocumentIdentity(PathBuf::from("same.txt")), Some(RepoId(1)));
            first.update(cx, |state| state.dirty = true);
            second.update(cx, |state| state.dirty = true);
            owner.detach_repository(RepoId(1), cx);
            owner.release_clean(first.id());
            assert_ne!(first.id(), second.id());
            assert_eq!(owner.at_path(Path::new("same.txt")).len(), 2);
            assert!(owner.unsaved_paths(Some(RepoId(1))).is_empty());
            assert_eq!(owner.unsaved_paths(None).len(), 2);
        });
    }

    #[test]
    fn a_new_read_rejects_the_old_generation() {
        let mut state = BufferState::new(DocumentIdentity(PathBuf::from("file.txt")), None);
        let previous = state.begin_read();
        let current = state.begin_read();
        assert!(!state.accepts_read(previous));
        assert!(state.accepts_read(current));
        state.read = BufferRead::Ready;
        assert!(!state.accepts_read(current));
    }

    #[gpui::test]
    fn save_completion_keeps_newer_edits_and_failed_save_as_keeps_the_original_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let identity = DocumentIdentity(PathBuf::from("original.txt"));
            let document = owner.create(identity.clone(), Some(RepoId(1)));
            document.seat_text(10, None, cx);
            document.edit(20, Some(3), cx);
            document.retain_text("saved revision".into(), 4, 20);
            assert!(document.begin_save(
                BufferSave::new(OperationId(1), identity.0.clone(), 20),
                "saved revision".into(),
                cx
            ));
            document.edit(30, Some(5), cx);
            document.retain_text("newer revision".into(), 6, 30);
            assert!(!document.finish_save(OperationId(2), Ok(None), cx));
            assert_eq!(
                document.read().saving.as_ref().unwrap().operation,
                OperationId(1)
            );
            assert!(document.finish_save(OperationId(1), Ok(None), cx));
            assert!(document.read().dirty);
            assert_eq!(document.read().saved_fingerprint, Some(20));
            assert_eq!(document.stashed().unwrap().text.as_ref(), "newer revision");
            assert_eq!(document.read().first_dirty_line, Some(3));

            let destination = PathBuf::from("save-as.txt");
            assert!(document.begin_save(
                BufferSave::new(OperationId(2), destination.clone(), 30),
                "newer revision".into(),
                cx
            ));
            assert!(document.finish_save(OperationId(2), Err("write failed".into()), cx));
            document.detach(cx);
            owner.release_clean(document.id());
            assert_eq!(document.read().identity, identity);
            assert_eq!(document.read().saved_fingerprint, Some(20));
            assert_eq!(
                document.read().failed_destination.as_ref(),
                Some(&destination)
            );
            assert!(document.read().dirty);
            assert_eq!(owner.at_path(&identity.0)[0].id(), document.id());
        });
    }

    #[gpui::test]
    fn dirty_revision_changes_during_mutation_without_rendering(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let owner = DocumentOwner::new(std::sync::Weak::new(), cx);
            let document =
                owner.create(DocumentIdentity(PathBuf::from("file.txt")), Some(RepoId(1)));
            document.seat_text(10, None, cx);
            let clean_revision = owner.revision(cx);
            document.edit(20, None, cx);
            let dirty_revision = owner.revision(cx);
            assert_ne!(dirty_revision, clean_revision);
            assert_eq!(
                owner.dirty_paths(RepoId(1)),
                vec![PathBuf::from("file.txt")]
            );
            document.edit(30, None, cx);
            assert_eq!(owner.revision(cx), dirty_revision);
            document.edit(10, None, cx);
            assert_ne!(owner.revision(cx), dirty_revision);
            assert!(owner.dirty_paths(RepoId(1)).is_empty());
        });
    }
}
