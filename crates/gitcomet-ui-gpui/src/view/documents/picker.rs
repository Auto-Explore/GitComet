//! The status bar's Documents picker: recent and unsaved files, a typed path,
//! and the system Open dialog, floating above the button that opened it.

use super::*;
use std::rc::Rc;

const PICKER_WIDTH_PX: f32 = 520.0;
/// Space between the picker and the button it rises from.
const ANCHOR_GAP_PX: f32 = 6.0;

#[derive(Clone, Debug, PartialEq)]
enum PickerEntry {
    /// An absolute (or `~`) path typed into the query.
    Typed(PathBuf),
    /// Unsaved text, addressed by buffer so a missing file cannot hide it.
    Unsaved(PathBuf, gpui::EntityId),
    Recent(PathBuf),
}

/// The query as a path to open directly, when it spells one.
fn typed_path(query: &str) -> Option<PathBuf> {
    let query = query.trim();
    let path = match query.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
            PathBuf::from(home).join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(query),
    };
    path.is_absolute().then_some(path)
}

fn picker_item(entry: &PickerEntry, query: &str) -> components::PickerPromptItem {
    let path = match entry {
        PickerEntry::Typed(path) | PickerEntry::Unsaved(path, _) | PickerEntry::Recent(path) => {
            path
        }
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let parent = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    let icon = file_icons::file_icon_for_path(path);
    match entry {
        // Carries the query itself so the row survives the filter it creates.
        PickerEntry::Typed(_) => {
            components::PickerPromptItem::from_parts([components::PickerPromptItemPart::path(
                query.trim().to_string(),
            )])
            .secondary_parts([components::PickerPromptItemPart::new("Open this path")
                .searchable(false)
                .tooltip(false)])
            .icon(icon)
            .section("Path")
        }
        PickerEntry::Unsaved(..) => components::PickerPromptItem::plain(name)
            .secondary_parts([components::PickerPromptItemPart::path(parent)])
            .icon(icon)
            .section("Unsaved"),
        PickerEntry::Recent(_) => components::PickerPromptItem::plain(name)
            .secondary_parts([components::PickerPromptItemPart::path(parent)])
            .icon(icon)
            .section("Recent")
            .removable(),
    }
}

pub(crate) struct DocumentPicker {
    theme: AppTheme,
    root: WeakEntity<GitCometView>,
    documents: WeakEntity<DocumentsView>,
    recents: Entity<RecentDocumentList>,
    search: Entity<TextInput>,
    scroll: gpui::ScrollHandle,
    /// Repainted by id when the picker opens or closes; it draws the button.
    status_bar: gpui::EntityId,
    open: bool,
    anchor: Bounds<Pixels>,
    query: String,
    /// Display (post-filter) index of the keyboard selection.
    selected: Option<usize>,
    row_menu: Option<(PathBuf, gpui::Point<Pixels>)>,
    restore_focus: Option<FocusHandle>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl DocumentPicker {
    pub(in crate::view) fn new(
        theme: AppTheme,
        root: WeakEntity<GitCometView>,
        documents: &Entity<DocumentsView>,
        status_bar: gpui::EntityId,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let recents = shared_recents(cx);
        let search = cx.new(|cx| {
            let mut input = TextInput::new_inert(
                TextInputOptions {
                    placeholder: "Search recent files or type a path…".into(),
                    leading_icon: Some("icons/zoom.svg"),
                    chromeless: true,
                    ..Default::default()
                },
                cx,
            );
            input.set_theme(theme, cx);
            input
        });
        let subscriptions = vec![
            cx.observe(&recents, |this, _, cx| {
                if this.open {
                    cx.notify();
                }
            }),
            cx.observe(documents, |this, _, cx| {
                if this.open {
                    cx.notify();
                }
            }),
            cx.observe_in(&search, window, |this, input, window, cx| {
                this.handle_input(input, window, cx);
            }),
        ];
        Self {
            theme,
            root,
            documents: documents.downgrade(),
            recents,
            search,
            scroll: gpui::ScrollHandle::new(),
            status_bar,
            open: false,
            anchor: Bounds::default(),
            query: String::new(),
            selected: None,
            row_menu: None,
            restore_focus: None,
            _subscriptions: subscriptions,
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.search
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    pub(in crate::view) fn is_open(&self) -> bool {
        self.open
    }

    /// Opens above `anchor`, or closes when already open.
    pub(in crate::view) fn toggle(
        &mut self,
        anchor: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.open {
            self.close(window, cx);
            return;
        }
        self.open = true;
        self.anchor = anchor;
        self.query.clear();
        self.selected = None;
        self.row_menu = None;
        self.scroll.set_offset(point(px(0.0), px(0.0)));
        let focus = self.search.read(cx).focus_handle();
        self.restore_focus = window.focused(cx).filter(|focused| *focused != focus);
        self.search.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_text("", cx);
        });
        window.focus(&focus, cx);
        gpui::AppContext::notify(cx, self.status_bar);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        self.row_menu = None;
        if let Some(focus) = self.restore_focus.take() {
            window.focus(&focus, cx);
        } else {
            window.blur(cx);
        }
        gpui::AppContext::notify(cx, self.status_bar);
        cx.notify();
    }

    fn entries(&self, cx: &gpui::App) -> Vec<PickerEntry> {
        let mut entries: Vec<PickerEntry> = typed_path(&self.query)
            .map(PickerEntry::Typed)
            .into_iter()
            .collect();
        match self.documents.upgrade() {
            Some(documents) => {
                entries.extend(documents.read(cx).picker_entries(cx).into_iter().map(
                    |(path, buffer)| match buffer {
                        Some(id) => PickerEntry::Unsaved(path, id),
                        None => PickerEntry::Recent(path),
                    },
                ))
            }
            None => entries.extend(
                self.recents
                    .read(cx)
                    .paths
                    .iter()
                    .cloned()
                    .map(PickerEntry::Recent),
            ),
        }
        entries
    }

    fn items(&self, entries: &[PickerEntry]) -> Rc<[components::PickerPromptItem]> {
        entries
            .iter()
            .map(|entry| picker_item(entry, &self.query))
            .collect::<Vec<_>>()
            .into()
    }

    fn handle_input(
        &mut self,
        input: Entity<TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (escape, up, shift_tab, down, tab, enter) = input.update(cx, |input, _| {
            (
                input.take_escape_pressed(),
                input.take_arrow_up_pressed(),
                input.take_shift_tab_pressed(),
                input.take_arrow_down_pressed(),
                input.take_tab_pressed(),
                input.take_enter_pressed(),
            )
        });
        if !self.open {
            return;
        }
        if escape {
            self.close(window, cx);
            return;
        }
        let query = input.read(cx).text().trim().to_string();
        let query_changed = query != self.query;
        // The input also notifies for caret blinks and selection moves.
        if !query_changed && !up && !shift_tab && !down && !tab && !enter {
            return;
        }
        if query_changed {
            self.query = query;
            self.selected = None;
            self.scroll.set_offset(point(px(0.0), px(0.0)));
        }
        let entries = self.entries(cx);
        let items = self.items(&entries);
        let layout = components::picker_prompt_layout(&items, &self.query);
        let len = layout.item_indices.len();
        if up || shift_tab {
            self.selected = match (self.selected, len) {
                (_, 0) => None,
                (Some(ix), _) if ix > 0 => Some(ix.min(len) - 1),
                (_, len) => Some(len - 1),
            };
        } else if down || tab {
            self.selected = match (self.selected, len) {
                (_, 0) => None,
                (Some(ix), len) if ix + 1 < len => Some(ix + 1),
                _ => Some(0),
            };
        } else if enter {
            // Without arrowing, Enter takes the first row the query left.
            let ix = self.selected.unwrap_or(0).min(len.saturating_sub(1));
            if let Some(entry) = layout
                .item_indices
                .get(ix)
                .and_then(|original| entries.get(*original))
                .cloned()
            {
                self.activate(entry, window, cx);
            }
            return;
        }
        if let Some(child) = self
            .selected
            .and_then(|ix| layout.child_indices.get(ix).copied())
        {
            self.scroll.scroll_to_item(child);
        }
        cx.notify();
    }

    fn activate(&mut self, entry: PickerEntry, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.close(window, cx);
        let _ = self.root.update(cx, |root, cx| match entry {
            PickerEntry::Unsaved(_, id) => root.show_document_buffer(id, cx),
            PickerEntry::Typed(path) | PickerEntry::Recent(path) => {
                root.open_document_paths(vec![path], cx)
            }
        });
    }

    fn browse(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        self.close(window, cx);
        let _ = self
            .root
            .update(cx, |root, cx| root.prompt_open_document(cx));
    }

    fn render_footer(&self, has_unsaved: bool, cx: &mut gpui::Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let scaled_px = crate::ui_scale::scaler(ui_scale::current(cx).percent);
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px(scaled_px(8.0))
            .py(scaled_px(6.0))
            .border_t_1()
            .border_color(theme.colors.stroke.subtle)
            .child(
                components::Button::new("documents_open_file", "Open File…")
                    .start_slot(svg_icon(
                        "icons/folder.svg",
                        theme.colors.foreground.secondary,
                        scaled_px(14.0),
                    ))
                    .style(components::ButtonStyle::Subtle)
                    .borderless()
                    .on_click(theme, cx, |this, _, window, cx| this.browse(window, cx))
                    .gitcomet_tooltip(theme, "Choose any file to view or edit".into()),
            )
            .when(has_unsaved, |footer| {
                footer.child(
                    components::Button::new("documents_save_all", "Save All")
                        .style(components::ButtonStyle::Subtle)
                        .borderless()
                        .on_click(theme, cx, |this, _, _, cx| {
                            let _ = this.root.update(cx, |root, cx| {
                                root.main_pane
                                    .update(cx, |pane, cx| pane.save_all_file_edits(cx));
                                root.documents.update(cx, |docs, cx| docs.save_all(cx));
                            });
                        })
                        .gitcomet_tooltip(theme, "Save every unsaved document".into()),
                )
            })
    }

    fn render_row_menu(
        &self,
        path: PathBuf,
        position: gpui::Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        gpui::anchored()
            .position(position)
            .snap_to_window()
            .child(
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
                                components::ContextMenuText::new("Remove from recent documents"),
                            )
                            .on_select(
                                theme,
                                ui_scale::current(cx).percent,
                                cx,
                                move |this, _: &gpui::ClickEvent, _, cx| {
                                    update_recent(path.clone(), true, cx);
                                    this.row_menu = None;
                                    cx.notify();
                                },
                            ),
                        ),
                    )),
            )
            .into_any_element()
    }
}

impl Render for DocumentPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let theme = self.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let entries = self.entries(cx);
        let has_unsaved = entries
            .iter()
            .any(|entry| matches!(entry, PickerEntry::Unsaved(..)));
        let items = self.items(&entries);
        let layout = Rc::new(components::picker_prompt_layout(&items, &self.query));
        let select_entries = entries.clone();
        let menu_entries = entries.clone();
        let prompt = components::PickerPrompt::new(self.search.clone(), self.scroll.clone())
            .prebuilt_items(items, layout)
            .selected_index(self.selected)
            .selected_hint("Enter")
            .empty_text("No recent documents")
            .remove_tooltip("Remove from recent documents")
            .padded_query_row()
            .on_context_menu(cx.listener(
                move |this, event: &components::PickerPromptContextMenuEvent, _, cx| {
                    this.row_menu = match menu_entries.get(event.original_index) {
                        Some(PickerEntry::Recent(path)) => Some((path.clone(), event.position)),
                        _ => None,
                    };
                    cx.notify();
                },
            ))
            .render_with_remove(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, _, window, cx| {
                    if let Some(entry) = select_entries.get(ix).cloned() {
                        this.activate(entry, window, cx);
                    }
                },
                move |_, ix, _, cx| {
                    if let Some(PickerEntry::Recent(path)) = entries.get(ix) {
                        update_recent(path.clone(), true, cx);
                    }
                },
            );
        let anchor = point(
            self.anchor.right(),
            self.anchor.top() - scaled_px(ANCHOR_GAP_PX),
        );
        let row_menu = self
            .row_menu
            .clone()
            .map(|(path, position)| self.render_row_menu(path, position, cx));
        let footer = self.render_footer(has_unsaved, cx);
        let dismiss = cx.listener(|this, _: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            this.close(window, cx);
        });
        div()
            .id("documents_picker_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("documents_picker_scrim")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .occlude()
                    .on_any_mouse_down(dismiss),
            )
            .child(
                gpui::anchored()
                    .position(anchor)
                    .anchor(Anchor::BottomRight)
                    .snap_to_window_with_margin(scaled_px(8.0))
                    .child(
                        components::popover_surface(theme)
                            .id("documents_picker")
                            .debug_selector(|| "documents_picker".into())
                            .w(scaled_px(PICKER_WIDTH_PX))
                            .flex()
                            .flex_col()
                            .occlude()
                            .child(prompt)
                            .child(footer),
                    ),
            )
            .children(row_menu)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::typed_path;
    use std::path::PathBuf;

    #[test]
    fn only_absolute_and_home_relative_queries_are_paths() {
        assert_eq!(typed_path("notes"), None);
        assert_eq!(typed_path("docs/notes.md"), None);
        #[cfg(unix)]
        assert_eq!(
            typed_path(" /tmp/notes.md "),
            Some(PathBuf::from("/tmp/notes.md"))
        );
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(
                typed_path("~/notes.md"),
                Some(PathBuf::from(home).join("notes.md"))
            );
        }
        assert_eq!(typed_path("~notes"), None);
    }
}
