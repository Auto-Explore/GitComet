use super::panes::main::file_editor::{
    StashedFileEdit, encode_for_save, file_editor_gutter_width, file_editor_line_for_visual_row,
    file_editor_text_fingerprint,
};
use super::panes::main::{
    TextDecodeRequest, preflight_worktree_file_for_editing, read_worktree_file_version_for_editing,
};
use super::*;
use crate::kit::{TextInput, TextInputOptions};
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity, Operation, OperationId, Request};
use gitcomet_core::text_format::{SideKind, SideTextFormat};
use gitcomet_state::model::SidebarMode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod picker;
pub(crate) use picker::DocumentPicker;

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
    store: Arc<AppStore>,
    ui_model: Entity<AppUiModel>,
    recents: Entity<RecentDocumentList>,
    buffers: BTreeMap<gpui::EntityId, Entity<StandaloneBuffer>>,
    active: Option<gpui::EntityId>,
}

impl DocumentsView {
    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        for buffer in self.buffers.values() {
            buffer.update(cx, |buffer, cx| {
                buffer.theme = theme;
                buffer
                    .input
                    .update(cx, |input, cx| input.set_theme(theme, cx));
                buffer
                    .path_input
                    .update(cx, |input, cx| input.set_theme(theme, cx));
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
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        Self {
            theme,
            root,
            store,
            ui_model,
            recents: shared_recents(cx),
            buffers: BTreeMap::new(),
            active: None,
        }
    }

    fn insert_buffer(
        &mut self,
        path: PathBuf,
        initial: Option<(StashedFileEdit, Option<DiskVersion>)>,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::EntityId {
        let root = self.root.clone();
        let buffer = cx.new(|cx| {
            StandaloneBuffer::new(
                path,
                self.theme,
                root,
                self.store.clone(),
                self.ui_model.clone(),
                initial,
                cx,
            )
        });
        let subscription = cx.observe(&buffer, |this, _, cx| {
            this.prune_clean_buffers(cx);
            cx.notify();
        });
        // The observer is released together with the buffer it watches.
        buffer.update(cx, |buffer, _| buffer.subscriptions.push(subscription));
        let id = buffer.entity_id();
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
            .filter(|(_, b)| b.read(cx).identity.0 == path)
            .max_by_key(|(_, b)| b.read(cx).dirty)
            .map(|(id, _)| *id);
        let id = if let Some(id) = existing {
            self.buffers[&id].update(cx, |buffer, cx| {
                if !buffer.dirty && buffer.saving.is_none() {
                    buffer.reload(cx);
                }
            });
            id
        } else {
            self.insert_buffer(path, None, cx)
        };
        self.activate_buffer(id, cx);
    }

    fn activate_buffer(&mut self, id: gpui::EntityId, cx: &mut gpui::Context<Self>) {
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
            Some(*id) == self.active
                || buffer.dirty
                || buffer.saving.is_some()
                // An input notification may still be queued when navigation occurs.
                || buffer.has_edits(cx)
        });
    }

    /// Unsaved buffers by ID, then history. Addressing unsaved text by buffer
    /// keeps it reachable after a file goes missing or another file takes its path.
    fn picker_entries(&self, cx: &gpui::App) -> Vec<(PathBuf, Option<gpui::EntityId>)> {
        let mut entries: Vec<(PathBuf, Option<gpui::EntityId>)> = self
            .buffers
            .iter()
            .filter(|(_, b)| b.read(cx).dirty || b.read(cx).saving.is_some())
            .map(|(id, b)| (b.read(cx).identity.0.clone(), Some(*id)))
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
        identity: DocumentIdentity,
        buffer: StashedFileEdit,
        version: Option<DiskVersion>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.insert_buffer(identity.0, Some((buffer, version)), cx);
        cx.notify();
    }

    pub(super) fn unsaved_labels(&self, cx: &gpui::App) -> Vec<SharedString> {
        self.buffers
            .iter()
            .filter(|(_, b)| b.read(cx).dirty || b.read(cx).saving.is_some())
            .map(|(_, b)| b.read(cx).identity.0.display().to_string().into())
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

    pub(crate) fn saves_drained(&self, cx: &gpui::App) -> bool {
        self.buffers.values().all(|b| b.read(cx).saving.is_none())
    }

    pub(crate) fn filesystem_has_unsaved(&self, paths: &[PathBuf], cx: &gpui::App) -> bool {
        self.buffers.values().any(|b| {
            let b = b.read(cx);
            (b.dirty || b.saving.is_some()) && paths.iter().any(|p| b.identity.0.starts_with(p))
        })
    }
    pub(crate) fn filesystem_resolve(
        &mut self,
        paths: &[PathBuf],
        save: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, cx| {
                if paths.iter().any(|p| b.identity.0.starts_with(p)) {
                    if save {
                        b.save(None, false, cx);
                    } else {
                        b.discard(cx);
                    }
                }
            });
        }
    }
    pub(crate) fn filesystem_pause(&mut self, id: OperationId, cx: &mut gpui::Context<Self>) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, _| {
                b.pauses.insert(id);
            });
        }
    }
    pub(crate) fn filesystem_finish(
        &mut self,
        id: OperationId,
        changes: &[gitcomet_core::filesystem::PathChange],
        versions: &BTreeMap<PathBuf, DiskVersion>,
        cx: &mut gpui::Context<Self>,
    ) {
        for buffer in self.buffers.values() {
            buffer.update(cx, |b, cx| {
                let identity = b.identity.retarget(changes);
                if identity != b.identity {
                    b.identity = identity;
                    b.syntax_key = None;
                    b.refresh_syntax(cx);
                }
                if let Some(version) = versions.get(&b.identity.0)
                    && b.version
                        .as_ref()
                        .is_some_and(|old| old.same_contents(version))
                {
                    b.version = Some(version.clone());
                }
                b.path_input.update(cx, |input, cx| {
                    input.set_text(b.identity.0.display().to_string(), cx)
                });
                if b.pauses.remove(&id) && b.pauses.is_empty() && b.loading {
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
    identity: DocumentIdentity,
    theme: AppTheme,
    root: WeakEntity<GitCometView>,
    store: Arc<AppStore>,
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
    path_input: Entity<TextInput>,
    version: Option<DiskVersion>,
    /// How the file was read, so saves write it back the same way.
    text_format: Option<SideTextFormat>,
    saved_fingerprint: Option<u64>,
    load_generation: u64,
    load_task: Option<gpui::Task<()>>,
    dirty: bool,
    editing: bool,
    loading: bool,
    image: bool,
    error: Option<String>,
    saving: Option<(OperationId, PathBuf, u64)>,
    failed_destination: Option<PathBuf>,
    pauses: std::collections::BTreeSet<OperationId>,
    subscriptions: Vec<gpui::Subscription>,
}

impl StandaloneBuffer {
    fn has_edits(&self, cx: &gpui::App) -> bool {
        self.editing
            && !self.loading
            && Some(file_editor_text_fingerprint(
                &self.input.read(cx).text_snapshot(),
            )) != self.saved_fingerprint
    }

    fn new(
        path: PathBuf,
        theme: AppTheme,
        root: WeakEntity<GitCometView>,
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        initial: Option<(StashedFileEdit, Option<DiskVersion>)>,
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
            input.set_theme(theme, cx);
            input.set_content_width_layout(true);
            input.set_vertical_scroll_handle(Some(scroll.clone()));
            input
        });
        let path_input = cx.new(|cx| {
            let mut input = TextInput::new_inert(TextInputOptions::selectable(), cx);
            input.set_theme(theme, cx);
            // Inherits the header's muted label style.
            input.set_display_text(cx);
            input.set_text(path.display().to_string(), cx);
            input
        });
        let subscriptions = vec![
            cx.observe(&input, |this, _, cx| {
                if !this.loading {
                    this.dirty = this.has_edits(cx);
                    this.refresh_syntax(cx);
                }
                cx.notify();
            }),
            cx.observe(&ui_model, |this, model, cx| {
                let Some((id, _, _)) = &this.saving else {
                    return;
                };
                let Some(result) = model
                    .read(cx)
                    .state
                    .filesystem
                    .completed
                    .iter()
                    .find(|r| r.id == *id)
                    .cloned()
                else {
                    return;
                };
                let (_, path, fingerprint) = this.saving.take().unwrap();
                this.store
                    .dispatch(Msg::AcknowledgeFilesystemResults(vec![result.id]));
                if result.succeeded() {
                    this.identity = DocumentIdentity(path);
                    this.path_input.update(cx, |input, cx| {
                        input.set_text(this.identity.0.display().to_string(), cx)
                    });
                    this.version = result.saved_version.clone();
                    this.saved_fingerprint = Some(fingerprint);
                    this.dirty = this.has_edits(cx);
                    this.error = None;
                    this.failed_destination = None;
                    update_recent(this.identity.0.clone(), false, cx);
                    this.syntax_key = None;
                    this.refresh_syntax(cx);
                } else {
                    this.failed_destination = Some(path);
                    this.error = Some(
                        result
                            .items
                            .iter()
                            .filter_map(|i| {
                                if let gitcomet_core::filesystem::ItemOutcome::Failed(e) =
                                    &i.outcome
                                {
                                    Some(e.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                cx.notify();
            }),
        ];
        let image = initial.is_none() && image_format_for_path(&path).is_some();
        let wrap = wraps_by_default(&path);
        let mut buffer = Self {
            identity: DocumentIdentity(path),
            theme,
            root,
            store,
            input,
            scroll,
            gutter_scroll: gpui::UniformListScrollHandle::new(),
            gutter_row_height: theme.editor_row_height(ui_scale::current(cx).percent),
            wrap_row_starts: Vec::new(),
            wrap,
            syntax_key: None,
            syntax_serial: 0,
            syntax_task: None,
            path_input,
            version: None,
            text_format: None,
            saved_fingerprint: None,
            load_generation: 0,
            load_task: None,
            dirty: false,
            editing: initial.is_some(),
            loading: false,
            image,
            error: None,
            saving: None,
            failed_destination: None,
            pauses: Default::default(),
            subscriptions,
        };
        if let Some((edit, version)) = initial {
            buffer.version = version;
            buffer.text_format = edit.text_format;
            buffer.saved_fingerprint = Some(edit.saved_fingerprint);
            buffer.dirty = true;
            buffer.input.update(cx, |input, cx| {
                input.set_line_ending(TextInput::detect_line_ending(&edit.text));
                input.set_text(edit.text, cx);
                input.set_cursor_offset(edit.cursor, cx);
            });
        } else {
            buffer.reload(cx);
        }
        buffer
    }

    fn reload(&mut self, cx: &mut gpui::Context<Self>) {
        if self.saving.is_some() || !self.pauses.is_empty() {
            return;
        }
        self.load_generation = self.load_generation.wrapping_add(1);
        let generation = self.load_generation;
        self.loading = true;
        self.input
            .update(cx, |input, cx| input.set_read_only(true, cx));
        let path = self.identity.0.clone();
        let identity = self.identity.clone();
        let image = self.image;
        self.load_task = Some(cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                preflight_worktree_file_for_editing(&path)?;
                let _guard = gitcomet_core::filesystem::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
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
                if read_worktree_file_version_for_editing(&path)? != version {
                    return Err("File changed while reading. Open it again.".into());
                }
                Ok::<_, String>((version, text, format))
            })
            .await;
            let _ = view.update(cx, |this: &mut StandaloneBuffer, cx| {
                if this.load_generation != generation {
                    return;
                }
                if this.identity != identity {
                    this.reload(cx);
                    return;
                }
                this.loading = false;
                match result {
                    Ok((version, text, format)) => {
                        this.version = Some(version);
                        this.text_format = format;
                        // Text that cannot be written back as it was read is view-only.
                        let read_only =
                            !this.editing || format.is_some_and(|format| !format.is_writable());
                        this.input.update(cx, |input, cx| {
                            input.set_line_ending(TextInput::detect_line_ending(&text));
                            input.set_text(text, cx);
                            input.set_read_only(read_only, cx);
                        });
                        this.saved_fingerprint = Some(file_editor_text_fingerprint(
                            &this.input.read(cx).text_snapshot(),
                        ));
                        this.dirty = false;
                        this.error = None;
                        this.failed_destination = None;
                    }
                    Err(error) => this.error = Some(error),
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
        if !self.pauses.is_empty()
            || self.loading
            || self.image
            || self.saving.is_some()
            || (!self.dirty && destination.is_none())
        {
            return;
        }
        let text = SharedString::from(self.input.read(cx).text().to_string());
        let contents = match encode_for_save(text, self.text_format) {
            Ok((bytes, _)) => Arc::from(bytes.as_bytes()),
            Err(error) => {
                // Nothing written; the buffer stays dirty.
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let path = destination.unwrap_or_else(|| self.identity.0.clone());
        let request = Request::new(Operation::Save {
            path: path.clone(),
            worktree: None,
            contents,
            expected: if path == self.identity.0 {
                self.version.clone()
            } else {
                None
            },
            overwrite,
        });
        self.saving = Some((
            request.id,
            path,
            file_editor_text_fingerprint(&self.input.read(cx).text_snapshot()),
        ));
        self.store.dispatch(Msg::FilesystemRequest(request));
        cx.notify();
    }

    fn discard(&mut self, cx: &mut gpui::Context<Self>) {
        // Discard is final even if the file vanished before the reload. Mark
        // the retained text clean and read-only while showing that error.
        self.dirty = false;
        self.editing = false;
        self.saved_fingerprint = Some(file_editor_text_fingerprint(
            &self.input.read(cx).text_snapshot(),
        ));
        self.input
            .update(cx, |input, cx| input.set_read_only(true, cx));
        self.reload(cx);
    }

    fn can_edit(&self) -> bool {
        let editable_image = self
            .identity
            .0
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        !((self.image && !editable_image)
            || self.loading
            || (self.error.is_some() && self.version.is_none() && !self.dirty)
            || self.text_format.is_some_and(|format| !format.is_writable()))
    }

    /// Enters edit mode, or leaves it once nothing is left to save.
    fn toggle_editing(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if self.editing {
            if self.dirty || self.saving.is_some() || self.has_edits(cx) {
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
            .update(cx, |input, cx| input.set_read_only(self.loading, cx));
        window.focus(&self.input.read(cx).focus_handle(), cx);
        cx.notify();
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let answer = cx.prompt_for_new_path(
            self.identity.0.parent().unwrap_or(Path::new("/")),
            self.identity.0.file_name().and_then(|n| n.to_str()),
        );
        cx.spawn_in(window, async move |view, cx| {
            if let Ok(Ok(Some(path))) = answer.await {
                let _ = view.update_in(cx, |this, _, cx| this.save(Some(path), false, cx));
            }
        })
        .detach();
    }

    fn refresh_syntax(&mut self, cx: &mut gpui::Context<Self>) {
        if self.loading || self.image {
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
        let language = rows::diff_syntax_language_for_path(&self.identity.0);
        let rope = snapshot.rope();
        let source_len = snapshot.len();
        let heuristic_rope = rope.clone();
        let provider = crate::kit::HighlightProvider::with_pending(
            move |range: std::ops::Range<usize>| crate::kit::HighlightProviderResult {
                highlights: language
                    .map(|language| {
                        panes::main::helpers::resolved_output_heuristic_highlights_for_range(
                            theme,
                            &heuristic_rope,
                            language,
                            range,
                        )
                    })
                    .unwrap_or_default(),
                pending: false,
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
                        let snapshot = syntax.snapshot(theme);
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
        let path = &self.identity.0;
        let name: SharedString = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
            .into();
        let icon = file_icons::file_icon_for_path(path);
        let icon_color = file_icons::file_icon_color(icon, theme.is_dark)
            .unwrap_or(theme.colors.foreground.secondary);
        let unsaved = self.dirty || self.saving.is_some();
        let save_shortcut = crate::view::shortcut_labels::secondary_shortcut("S");

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
            if self.text_format.is_some_and(|format| !format.is_writable()) {
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
            .when(!self.image && !self.loading, |controls| {
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
        if self.editing {
            controls = controls
                .child(
                    components::Button::new("document_discard", "Discard")
                        .style(components::ButtonStyle::Subtle)
                        .borderless()
                        .disabled(!self.dirty || self.saving.is_some())
                        .on_click(theme, cx, |this, _, _, cx| this.discard(cx))
                        .gitcomet_tooltip(theme, "Throw away the unsaved changes".into()),
                )
                .child(
                    components::Button::new("document_save", "Save")
                        .style(components::ButtonStyle::Outlined)
                        .disabled(!self.dirty || self.saving.is_some())
                        .on_click(theme, cx, |this, _, _, cx| this.save(None, false, cx))
                        .gitcomet_tooltip(theme, format!("Save ({save_shortcut})").into()),
                );
        }
        controls = controls
            .child(
                components::Button::new("document_save_as", "Save As…")
                    .style(components::ButtonStyle::Subtle)
                    .borderless()
                    .disabled(self.image || self.loading || (self.version.is_none() && !self.dirty))
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
            .when(self.dirty || self.failed_destination.is_some(), |d| {
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
                                        this.save(this.failed_destination.clone(), true, cx)
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
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let _ = self.subscriptions.len();
        // Text stays on screen through a failed save or a vanished file; only a
        // read that never produced any shows the failure in its place.
        let has_text = !self.image
            && !self.loading
            && (self.error.is_none()
                || self.version.is_some()
                || self.dirty
                || self.saving.is_some());
        let image_ready = self.image && !self.loading && self.error.is_none();
        let notice = self
            .error
            .clone()
            .filter(|_| has_text)
            .map(|error| self.render_notice(error, cx));
        let header = self.render_header(ui_scale_percent, cx);
        let body: AnyElement = if self.loading {
            components::empty_state_message(theme, "Loading…")
                .size_full()
                .into_any_element()
        } else if image_ready {
            div()
                .id("document_image")
                .debug_selector(|| "document_image".into())
                .size_full()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.colors.editor.background)
                .child(
                    gpui::img(self.identity.0.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
                .into_any_element()
        } else if has_text {
            self.render_text(ui_scale_percent, cx)
        } else {
            components::empty_state(
                theme,
                "Cannot open this file",
                self.error.clone().unwrap_or_default(),
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
    fn show_document_buffer(&mut self, id: gpui::EntityId, cx: &mut gpui::Context<Self>) {
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
