//! The host side of the extension API: the frozen registry as a global, the
//! weak `WindowHost` behind every window, and command dispatch.
//!
//! With no extension registered nothing here allocates or runs: the registry
//! global is absent, windows skip every lookup, and no task is spawned.

use super::command_palette::{CommandEntry, Needs};
use super::shortcut_labels::Shortcut;
use super::*;
use gitcomet_extension_api::{
    CommandContext, DialogContent, DialogHandle, EntryOrigin, ExtensionId, GateDecision, HostError,
    MenuLocation, Registry, RepositoryEntryRequest, RepositoryHandle, StateObserver, WindowHost,
    host::WindowHostImpl, storage::StorageError,
};
use gitcomet_extension_api::{HostNotifier, ShellEvent, Slot, WindowExtension};
use gitcomet_state::msg::Msg;
use gitcomet_state::session::WorkspaceId;
use gpui::{Action, KeyBinding, WindowId};
use schemars::JsonSchema;
use serde::Deserialize;
use std::rc::Rc;

type WindowExtensions = Rc<std::cell::RefCell<Vec<Box<dyn WindowExtension>>>>;

/// Runs extension command `id` (a `<extension>/<local>` contribution id) in
/// the focused window. Extension key bindings dispatch it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = extension)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunExtensionCommand {
    pub(crate) id: String,
}

/// The registry, installed once before the first window opens.
pub(crate) struct ExtensionHost {
    registry: Rc<Registry>,
    /// Palette rows for every command, leaked once by [`install_bindings`];
    /// empty while commands are not offered (focused tool windows).
    palette: &'static [CommandEntry],
    /// Whether commands reach palettes, menus, and key bindings.
    commands: bool,
}

impl gpui::Global for ExtensionHost {}

/// Installs a non-empty registry for every window kind: gates, close guards,
/// and window hooks read it. An empty registry installs nothing, so every
/// lookup below short-circuits on a missing global.
pub(crate) fn install_registry(registry: Registry, cx: &mut App) {
    if registry.is_empty() {
        return;
    }
    cx.set_global(ExtensionHost {
        registry: Rc::new(registry),
        palette: &[],
        commands: false,
    });
}

/// Offers the installed extensions' commands: key bindings, the app-level
/// command action, palette rows, and menu entries. Main windows only, like
/// the palette.
///
/// Call before the host binds its own keys: later bindings win, so a host
/// chord an extension also claims stays the host's.
pub(crate) fn install_bindings(cx: &mut App) {
    let Some(registry) = registry(cx) else {
        return;
    };
    let palette = palette_rows(&registry);
    cx.bind_keys(registry.key_bindings().iter().map(|binding| {
        KeyBinding::new(
            &binding.keystrokes,
            RunExtensionCommand {
                id: binding.command.to_string(),
            },
            binding.context.as_deref(),
        )
    }));
    // Reaches a window with nothing focused in it; a focused window's own
    // handler claims the action first.
    cx.on_action(|action: &RunExtensionCommand, cx| {
        let id = action.id.clone();
        cx.defer(move |cx| {
            let _ = crate::app::update_active_or_existing_normal_gitcomet_window(cx, |view, cx| {
                view.run_extension_command(&id, cx)
            });
        });
    });
    let host = cx.global_mut::<ExtensionHost>();
    host.palette = palette;
    host.commands = true;
}

/// A main-window launch: the registry and its commands (tests and benches).
#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn install(registry: Registry, cx: &mut App) {
    install_registry(registry, cx);
    install_bindings(cx);
}

/// One palette row per command, grouped by category in first-seen order so
/// each category gets one header.
fn palette_rows(registry: &Registry) -> &'static [CommandEntry] {
    fn leak(text: impl Into<String>) -> &'static str {
        Box::leak(text.into().into_boxed_str())
    }
    let mut categories: Vec<&str> = Vec::new();
    for (_, command) in registry.commands() {
        if !categories.contains(&command.category.as_ref()) {
            categories.push(&command.category);
        }
    }
    let mut rows: Vec<CommandEntry> = registry
        .commands()
        .iter()
        .map(|(id, command)| {
            let shortcut = registry
                .key_bindings()
                .iter()
                .find(|binding| &binding.command == id && binding.context.is_none())
                .map_or(Shortcut::None, |binding| {
                    Shortcut::Keystrokes(leak(binding.keystrokes.as_ref()))
                });
            CommandEntry {
                id: leak(format!("{EXTENSION_COMMAND_PREFIX}{id}")),
                label: leak(command.label.as_ref()),
                shortcut,
                category: leak(command.category.as_ref()),
                requires_repo: command.requires_repository,
                keywords: leak(command.keywords.as_ref()),
                needs: Needs::Nothing,
            }
        })
        .collect();
    rows.sort_by_key(|row| {
        categories
            .iter()
            .position(|category| *category == row.category)
    });
    Box::leak(rows.into_boxed_slice())
}

/// The one decision every repository entry passes before a window opens
/// it: extension gates in registration order, first denial wins. Without
/// gates this is `Allow` and allocates nothing.
pub(crate) fn entry_decision(
    path: &std::path::Path,
    origin: EntryOrigin,
    cx: &App,
) -> GateDecision {
    let Some(host) = cx.try_global::<ExtensionHost>() else {
        return GateDecision::Allow;
    };
    let gates = host.registry.entry_gates();
    if gates.is_empty() {
        return GateDecision::Allow;
    }
    let request = RepositoryEntryRequest {
        path: path.to_path_buf(),
        origin,
    };
    gates
        .iter()
        .map(|(_, gate)| {
            super::perf::extension_dispatch();
            gate(&request, cx)
        })
        .find(|decision| matches!(decision, GateDecision::Deny { .. }))
        .unwrap_or(GateDecision::Allow)
}

/// `paths` without the denied ones, and the denial reasons.
pub(crate) fn filter_entries(
    paths: Vec<std::path::PathBuf>,
    origin: EntryOrigin,
    cx: &App,
) -> (Vec<std::path::PathBuf>, Vec<SharedString>) {
    let mut denials = Vec::new();
    let allowed = paths
        .into_iter()
        .filter(|path| match entry_decision(path, origin, cx) {
            GateDecision::Allow => true,
            GateDecision::Deny { reason } => {
                denials.push(reason);
                false
            }
        })
        .collect();
    (allowed, denials)
}

/// An extension command offered in a menu.
#[derive(Clone, Debug)]
pub(in crate::view) struct ExtensionMenuEntry {
    /// The command's contribution id.
    pub(in crate::view) id: SharedString,
    pub(in crate::view) label: SharedString,
    pub(in crate::view) requires_repository: bool,
}

/// The extension entries of `location`, in registration order; none while
/// commands are not offered.
pub(in crate::view) fn menu_entries(location: MenuLocation, cx: &App) -> Rc<[ExtensionMenuEntry]> {
    let Some(registry) = cx
        .try_global::<ExtensionHost>()
        .filter(|host| host.commands)
        .map(|host| Rc::clone(&host.registry))
    else {
        return Rc::from([]);
    };
    registry
        .menu_items(location)
        .filter_map(|item| {
            let command = registry.command(&item.command)?;
            Some(ExtensionMenuEntry {
                id: item.command.to_string().into(),
                label: command.label.clone(),
                requires_repository: command.requires_repository,
            })
        })
        .collect()
}

/// The macOS menu-bar items for [`MenuLocation::Application`].
#[cfg(target_os = "macos")]
pub(crate) fn macos_menu_items(cx: &App) -> Vec<gpui::MenuItem> {
    menu_entries(MenuLocation::Application, cx)
        .iter()
        .map(|entry| {
            gpui::MenuItem::action(
                entry.label.clone(),
                RunExtensionCommand {
                    id: entry.id.to_string(),
                },
            )
        })
        .collect()
}

/// The extension rows the command palette lists after the built-in ones.
pub(in crate::view) fn palette_entries(cx: &App) -> &'static [CommandEntry] {
    cx.try_global::<ExtensionHost>()
        .map_or(&[], |host| host.palette)
}

/// The registry, or `None` when no extension is registered.
pub(crate) fn registry(cx: &App) -> Option<Rc<Registry>> {
    cx.try_global::<ExtensionHost>()
        .map(|host| Rc::clone(&host.registry))
}

/// A `RepositoryHandle` for `repo` in `window`.
pub(in crate::view) fn repository_handle(window: WindowId, repo: &RepoState) -> RepositoryHandle {
    RepositoryHandle::new(window, repo.id, repo.lifetime(), repo.spec.workdir.clone())
}

/// The weak host behind one main window. It never reads the view entity:
/// state comes from a snapshot the view publishes, so extension code may use
/// its handles from inside the host's own updates (close guards, gates).
struct HostWindow {
    open: Rc<std::cell::Cell<bool>>,
    kind: gitcomet_core::identity::WindowKind,
    notifier: HostNotifier,
    window_id: WindowId,
    window_handle: gpui::AnyWindowHandle,
    view: WeakEntity<GitCometView>,
    /// Weak: an extension holding this handle must not keep a closed
    /// window's store alive.
    store: std::sync::Weak<AppStore>,
    state: Rc<std::cell::RefCell<Arc<AppState>>>,
    theme: Rc<std::cell::Cell<AppTheme>>,
    observers: Rc<StateObservers>,
    bottom_panels: super::extension_panels::SharedBottomPanels,
}

/// State observers of one window, notified at most once per update cycle.
#[derive(Default)]
struct StateObservers {
    next_id: std::cell::Cell<u64>,
    observers: std::cell::RefCell<Vec<(u64, StateObserver)>>,
    pending: std::cell::Cell<bool>,
    /// The window's host, weak so observers never keep it alive.
    host: std::cell::RefCell<Option<std::rc::Weak<HostWindow>>>,
}

impl StateObservers {
    /// Schedules one notification for however many changes land before it.
    fn changed(self: &Rc<Self>, cx: &mut App) {
        if self.observers.borrow().is_empty() || self.pending.replace(true) {
            return;
        }
        let this = Rc::clone(self);
        cx.defer(move |cx| {
            this.pending.set(false);
            let Some(host) = this.host.borrow().as_ref().and_then(std::rc::Weak::upgrade) else {
                return;
            };
            let host = WindowHost::new(host);
            // A snapshot of the list: callbacks may subscribe or unsubscribe.
            let observers: Vec<StateObserver> = this
                .observers
                .borrow()
                .iter()
                .map(|(_, observer)| Rc::clone(observer))
                .collect();
            for observer in observers {
                if !host.is_open(cx) {
                    return;
                }
                observer(&host, cx);
            }
        });
    }
}

impl HostWindow {
    fn present_dialog(
        &self,
        title: SharedString,
        content: DialogContent,
        anchor: Option<Point<Pixels>>,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        let view = self.view.upgrade().ok_or(HostError::WindowClosed)?;
        let window = self.window_handle;
        let dialog_id = next_dialog_id();
        // Deferred: an extension may call this from inside the host's own
        // update, and the popover host is updated through the root view.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |root, cx| {
                    root.open_extension_dialog(dialog_id, title, content, window, cx);
                    if let Some(anchor) = anchor {
                        root.popover_host.update(cx, |host, cx| {
                            host.reanchor_hosted(dialog_id, anchor, window, cx)
                        });
                    }
                });
            });
        });
        let weak = self.view.clone();
        Ok(DialogHandle::new(move |cx| {
            let Some(view) = weak.upgrade() else {
                return;
            };
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    view.update(cx, |root, cx| {
                        root.close_extension_dialog(dialog_id, window, cx);
                    });
                });
            });
        }))
    }

    /// This window's `WindowHost`, for panes that observe it.
    fn host(&self) -> Result<WindowHost, HostError> {
        self.observers
            .host
            .borrow()
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .map(|host| WindowHost::new(host))
            .ok_or(HostError::WindowClosed)
    }

    fn live(&self) -> Result<(), HostError> {
        if self.open.get() && self.view.upgrade().is_some() {
            Ok(())
        } else {
            Err(HostError::WindowClosed)
        }
    }

    /// Opens `launch` after this update; a failure is reported in the window.
    fn launch(&self, launch: super::platform_open::Launch, cx: &mut App) -> Result<(), HostError> {
        self.live()?;
        let view = self.view.clone();
        super::platform_open::launch_later(
            launch,
            move |err, cx| {
                let _ = view.update(cx, |root, cx| {
                    let report = ErrorReport::message(
                        root.active_repo_id(),
                        format!("Could not open: {err}"),
                    );
                    root.report_error(report, cx);
                });
            },
            cx,
        )
        .map_err(|err| HostError::InvalidRequest(err.to_string().into()))
    }

    fn workspace_id(&self, cx: &App) -> Result<WorkspaceId, HostError> {
        self.live()?;
        crate::workspaces::with_workspace_for_window(cx, self.window_id, |workspace| workspace.id)
            .ok_or(HostError::Unsupported)
    }
}

impl WindowHostImpl for HostWindow {
    fn repository_reader(&self) -> gitcomet_extension_api::RepositoryReader {
        let store = self.store.clone();
        gitcomet_extension_api::RepositoryReader::new(self.window_id, move |repo_id, lifetime| {
            store.upgrade()?.repository(repo_id, lifetime)
        })
    }

    fn navigate(
        &self,
        repository: &RepositoryHandle,
        target: gitcomet_extension_api::ViewTarget,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.live()?;
        let view = self.view.clone();
        let handle = self.window_handle;
        let repository = repository.clone();
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let _ = view.update(cx, |root, cx| {
                    if !root.state.repos.iter().any(|repo| {
                        repo.id == repository.repo_id() && repo.lifetime() == repository.lifetime()
                    }) {
                        return;
                    }
                    let index = match target {
                        gitcomet_extension_api::ViewTarget::History => None,
                        gitcomet_extension_api::ViewTarget::Extension(ref id) => {
                            let Some(index) = root.repository_view_index(id) else {
                                return;
                            };
                            Some(index)
                        }
                        _ => return,
                    };
                    root.select_repository_view(&repository, index, window, cx);
                });
            });
        });
        Ok(())
    }

    fn open_settings_at(
        &self,
        target: gitcomet_extension_api::SettingsTarget,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.live()?;
        cx.defer(move |cx| super::settings_window::open_settings_at(target, cx));
        Ok(())
    }
    fn kind(&self) -> gitcomet_core::identity::WindowKind {
        self.kind
    }
    fn notifier(&self) -> HostNotifier {
        self.notifier.clone()
    }
    fn window_id(&self) -> WindowId {
        self.window_id
    }

    fn is_open(&self, _cx: &App) -> bool {
        self.live().is_ok()
    }

    fn active_repository(&self, _cx: &App) -> Result<Option<RepositoryHandle>, HostError> {
        self.live()?;
        let state = self.state.borrow();
        Ok(state.active_repo.and_then(|active| {
            state
                .repos
                .iter()
                .find(|repo| repo.id == active)
                .map(|repo| repository_handle(self.window_id, repo))
        }))
    }

    fn state(&self, _cx: &App) -> Result<Arc<AppState>, HostError> {
        self.live()?;
        Ok(Arc::clone(&self.state.borrow()))
    }

    fn theme(&self, _cx: &App) -> AppTheme {
        self.theme.get()
    }

    fn observe_state(&self, observer: StateObserver) -> Result<u64, HostError> {
        self.live()?;
        let id = self.observers.next_id.get() + 1;
        self.observers.next_id.set(id);
        self.observers.observers.borrow_mut().push((id, observer));
        Ok(id)
    }

    fn create_diff_pane(
        &self,
        repository: &RepositoryHandle,
        target: gitcomet_core::domain::DiffTarget,
        options: gitcomet_extension_api::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<gitcomet_extension_api::DiffPane, HostError> {
        let host = self.host()?;
        let store = self.store.clone();
        let repository = repository.clone();
        let entity = cx.new(|cx| {
            let mut pane = super::hosted::diff_pane::DiffPaneView::new(
                host, store, repository, target, options, cx,
            );
            pane.attach_root(self.view.clone());
            pane
        });
        Ok(gitcomet_extension_api::DiffPane::new(Rc::new(
            super::hosted::diff_pane::HostedDiffPane { entity },
        )))
    }

    fn create_snapshot_pane(
        &self,
        snapshot: gitcomet_extension_api::DiffSnapshot,
        options: gitcomet_extension_api::DiffPaneOptions,
        cx: &mut App,
    ) -> Result<gitcomet_extension_api::DiffPane, HostError> {
        let host = self.host()?;
        let entity = cx.new(|cx| {
            let mut pane =
                super::hosted::diff_pane::DiffPaneView::snapshot(host, snapshot, options, cx);
            pane.attach_root(self.view.clone());
            pane
        });
        Ok(gitcomet_extension_api::DiffPane::new(Rc::new(
            super::hosted::diff_pane::HostedDiffPane { entity },
        )))
    }

    fn create_file_list(
        &self,
        repository: &RepositoryHandle,
        source: gitcomet_extension_api::ChangeSource,
        on_select: gitcomet_extension_api::FileSelected,
        cx: &mut App,
    ) -> Result<gitcomet_extension_api::FileList, HostError> {
        let host = self.host()?;
        let store = self.store.clone();
        let repository = repository.clone();
        let entity = cx.new(|cx| {
            super::hosted::file_list::FileListView::new(
                host, store, repository, source, on_select, cx,
            )
        });
        Ok(gitcomet_extension_api::FileList::new(Rc::new(
            super::hosted::file_list::HostedFileList { entity },
        )))
    }

    fn open_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &gitcomet_extension_api::ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError> {
        let view = self.view.upgrade().ok_or(HostError::WindowClosed)?;
        let index = self
            .bottom_panels
            .borrow()
            .index_of(panel)
            .ok_or(HostError::Unsupported)?;
        let key = (repository.repo_id(), repository.lifetime());
        self.bottom_panels.borrow_mut().open(key, index);
        let window = self.window_handle;
        // Deferred like dialogs: the root builds the view in its own update.
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                view.update(cx, |root, cx| {
                    root.show_extension_bottom_panel(key, index, window, cx);
                });
            });
        });
        Ok(())
    }

    fn close_bottom_panel(
        &self,
        repository: &RepositoryHandle,
        panel: &gitcomet_extension_api::ContributionId,
        cx: &mut App,
    ) -> Result<(), HostError> {
        let view = self.view.upgrade().ok_or(HostError::WindowClosed)?;
        let index = self
            .bottom_panels
            .borrow()
            .index_of(panel)
            .ok_or(HostError::Unsupported)?;
        let key = (repository.repo_id(), repository.lifetime());
        if self.bottom_panels.borrow_mut().close(key, index) {
            cx.defer(move |cx| {
                view.update(cx, |root, cx| {
                    root.extension_bottom_panel_closed(key, index, cx);
                });
            });
        }
        Ok(())
    }

    fn is_bottom_panel_open(
        &self,
        repository: &RepositoryHandle,
        panel: &gitcomet_extension_api::ContributionId,
        _cx: &App,
    ) -> bool {
        let panels = self.bottom_panels.borrow();
        panels.index_of(panel).is_some_and(|index| {
            panels.is_open((repository.repo_id(), repository.lifetime()), index)
        })
    }

    fn watch_repository(
        &self,
        repository: &RepositoryHandle,
        _cx: &App,
    ) -> Result<gitcomet_extension_api::RepositoryWatch, HostError> {
        let store = self.store.upgrade().ok_or(HostError::WindowClosed)?;
        Ok(gitcomet_extension_api::RepositoryWatch::new(Box::new(
            store.watch_repository(repository.repo_id(), repository.lifetime()),
        )))
    }

    fn watch_worktree(
        &self,
        repository: &RepositoryHandle,
        path: &std::path::Path,
        _cx: &App,
    ) -> Result<gitcomet_extension_api::RepositoryWatch, HostError> {
        let store = self.store.upgrade().ok_or(HostError::WindowClosed)?;
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            repository.workdir().join(path)
        };
        Ok(gitcomet_extension_api::RepositoryWatch::new(Box::new(
            store.watch_worktree(repository.repo_id(), repository.lifetime(), path),
        )))
    }

    fn highlight_line(
        &self,
        path: &std::path::Path,
        text: &str,
        _cx: &App,
    ) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        let Some(language) = crate::view::rows::diff_syntax_language_for_path(path) else {
            return Vec::new();
        };
        crate::view::rows::syntax_highlights_for_line(
            self.theme.get(),
            text,
            language,
            crate::view::rows::DiffSyntaxMode::HeuristicOnly,
        )
    }

    fn unobserve_state(&self, id: u64) {
        self.observers
            .observers
            .borrow_mut()
            .retain(|(observer_id, _)| *observer_id != id);
    }

    fn is_current(&self, repository: &RepositoryHandle, _cx: &App) -> bool {
        self.live().is_ok()
            && self.state.borrow().repos.iter().any(|repo| {
                repo.id == repository.repo_id() && repo.lifetime() == repository.lifetime()
            })
    }

    fn dispatch(&self, msg: Msg, _cx: &mut App) -> Result<(), HostError> {
        self.live()?;
        let store = self.store.upgrade().ok_or(HostError::WindowClosed)?;
        store.dispatch(msg);
        Ok(())
    }

    fn open_dialog(
        &self,
        title: SharedString,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.present_dialog(title, content, None, cx)
    }

    fn open_popover(
        &self,
        title: SharedString,
        anchor: Point<Pixels>,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.present_dialog(title, content, Some(anchor), cx)
    }

    fn open_menu(
        &self,
        anchor: Point<Pixels>,
        items: Vec<gitcomet_extension_api::HostedMenuItem>,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.live()?;
        let weak = self.view.clone();
        let window = self.window_handle;
        let id = next_dialog_id();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |root, cx| {
                    root.popover_host.update(cx, |host, cx| {
                        host.open_hosted_menu(id, items, anchor, window, cx)
                    });
                });
            });
        });
        let weak = self.view.clone();
        Ok(DialogHandle::new(move |cx| {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = weak.update(cx, |root, cx| root.close_extension_dialog(id, window, cx));
                });
            })
        }))
    }

    fn toast(
        &self,
        kind: gitcomet_extension_api::NotificationKind,
        message: SharedString,
        actions: Vec<gitcomet_extension_api::HostedAction>,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.live()?;
        let weak = self.view.clone();
        cx.defer(move |cx| {
            let _ = weak.update(cx, |root, cx| {
                use gitcomet_extension_api::NotificationKind;
                if kind == NotificationKind::Error {
                    root.report_error(
                        ErrorReport {
                            repo_id: root.active_repo_id(),
                            message: message.to_string(),
                            actions: actions.into_iter().map(ErrorAction::Hosted).collect(),
                        },
                        cx,
                    );
                } else {
                    let kind = match kind {
                        NotificationKind::Success => components::ToastKind::Success,
                        _ => components::ToastKind::Warning,
                    };
                    root.toast_host.update(cx, |host, cx| {
                        host.push_hosted_toast(kind, message.to_string(), actions, cx)
                    });
                }
            });
        });
        Ok(())
    }

    fn open_window(
        &self,
        title: SharedString,
        content: gitcomet_extension_api::WindowContent,
        on_closed: gitcomet_extension_api::OnWindowClosed,
        cx: &mut App,
    ) -> Result<gitcomet_extension_api::PopOutWindow, HostError> {
        self.live()?;
        super::pop_out::open(self.host()?, self.window_id, title, content, on_closed, cx)
    }

    fn open_url(&self, url: &str, cx: &mut App) -> Result<(), HostError> {
        self.launch(super::platform_open::Launch::Url(url.to_string()), cx)
    }

    fn open_path(&self, path: &std::path::Path, cx: &mut App) -> Result<(), HostError> {
        self.launch(super::platform_open::Launch::Path(path.to_path_buf()), cx)
    }

    fn notify(&self, message: SharedString, cx: &mut App) -> Result<(), HostError> {
        let view = self.view.upgrade().ok_or(HostError::WindowClosed)?;
        cx.defer(move |cx| {
            view.update(cx, |root, cx| {
                root.push_toast(components::ToastKind::Success, message.to_string(), cx);
            });
        });
        Ok(())
    }

    fn workspace_state(
        &self,
        extension: &ExtensionId,
        cx: &App,
    ) -> Result<Option<serde_json::Value>, HostError> {
        let workspace_id = self.workspace_id(cx)?;
        Ok(crate::workspaces::workspace(cx, workspace_id)
            .and_then(|workspace| workspace.extensions.get(extension.as_str()).cloned()))
    }

    fn set_workspace_state(
        &self,
        extension: &ExtensionId,
        value: serde_json::Value,
        cx: &mut App,
    ) -> Result<Result<(), StorageError>, HostError> {
        let workspace_id = self.workspace_id(cx)?;
        Ok(crate::workspaces::set_workspace_extension_state(
            cx,
            workspace_id,
            extension.as_str(),
            Some(value),
        )
        .map(|_| ())
        .map_err(StorageError::from))
    }
}

/// A main window's side of the extension host: its [`WindowHost`] and the
/// state snapshot behind it. Only exists when an extension is registered.
pub(in crate::view) struct ExtensionWindow {
    instances: WindowExtensions,
    notifier: HostNotifier,
    _notifications: gpui::Task<()>,
    _closed: gpui::Subscription,
    host: WindowHost,
    state: Rc<std::cell::RefCell<Arc<AppState>>>,
    theme: Rc<std::cell::Cell<AppTheme>>,
    observers: Rc<StateObservers>,
    bottom_panels: super::extension_panels::SharedBottomPanels,
}

impl ExtensionWindow {
    pub(in crate::view) fn new(
        window: &Window,
        store: &Arc<AppStore>,
        state: Arc<AppState>,
        theme: AppTheme,
        kind: gitcomet_core::identity::WindowKind,
        cx: &mut gpui::Context<GitCometView>,
    ) -> Option<Self> {
        // The focused difftool hosts its own pane, so it has a host even
        // without extensions.
        let registry = registry(cx).or_else(|| {
            (kind == gitcomet_core::identity::WindowKind::FocusedDiff)
                .then(|| Rc::new(Registry::default()))
        })?;
        let bottom_panels = Rc::new(std::cell::RefCell::new(
            super::extension_panels::BottomPanels::new(registry.bottom_panels()),
        ));
        let state = Rc::new(std::cell::RefCell::new(state));
        let theme = Rc::new(std::cell::Cell::new(theme));
        let observers = Rc::new(StateObservers::default());
        let open = Rc::new(std::cell::Cell::new(true));
        let instances: WindowExtensions = Rc::default();
        let (wake, notifications) = smol::channel::bounded(1);
        let sender = wake.clone();
        let notifier = HostNotifier::new(move || {
            let _ = sender.try_send(());
        });
        let pending = notifier.clone();
        let notification_task = cx.spawn(async move |view, cx| {
            while notifications.recv().await.is_ok() {
                let slots = pending.take_pending();
                if view
                    .update(cx, |view, cx| {
                        for slot in Slot::ALL.iter().copied() {
                            if slots & slot.mask() != 0 {
                                view.invalidate_extension_slot(slot, cx);
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let window_handle = window.window_handle();
        let host_window = Rc::new(HostWindow {
            open: open.clone(),
            kind,
            notifier: notifier.clone(),
            window_id: window_handle.window_id(),
            window_handle,
            view: cx.weak_entity(),
            store: Arc::downgrade(store),
            state: Rc::clone(&state),
            theme: Rc::clone(&theme),
            observers: Rc::clone(&observers),
            bottom_panels: Rc::clone(&bottom_panels),
        });
        *observers.host.borrow_mut() = Some(Rc::downgrade(&host_window));
        let closing_host = Rc::downgrade(&host_window);
        let closing_instances = instances.clone();
        let closing_notifier = notifier.clone();
        let closed = cx.on_window_closed(move |cx, closed| {
            if closed != window_handle.window_id() {
                return;
            }
            open.set(false);
            closing_notifier.close();
            wake.close();
            if let Some(host) = closing_host.upgrade() {
                let handle = WindowHost::new(host.clone());
                for instance in closing_instances.borrow_mut().iter_mut() {
                    crate::view::perf::extension_dispatch();
                    instance.on_event(&ShellEvent::WindowClosed, &handle, cx);
                }
                closing_instances.borrow_mut().clear();
                *host.state.borrow_mut() = Arc::default();
                host.observers.observers.borrow_mut().clear();
                *host.bottom_panels.borrow_mut() = Default::default();
            }
        });
        Some(Self {
            instances,
            notifier,
            _notifications: notification_task,
            _closed: closed,
            host: WindowHost::new(host_window),
            state,
            theme,
            observers,
            bottom_panels,
        })
    }

    pub(in crate::view) fn host(&self) -> WindowHost {
        self.host.clone()
    }

    pub(in crate::view) fn bottom_panels(&self) -> super::extension_panels::SharedBottomPanels {
        Rc::clone(&self.bottom_panels)
    }

    /// Publishes the view's latest state to extension handles and schedules
    /// one notification for its observers, if any.
    pub(in crate::view) fn set_state(&self, state: &Arc<AppState>, cx: &mut App) {
        if Arc::ptr_eq(&self.state.borrow(), state) {
            return;
        }
        let previous = self.state.replace(Arc::clone(state));
        self.observers.changed(cx);
        if !self.instances.borrow().is_empty() {
            let handle = |repo: &RepoState| repository_handle(self.host.id(), repo);
            let mut events = Vec::new();
            for old in &previous.repos {
                if !state
                    .repos
                    .iter()
                    .any(|repo| repo.id == old.id && repo.lifetime() == old.lifetime())
                {
                    events.push(ShellEvent::RepositoryClosed(handle(old)));
                }
            }
            for repo in &state.repos {
                if !previous
                    .repos
                    .iter()
                    .any(|old| repo.id == old.id && repo.lifetime() == old.lifetime())
                {
                    events.push(ShellEvent::RepositoryOpened(handle(repo)));
                }
            }
            if previous.active_repo != state.active_repo {
                events.push(ShellEvent::ActiveRepositoryChanged(
                    state
                        .active_repo
                        .and_then(|id| state.repos.iter().find(|repo| repo.id == id))
                        .map(handle),
                ));
            }
            events.push(ShellEvent::StateChanged);
            for event in events {
                self.emit(event, cx);
            }
        }
    }

    pub(in crate::view) fn set_theme(&self, theme: AppTheme, cx: &mut App) {
        self.theme.set(theme);
        self.emit(ShellEvent::ThemeChanged, cx);
    }

    pub(in crate::view) fn emit(&self, event: ShellEvent, cx: &mut App) {
        if self.instances.borrow().is_empty() {
            return;
        }
        let instances = Rc::downgrade(&self.instances);
        let host = self.host.clone();
        cx.defer(move |cx| {
            if !host.is_open(cx) {
                return;
            }
            if let Some(instances) = instances.upgrade() {
                for instance in instances.borrow_mut().iter_mut() {
                    crate::view::perf::extension_dispatch();
                    instance.on_event(&event, &host, cx);
                }
            }
        });
    }
}

impl Drop for ExtensionWindow {
    /// The window is gone: handles extensions still hold keep neither its
    /// last snapshot nor their observers.
    fn drop(&mut self) {
        self.notifier.close();
        self.instances.borrow_mut().clear();
        *self.state.borrow_mut() = Arc::default();
        self.observers.observers.borrow_mut().clear();
        // Built panels can hold host handles, which share this same panel map.
        // Break that ownership cycle when the window goes away.
        *self.bottom_panels.borrow_mut() = Default::default();
    }
}

pub(in crate::view) fn next_dialog_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Builds the window's status items and chrome and tells every extension
/// the window opened. Deferred so both run after the window's own
/// construction. Only main windows have a status bar to fill.
pub(crate) fn window_opened(view: &Entity<GitCometView>, cx: &mut App) {
    let Some(registry) = registry(cx) else {
        return;
    };
    let Some(host) = view.read(cx).extension_window.as_ref().map(|w| w.host()) else {
        return;
    };
    let has_status_bar = renders_full_chrome(view.read(cx).view_mode);
    let window_handle = view.read(cx).window_handle;
    let view = view.downgrade();
    cx.defer(move |cx| {
        let _ = window_handle.update(cx, |_, window, cx| {
            let instances: Vec<_> = registry
                .instances()
                .iter()
                .filter_map(|extension| {
                    crate::view::perf::extension_dispatch();
                    extension.window_opened(host.clone(), window, cx)
                })
                .collect();
            if let Some(view) = view.upgrade() {
                view.update(cx, |root, _| {
                    if let Some(extension) = &root.extension_window {
                        *extension.instances.borrow_mut() = instances;
                    }
                });
            }
        });
        if has_status_bar && !registry.status_items().is_empty() {
            let _ = window_handle.update(cx, |_, window, cx| {
                let items = registry
                    .status_items()
                    .iter()
                    .map(|(_, item)| {
                        super::perf::extension_dispatch();
                        (item.view.clone(), (item.build)(host.clone(), window, cx))
                    })
                    .collect();
                if let Some(view) = view.upgrade() {
                    view.update(cx, |root, cx| {
                        root.bottom_status_bar
                            .update(cx, |bar, cx| bar.set_extension_items(items, cx));
                    });
                }
            });
        }
        if registry.edition_strip().is_some() || registry.title_bar_brand().is_some() {
            let _ = window_handle.update(cx, |_, window, cx| {
                let edition = registry
                    .edition_strip()
                    .filter(|_| has_status_bar)
                    .map(|item| {
                        super::perf::extension_dispatch();
                        (item.build)(host.clone(), window, cx)
                    });
                let brand = registry.title_bar_brand().map(|item| {
                    super::perf::extension_dispatch();
                    (item.build)(host.clone(), window, cx)
                });
                let _ = view.update(cx, |root, cx| {
                    root.bottom_status_bar
                        .update(cx, |bar, cx| bar.set_edition_strip(edition, cx));
                    root.title_bar
                        .update(cx, |bar, cx| bar.set_brand(brand, cx));
                });
            });
        }
    });
}

/// Palette ids of extension commands: this prefix, then the contribution id.
const EXTENSION_COMMAND_PREFIX: &str = "extension:";

/// The contribution id inside an extension command's palette id.
pub(in crate::view) fn command_id_from_palette(palette_id: &str) -> Option<&str> {
    palette_id.strip_prefix(EXTENSION_COMMAND_PREFIX)
}

impl GitCometView {
    pub(in crate::view) fn invalidate_extension_slot(
        &mut self,
        slot: Slot,
        cx: &mut gpui::Context<Self>,
    ) {
        match slot {
            Slot::Gate => {
                if let Some(gates) = &mut self.window_gates {
                    gates.invalidate();
                }
                cx.notify();
            }
            Slot::ActionBar | Slot::Navigation => self.action_bar.update(cx, |_, cx| cx.notify()),
            Slot::TitleBar => self.title_bar.update(cx, |_, cx| cx.notify()),
            Slot::Status => self.bottom_status_bar.update(cx, |_, cx| cx.notify()),
            Slot::Details => self.details_pane.update(cx, |_, cx| cx.notify()),
            Slot::Sidebar => self.sidebar_pane.update(cx, |_, cx| cx.notify()),
            Slot::History => self.main_pane.update(cx, |pane, cx| {
                pane.history_view.update(cx, |_, cx| cx.notify());
            }),
            _ => cx.notify(),
        }
    }
    /// Runs extension command `id` (`<extension>/<local>`) in this window,
    /// for the active repository. Returns whether `id` named a command.
    pub(in crate::view) fn run_extension_command(
        &mut self,
        id: &str,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.run_extension_command_for(id, None, cx)
    }

    /// Like [`Self::run_extension_command`], for repository `target` (a
    /// repository tab's menu) instead of the active one.
    pub(in crate::view) fn run_extension_command_for(
        &mut self,
        id: &str,
        target: Option<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(registry) = registry(cx) else {
            return false;
        };
        let Some((_, command)) = registry
            .commands()
            .iter()
            .find(|(command_id, _)| command_id.to_string() == id)
        else {
            return false;
        };
        let Some(host) = self.extension_window.as_ref().map(ExtensionWindow::host) else {
            return false;
        };
        let run = Rc::clone(&command.run);
        let requires_repository = command.requires_repository;
        let window_handle = self.window_handle;
        let window_id = window_handle.window_id();
        let target = match target {
            Some(repo_id) => {
                let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
                    return true;
                };
                Some(repository_handle(window_id, repo))
            }
            None => None,
        };
        // Deferred: the command reads this window through its host, and the
        // view is borrowed while it updates.
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let repository = match target {
                    Some(target) => {
                        if host.check(&target, cx).is_err() {
                            return;
                        }
                        Some(target)
                    }
                    None => host.active_repository(cx).ok().flatten(),
                };
                if requires_repository && repository.is_none() {
                    return;
                }
                crate::view::perf::extension_dispatch();
                run(
                    CommandContext {
                        window: host,
                        repository,
                    },
                    window,
                    cx,
                );
            });
        });
        true
    }
}

impl GitCometView {
    /// Says why an extension refused to open a repository.
    pub(crate) fn show_repository_entry_denial(
        &mut self,
        reason: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        self.push_toast(components::ToastKind::Warning, reason.to_string(), cx);
    }

    /// Shows an extension's dialog.
    pub(in crate::view) fn open_extension_dialog(
        &mut self,
        dialog_id: u64,
        title: SharedString,
        content: DialogContent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let content = content(window, cx);
        self.popover_host.update(cx, |host, cx| {
            host.open_extension_dialog(dialog_id, title, content, window, cx);
        });
    }

    pub(in crate::view) fn close_extension_dialog(
        &mut self,
        dialog_id: u64,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.popover_host.update(cx, |host, cx| {
            host.close_extension_dialog(dialog_id, window, cx);
        });
    }
}
