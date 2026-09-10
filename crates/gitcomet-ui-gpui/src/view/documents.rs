use super::panes::main::file_editor::{StashedFileEdit, file_editor_text_fingerprint};
use super::*;
use crate::kit::{TextInput, TextInputOptions};
use gitcomet_core::filesystem::{DiskVersion, DocumentIdentity, Operation, OperationId, Request};
use gitcomet_state::model::SidebarMode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
    picker: bool,
    search: Entity<TextInput>,
    scroll: gpui::ScrollHandle,
    row_menu: Option<(PathBuf, gpui::Point<Pixels>)>,
    subscriptions: Vec<gpui::Subscription>,
}

impl DocumentsView {
    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.search
            .update(cx, |input, cx| input.set_theme(theme, cx));
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
        let recents = shared_recents(cx);
        let search = cx.new(|cx| {
            TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Search documents…".into(),
                    leading_icon: Some("icons/zoom.svg"),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            )
        });
        let subscriptions = vec![
            cx.observe(&recents, |_, _, cx| cx.notify()),
            cx.observe(&search, |_, _, cx| cx.notify()),
        ];
        Self {
            theme,
            root,
            store,
            ui_model,
            recents,
            buffers: BTreeMap::new(),
            active: None,
            picker: true,
            search,
            scroll: gpui::ScrollHandle::new(),
            row_menu: None,
            subscriptions,
        }
    }

    pub(super) fn toggle_picker(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.picker = !self.picker || self.active.is_none();
        if self.picker {
            window.focus(&self.search.read(cx).focus_handle(), cx);
        }
        cx.notify();
    }

    fn insert_buffer(
        &mut self,
        path: PathBuf,
        initial: Option<(StashedFileEdit, Option<DiskVersion>)>,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::EntityId {
        let buffer = cx.new(|cx| {
            StandaloneBuffer::new(
                path,
                self.theme,
                self.store.clone(),
                self.ui_model.clone(),
                initial,
                cx,
            )
        });
        self.subscriptions
            .push(cx.observe(&buffer, |_, _, cx| cx.notify()));
        let id = buffer.entity_id();
        self.buffers.insert(id, buffer);
        id
    }

    pub(super) fn open(&mut self, path: PathBuf, display: bool, cx: &mut gpui::Context<Self>) {
        update_recent(path.clone(), false, cx);
        let existing = self
            .buffers
            .iter()
            .filter(|(_, b)| b.read(cx).identity.0 == path)
            .max_by_key(|(_, b)| b.read(cx).dirty)
            .map(|(id, _)| *id);
        let id = existing.unwrap_or_else(|| self.insert_buffer(path, None, cx));
        if display {
            self.active = Some(id);
            self.picker = false;
        }
        cx.notify();
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
                b.pauses.remove(&id);
                cx.notify();
            });
        }
        cx.notify();
    }

    fn picker_view(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        // The first section addresses buffers by ID, so a missing file or a
        // replacement at the same pathname cannot make unsaved text inaccessible.
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
        let items: Rc<[components::PickerPromptItem]> = entries
            .iter()
            .map(|(path, buffer)| {
                let item = components::PickerPromptItem::plain(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                )
                .secondary_parts([components::PickerPromptItemPart::path(
                    path.display().to_string(),
                )])
                .icon("icons/file.svg");
                if buffer.is_some() {
                    item.section("Unsaved documents")
                } else {
                    item.section("Recent documents").removable()
                }
            })
            .collect::<Vec<_>>()
            .into();
        let layout = Rc::new(components::picker_prompt_layout(
            &items,
            self.search.read(cx).text().trim(),
        ));
        let open_entries = entries.clone();
        let menu_entries = entries.clone();
        let prompt = components::PickerPrompt::new(self.search.clone(), self.scroll.clone())
            .prebuilt_items(items, layout)
            .remove_tooltip("Remove from recent documents")
            .accent_selection()
            .padded_query_row()
            .on_context_menu(cx.listener(
                move |this, event: &components::PickerPromptContextMenuEvent, _, cx| {
                    this.row_menu = menu_entries
                        .get(event.original_index)
                        .filter(|(_, id)| id.is_none())
                        .map(|(p, _)| (p.clone(), event.position));
                    cx.notify();
                },
            ))
            .render_with_remove(
                self.theme,
                ui_scale::current(cx).percent,
                cx,
                move |this, ix, _, _, cx| {
                    if let Some((path, buffer)) = open_entries.get(ix) {
                        if let Some(id) = buffer {
                            this.active = Some(*id);
                            this.picker = false;
                            cx.notify();
                        } else {
                            let path = path.clone();
                            let _ = this
                                .root
                                .update(cx, |root, cx| root.open_document_paths(vec![path], cx));
                        }
                    }
                },
                move |_, ix, _, cx| {
                    if let Some((path, None)) = entries.get(ix) {
                        update_recent(path.clone(), true, cx);
                    }
                },
            );
        let theme = self.theme;
        div()
            .id("documents_picker")
            .debug_selector(|| "documents_picker".into())
            .w(px(560.))
            .max_w_full()
            .bg(theme.colors.surface.raised)
            .border_1()
            .border_color(theme.colors.stroke.default)
            .child(
                components::Button::new("documents_open_file", "Open File…").on_click(
                    theme,
                    cx,
                    |this, _, _, cx| {
                        let _ = this
                            .root
                            .update(cx, |root, cx| root.prompt_open_document(cx));
                    },
                ),
            )
            .child(
                components::Button::new("documents_save_all", "Save All").on_click(
                    theme,
                    cx,
                    |this, _, _, cx| {
                        let _ = this.root.update(cx, |root, cx| {
                            root.main_pane
                                .update(cx, |pane, cx| pane.save_all_file_edits(cx))
                        });
                        this.save_all(cx);
                    },
                ),
            )
            .child(prompt)
            .when_some(self.row_menu.clone(), |d, (path, position)| {
                d.child(
                    gpui::anchored().position(position).snap_to_window().child(
                        components::popover_surface(theme)
                            .id("documents_row_menu")
                            .occlude()
                            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.row_menu = None;
                                cx.notify();
                            }))
                            .child(components::context_menu(
                                theme,
                                div().p_1().child(
                                    components::ContextMenuEntry::new(
                                        "documents_remove_recent",
                                        components::ContextMenuText::new(
                                            "Remove from recent documents",
                                        ),
                                    )
                                    .render(theme, ui_scale::current(cx).percent, cx)
                                    .on_click(cx.listener(
                                        move |this, _: &gpui::ClickEvent, _, cx| {
                                            update_recent(path.clone(), true, cx);
                                            this.row_menu = None;
                                            cx.notify();
                                        },
                                    )),
                                ),
                            )),
                    ),
                )
            })
            .into_any_element()
    }
}

impl Render for DocumentsView {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let content = self
            .active
            .as_ref()
            .and_then(|p| self.buffers.get(p))
            .cloned();
        let picker = self.picker.then(|| self.picker_view(cx));
        div()
            .id("documents_canvas")
            .debug_selector(|| "documents_canvas".into())
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(self.theme.colors.surface.canvas)
            .when_some(content, |d, content| d.child(content))
            .when_some(picker, |d, picker| {
                d.child(div().absolute().bottom_2().left_2().child(picker))
            })
    }
}

struct StandaloneBuffer {
    identity: DocumentIdentity,
    theme: AppTheme,
    store: Arc<AppStore>,
    input: Entity<TextInput>,
    scroll: gpui::ScrollHandle,
    syntax_key: Option<(u64, u64)>,
    syntax_serial: u64,
    syntax_task: Option<gpui::Task<()>>,
    path_input: Entity<TextInput>,
    version: Option<DiskVersion>,
    saved: SharedString,
    saved_fingerprint: Option<u64>,
    load_generation: u64,
    dirty: bool,
    editing: bool,
    loading: bool,
    image: bool,
    error: Option<String>,
    saving: Option<(OperationId, PathBuf, SharedString, u64)>,
    failed_destination: Option<PathBuf>,
    pauses: std::collections::BTreeSet<OperationId>,
    subscriptions: Vec<gpui::Subscription>,
}

impl StandaloneBuffer {
    fn new(
        path: PathBuf,
        theme: AppTheme,
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
            let mut input = TextInput::new_inert(
                TextInputOptions {
                    read_only: true,
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            );
            input.set_text(path.display().to_string(), cx);
            input
        });
        let subscriptions = vec![
            cx.observe(&input, |this, input, cx| {
                if !this.loading {
                    this.dirty = this.editing
                        && Some(file_editor_text_fingerprint(
                            &input.read(cx).text_snapshot(),
                        )) != this.saved_fingerprint;
                    this.refresh_syntax(cx);
                }
                cx.notify();
            }),
            cx.observe(&ui_model, |this, model, cx| {
                let Some((id, _, _, _)) = &this.saving else {
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
                let (_, path, saved, fingerprint) = this.saving.take().unwrap();
                this.store
                    .dispatch(Msg::AcknowledgeFilesystemResults(vec![result.id]));
                if result.succeeded() {
                    this.identity = DocumentIdentity(path);
                    this.path_input.update(cx, |input, cx| {
                        input.set_text(this.identity.0.display().to_string(), cx)
                    });
                    this.saved = saved;
                    this.version = result.saved_version.clone();
                    this.saved_fingerprint = Some(fingerprint);
                    this.dirty = this.input.read(cx).text() != this.saved.as_ref();
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
        let mut buffer = Self {
            identity: DocumentIdentity(path),
            theme,
            store,
            input,
            scroll,
            syntax_key: None,
            syntax_serial: 0,
            syntax_task: None,
            path_input,
            version: None,
            saved: SharedString::default(),
            saved_fingerprint: None,
            load_generation: 0,
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
            buffer.saved_fingerprint = Some(edit.saved_fingerprint);
            buffer.dirty = true;
            buffer.input.update(cx, |input, cx| {
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
        let path = self.identity.0.clone();
        let identity = self.identity.clone();
        let image = self.image;
        cx.spawn(async move |view, cx| {
            let result = crate::ui_runtime::background_compute(move || {
                let _guard = gitcomet_core::filesystem::global()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let version = DiskVersion::read(&path).map_err(|e| e.to_string())?;
                let text = if image {
                    SharedString::default()
                } else {
                    panes::read_worktree_file_for_editing(&path)?
                };
                if DiskVersion::read(&path).map_err(|e| e.to_string())? != version {
                    return Err("File changed while reading. Open it again.".into());
                }
                Ok::<_, String>((version, text))
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
                    Ok((version, text)) => {
                        this.version = Some(version);
                        this.saved = text.clone();
                        this.input.update(cx, |input, cx| input.set_text(text, cx));
                        this.saved_fingerprint = Some(file_editor_text_fingerprint(
                            &this.input.read(cx).text_snapshot(),
                        ));
                        this.dirty = false;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
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
        let path = destination.unwrap_or_else(|| self.identity.0.clone());
        let contents: SharedString = self.input.read(cx).text().to_string().into();
        let request = Request::new(Operation::Save {
            path: path.clone(),
            contents: Arc::from(contents.as_bytes()),
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
            contents,
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

impl Render for StandaloneBuffer {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let row_height = ui_scale::design_px_from_percent(20.0, ui_scale::current(cx).percent);
        self.input
            .update(cx, |input, cx| input.set_line_height(Some(row_height), cx));
        let editable_image = self
            .identity
            .0
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        let error = self.error.clone();
        let _ = self.subscriptions.len();
        div()
            .id("standalone_document")
            .size_full()
            .flex()
            .flex_col()
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if (event.keystroke.modifiers.platform || event.keystroke.modifiers.control)
                    && event.keystroke.key == "s"
                {
                    cx.stop_propagation();
                    this.save(None, false, cx);
                }
                let _ = window;
            }))
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(theme.colors.stroke.default)
                    .child(self.path_input.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_2()
                    .child(
                        components::Button::new(
                            "document_edit",
                            if self.editing { "Editing" } else { "Edit" },
                        )
                        .disabled(
                            (self.image && !editable_image)
                                || self.loading
                                || (self.error.is_some() && self.version.is_none()),
                        )
                        .on_click(theme, cx, |this, _, window, cx| {
                            this.editing = true;
                            if this.image {
                                this.image = false;
                                this.reload(cx);
                            }
                            this.input
                                .update(cx, |input, cx| input.set_read_only(false, cx));
                            window.focus(&this.input.read(cx).focus_handle(), cx);
                            cx.notify();
                        }),
                    )
                    .child(
                        components::Button::new("document_save", "Save")
                            .disabled(!self.dirty || self.saving.is_some())
                            .on_click(theme, cx, |this, _, _, cx| this.save(None, false, cx)),
                    )
                    .child(
                        components::Button::new("document_save_as", "Save As…")
                            .disabled(
                                self.image
                                    || self.loading
                                    || (self.version.is_none() && !self.dirty),
                            )
                            .on_click(theme, cx, |this, _, window, cx| {
                                let answer = cx.prompt_for_new_path(
                                    this.identity.0.parent().unwrap_or(Path::new("/")),
                                    this.identity.0.file_name().and_then(|n| n.to_str()),
                                );
                                cx.spawn_in(window, async move |view, cx| {
                                    if let Ok(Ok(Some(path))) = answer.await {
                                        let _ = view.update_in(cx, |this, _, cx| {
                                            this.save(Some(path), false, cx)
                                        });
                                    }
                                })
                                .detach();
                            }),
                    )
                    .when(self.dirty, |d| d.child(div().text_sm().child("Unsaved"))),
            )
            .when_some(error, |d, error| {
                d.child(div().p_2().child(error).when(self.dirty, |d| {
                    d.child(
                        components::Button::new("document_replace_disk", "Replace disk contents…")
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
                }))
            })
            .when(self.loading, |d| d.child("Loading document…"))
            .when(self.image && !self.loading && self.error.is_none(), |d| {
                d.child(
                    gpui::img(self.identity.0.clone())
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
            })
            .when(!self.image && !self.loading, |d| {
                d.child(
                    div()
                        .id("document_editor_scroll")
                        .debug_selector(|| "document_editor_scroll".into())
                        .flex_1()
                        .min_h(px(0.))
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .items_start()
                        .font_family(crate::font_preferences::current_editor_font_family(cx))
                        .bg(theme.colors.editor.background)
                        .overflow_scroll()
                        .track_scroll(&self.scroll)
                        .child(self.input.clone()),
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
    pub(in crate::view) fn show_repository_canvas(&mut self, cx: &mut gpui::Context<Self>) {
        self.documents_active = false;
        for (_, _, display) in &mut self.document_routing.pending {
            *display = false;
        }
        self.document_routing.generation = self.document_routing.generation.wrapping_add(1);
        self.bottom_status_bar.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    pub(in crate::view) fn toggle_documents(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        for (_, _, display) in &mut self.document_routing.pending {
            *display = false;
        }
        self.document_routing.generation = self.document_routing.generation.wrapping_add(1);
        self.documents_active = true;
        self.documents
            .update(cx, |docs, cx| docs.toggle_picker(window, cx));
        self.bottom_status_bar.update(cx, |_, cx| cx.notify());
        cx.notify();
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

    pub(in crate::view) fn open_document_paths(
        &mut self,
        paths: Vec<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        super::native_transfers::mark_handled(&paths, cx);
        for (_, _, display) in &mut self.document_routing.pending {
            *display = false;
        }
        self.document_routing.generation = self.document_routing.generation.wrapping_add(1);
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
                this.bottom_status_bar.update(cx, |_, cx| cx.notify());
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
            let Ok(path) = path.strip_prefix(&root).map(Path::to_path_buf) else {
                continue;
            };
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
        self.bottom_status_bar.update(cx, |_, cx| cx.notify());
        cx.notify();
    }
}
