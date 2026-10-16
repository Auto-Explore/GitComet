use super::panes::diff::TextDecodeRequest;
use super::panes::diff::file_editor::{file_editor_gutter_width, file_editor_line_for_visual_row};
use super::panes::diff::helpers::state::FileImagePreviewAnimationSide;
use super::panes::diff::preflight_worktree_file_for_editing;
use super::panes::diff::read_worktree_file_version_for_editing;
use super::*;
use crate::kit::{TextInput, TextInputOptions};
use buffers::{BufferId, BufferSave, DocumentBuffer, DocumentOwner, SaveOrigin};
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity, Operation, OperationId, Request};
use gitcomet_core::text_format::{SideKind, SideTextFormat};
use gitcomet_state::model::SidebarMode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) mod buffers;
#[cfg(test)]
mod diagram_tests;
pub(in crate::view) mod disk;
mod image_preview;
mod picker;
mod presenters;
pub(in crate::view) mod repository;
pub(in crate::view) mod text;
pub(crate) use picker::DocumentPicker;
use text::{encode_for_save, file_editor_text_fingerprint};

struct RecentDocuments(Entity<RecentDocumentList>);
impl gpui::Global for RecentDocuments {}
struct RecentDocumentList {
    paths: Vec<PathBuf>,
}

fn shared_recents(cx: &mut gpui::App) -> Entity<RecentDocumentList> {
    if let Some(recents) = cx.try_global::<RecentDocuments>() {
        return recents.0.clone();
    }
    let list = cx.new(|_| RecentDocumentList {
        paths: session::load().recent_documents,
    });
    cx.set_global(RecentDocuments(list.clone()));
    list
}

fn update_recent(path: PathBuf, remove: bool, cx: &mut gpui::App) {
    shared_recents(cx).update(cx, |recents, cx| {
        recents.paths.retain(|p| p != &path);
        if !remove {
            recents.paths.insert(0, path.clone());
        }
        recents.paths.truncate(session::MAX_RECENT_DOCUMENTS);
        cx.notify();
    });
    #[cfg(not(test))]
    session::enqueue_recent_document(path, remove);
}

pub(crate) struct DocumentsView {
    theme: AppTheme,
    root: WeakEntity<GitCometView>,
    ui_model: Entity<AppUiModel>,
    recents: Entity<RecentDocumentList>,
    owner: DocumentOwner,
    buffers: BTreeMap<BufferId, Entity<StandaloneBuffer>>,
    active: Option<BufferId>,
}

impl DocumentsView {
    pub(crate) fn document_owner(&self) -> DocumentOwner {
        self.owner.clone()
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        for buffer in self.buffers.values() {
            buffer.update(cx, |buffer, cx| {
                buffer.theme = theme;
                buffer.syntax_key = None;
                buffer.refresh_syntax(cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    pub(super) fn new(
        theme: AppTheme,
        root: WeakEntity<GitCometView>,
        ui_model: Entity<AppUiModel>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let owner = ui_model.read(cx).documents.clone();
        owner.register_documents(cx.entity().downgrade());
        Self {
            theme,
            root,
            ui_model,
            recents: shared_recents(cx),
            owner,
            buffers: BTreeMap::new(),
            active: None,
        }
    }

    fn insert_buffer(
        &mut self,
        path: PathBuf,
        initial: Option<DocumentBuffer>,
        cx: &mut gpui::Context<Self>,
    ) -> BufferId {
        let root = self.root.clone();
        let buffer = cx.new(|cx| {
            StandaloneBuffer::new(path, self.theme, root, self.ui_model.clone(), initial, cx)
        });
        let subscription = cx.observe(&buffer, |this, _, cx| {
            this.prune_clean_buffers(cx);
            cx.notify();
        });
        // The observer is released together with the buffer it watches.
        buffer.update(cx, |buffer, _| buffer.subscriptions.push(subscription));
        let id = buffer.read(cx).document.id();
        self.buffers.insert(id, buffer);
        id
    }

    pub(super) fn open(&mut self, path: PathBuf, display: bool, cx: &mut gpui::Context<Self>) {
        update_recent(path.clone(), false, cx);
        // Background opens are history entries until the user selects one.
        if !display {
            return;
        }
        let existing = self
            .buffers
            .iter()
            .filter(|(_, b)| b.read(cx).identity().0 == path)
            .max_by_key(|(_, b)| b.read(cx).dirty())
            .map(|(id, _)| *id);
        let id = if let Some(id) = existing {
            self.buffers[&id].update(cx, |buffer, cx| {
                if !buffer.dirty() && buffer.saving().is_none() {
                    buffer.reload(cx);
                }
            });
            id
        } else {
            self.insert_buffer(path, None, cx)
        };
        self.activate_buffer(id, cx);
    }

    fn activate_buffer(&mut self, id: BufferId, cx: &mut gpui::Context<Self>) {
        if !self.buffers.contains_key(&id) {
            return;
        }
        self.active = Some(id);
        self.prune_clean_buffers(cx);
        cx.notify();
    }

    fn prune_clean_buffers(&mut self, cx: &gpui::App) {
        self.buffers.retain(|id, buffer| {
            let buffer = buffer.read(cx);
            let retain = Some(*id) == self.active
                || buffer.dirty()
                || buffer.saving().is_some()
                // An input notification may still be queued when navigation occurs.
                || buffer.has_edits(cx);
            if !retain {
                self.owner.release_clean(*id);
            }
            retain
        });
    }

    /// Unsaved buffers by ID, then history. Addressing unsaved text by buffer
    /// keeps it reachable after a file goes missing or another file takes its path.
    fn picker_entries(&self, cx: &gpui::App) -> Vec<(PathBuf, Option<BufferId>)> {
        let mut entries: Vec<(PathBuf, Option<BufferId>)> = self
            .buffers
            .iter()
            .filter(|(_, b)| b.read(cx).dirty() || b.read(cx).saving().is_some())
            .map(|(id, b)| (b.read(cx).identity().0.clone(), Some(*id)))
            .collect();
        let unsaved_paths: Vec<_> = entries.iter().map(|(p, _)| p.clone()).collect();
        entries.extend(
            self.recents
                .read(cx)
                .paths
                .iter()
                .filter(|p| !unsaved_paths.contains(p))
                .cloned()
                .map(|p| (p, None)),
        );
        entries
    }

    pub(in crate::view) fn adopt(
        &mut self,
        document: DocumentBuffer,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.buffers.contains_key(&document.id()) {
            return;
        }
        let path = document.read().identity().0.clone();
        self.insert_buffer(path, Some(document), cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn unsaved_labels(&self, cx: &gpui::App) -> Vec<SharedString> {
        self.buffers
            .iter()
            .filter(|(_, b)| b.read(cx).dirty() || b.read(cx).saving().is_some())
            .map(|(_, b)| b.read(cx).identity().0.display().to_string().into())
            .collect()
    }

    pub(super) fn save_all(&mut self, cx: &mut gpui::Context<Self>) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |buffer, cx| buffer.save(None, false, cx));
        }
    }

    pub(super) fn discard_all(&mut self, cx: &mut gpui::Context<Self>) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |buffer, cx| buffer.discard(cx));
        }
    }

    fn process_presented_saves(
        &mut self,
        state: &AppState,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<OperationId> {
        self.buffers
            .values()
            .filter_map(|buffer| buffer.update(cx, |buffer, cx| buffer.process_save(state, cx)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn saves_drained(&self, cx: &gpui::App) -> bool {
        self.owner.saves_drained(None) && self.owner.operations_drained(cx)
    }

    pub(crate) fn filesystem_has_unsaved(&self, paths: &[PathBuf], _: &gpui::App) -> bool {
        self.owner.has_unsaved_within(paths)
    }

    fn resolve_presented(&mut self, paths: &[PathBuf], save: bool, cx: &mut gpui::Context<Self>) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, cx| {
                if paths.iter().any(|p| b.identity().0.starts_with(p)) {
                    if save {
                        b.save(None, false, cx);
                    } else {
                        b.discard(cx);
                    }
                }
            });
        }
    }
    fn pause_presented(&mut self, id: OperationId, cx: &mut gpui::Context<Self>) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, _| {
                b.document.pause(id);
            });
        }
    }
    fn finish_presented(
        &mut self,
        id: OperationId,
        changes: &[gitcomet_core::filesystem::PathChange],
        versions: &BTreeMap<PathBuf, DiskVersion>,
        cx: &mut gpui::Context<Self>,
    ) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, cx| {
                let previous = b.identity();
                b.document.retarget(changes, versions, cx);
                if previous != b.identity() {
                    b.syntax_key = None;
                    b.refresh_syntax(cx);
                }
                b.path_input.update(cx, |input, cx| {
                    input.set_text(b.identity().0.display().to_string(), cx)
                });
                let resume = b.document.resume(id);
                if resume {
                    b.reload(cx);
                }
                cx.notify();
            });
        }
        cx.notify();
    }
}

impl Render for DocumentsView {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let content = self
            .active
            .as_ref()
            .and_then(|p| self.buffers.get(p))
            .cloned();
        div()
            .id("documents_canvas")
            .debug_selector(|| "documents_canvas".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .map(|d| match content {
                Some(buffer) => d.child(buffer),
                None => d.items_center().justify_center().child(
                    components::empty_state(
                        theme,
                        "No document open",
                        "Open any file to view or edit it here.",
                    )
                    .child(
                        components::Button::new("documents_empty_open_file", "Open File…")
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, |this, _, _, cx| {
                                let _ = this
                                    .root
                                    .update(cx, |root, cx| root.prompt_open_document(cx));
                            }),
                    ),
                ),
            })
    }
}

/// Prose reads better wrapped; code keeps its columns.
fn wraps_by_default(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            [
                "md", "markdown", "mdx", "txt", "text", "rst", "adoc", "asciidoc", "org", "tex",
            ]
            .iter()
            .any(|prose| extension.eq_ignore_ascii_case(prose))
        })
}

struct StandaloneBuffer {
    image_group: Option<Entity<crate::view::image_preview::Group>>,
    image_input: crate::view::image_preview::Input,
    image_animation: FileImagePreviewAnimationSide,
    image_timer: crate::view::image_preview::FrameTimer,
    document: DocumentBuffer,
    owner: DocumentOwner,
    theme: AppTheme,
    root: WeakEntity<GitCometView>,
    input: Entity<TextInput>,
    scroll: gpui::ScrollHandle,
    gutter_scroll: gpui::UniformListScrollHandle,
    gutter_row_height: Pixels,
    /// First gutter row of each line while wrapping; empty otherwise.
    wrap_row_starts: Vec<usize>,
    wrap: bool,
    syntax_key: Option<(u64, u64)>,
    syntax_serial: u64,
    syntax_task: Option<gpui::Task<()>>,
    diagram_key: Option<(PathBuf, u64, u64)>,
    diagram_input: Option<super::diagram_preview::DiagramInput>,
    preview_mode: RenderedPreviewMode,
    path_input: Entity<TextInput>,
    load_task: Option<gpui::Task<()>>,
    editing: bool,
    image: bool,
    subscriptions: Vec<gpui::Subscription>,
}

impl StandaloneBuffer {
    fn identity(&self) -> DocumentIdentity {
        self.document.read().identity().clone()
    }

    fn version(&self) -> Option<DiskVersion> {
        self.document.read().version().clone()
    }

    fn text_format(&self) -> Option<SideTextFormat> {
        self.document.read().text_format()
    }

    fn saved_fingerprint(&self) -> Option<u64> {
        self.document.read().saved_fingerprint()
    }

    fn dirty(&self) -> bool {
        self.document.read().dirty()
    }

    fn loading(&self) -> bool {
        self.document.read().loading()
    }

    fn error(&self) -> Option<String> {
        self.document.read().error().map(ToString::to_string)
    }

    fn saving(&self) -> Option<BufferSave> {
        self.document.read().saving().clone()
    }

    fn failed_destination(&self) -> Option<PathBuf> {
        self.document.read().failed_destination().clone()
    }

    fn has_edits(&self, cx: &gpui::App) -> bool {
        self.editing
            && !self.loading()
            && Some(file_editor_text_fingerprint(
                &self.input.read(cx).text_snapshot(),
            )) != self.saved_fingerprint()
    }

    fn new(
        path: PathBuf,
        theme: AppTheme,
        root: WeakEntity<GitCometView>,
        ui_model: Entity<AppUiModel>,
        initial: Option<DocumentBuffer>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let scroll = gpui::ScrollHandle::new();
        let input = cx.new(|cx| {
            let mut input = TextInput::new_inert(
                TextInputOptions {
                    multiline: true,
                    read_only: initial.is_none(),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            );
            input.set_content_width_layout(true);
            input.set_vertical_scroll_handle(Some(scroll.clone()));
            input
        });
        let path_input = cx.new(|cx| {
            let mut input = TextInput::new_inert(TextInputOptions::selectable(), cx);
            // Inherits the header's muted label style.
            input.set_display_text(cx);
            input.set_text(path.display().to_string(), cx);
            input
        });
        let subscriptions = vec![cx.observe(&input, |this, _, cx| {
            if !this.loading() {
                if this.editing {
                    this.document.edit(
                        file_editor_text_fingerprint(&this.input.read(cx).text_snapshot()),
                        None,
                        cx,
                    );
                }
                this.refresh_syntax(cx);
            }
            cx.notify();
        })];
        let image = initial.is_none() && image_format_for_path(&path).is_some();
        let wrap = wraps_by_default(&path);
        cx.on_release(|this, cx| {
            if !this.document.read().loading() {
                let input = this.input.read(cx);
                let fingerprint = file_editor_text_fingerprint(&input.text_snapshot());
                let text = input.text().to_string().into();
                let cursor = input.cursor_offset();
                this.document.capture_fingerprint(fingerprint, cx);
                this.document.retain_text(text, cursor, fingerprint);
            }
            this.owner.adopt(this.document.clone(), cx);
            if let Some(image) = this.image_input.preview.take() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let mut buffer = Self {
            image_group: None,
            image_input: Default::default(),
            image_animation: Default::default(),
            image_timer: Default::default(),
            owner: ui_model.read(cx).documents.clone(),
            document: initial.clone().unwrap_or_else(|| {
                ui_model
                    .read(cx)
                    .documents
                    .create(DocumentIdentity(path), None)
            }),
            theme,
            root,
            input,
            scroll,
            gutter_scroll: gpui::UniformListScrollHandle::new(),
            gutter_row_height: theme.editor_row_height(ui_scale::current(cx).percent),
            wrap_row_starts: Vec::new(),
            wrap,
            syntax_key: None,
            syntax_serial: 0,
            syntax_task: None,
            diagram_key: None,
            diagram_input: None,
            preview_mode: RenderedPreviewMode::Rendered,
            path_input,
            load_task: None,
            editing: initial.is_some(),
            image,
            subscriptions,
        };
        if let Some(edit) = initial.as_ref().and_then(DocumentBuffer::stashed) {
            buffer.input.update(cx, |input, cx| {
                input.set_line_ending(TextInput::detect_line_ending(&edit.text));
                input.set_text(edit.text, cx);
                input.set_cursor_offset(edit.cursor, cx);
            });
        } else {
            buffer.editing = false;
            buffer.document.begin_read(true, cx);
            buffer.reload(cx);
        }
        buffer
    }

    fn process_save(
        &mut self,
        state: &AppState,
        cx: &mut gpui::Context<Self>,
    ) -> Option<OperationId> {
        let pending = self.saving()?;
        let result = state
            .filesystem
            .completed
            .iter()
            .find(|r| r.id == pending.operation)
            .cloned()?;
        let outcome = if result.succeeded() {
            Ok(result.saved_version.clone())
        } else {
            Err(result
                .items
                .iter()
                .filter_map(|item| match &item.outcome {
                    gitcomet_core::filesystem::ItemOutcome::Failed(message) => {
                        Some(message.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
                .into())
        };
        self.document.capture_fingerprint(
            file_editor_text_fingerprint(&self.input.read(cx).text_snapshot()),
            cx,
        );
        self.document.finish_save(result.id, outcome, cx);
        if result.succeeded() {
            self.path_input.update(cx, |input, cx| {
                input.set_text(self.identity().0.display().to_string(), cx)
            });
            update_recent(self.identity().0.clone(), false, cx);
            self.syntax_key = None;
            self.refresh_syntax(cx);
        }
        if self.loading() {
            self.reload(cx);
        }
        cx.notify();
        Some(result.id)
    }

    fn reload(&mut self, cx: &mut gpui::Context<Self>) {
        if self.saving().is_some() || !self.document.read().pauses().is_empty() {
            return;
        }
        let generation = self.document.begin_read(true, cx);
        self.input
            .update(cx, |input, cx| input.set_read_only(true, cx));
        let path = self.identity().0.clone();
        let identity = self.identity().clone();
        let image = self.image;
        let filesystem = self.owner.filesystem();
        self.load_task = Some(cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                preflight_worktree_file_for_editing(&path)?;
                let lock = || filesystem.lock().unwrap_or_else(|e| e.into_inner());
                let guard = lock();
                let version = read_worktree_file_version_for_editing(&path)?;
                // Outside a repository there are no attributes: detect only.
                let request = TextDecodeRequest {
                    kind: SideKind::Worktree,
                    attributes: Arc::default(),
                    encoding: None,
                };
                let (text, format) = if image {
                    (SharedString::default(), None)
                } else {
                    let (text, _, _, format) =
                        panes::read_worktree_file_for_editing(&path, &request)?;
                    (text, Some(format))
                };
                // Image bytes are read without holding up other file operations;
                // the version check below still ties them to `version`.
                let (guard, image_bytes) = if image {
                    drop(guard);
                    use std::io::Read as _;
                    let file = std::fs::File::open(&path).map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    file.take(512 * 1024 * 1024 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|error| error.to_string())?;
                    if bytes.len() > 512 * 1024 * 1024 {
                        return Err("Image exceeds the preview limit".into());
                    }
                    (lock(), Some(bytes))
                } else {
                    (guard, None)
                };
                if read_worktree_file_version_for_editing(&path)? != version {
                    return Err("File changed while reading. Open it again.".into());
                }
                drop(guard);
                let image_input = image_bytes
                    .map(|bytes| image_preview::decode_image(&path, &bytes))
                    .transpose()?;
                Ok::<_, String>((version, text, format, image_input))
            })
            .await;
            let _ = view.update(cx, |this: &mut StandaloneBuffer, cx| {
                if !this.document.read().accepts_read(generation) {
                    return;
                }
                if this.identity() != identity {
                    this.reload(cx);
                    return;
                }
                match result {
                    Ok((version, text, format, image_input)) => {
                        if let Some(old) = this.image_input.preview.take() {
                            cx.defer(move |cx| {
                                cx.drop_image(old, None);
                            });
                        }
                        this.image_input = image_input.unwrap_or_default();
                        this.image_animation = Default::default();
                        this.image_timer = Default::default();
                        // Text that cannot be written back as it was read is view-only.
                        let read_only =
                            !this.editing || format.is_some_and(|format| !format.is_writable());
                        this.input.update(cx, |input, cx| {
                            input.set_line_ending(TextInput::detect_line_ending(&text));
                            input.set_text(text, cx);
                            input.set_read_only(read_only, cx);
                        });
                        let fingerprint =
                            file_editor_text_fingerprint(&this.input.read(cx).text_snapshot());
                        this.document.finish_read(
                            generation,
                            version,
                            format,
                            Default::default(),
                            cx,
                        );
                        this.document.seat_text(fingerprint, None, cx);
                    }
                    Err(error) => this.document.fail_read(generation, error.into(), cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn save(
        &mut self,
        destination: Option<PathBuf>,
        overwrite: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.document.read().pauses().is_empty()
            || self.loading()
            || self.image
            || self.saving().is_some()
            || (!self.dirty() && destination.is_none())
        {
            return;
        }
        let text = SharedString::from(self.input.read(cx).text().to_string());
        let (contents, pending_write) = match encode_for_save(text, self.text_format()) {
            Ok((bytes, pending_write)) => (Arc::from(bytes.as_bytes()), pending_write),
            Err(error) => {
                // Nothing written; the buffer stays dirty.
                self.document
                    .report_write_error(error.to_string().into(), cx);
                cx.notify();
                return;
            }
        };
        let path = destination.unwrap_or_else(|| self.identity().0.clone());
        let request = Request::new(Operation::Save {
            path: path.clone(),
            worktree: None,
            contents,
            expected: if path == self.identity().0 {
                self.version().clone()
            } else {
                None
            },
            overwrite,
        });
        let fingerprint = file_editor_text_fingerprint(&self.input.read(cx).text_snapshot());
        self.owner.save(
            &self.document,
            SaveOrigin::Detached,
            request,
            fingerprint,
            pending_write,
            cx,
        );
        cx.notify();
    }

    fn discard(&mut self, cx: &mut gpui::Context<Self>) {
        // Discard is final even if the file vanished before the reload. Mark
        // the retained text clean and read-only while showing that error.
        self.editing = false;
        self.document.prepare_reload(cx);
        self.input
            .update(cx, |input, cx| input.set_read_only(true, cx));
        self.reload(cx);
    }

    fn can_edit(&self) -> bool {
        let editable_image = self
            .identity()
            .0
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        !((self.image && !editable_image)
            || self.loading()
            || (self.error().is_some() && self.version().is_none() && !self.dirty())
            || self
                .text_format()
                .is_some_and(|format| !format.is_writable()))
    }

    /// Enters edit mode, or leaves it once nothing is left to save.
    fn toggle_editing(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if self.editing {
            if self.dirty() || self.saving().is_some() || self.has_edits(cx) {
                return;
            }
            self.editing = false;
            self.input
                .update(cx, |input, cx| input.set_read_only(true, cx));
            cx.notify();
            return;
        }
        if !self.can_edit() {
            return;
        }
        self.editing = true;
        if self.image {
            self.image = false;
            self.reload(cx);
        }
        self.input
            .update(cx, |input, cx| input.set_read_only(self.loading(), cx));
        window.focus(&self.input.read(cx).focus_handle(), cx);
        cx.notify();
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let answer = cx.prompt_for_new_path(
            self.identity().0.parent().unwrap_or(Path::new("/")),
            self.identity().0.file_name().and_then(|n| n.to_str()),
        );
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(path))) = answer.await {
                let _ = view.update_in(cx, |this, _, cx| this.save(Some(path), false, cx));
            }
        })
        .detach();
    }

    fn ensure_diagram_preview(&mut self, cx: &mut gpui::Context<Self>) {
        use super::diagram_preview::DiagramInput;
        use gitcomet_diagrams::DiagramKind;
        if self.loading() || self.image {
            return;
        }
        let snapshot = self.input.read(cx).text_snapshot();
        let key = (
            self.identity().0.clone(),
            snapshot.model_id(),
            snapshot.revision(),
        );
        if self.diagram_key.as_ref() == Some(&key) {
            return;
        }
        self.diagram_key = Some(key);
        self.diagram_input = DiagramKind::for_path(&self.identity().0).map(|kind| {
            if snapshot.len() <= gitcomet_diagrams::MAX_SOURCE_BYTES {
                DiagramInput::Text {
                    kind,
                    text: snapshot.rope().to_string().into(),
                }
            } else {
                // Never fall back to the saved file for an oversized unsaved buffer.
                DiagramInput::TooLarge {
                    revision: format!("{}:{}", snapshot.model_id(), snapshot.revision()).into(),
                }
            }
        });
    }

    fn refresh_syntax(&mut self, cx: &mut gpui::Context<Self>) {
        if self.loading() || self.image {
            return;
        }
        let snapshot = self.input.read(cx).text_snapshot();
        let key = (snapshot.model_id(), snapshot.revision());
        if self.syntax_key == Some(key) {
            return;
        }
        self.syntax_key = Some(key);
        self.syntax_serial = self.syntax_serial.wrapping_add(1);
        let serial = self.syntax_serial;
        let theme = self.theme;
        let language = rows::diff_syntax_language_for_path(&self.identity().0);
        let rope = snapshot.rope();
        let source_len = snapshot.len();
        let heuristic_rope = rope.clone();
        let provider = crate::kit::HighlightProvider::with_pending(
            move |range: std::ops::Range<usize>| {
                crate::kit::HighlightProviderResult {
                highlights: language
                    .map(|language| {
                        panes::diff::helpers::presentation::resolved_output_heuristic_highlights_for_range(
                            theme,
                            &heuristic_rope,
                            language,
                            range,
                        )
                    })
                    .unwrap_or_default(),
                pending: false,
            }
            },
            || 0,
            || false,
        );
        self.input.update(cx, |input, cx| {
            input.set_highlight_provider_with_key(serial.wrapping_mul(2), provider, source_len, cx)
        });
        self.syntax_task = language.map(|language| {
            cx.spawn(async move |view, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                let syntax = crate::ui_runtime::background_compute(move || {
                    rows::LiveSyntaxDocument::new(language, rope, Arc::default(), None)
                })
                .await;
                let _ = view.update(cx, |this, cx| {
                    if this.syntax_serial != serial {
                        return;
                    }
                    if let Some(syntax) = syntax {
                        let snapshot = syntax
                            .snapshot(crate::kit::syntax_style::SyntaxHighlightPalette::new(theme));
                        let provider = crate::kit::HighlightProvider::with_pending(
                            move |range| crate::kit::HighlightProviderResult {
                                highlights: snapshot.highlights_for_byte_range(range),
                                pending: false,
                            },
                            || 0,
                            || false,
                        );
                        this.input.update(cx, |input, cx| {
                            input.set_highlight_provider_with_key(
                                serial.wrapping_mul(2).wrapping_add(1),
                                provider,
                                source_len,
                                cx,
                            )
                        });
                    }
                });
            })
        });
    }
}

#[cfg(test)]
mod tests;

impl StandaloneBuffer {
    /// Path, state and the buffer's actions, laid out like the diff panel's
    /// file header so the two canvases read as one family.
    fn render_header(&self, ui_scale_percent: u32, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let path = &self.identity().0;
        let name: SharedString = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
            .into();
        let icon = file_icons::file_icon_for_path(path);
        let icon_color = file_icons::file_icon_color(icon, theme.is_dark)
            .unwrap_or(theme.colors.foreground.secondary);
        let unsaved = self.dirty() || self.saving().is_some();
        let save_shortcut = crate::kit::components::secondary_shortcut("S");

        let title = div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .items_center()
            .gap_2()
            .overflow_hidden()
            .child(svg_icon(icon, icon_color, scaled_px(14.0)))
            .child(
                div()
                    .debug_selector(|| "document_title".into())
                    .flex_shrink_0()
                    .max_w(gpui::relative(0.6))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child(name),
            )
            .when(unsaved, |title| {
                title.child(
                    div()
                        .id("document_unsaved_dot")
                        .debug_selector(|| "document_unsaved_dot".into())
                        .flex_shrink_0()
                        .size(scaled_px(7.0))
                        .rounded_full()
                        .bg(theme.colors.status.warning.foreground)
                        .gitcomet_tooltip(theme, "Unsaved changes".into()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(self.path_input.clone()),
            );

        let can_edit = self.can_edit();
        let edit_locked = self.editing && unsaved;
        let edit_tooltip: SharedString = if edit_locked {
            format!("Save ({save_shortcut}) or discard the changes to stop editing").into()
        } else if self.editing {
            "Stop editing".into()
        } else if !can_edit {
            if self
                .text_format()
                .is_some_and(|format| !format.is_writable())
            {
                "This file's encoding cannot be written back; it opens read-only".into()
            } else {
                "This file cannot be edited".into()
            }
        } else {
            "Edit this file".into()
        };
        let mut controls = div()
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .when(!self.image && !self.loading(), |controls| {
                controls.child(
                    components::Button::new("document_wrap", "Wrap")
                        .style(components::ButtonStyle::Subtle)
                        .borderless()
                        .selected(self.wrap)
                        .on_click(theme, cx, |this, _, _, cx| {
                            this.wrap = !this.wrap;
                            cx.notify();
                        })
                        .gitcomet_tooltip(
                            theme,
                            if self.wrap {
                                "Stop wrapping long lines".into()
                            } else {
                                "Wrap long lines".into()
                            },
                        ),
                )
            })
            .child(
                components::Button::new("document_edit", if unsaved { "Edit •" } else { "Edit" })
                    .style(components::ButtonStyle::Subtle)
                    .borderless()
                    .selected(self.editing)
                    .disabled(if self.editing { edit_locked } else { !can_edit })
                    .on_click(theme, cx, |this, _, window, cx| {
                        this.toggle_editing(window, cx)
                    })
                    .gitcomet_tooltip(theme, edit_tooltip),
            );
        if !self.loading()
            && let Some(kind @ RenderedPreviewKind::Diagram) =
                preview_path_rendered_kind(&self.identity().0)
        {
            controls = controls.child(super::rendered_preview::mode_toggle(
                kind,
                if self.editing {
                    RenderedPreviewMode::Source
                } else {
                    self.preview_mode
                },
                self.editing,
                theme,
                cx,
                |this, mode, _, cx| {
                    this.preview_mode = mode;
                    cx.notify();
                },
            ));
        }
        if self.editing {
            controls = controls
                .child(
                    components::Button::new("document_discard", "Discard")
                        .style(components::ButtonStyle::Subtle)
                        .borderless()
                        .disabled(!self.dirty() || self.saving().is_some())
                        .on_click(theme, cx, |this, _, _, cx| this.discard(cx))
                        .gitcomet_tooltip(theme, "Throw away the unsaved changes".into()),
                )
                .child(
                    components::Button::new("document_save", "Save")
                        .style(components::ButtonStyle::Outlined)
                        .disabled(!self.dirty() || self.saving().is_some())
                        .on_click(theme, cx, |this, _, _, cx| this.save(None, false, cx))
                        .gitcomet_tooltip(theme, format!("Save ({save_shortcut})").into()),
                );
        }
        controls = controls
            .child(
                components::Button::new("document_save_as", "Save As…")
                    .style(components::ButtonStyle::Subtle)
                    .borderless()
                    .disabled(
                        self.image || self.loading() || (self.version().is_none() && !self.dirty()),
                    )
                    .on_click(theme, cx, |this, _, window, cx| {
                        this.prompt_save_as(window, cx)
                    })
                    .gitcomet_tooltip(theme, "Save a copy under another name".into()),
            )
            .child(
                components::Button::new("document_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        scaled_px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _, _, cx| {
                        let _ = this
                            .root
                            .update(cx, |root, cx| root.show_repository_canvas(cx));
                    })
                    .gitcomet_tooltip(
                        theme,
                        if unsaved {
                            "Close the viewer (unsaved changes are kept under Documents)".into()
                        } else {
                            "Close the viewer".into()
                        },
                    ),
            );

        div()
            .debug_selector(|| "document_header".into())
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .h(components::content_header_height(
                ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
            ))
            .px_2()
            .bg(crate::theme::content_header_bg(theme))
            .border_b_1()
            .border_color(theme.colors.stroke.default)
            .child(title)
            .child(controls)
    }

    /// A failed read or save over content that is still on screen.
    fn render_notice(&self, error: String, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let warning = theme.colors.status.warning.foreground;
        div()
            .debug_selector(|| "document_notice".into())
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_1()
            .bg(with_alpha(warning, if theme.is_dark { 0.12 } else { 0.10 }))
            .border_b_1()
            .border_color(with_alpha(warning, 0.35))
            .child(svg_icon(
                "icons/warning.svg",
                warning,
                ui_scale::design_px_from_percent(14.0, ui_scale::current(cx).percent),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.primary)
                    .child(error),
            )
            .when(self.dirty() || self.failed_destination().is_some(), |d| {
                d.child(
                    components::Button::new("document_replace_disk", "Replace disk contents…")
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |_this, _, window, cx| {
                            let answer = window.prompt(
                                gpui::PromptLevel::Warning,
                                "Replace the current file on disk?",
                                Some("Another application may have saved a newer version."),
                                &[
                                    gpui::PromptButton::cancel("Cancel"),
                                    gpui::PromptButton::new("Replace"),
                                ],
                                cx,
                            );
                            cx.spawn_in(window, async move |view, cx| {
                                if answer.await.ok() == Some(1) {
                                    let _ = view.update_in(cx, |this, _, cx| {
                                        this.save(this.failed_destination().clone(), true, cx)
                                    });
                                }
                            })
                            .detach();
                        }),
                )
            })
    }

    /// Refresh the wrapped-row projection and return the gutter's row count.
    /// Falls back to one row per line until the input's wrap pass catches up.
    fn rebuild_wrap_row_starts(
        &mut self,
        line_count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> usize {
        let mut starts = std::mem::take(&mut self.wrap_row_starts);
        starts.clear();
        let total = self.input.read_with(cx, |input, _| {
            let rows_per_line = input.wrap_row_counts();
            if rows_per_line.len() != line_count {
                return None;
            }
            let mut total = 0usize;
            for rows in rows_per_line {
                starts.push(total);
                total = total.saturating_add((*rows).max(1));
            }
            Some(total)
        });
        if total.is_none() {
            starts.clear();
        }
        self.wrap_row_starts = starts;
        total.unwrap_or(line_count)
    }

    fn render_gutter_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        _cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let row_height = this.gutter_row_height;
        range
            .map(|visual_ix| {
                let (ix, is_continuation) =
                    file_editor_line_for_visual_row(&this.wrap_row_starts, visual_ix);
                div()
                    .h(row_height)
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.editor.line_number)
                    .child(if is_continuation {
                        String::new()
                    } else {
                        (ix + 1).to_string()
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// Line-numbered text, laid out like the repository file editor.
    fn render_text(&mut self, ui_scale_percent: u32, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let wrap = self.wrap;
        let row_height = theme.editor_row_height(ui_scale_percent);
        self.gutter_row_height = row_height;
        self.input.update(cx, |input, cx| {
            input.set_line_height(Some(row_height), cx);
            input.set_soft_wrap(wrap, cx);
            input.set_content_width_layout(!wrap);
        });
        let line_count = self
            .input
            .read_with(cx, |input, _| input.text_snapshot().line_count())
            .max(1);
        let gutter_rows = if wrap {
            self.rebuild_wrap_row_starts(line_count, cx)
        } else {
            self.wrap_row_starts.clear();
            line_count
        };
        let gutter_width = file_editor_gutter_width(
            line_count,
            true,
            ui_scale::UiScale::from_percent(ui_scale_percent).with_appearance(theme.metrics),
        );
        let editor_scroll = self.scroll.clone();
        let gutter_scroll = self.gutter_scroll.clone();
        let scrollbar_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
        let horizontal_gutter = if wrap {
            px(0.0)
        } else {
            components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal)
        };
        // The editor is the master: its offset is copied into the virtualized
        // gutter before layout, and again after prepaint for caret autoscroll.
        {
            let target_y = editor_scroll.offset().y;
            let base = gutter_scroll.0.borrow().base_handle.clone();
            if base.offset().y != target_y {
                base.set_offset(point(px(0.0), target_y));
            }
        }
        let gutter = uniform_list(
            "document_gutter_list",
            gutter_rows,
            cx.processor(Self::render_gutter_rows),
        )
        .h_full()
        .min_h(px(0.0))
        .track_scroll(&self.gutter_scroll);

        div()
            .on_children_prepainted({
                let editor_scroll = editor_scroll.clone();
                let gutter_scroll = gutter_scroll.clone();
                move |_bounds, window, _cx| {
                    let target_y = editor_scroll.offset().y;
                    let base = gutter_scroll.0.borrow().base_handle.clone();
                    if base.offset().y != target_y {
                        base.set_offset(point(px(0.0), target_y));
                        window.refresh();
                    }
                }
            })
            .id("document_text")
            .relative()
            .flex()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .bg(theme.colors.editor.background)
            .font_family(crate::font_preferences::current_editor_font_family(cx))
            .child(
                div()
                    .id("document_gutter")
                    .debug_selector(|| "document_gutter".into())
                    .w(gutter_width)
                    .h_full()
                    .pb(horizontal_gutter)
                    .min_h(px(0.0))
                    .flex_shrink_0()
                    .bg(theme.colors.editor.gutter_background)
                    .border_r_1()
                    .border_color(theme.colors.editor.indent_guide)
                    // A wheel over the numbers moves the text, not the gutter.
                    .on_scroll_wheel({
                        let editor_scroll = editor_scroll.clone();
                        move |event, window, cx| {
                            let delta = event.delta.pixel_delta(window.line_height());
                            let offset = editor_scroll.offset();
                            let max_y = editor_scroll.max_offset().y.max(px(0.0));
                            let next_y = (offset.y + delta.y).clamp(-max_y, px(0.0));
                            if next_y != offset.y {
                                editor_scroll.set_offset(point(offset.x, next_y));
                                window.refresh();
                            }
                            cx.stop_propagation();
                        }
                    })
                    .child(gutter),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .h_full()
                    .pb(horizontal_gutter)
                    .min_h(px(0.0))
                    .child(
                        div()
                            .id("document_editor_scroll")
                            .debug_selector(|| "document_editor_scroll".into())
                            // A column, so a content-width input overflows to
                            // the right and gives the container a horizontal range.
                            .flex()
                            .flex_col()
                            .when(wrap, |d| d.items_stretch())
                            .when(!wrap, |d| d.items_start())
                            .w_full()
                            .min_w(px(0.0))
                            .h_full()
                            .min_h(px(0.0))
                            .pl_2()
                            .pr(scrollbar_gutter)
                            .when(wrap, |d| {
                                restrict_scroll_to_vertical_axis(
                                    d.overflow_hidden().overflow_y_scroll(),
                                )
                            })
                            .when(!wrap, |d| d.overflow_scroll())
                            .track_scroll(&self.scroll)
                            .child(self.input.clone()),
                    )
                    // Outside the moving surface, or the offset moves the track too.
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom(horizontal_gutter)
                            .w(scrollbar_gutter)
                            .child(
                                components::Scrollbar::new(
                                    "document_editor_scrollbar",
                                    editor_scroll.clone(),
                                )
                                .render(theme),
                            ),
                    )
                    .when(!wrap, |container| {
                        container.child(
                            div()
                                .absolute()
                                .left_0()
                                .right(scrollbar_gutter)
                                .bottom_0()
                                .h(horizontal_gutter)
                                .child(
                                    components::Scrollbar::horizontal(
                                        "document_editor_hscrollbar",
                                        editor_scroll.clone(),
                                    )
                                    .render(theme),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for StandaloneBuffer {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let _ = self.subscriptions.len();
        self.ensure_diagram_preview(cx);
        // Text stays on screen through a failed save or a vanished file; only a
        // read that never produced any shows the failure in its place.
        let has_text = !self.image
            && !self.loading()
            && (self.error().is_none()
                || self.version().is_some()
                || self.dirty()
                || self.saving().is_some());
        let notice = self
            .error()
            .clone()
            .filter(|_| has_text)
            .map(|error| self.render_notice(error, cx));
        let header = self.render_header(ui_scale_percent, cx);
        let mut image_controls = None;
        let body: AnyElement = if self.loading() {
            components::empty_state_message(theme, "Loading…")
                .size_full()
                .into_any_element()
        } else if self.image_ready() {
            let (image, controls) = self.render_image(window, cx);
            image_controls = controls;
            super::rendered_preview::render(
                "document_image",
                super::rendered_preview::PreviewContent::File(image),
                theme,
                ui_scale_percent,
            )
        } else if has_text
            && self.diagram_input.is_some()
            && self.preview_mode == RenderedPreviewMode::Rendered
            && !self.editing
        {
            let input = self.diagram_input.clone().unwrap();
            super::rendered_preview::render(
                "document_diagram",
                super::rendered_preview::PreviewContent::File(
                    super::diagram_preview::DiagramPreview {
                        id: "document_diagram".into(),
                        theme,
                        source_element: None,
                        force_source: false,
                        placement: super::diagram_preview::DiagramPlacement::Pane(input),
                    }
                    .into_any_element(),
                ),
                theme,
                ui_scale_percent,
            )
        } else if has_text {
            self.render_text(ui_scale_percent, cx)
        } else {
            components::empty_state(
                theme,
                "Cannot open this file",
                self.error().clone().unwrap_or_default(),
            )
            .size_full()
            .into_any_element()
        };
        div()
            .id("standalone_document")
            .debug_selector(|| "standalone_document".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors.surface.canvas)
            .capture_key_down(
                cx.listener(|this, event: &gpui::KeyDownEvent, _window, cx| {
                    if (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                        && event.keystroke.key == "s"
                    {
                        cx.stop_propagation();
                        this.save(None, false, cx);
                    }
                }),
            )
            .child(header)
            .when_some(notice, |d, notice| d.child(notice))
            .child(div().flex_1().min_h(px(0.0)).min_w(px(0.0)).child(body))
            .when_some(image_controls, |d, controls| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .py_1()
                        .border_t_1()
                        .border_color(theme.colors.stroke.default)
                        .child(controls),
                )
            })
    }
}

#[derive(Default)]
pub(super) struct Routing {
    generation: u64,
    pub(super) pending: Vec<(PathBuf, PathBuf, bool)>,
    previous_failures: BTreeMap<PathBuf, Option<OperationId>>,
}

impl GitCometView {
    /// The bar draws the Documents button's state. Notified by id: the bar's
    /// own buttons reach these methods from inside its update.
    fn notify_bottom_status_bar(&self, cx: &mut gpui::Context<Self>) {
        gpui::AppContext::notify(cx, self.bottom_status_bar.entity_id());
    }

    /// Drops any queued foreground open so a late reply cannot steal focus.
    fn supersede_document_routing(&mut self) {
        for (_, _, display) in &mut self.document_routing.pending {
            *display = false;
        }
        self.document_routing.generation = self.document_routing.generation.wrapping_add(1);
    }

    pub(in crate::view) fn show_repository_canvas(&mut self, cx: &mut gpui::Context<Self>) {
        self.documents_active = false;
        self.supersede_document_routing();
        self.notify_bottom_status_bar(cx);
        cx.notify();
    }

    fn show_documents_canvas(&mut self, cx: &mut gpui::Context<Self>) {
        self.documents_active = true;
        self.notify_bottom_status_bar(cx);
        cx.notify();
    }

    /// The status bar's Documents button: opens the picker above it.
    pub(in crate::view) fn toggle_document_picker(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.document_picker
            .update(cx, |picker, cx| picker.toggle(anchor, window, cx));
        self.notify_bottom_status_bar(cx);
    }

    pub(in crate::view) fn document_picker_open(&self, cx: &gpui::App) -> bool {
        self.document_picker.read(cx).is_open()
    }

    /// Shows a retained buffer, such as unsaved text whose file is gone.
    fn show_document_buffer(&mut self, id: BufferId, cx: &mut gpui::Context<Self>) {
        self.supersede_document_routing();
        self.documents
            .update(cx, |docs, cx| docs.activate_buffer(id, cx));
        self.show_documents_canvas(cx);
    }

    pub(in crate::view) fn prompt_open_document(&mut self, cx: &mut gpui::Context<Self>) {
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open File".into()),
        });
        cx.spawn(async move |view, cx| {
            if let Ok(Ok(Some(paths))) = answer.await {
                let _ = view.update(cx, |this, cx| this.open_document_paths(paths, cx));
            }
        })
        .detach();
    }

    pub(crate) fn open_document_paths(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        super::native_transfers::mark_handled(&paths, cx);
        self.supersede_document_routing();
        let generation = self.document_routing.generation;
        let store = self.store.clone();
        cx.spawn(async move |view, cx| {
            let results = crate::ui_runtime::background_compute(move || {
                paths
                    .into_iter()
                    .map(|path| {
                        let identity = gitcomet_core::filesystem::absolute_identity(&path)
                            .map_err(|e| e.to_string())?;
                        if !std::fs::metadata(&identity)
                            .map_err(|e| e.to_string())?
                            .is_file()
                        {
                            return Err(format!("{} is not a file", identity.display()));
                        }
                        store
                            .discover_file_repository(&identity)
                            .map(|repo| (identity, repo))
                            .map_err(|e| e.to_string())
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let _ = view.update(cx, |this, cx| {
                let mut first = true;
                for result in results {
                    match result {
                        Ok((path, Some(repository))) => {
                            let display = first && this.document_routing.generation == generation;
                            first = false;
                            if !this
                                .state
                                .repos
                                .iter()
                                .any(|r| r.spec.workdir == repository)
                                && crate::app::route_document_to_existing_window(
                                    &repository,
                                    &path,
                                    display,
                                    cx,
                                )
                            {
                                continue;
                            }
                            this.queue_repository_document(repository, path, display, cx);
                        }
                        Ok((path, None)) => {
                            let display = first && this.document_routing.generation == generation;
                            first = false;
                            this.documents
                                .update(cx, |docs, cx| docs.open(path, display, cx));
                            if display {
                                this.documents_active = true;
                            }
                        }
                        Err(error) => this.push_toast(components::ToastKind::Error, error, cx),
                    }
                }
                this.notify_bottom_status_bar(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn queue_repository_document(
        &mut self,
        repository: PathBuf,
        path: PathBuf,
        display: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.document_routing
            .previous_failures
            .entry(repository.clone())
            .or_insert_with(|| {
                self.state
                    .repository_open_failures
                    .get(&repository)
                    .map(|(id, _)| *id)
            });
        if !self
            .state
            .repos
            .iter()
            .any(|r| r.spec.workdir == repository)
        {
            self.store.dispatch(Msg::OpenDocumentRepository {
                path: repository.clone(),
                // Only the still-current routing request activates a ready
                // tab. Opening in the background must not cancel active loads.
                activate: false,
            });
        }
        self.document_routing
            .pending
            .push((repository, path, display));
        self.finish_document_routing(cx);
    }

    pub(super) fn finish_document_routing(&mut self, cx: &mut gpui::Context<Self>) {
        let mut pending = vec![];
        for (root, path, display) in std::mem::take(&mut self.document_routing.pending) {
            let Some(repo) = self.state.repos.iter().find(|r| r.spec.workdir == root) else {
                if let Some((id, error)) = self.state.repository_open_failures.get(&root)
                    && self
                        .document_routing
                        .previous_failures
                        .get(&root)
                        .copied()
                        .flatten()
                        != Some(*id)
                {
                    self.push_toast(
                        components::ToastKind::Error,
                        format!("Could not open {}: {error}", path.display()),
                        cx,
                    );
                    continue;
                }
                pending.push((root, path, display));
                continue;
            };
            if matches!(repo.open, Loadable::Loading | Loadable::NotLoaded) {
                pending.push((root, path, display));
                continue;
            }
            if !matches!(repo.open, Loadable::Ready(())) {
                continue;
            }
            let Ok(relative_path) = path.strip_prefix(&root).map(Path::to_path_buf) else {
                continue;
            };
            update_recent(path, false, cx);
            let path = relative_path;
            if display {
                self.documents_active = false;
                self.store.dispatch(Msg::SetActiveRepo { repo_id: repo.id });
                self.store.dispatch(Msg::SetSidebarMode {
                    mode: SidebarMode::Files,
                });
                self.store.dispatch(Msg::SetFileBrowserSource {
                    repo_id: repo.id,
                    source: gitcomet_core::domain::FileSource::WorkingDirectory,
                });
                self.store.dispatch(Msg::RevealFileBrowserPath {
                    repo_id: repo.id,
                    path: path.clone(),
                });
                self.store.dispatch(Msg::OpenFileContent {
                    repo_id: repo.id,
                    source: gitcomet_core::domain::FileSource::WorkingDirectory,
                    path,
                });
            } else {
                self.store.dispatch(Msg::RememberDocumentInRepository {
                    repo_id: repo.id,
                    path,
                });
            }
        }
        self.document_routing.pending = pending;
        self.document_routing.previous_failures.retain(|root, _| {
            self.document_routing
                .pending
                .iter()
                .any(|(pending, _, _)| pending == root)
        });
        self.notify_bottom_status_bar(cx);
        cx.notify();
    }
}
