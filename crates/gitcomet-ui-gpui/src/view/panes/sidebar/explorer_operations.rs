use super::*;
use crate::kit::{ScrollbarAxis, ScrollbarDriver};
use gitcomet_core::domain::FileSource;
use gitcomet_core::filesystem::{Operation, Request, TransferIntent};
use std::path::Path;
use std::time::Duration;

/// How long a dragged item has to rest on a collapsed folder before it opens.
const EXPLORER_HOVER_EXPAND_DELAY: Duration = Duration::from_millis(600);
/// How close to an edge of the tree a dragged item has to get before the list
/// starts scrolling under it. Design pixels, scaled with the UI.
const EXPLORER_DRAG_SCROLL_EDGE_PX: f32 = 24.0;
/// Speed of that scroll, integrated over elapsed time so it does not depend on
/// how promptly the executor runs the ticks.
const EXPLORER_DRAG_SCROLL_PX_PER_SEC: f32 = 400.0;
/// Ceiling on the gap between two steps, so a stalled frame cannot launch the
/// list across a whole screen at once.
const EXPLORER_DRAG_SCROLL_MAX_STEP: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum ExplorerAction {
    NewFile,
    NewFolder,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Rename,
    Trash,
    Delete,
    Undo,
    Redo,
}

pub(super) struct NameEdit {
    pub repo_id: RepoId,
    pub path: PathBuf,
    pub action: ExplorerAction,
    pub is_directory: bool,
    pub reveal: bool,
}

/// Holding the rows keeps pointer identity safe as a cache key across tree revisions.
pub(super) struct DropRegion {
    rows: Rc<[FileBrowserVisibleRow]>,
    target: PathBuf,
    range: std::ops::Range<usize>,
}

type ExplorerHit = (PathBuf, Option<(Arc<PathBuf>, bool)>);

#[derive(Clone)]
pub(in crate::view) struct ExplorerDrag {
    /// Shared by the selected rows and the active transfer.
    pub paths: Rc<[PathBuf]>,
}

pub(super) struct ExplorerDragPreview {
    label: SharedString,
    icon: Option<&'static str>,
    theme: AppTheme,
    grab_offset: gpui::Point<Pixels>,
}

impl ExplorerDragPreview {
    pub(super) fn new(
        drag: &ExplorerDrag,
        is_directory: bool,
        theme: AppTheme,
        grab_offset: gpui::Point<Pixels>,
    ) -> Self {
        let single = drag.paths.len() == 1;
        let path = drag
            .paths
            .first()
            .map(PathBuf::as_path)
            .unwrap_or(Path::new(""));
        let label = if single {
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{} items", drag.paths.len())
        };
        let icon = single.then(|| {
            if is_directory {
                file_icons::folder_icon(false)
            } else {
                file_icons::file_icon_for_path(path)
            }
        });
        Self {
            label: label.into(),
            icon,
            theme,
            grab_offset,
        }
    }
}

impl Render for ExplorerDragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let scale = ui_scale::current(cx).percent;
        let scaled = |value| ui_scale::design_px_from_percent(value, scale);
        // GPUI subtracts the row's grab offset. Cancel it here so the bubble
        // follows the cursor at a fixed distance, regardless of where we grabbed.
        div()
            .pl(self.grab_offset.x + scaled(12.0))
            .pt(self.grab_offset.y + scaled(16.0))
            .child(
                div()
                    .debug_selector(|| "explorer_drag_preview".into())
                    .flex()
                    .items_center()
                    .gap(scaled(6.0))
                    .max_w(scaled(320.0))
                    .px(scaled(8.0))
                    .py(scaled(4.0))
                    .rounded(scaled(theme.radii.row.max(6.0)))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.raised)
                    .text_color(theme.colors.foreground.primary)
                    .text_size(scaled(12.0))
                    .when_some(self.icon, |bubble, icon| {
                        let tint = file_icons::file_icon_color(icon, theme.is_dark)
                            .unwrap_or(theme.colors.foreground.secondary);
                        bubble.child(crate::view::icons::svg_icon(icon, tint, scaled(14.0)))
                    })
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.label.clone()),
                    )
                    .when(copy_modifier(window.modifiers()), |bubble| {
                        bubble.child(
                            div()
                                .debug_selector(|| "explorer_drag_copy_badge".into())
                                .flex_none()
                                .child("+"),
                        )
                    }),
            )
    }
}

fn copy_modifier(modifiers: gpui::Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.control
    }
}

impl SidebarPaneView {
    pub(super) fn show_repository_canvas(&self, cx: &mut gpui::Context<Self>) {
        let _ = self
            .root_view
            .update(cx, |root, cx| root.show_repository_canvas(cx));
    }

    fn explorer_visible_paths(&self, cx: &gpui::App) -> Vec<PathBuf> {
        let Some(repo) = self.active_repo() else {
            return vec![];
        };
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            return vec![];
        };
        self.file_browser_visible_rows(cx)
            .iter()
            .filter_map(|row| {
                row.entry_index()
                    .and_then(|i| entries.get(i))
                    .map(|e| (*e.path).clone())
            })
            .collect()
    }

    pub(super) fn explorer_select(
        &mut self,
        path: PathBuf,
        modifiers: gpui::Modifiers,
        menu: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        window.focus(&self.explorer_focus, cx);
        self.explorer_pending_focus = Some((repo_id, path.clone()));
        self.store.dispatch(Msg::SelectExplorerPath {
            repo_id,
            path,
            visible: self.explorer_visible_paths(cx),
            toggle: modifiers.control || modifiers.platform,
            range: modifiers.shift,
            context_menu: menu,
        });
    }

    fn explorer_target(&self, path: Option<&Path>) -> PathBuf {
        let is_directory = path.is_some_and(|path| self.active_repo().is_some_and(|r| matches!(&r.file_browser.entries, Loadable::Ready(entries) if entries.iter().any(|entry| entry.path.as_path() == path && entry.kind == FileEntryKind::Directory))));
        gitcomet_state::explorer::Selection::destination(path, is_directory)
    }

    fn explorer_focused_path(&self) -> Option<&Path> {
        self.explorer_pending_focus
            .as_ref()
            .filter(|(id, _)| self.active_repo_id() == Some(*id))
            .map(|(_, path)| path.as_path())
            .or_else(|| {
                self.active_repo()?
                    .file_browser
                    .selection
                    .focused
                    .as_deref()
            })
    }

    fn explorer_focus_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(repo_id) = self.active_repo_id() {
            window.focus(&self.explorer_focus, cx);
            self.explorer_pending_focus = Some((repo_id, path.clone()));
            self.store
                .dispatch(Msg::FocusExplorerPath { repo_id, path });
        }
    }

    pub(super) fn explorer_folder_click(
        &mut self,
        path: PathBuf,
        modifiers: gpui::Modifiers,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if modifiers.control || modifiers.platform || modifiers.shift {
            self.explorer_select(path, modifiers, false, window, cx);
        } else if let Some(repo_id) = self.active_repo_id() {
            self.explorer_focus_path(path.clone(), window, cx);
            // The reducer preserves the forced expansion of a filtered tree.
            self.store
                .dispatch(Msg::ToggleFileBrowserDir { repo_id, path });
        }
    }

    pub(super) fn explorer_background_click(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(
            self.explorer_hit_at(window.mouse_position(), cx),
            Some((_, None))
        ) {
            self.explorer_focus_path(PathBuf::new(), window, cx);
        }
    }

    pub(super) fn explorer_sources(&self, path: Option<&Path>) -> Vec<PathBuf> {
        let Some(repo) = self.active_repo() else {
            return vec![];
        };
        let selected = &repo.file_browser.selection.paths;
        if path.is_none_or(|p| selected.contains(p)) && !selected.is_empty() {
            selected.iter().map(|p| repo.spec.workdir.join(p)).collect()
        } else {
            path.filter(|p| !p.as_os_str().is_empty())
                .map(|p| vec![repo.spec.workdir.join(p)])
                .unwrap_or_default()
        }
    }

    pub(in crate::view) fn explorer_action(
        &mut self,
        action: ExplorerAction,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        if repo.file_browser.source != FileSource::WorkingDirectory {
            return;
        }
        let repo_id = repo.id;
        let root = repo.spec.workdir.clone();
        // Keyboard source actions pass None: selection wins, with focus as a
        // fallback. Menus pass their explicit path; paste passes its destination.
        let source_path = path.as_deref().or_else(|| {
            repo.file_browser
                .selection
                .paths
                .is_empty()
                .then_some(self.explorer_focused_path())
                .flatten()
        });
        let sources = self.explorer_sources(source_path);
        let destination = root.join(self.explorer_target(path.as_deref()));
        let mut ownership = None;
        let operation = match action {
            ExplorerAction::Copy | ExplorerAction::Cut => {
                #[cfg(target_os = "windows")]
                if action == ExplorerAction::Cut {
                    let _ = self
                        .root_view
                        .update(cx, |root, cx| root.prepare_native_cut(sources, cx));
                    return;
                }
                if !sources.is_empty() {
                    crate::clipboard::write_files(
                        cx,
                        sources,
                        if action == ExplorerAction::Cut {
                            TransferIntent::Move
                        } else {
                            TransferIntent::Copy
                        },
                    );
                }
                cx.notify();
                return;
            }
            ExplorerAction::Paste => {
                let Some(mut payload) = crate::clipboard::read_files(cx) else {
                    return;
                };
                if cfg!(target_os = "macos") && window.modifiers().alt {
                    payload.intent = TransferIntent::Move;
                }
                ownership = Some(payload.ownership);
                Operation::Transfer {
                    sources: payload.paths,
                    destination,
                    intent: payload.intent,
                }
            }
            ExplorerAction::NewFile | ExplorerAction::NewFolder | ExplorerAction::Rename => {
                let path = if action == ExplorerAction::Rename {
                    if sources.len() != 1 {
                        return;
                    }
                    sources[0].clone()
                } else {
                    destination
                };
                let initial = if action == ExplorerAction::Rename {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    String::new()
                };
                let is_directory = action == ExplorerAction::NewFolder
                    || (action == ExplorerAction::Rename
                        && matches!(&repo.file_browser.entries,
                        Loadable::Ready(entries) if entries.iter().any(|entry|
                            entry.kind == FileEntryKind::Directory && root.join(entry.path.as_ref()) == path)));
                self.explorer_name_input.update(cx, |input, cx| {
                    input.set_text(initial, cx);
                });
                if action != ExplorerAction::Rename
                    && let Ok(relative) = path.strip_prefix(&root)
                    && !relative.as_os_str().is_empty()
                    && !repo
                        .file_browser
                        .expanded_dirs
                        .contains(&relative.to_path_buf())
                {
                    self.store.dispatch(Msg::ToggleFileBrowserDir {
                        repo_id,
                        path: relative.to_path_buf(),
                    });
                }
                self.explorer_name_edit = Some(NameEdit {
                    repo_id,
                    path,
                    action,
                    is_directory,
                    reveal: true,
                });
                window.focus(&self.explorer_name_input.read(cx).focus_handle(), cx);
                cx.notify();
                return;
            }
            ExplorerAction::Duplicate => Operation::Duplicate { sources },
            ExplorerAction::Trash => Operation::Trash { sources },
            ExplorerAction::Delete => Operation::DeletePermanently {
                sources,
                confirmed: false,
            },
            ExplorerAction::Undo => Operation::Undo,
            ExplorerAction::Redo => Operation::Redo,
        };
        let _ = self.root_view.update(cx, |root, cx| {
            root.submit_filesystem_operation(Request::new(operation), ownership, window, cx)
        });
    }

    pub(super) fn explorer_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.explorer_name_edit.is_some() {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.explorer_name_edit = None;
                    window.focus(&self.explorer_focus, cx);
                    cx.stop_propagation();
                    cx.notify();
                }
                "enter" => {
                    cx.stop_propagation();
                    let name = self.explorer_name_input.read(cx).text().to_string();
                    if let Err(error) =
                        gitcomet_core::filesystem::validate_name(std::ffi::OsStr::new(&name))
                    {
                        self.store.dispatch(Msg::ShowBannerError {
                            repo_id: self.active_repo_id(),
                            message: error.to_string(),
                        });
                        return;
                    }
                    let edit = self.explorer_name_edit.take().unwrap();
                    if self.active_repo_id() != Some(edit.repo_id) {
                        return;
                    }
                    let operation = match edit.action {
                        ExplorerAction::Rename => Operation::Rename {
                            source: edit.path,
                            name: name.into(),
                        },
                        ExplorerAction::NewFolder => Operation::CreateDirectory {
                            path: edit.path.join(name),
                        },
                        _ => Operation::CreateFile {
                            path: edit.path.join(name),
                        },
                    };
                    window.focus(&self.explorer_focus, cx);
                    let _ = self.root_view.update(cx, |root, cx| {
                        root.submit_filesystem_operation(Request::new(operation), None, window, cx)
                    });
                    cx.notify();
                }
                _ => {}
            }
            return;
        }
        if !self.explorer_focus.is_focused(window) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let primary = modifiers.secondary();
        let path = self.explorer_focused_path().map(Path::to_path_buf);
        if matches!(event.keystroke.key.as_str(), "up" | "down" | "home" | "end") {
            let visible = self.explorer_visible_paths(cx);
            let current = path
                .as_ref()
                .and_then(|p| visible.iter().position(|v| v == p));
            let index = match event.keystroke.key.as_str() {
                "up" => current.unwrap_or(0).saturating_sub(1),
                "home" => 0,
                "end" => visible.len().saturating_sub(1),
                _ => current.map_or(0, |index| (index + 1).min(visible.len().saturating_sub(1))),
            };
            if let Some(path) = visible.get(index) {
                if primary && !modifiers.shift {
                    self.explorer_focus_path(path.clone(), window, cx);
                } else {
                    self.explorer_select(path.clone(), modifiers, false, window, cx);
                }
                let row = self
                    .active_repo()
                    .and_then(|r| match &r.file_browser.entries {
                        Loadable::Ready(entries) => {
                            self.file_browser_visible_rows(cx).iter().position(|row| {
                                row.entry_index()
                                    .and_then(|i| entries.get(i))
                                    .is_some_and(|entry| entry.path.as_path() == path)
                            })
                        }
                        _ => None,
                    })
                    .unwrap_or(index);
                self.file_browser_scroll
                    .scroll_to_item(row, gpui::ScrollStrategy::Center);
            }
            cx.stop_propagation();
            return;
        }
        let action = if primary {
            match event.keystroke.key.as_str() {
                "c" => Some(ExplorerAction::Copy),
                "x" => Some(ExplorerAction::Cut),
                "v" => Some(ExplorerAction::Paste),
                "d" => Some(ExplorerAction::Duplicate),
                "z" if modifiers.shift => Some(ExplorerAction::Redo),
                "z" => Some(ExplorerAction::Undo),
                "y" => Some(ExplorerAction::Redo),
                "a" => {
                    if let Some(repo_id) = self.active_repo_id() {
                        self.store.dispatch(Msg::SelectAllExplorerPaths {
                            repo_id,
                            visible: self.explorer_visible_paths(cx),
                        });
                    }
                    cx.stop_propagation();
                    None
                }
                _ => None,
            }
        } else {
            match event.keystroke.key.as_str() {
                "f2" => Some(ExplorerAction::Rename),
                "delete" if modifiers.shift => Some(ExplorerAction::Delete),
                "delete" => Some(ExplorerAction::Trash),
                "escape" => {
                    crate::clipboard::cancel_cut(cx);
                    crate::view::native_transfers::cancel_preparing(cx);
                    window.cancel_drag(cx);
                    let _ = self
                        .root_view
                        .update(cx, |root, cx| root.cancel_filesystem_operations(cx));
                    self.clear_explorer_drag_state(cx);
                    cx.stop_propagation();
                    cx.notify();
                    None
                }
                "enter" | "right" | "left" => {
                    if let (Some(repo), Some(path)) = (self.active_repo(), path.clone()) {
                        let directory = matches!(&repo.file_browser.entries, Loadable::Ready(entries) if entries.iter().any(|e| e.path.as_path() == path && e.kind == FileEntryKind::Directory));
                        let expanded = repo.file_browser.expanded_dirs.contains(&path);
                        let key = event.keystroke.key.as_str();
                        if directory
                            && (key == "enter"
                                || (key == "right" && !expanded)
                                || (key == "left" && expanded))
                        {
                            self.store.dispatch(Msg::ToggleFileBrowserDir {
                                repo_id: repo.id,
                                path,
                            });
                        } else if key == "left" {
                            if let Some(parent) =
                                path.parent().filter(|p| !p.as_os_str().is_empty())
                            {
                                self.explorer_select(
                                    parent.to_path_buf(),
                                    gpui::Modifiers::default(),
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        } else if key == "right" && directory && expanded {
                            let visible = self.explorer_visible_paths(cx);
                            if let Some(index) = visible.iter().position(|p| p == &path)
                                && let Some(child) = visible
                                    .get(index + 1)
                                    .filter(|child| child.starts_with(&path))
                            {
                                self.explorer_select(
                                    child.clone(),
                                    gpui::Modifiers::default(),
                                    false,
                                    window,
                                    cx,
                                );
                            }
                        } else if key == "enter" {
                            self.show_repository_canvas(cx);
                            self.store.dispatch(Msg::OpenFileContent {
                                repo_id: repo.id,
                                source: repo.file_browser.source.clone(),
                                path,
                            });
                        }
                    }
                    cx.stop_propagation();
                    None
                }
                _ => None,
            }
        };
        if let Some(action) = action {
            cx.stop_propagation();
            let target = matches!(
                action,
                ExplorerAction::Paste | ExplorerAction::NewFile | ExplorerAction::NewFolder
            )
            .then_some(path)
            .flatten();
            self.explorer_action(action, target, window, cx);
        }
    }

    pub(super) fn explorer_name_entry(&self, cx: &mut gpui::Context<Self>) -> Option<AnyElement> {
        self.explorer_name_edit.as_ref()?;
        Some(
            div()
                .id("explorer_inline_name")
                .debug_selector(|| "explorer_inline_name".into())
                .flex_1()
                .min_w(px(0.0))
                .h(ui_scale::UiScale::current(cx).px(20.0))
                .px(ui_scale::UiScale::current(cx).px(3.0))
                .flex()
                .items_center()
                .overflow_hidden()
                .bg(self.theme.colors.surface.raised)
                .shadow(vec![gpui::BoxShadow {
                    color: self.theme.colors.accent.foreground.into_color(),
                    offset: gpui::Point::default(),
                    blur_radius: px(0.0),
                    spread_radius: px(1.0),
                    inset: true,
                }])
                .capture_key_down(cx.listener(Self::explorer_key_down))
                .child(self.explorer_name_input.clone())
                .into_any_element(),
        )
    }

    pub(super) fn explorer_empty_state(
        &self,
        theme: AppTheme,
        message: &'static str,
    ) -> AnyElement {
        let bounds = self.explorer_empty_bounds.clone();
        let highlighted = self
            .explorer_drop_target
            .as_ref()
            .is_some_and(|path| path.as_os_str().is_empty());
        div()
            .relative()
            .flex_1()
            .min_h(px(44.0))
            .debug_selector(|| "explorer_empty_area".into())
            .when(highlighted, |row| {
                row.bg(with_alpha(theme.colors.accent.foreground, 0.16))
            })
            .child(
                gpui::canvas(
                    move |area, _, _| bounds.set(Some(area)),
                    move |bounds, _, window, _| {
                        if highlighted {
                            window.paint_quad(gpui::outline(
                                bounds,
                                with_alpha(theme.colors.accent.foreground, 0.7),
                                gpui::BorderStyle::default(),
                            ));
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(components::empty_state(theme, "Files", message))
            .into_any_element()
    }

    pub(super) fn explorer_drop(
        &mut self,
        paths: Vec<PathBuf>,
        target: Option<PathBuf>,
        external: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        if repo.file_browser.source != FileSource::WorkingDirectory {
            return;
        }
        // A drop of our own outbound transfer arrives back through the
        // platform, so claim it either way: the outbound side must not also
        // remove the source. `&&` used to short-circuit the first of two calls,
        // which read as though one of them were redundant.
        let ours = crate::view::native_transfers::mark_handled(&paths, cx);
        let external = external && !ours;
        let native = window.take_file_drop();
        let native_source_move = external
            && native.as_ref().is_some_and(|transfer| {
                transfer.source_owns_move && transfer.operation == gpui::FileTransferOperation::Move
            });
        let completion_intent = if native_source_move {
            TransferIntent::Move
        } else if !external
            && native
                .as_ref()
                .is_some_and(|transfer| transfer.source_owns_move)
        {
            // Our own restored drag is journaled as a move below. Cancelling
            // the source-owned Move handshake prevents duplicate removal.
            TransferIntent::Copy
        } else if external {
            native
                .as_ref()
                .map(|transfer| {
                    if transfer.operation == gpui::FileTransferOperation::Move {
                        TransferIntent::Move
                    } else {
                        TransferIntent::Copy
                    }
                })
                .unwrap_or_else(|| {
                    if window.modifiers().shift {
                        TransferIntent::Move
                    } else {
                        TransferIntent::Copy
                    }
                })
        } else if copy_modifier(window.modifiers()) {
            TransferIntent::Copy
        } else {
            TransferIntent::Move
        };
        let intent = if native_source_move {
            TransferIntent::Copy
        } else if external {
            completion_intent
        } else if copy_modifier(window.modifiers()) {
            TransferIntent::Copy
        } else {
            TransferIntent::Move
        };
        let destination = repo
            .spec
            .workdir
            .join(self.explorer_target(target.as_deref()));
        self.clear_explorer_drag_state(cx);
        let _ = self.root_view.update(cx, |root, cx| {
            let mut request = Request::new(Operation::Transfer {
                sources: paths,
                destination,
                intent,
            });
            request.native_source_move = native_source_move;
            if let Some(native) = native {
                root.submit_filesystem_drop(request, native, completion_intent, window, cx);
            } else {
                root.submit_filesystem_operation(request, None, window, cx);
            }
        });
        cx.stop_propagation();
        cx.notify();
    }

    /// Drops every piece of drag state at once. This used to be open-coded at
    /// four sites and one of them forgot the scroll task.
    pub(super) fn clear_explorer_drag_state(&mut self, cx: &mut gpui::Context<Self>) {
        let had_state = self.explorer_drop_target.is_some()
            || self.explorer_hover_task.is_some()
            || self.explorer_scroll_task.is_some();
        self.explorer_drag_repo = None;
        self.explorer_drop_row = None;
        self.explorer_drop_target = None;
        self.explorer_hover_task = None;
        self.explorer_scroll_task = None;
        if had_state {
            cx.notify();
        }
    }

    /// Paths this process has cut, or `None` when the clipboard holds no cut.
    ///
    /// Recomputed only when our own clipboard revision moves, so a cut made in
    /// another application is not reflected until something here touches the
    /// clipboard -- the deliberate trade for not polling the platform.
    pub(super) fn explorer_cut_paths(&self, cx: &gpui::Context<Self>) -> Option<Rc<[PathBuf]>> {
        let revision = crate::clipboard::files_revision();
        if let Some((cached, paths)) = self.explorer_cut_cache.borrow().as_ref()
            && *cached == revision
        {
            return paths.clone();
        }
        let paths = crate::clipboard::read_files(cx)
            .filter(|payload| payload.intent == TransferIntent::Move)
            .map(|payload| Rc::from(payload.paths));
        *self.explorer_cut_cache.borrow_mut() = Some((revision, paths.clone()));
        paths
    }

    /// Destination and hovered entry are resolved together so highlighting and
    /// dropping agree, including blank space and the manually virtualized popup.
    fn explorer_hit_at(
        &self,
        position: gpui::Point<Pixels>,
        cx: &mut gpui::App,
    ) -> Option<ExplorerHit> {
        let repo = self.active_repo()?;
        if repo.file_browser.source != FileSource::WorkingDirectory
            || !matches!(repo.file_browser.entries, Loadable::Ready(_))
        {
            return None;
        }
        let rows = self.file_browser_visible_rows(cx);
        if rows.is_empty() {
            return self
                .explorer_empty_bounds
                .get()
                .filter(|bounds| bounds.contains(&position))
                .map(|_| (PathBuf::new(), None));
        }
        let scroll = self.explorer_scroll_handle();
        if !scroll.bounds().contains(&position)
            || (scroll.max_offset().y > px(0.0)
                && position.x >= scroll.bounds().right() - px(crate::kit::SCROLLBAR_GUTTER_PX))
        {
            return None;
        }
        let (origin, height) =
            if self.collapsed_popover_section == Some(CollapsedSidebarSection::Files) {
                let (bounds, height) = self.explorer_popover_rows.get()?;
                if position.x < bounds.left() || position.x >= bounds.right() {
                    return None;
                }
                (bounds.origin, height)
            } else {
                (
                    scroll.bounds().origin,
                    ui_scale::UiScale::current(cx).px(sidebar_list_row_height_px(self.theme)),
                )
            };
        if height <= px(0.0) {
            return None;
        }
        let y = position.y - origin.y - scroll.offset().y;
        if y < px(0.0) {
            return None;
        }
        let ix = (y / height).floor() as usize;
        if ix >= rows.len() {
            return Some((PathBuf::new(), None));
        }
        // Pinned buffers, their header and inline editors are never destinations.
        let entry_index = rows[ix].entry_index()?;
        let Loadable::Ready(entries) = &repo.file_browser.entries else {
            return None;
        };
        let entry = entries.get(entry_index)?;
        let directory = entry.kind == FileEntryKind::Directory;
        Some((
            gitcomet_state::explorer::Selection::destination(Some(&entry.path), directory),
            Some((Arc::clone(&entry.path), directory)),
        ))
    }

    fn explorer_row_at(
        &self,
        position: gpui::Point<Pixels>,
        cx: &mut gpui::App,
    ) -> Option<(Arc<PathBuf>, bool)> {
        self.explorer_hit_at(position, cx)?.1
    }

    pub(super) fn explorer_pointer_target(
        &self,
        window: &Window,
        cx: &mut gpui::App,
    ) -> Option<PathBuf> {
        self.explorer_hit_at(window.mouse_position(), cx)
            .map(|(target, _)| target)
    }

    /// Scan only when the destination or visible tree changes, never per move
    /// or per row. The range includes the header and every visible descendant.
    pub(super) fn explorer_drop_range(
        &self,
        rows: &Rc<[FileBrowserVisibleRow]>,
    ) -> std::ops::Range<usize> {
        let Some(target) = self.explorer_drop_target.as_ref() else {
            return 0..0;
        };
        let mut cache = self.explorer_drop_region.borrow_mut();
        if let Some(region) = cache.as_ref()
            && region.target == *target
            && Rc::ptr_eq(&region.rows, rows)
        {
            return region.range.clone();
        }
        let mut range = 0..0;
        if let Some(repo) = self.active_repo()
            && let Loadable::Ready(entries) = &repo.file_browser.entries
        {
            let mut indices = rows.iter().enumerate().filter_map(|(ix, row)| {
                let entry = entries.get(row.entry_index()?)?;
                (target.as_os_str().is_empty() || entry.path.starts_with(target)).then_some(ix)
            });
            if let Some(first) = indices.next() {
                range = first..indices.next_back().unwrap_or(first) + 1;
            }
        }
        *cache = Some(DropRegion {
            rows: Rc::clone(rows),
            target: target.clone(),
            range: range.clone(),
        });
        range
    }

    fn explorer_hover_at(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let hit = self.explorer_hit_at(position, cx);
        let target = hit.as_ref().map(|(target, _)| target.clone());
        let row = hit.and_then(|(_, row)| row);
        if self.explorer_drop_target == target
            && self.explorer_drop_row.as_ref() == row.as_ref().map(|(path, _)| path.as_ref())
        {
            return;
        }
        self.explorer_hover_task = None;
        self.explorer_drop_row = row.as_ref().map(|(path, _)| path.as_ref().clone());
        if self.explorer_drop_target != target {
            self.explorer_drop_target = target;
            cx.notify();
        }
        let Some((path, true)) = row else { return };
        let Some(repo) = self.active_repo() else {
            return;
        };
        if repo.file_browser.expanded_dirs.contains(&path)
            || file_browser_search_is_active(&repo.file_browser.search_query)
        {
            return;
        }
        let repo_id = repo.id;
        self.explorer_hover_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(EXPLORER_HOVER_EXPAND_DELAY)
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.explorer_hover_task = None;
                if !cx.has_active_drag() || this.active_repo_id() != Some(repo_id) {
                    this.clear_explorer_drag_state(cx);
                    return;
                }
                // Use the current list geometry: scrolling or expansion can have
                // moved a different row underneath a stationary pointer.
                if this
                    .explorer_row_at(window.mouse_position(), cx)
                    .is_none_or(|(current, directory)| !directory || current != path)
                {
                    this.explorer_hover_at(window.mouse_position(), window, cx);
                    return;
                }
                if this.active_repo().is_some_and(|repo| {
                    !repo.file_browser.expanded_dirs.contains(&path)
                        && !file_browser_search_is_active(&repo.file_browser.search_query)
                }) {
                    this.store.dispatch(Msg::ToggleFileBrowserDir {
                        repo_id,
                        path: path.as_ref().clone(),
                    });
                }
            });
        }));
    }

    fn explorer_scroll_handle(&self) -> gpui::ScrollHandle {
        if self.collapsed_popover_section == Some(CollapsedSidebarSection::Files) {
            self.collapsed_popover_scroll.clone()
        } else {
            self.file_browser_scroll.0.borrow().base_handle.clone()
        }
    }

    fn explorer_scroll_direction(&self, position: gpui::Point<Pixels>, cx: &mut gpui::App) -> f32 {
        let bounds = self.explorer_scroll_handle().bounds();
        if !bounds.contains(&position) || self.explorer_hit_at(position, cx).is_none() {
            return 0.0;
        }
        let edge = ui_scale::design_px_from_percent(
            EXPLORER_DRAG_SCROLL_EDGE_PX,
            ui_scale::current(cx).percent,
        );
        if position.y < bounds.top() + edge {
            1.0
        } else if position.y > bounds.bottom() - edge {
            -1.0
        } else {
            0.0
        }
    }

    /// One hover resolver and one frame-paced scroll task for the entire tree.
    /// Further moves update the pointer; they never restart the scroll clock.
    pub(super) fn explorer_drag_move(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.explorer_drag_repo != self.active_repo_id() {
            self.clear_explorer_drag_state(cx);
            self.explorer_drag_repo = self.active_repo_id();
        }
        self.explorer_hover_at(position, window, cx);
        if self.explorer_scroll_direction(position, cx) == 0.0 {
            self.explorer_scroll_task = None;
            return;
        }
        if self.explorer_scroll_task.is_some() {
            return;
        }
        let repo_id = self.active_repo_id();
        let mut previous_tick = cx.background_executor().now();
        self.explorer_scroll_task = Some(cx.spawn_in(window, async move |view, cx| {
            loop {
                let (send, receive) = futures::channel::oneshot::channel();
                if cx
                    .update(|window, _cx| {
                        window.on_next_frame(move |_, _| {
                            let _ = send.send(());
                        });
                    })
                    .is_err()
                    || receive.await.is_err()
                {
                    break;
                }
                let now = cx.background_executor().now();
                let step = now
                    .saturating_duration_since(previous_tick)
                    .min(EXPLORER_DRAG_SCROLL_MAX_STEP);
                previous_tick = now;
                let keep = view
                    .update_in(cx, |this, window, cx| {
                        if !cx.has_active_drag() || this.active_repo_id() != repo_id {
                            this.clear_explorer_drag_state(cx);
                            return false;
                        }
                        let direction = this.explorer_scroll_direction(window.mouse_position(), cx);
                        if direction == 0.0 {
                            return false;
                        }
                        let scroll = this.explorer_scroll_handle();
                        let max = ScrollbarDriver::max_offset(&scroll, ScrollbarAxis::Vertical);
                        let current = ScrollbarDriver::raw_offset(&scroll, ScrollbarAxis::Vertical);
                        if (direction > 0.0 && current >= px(0.0))
                            || (direction < 0.0 && current <= -max)
                        {
                            return false;
                        }
                        let delta =
                            px(EXPLORER_DRAG_SCROLL_PX_PER_SEC * step.as_secs_f32() * direction);
                        let next = (current + delta).clamp(-max, px(0.0));
                        if next != current {
                            ScrollbarDriver::set_axis_offset(
                                &scroll,
                                ScrollbarAxis::Vertical,
                                next,
                            );
                            this.explorer_hover_at(window.mouse_position(), window, cx);
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
            let _ = view.update_in(cx, |this, _, _| {
                this.explorer_scroll_task = None;
            });
        }));
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn preview_uses_basename_and_type_for_one_item_and_count_for_a_selection() {
        let theme = AppTheme::gitcomet_dark();
        let preview = |paths: &[&str], directory| {
            ExplorerDragPreview::new(
                &ExplorerDrag {
                    paths: paths.iter().map(PathBuf::from).collect::<Vec<_>>().into(),
                },
                directory,
                theme,
                gpui::Point::default(),
            )
        };
        let file = preview(&["/repo/src/main.rs"], false);
        assert_eq!(file.label.as_ref(), "main.rs");
        assert_eq!(
            file.icon,
            Some(file_icons::file_icon_for_path(Path::new("main.rs")))
        );
        let folder = preview(&["/repo/src"], true);
        assert_eq!(folder.label.as_ref(), "src");
        assert_eq!(folder.icon, Some(file_icons::folder_icon(false)));
        let selection = preview(&["/repo/src", "/repo/README.md"], true);
        assert_eq!(selection.label.as_ref(), "2 items");
        assert_eq!(selection.icon, None);
    }
}
