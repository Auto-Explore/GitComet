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
const EXPLORER_DRAG_SCROLL_TICK: Duration = Duration::from_millis(16);

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
}

#[derive(Clone)]
pub(in crate::view) struct ExplorerDrag {
    pub paths: Vec<PathBuf>,
}

impl Render for ExplorerDrag {
    fn render(&mut self, window: &mut Window, _cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let operation = if copy_modifier(window.modifiers()) {
            "Copy"
        } else {
            "Move"
        };
        div()
            .px_2()
            .py_1()
            .child(format!("{operation} {} item(s)", self.paths.len()))
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
        let sources = self.explorer_sources(path.as_deref());
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
        let path = self
            .active_repo()
            .and_then(|r| r.file_browser.selection.focused.clone());
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
                    if let Some(repo_id) = self.active_repo_id() {
                        self.store.dispatch(Msg::FocusExplorerPath {
                            repo_id,
                            path: path.clone(),
                        });
                    }
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
            self.explorer_action(action, path, window, cx);
        }
    }

    pub(super) fn explorer_name_entry(&self, cx: &gpui::Context<Self>) -> Option<AnyElement> {
        self.explorer_name_edit.as_ref()?;
        Some(
            div()
                .id("explorer_inline_name")
                .debug_selector(|| "explorer_inline_name".into())
                .px_2()
                .py_1()
                .capture_key_down(cx.listener(Self::explorer_key_down))
                .child(self.explorer_name_input.clone())
                .into_any_element(),
        )
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

    /// Whether the pointer is inside the row list itself, as opposed to the
    /// search field, the visibility toggles or the scrollbar beside it.
    pub(super) fn explorer_pointer_over_rows(&self, window: &Window) -> bool {
        let bounds = self.file_browser_scroll.0.borrow().base_handle.bounds();
        bounds.contains(&window.mouse_position())
    }

    /// Marks the row under the pointer as the drop destination.
    ///
    /// gpui dispatches `on_drag_move` to every registered listener of the
    /// matching drag type with no hitbox test, so every visible row runs this
    /// on every mouse move. The bounds check is what makes the pointer decide
    /// the target rather than whichever row happened to paint last.
    pub(super) fn explorer_hover(
        &mut self,
        path: &Path,
        is_directory: bool,
        row_bounds: gpui::Bounds<Pixels>,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let target = gitcomet_state::explorer::Selection::destination(Some(path), is_directory);
        if !row_bounds.contains(&position) {
            // Only the row that claimed the target may give it up. Keying that
            // on the destination instead would let a sibling file clear it:
            // `dir/a` and `dir` both resolve to `dir`.
            if self.explorer_drop_row.as_deref() == Some(path) {
                self.explorer_drop_row = None;
                self.explorer_drop_target = None;
                self.explorer_hover_task = None;
                cx.notify();
            }
            return;
        }
        if self.explorer_drop_row.as_deref() == Some(path) {
            return;
        }
        self.explorer_drop_row = Some(path.to_path_buf());
        self.explorer_drop_target = Some(target.clone());
        let repo_id = self.active_repo_id();
        self.explorer_hover_task = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor()
                .timer(EXPLORER_HOVER_EXPAND_DELAY)
                .await;
            let _ = view.update_in(cx, |this, window, cx| {
                if !cx.has_active_drag() || !row_bounds.contains(&window.mouse_position()) {
                    this.explorer_hover_task = None;
                    this.explorer_drop_row = None;
                    this.explorer_drop_target = None;
                    cx.notify();
                    return;
                }
                if this.active_repo_id() != repo_id
                    || this.explorer_drop_target.as_ref() != Some(&target)
                {
                    return;
                }
                if let Some(repo) = this.active_repo()
                    && !repo.file_browser.expanded_dirs.contains(&target)
                {
                    this.store.dispatch(Msg::ToggleFileBrowserDir {
                        repo_id: repo.id,
                        path: target,
                    });
                }
                this.explorer_hover_task = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Scrolls the tree while a dragged item is held against one of its edges.
    ///
    /// Registered once on the container rather than per row. The task is
    /// dropped and re-armed on every move, so it exists only while the pointer
    /// is actually inside an edge band.
    pub(super) fn explorer_drag_scroll(
        &mut self,
        position: gpui::Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // Dropping the old task cancels it, so at most one is ever live.
        self.explorer_scroll_task = None;
        let bounds = self.file_browser_scroll.0.borrow().base_handle.bounds();
        if !bounds.contains(&position) {
            return;
        }
        let edge = ui_scale::design_px_from_percent(
            EXPLORER_DRAG_SCROLL_EDGE_PX,
            ui_scale::current(cx).percent,
        );
        // Offsets run from -max (bottom) to 0 (top), so scrolling up is positive.
        let direction = if position.y < bounds.top() + edge {
            1.0
        } else if position.y > bounds.bottom() - edge {
            -1.0
        } else {
            return;
        };
        let scroll = self.file_browser_scroll.clone();
        self.explorer_scroll_task = Some(cx.spawn_in(window, async move |view, cx| {
            let mut previous_tick = std::time::Instant::now();
            loop {
                cx.background_executor()
                    .timer(EXPLORER_DRAG_SCROLL_TICK)
                    .await;
                let now = std::time::Instant::now();
                let step = now
                    .saturating_duration_since(previous_tick)
                    .min(EXPLORER_DRAG_SCROLL_MAX_STEP);
                previous_tick = now;
                let keep = view
                    .update_in(cx, |_this, _window, cx| {
                        if !cx.has_active_drag() {
                            return false;
                        }
                        let max = ScrollbarDriver::max_offset(&scroll, ScrollbarAxis::Vertical);
                        let current = ScrollbarDriver::raw_offset(&scroll, ScrollbarAxis::Vertical);
                        let delta =
                            px(EXPLORER_DRAG_SCROLL_PX_PER_SEC * step.as_secs_f32() * direction);
                        let next = (current + delta).clamp(-max, px(0.0));
                        if next == current {
                            // Already against the end of the list.
                            return false;
                        }
                        ScrollbarDriver::set_axis_offset(&scroll, ScrollbarAxis::Vertical, next);
                        // `set_offset` only writes a RefCell and never marks the
                        // window dirty; during a drag gpui repaints on mouse
                        // moves alone, so without this the list scrolls
                        // invisibly and jumps on the next move.
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
            let _ = view.update_in(cx, |this, _window, _cx| {
                this.explorer_scroll_task = None;
            });
        }));
    }
}
