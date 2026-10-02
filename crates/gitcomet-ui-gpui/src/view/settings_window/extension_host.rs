//! Settings has its own weak host and lifecycle, even without a repository.
use super::*;
use gitcomet_extension_api::host::WindowHostImpl;
use gitcomet_extension_api::*;
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

pub(super) struct SettingsExtensions {
    host: Rc<SettingsHost>,
    instances: Rc<RefCell<Vec<Box<dyn WindowExtension>>>>,
    _notifications: gpui::Task<()>,
    _closed: gpui::Subscription,
}

struct SettingsHost {
    weak: Weak<Self>,
    view: gpui::WeakEntity<SettingsWindowView>,
    window: gpui::AnyWindowHandle,
    open: Cell<bool>,
    theme: Cell<AppTheme>,
    notifier: HostNotifier,
}

impl SettingsExtensions {
    pub(super) fn new(
        window: &Window,
        theme: AppTheme,
        cx: &mut gpui::Context<SettingsWindowView>,
    ) -> Option<Self> {
        let registry = crate::view::extension_host::registry(cx)?;
        let (tx, rx) = smol::channel::bounded(1);
        let notifier = HostNotifier::new(move || {
            let _ = tx.try_send(());
        });
        let pending = notifier.clone();
        let task = cx.spawn(async move |view, cx| {
            while rx.recv().await.is_ok() {
                let slots = pending.take_pending();
                if view
                    .update(cx, |view, cx| {
                        if slots & Slot::Gate.mask() != 0
                            && let Some(gates) = &mut view.window_gates
                        {
                            gates.invalidate();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let window = window.window_handle();
        let host = Rc::new_cyclic(|weak| SettingsHost {
            weak: weak.clone(),
            view: cx.weak_entity(),
            window,
            open: Cell::new(true),
            theme: Cell::new(theme),
            notifier,
        });
        let instances: Rc<RefCell<Vec<Box<dyn WindowExtension>>>> = Rc::default();
        let opening_host = Rc::downgrade(&host);
        let opening_instances = Rc::downgrade(&instances);
        cx.defer(move |cx| {
            let (Some(host), Some(instances)) =
                (opening_host.upgrade(), opening_instances.upgrade())
            else {
                return;
            };
            let handle = WindowHost::new(host);
            let _ = window.update(cx, |_, window, cx| {
                *instances.borrow_mut() = registry
                    .instances()
                    .iter()
                    .filter_map(|extension| {
                        crate::view::perf::extension_dispatch();
                        extension.window_opened(handle.clone(), window, cx)
                    })
                    .collect();
            });
        });
        let closing = Rc::downgrade(&host);
        let closing_instances = Rc::downgrade(&instances);
        let closed = cx.on_window_closed(move |cx, id| {
            if id == window.window_id()
                && let Some(host) = closing.upgrade()
            {
                host.open.set(false);
                host.notifier.close();
                if let Some(instances) = closing_instances.upgrade() {
                    let handle = WindowHost::new(host);
                    for instance in instances.borrow_mut().iter_mut() {
                        crate::view::perf::extension_dispatch();
                        instance.on_event(&ShellEvent::WindowClosed, &handle, cx);
                    }
                    instances.borrow_mut().clear();
                }
            }
        });
        Some(Self {
            host,
            instances,
            _notifications: task,
            _closed: closed,
        })
    }
    pub(super) fn host(&self) -> WindowHost {
        WindowHost::new(self.host.clone())
    }
    pub(super) fn set_theme(&self, theme: AppTheme, cx: &mut App) {
        if self.host.theme.replace(theme) == theme {
            return;
        }
        let instances = Rc::downgrade(&self.instances);
        let host = self.host();
        cx.defer(move |cx| {
            if host.is_open(cx)
                && let Some(instances) = instances.upgrade()
            {
                for instance in instances.borrow_mut().iter_mut() {
                    crate::view::perf::extension_dispatch();
                    instance.on_event(&ShellEvent::ThemeChanged, &host, cx);
                }
            }
        });
    }
}
impl Drop for SettingsExtensions {
    fn drop(&mut self) {
        self.host.open.set(false);
        self.host.notifier.close();
        self.instances.borrow_mut().clear();
    }
}

impl SettingsHost {
    fn live(&self) -> Result<(), HostError> {
        if self.open.get() && self.view.upgrade().is_some() {
            Ok(())
        } else {
            Err(HostError::WindowClosed)
        }
    }
    fn unsupported<T>(&self) -> Result<T, HostError> {
        self.live()?;
        Err(HostError::Unsupported)
    }
    /// Opens `launch` after this update; a failure shows as an error notice.
    fn launch(
        &self,
        launch: crate::view::platform_open::Launch,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.live()?;
        let view = self.view.clone();
        crate::view::platform_open::launch_later(
            launch,
            move |err, cx| {
                let _ = view.update(cx, |view, cx| {
                    view.extension_notice = Some((
                        NotificationKind::Error,
                        format!("Could not open: {err}").into(),
                        Vec::new(),
                    ));
                    cx.notify();
                });
            },
            cx,
        )
        .map_err(|err| HostError::InvalidRequest(err.to_string().into()))
    }
    fn handle(&self) -> Result<WindowHost, HostError> {
        self.live()?;
        Ok(WindowHost::new(
            self.weak.upgrade().ok_or(HostError::WindowClosed)?,
        ))
    }
}

impl WindowHostImpl for SettingsHost {
    fn repository_reader(&self) -> RepositoryReader {
        RepositoryReader::new(self.window.window_id(), |_, _| None)
    }
    fn kind(&self) -> gitcomet_core::identity::WindowKind {
        gitcomet_core::identity::WindowKind::Settings
    }
    fn notifier(&self) -> HostNotifier {
        self.notifier.clone()
    }
    fn window_id(&self) -> gpui::WindowId {
        self.window.window_id()
    }
    fn is_open(&self, _: &App) -> bool {
        self.live().is_ok()
    }
    fn active_repository(&self, _: &App) -> Result<Option<RepositoryHandle>, HostError> {
        self.live()?;
        Ok(None)
    }
    fn state(&self, _: &App) -> Result<Arc<AppState>, HostError> {
        self.live()?;
        Ok(Arc::default())
    }
    fn theme(&self, _: &App) -> AppTheme {
        self.theme.get()
    }
    fn observe_state(&self, _: StateObserver) -> Result<u64, HostError> {
        self.live()?;
        Ok(0)
    }
    fn unobserve_state(&self, _: u64) {}
    fn navigate(&self, _: &RepositoryHandle, _: ViewTarget, _: &mut App) -> Result<(), HostError> {
        self.unsupported()
    }
    fn open_settings_at(&self, target: SettingsTarget, cx: &mut App) -> Result<(), HostError> {
        self.live()?;
        cx.defer(move |cx| super::open_settings_at(target, cx));
        Ok(())
    }
    fn create_diff_pane(
        &self,
        _: &RepositoryHandle,
        _: DiffTarget,
        _: DiffPaneOptions,
        _: &mut App,
    ) -> Result<DiffPane, HostError> {
        self.unsupported()
    }
    fn create_snapshot_pane(
        &self,
        snapshot: DiffSnapshot,
        options: DiffPaneOptions,
        cx: &mut App,
    ) -> Result<DiffPane, HostError> {
        let host = self.handle()?;
        let entity = cx.new(|cx| {
            crate::view::hosted::diff_pane::DiffPaneView::snapshot(host, snapshot, options, cx)
        });
        Ok(DiffPane::new(Rc::new(
            crate::view::hosted::diff_pane::HostedDiffPane { entity },
        )))
    }
    fn create_file_list(
        &self,
        _: &RepositoryHandle,
        _: ChangeSource,
        _: FileSelected,
        _: &mut App,
    ) -> Result<FileList, HostError> {
        self.unsupported()
    }
    fn open_bottom_panel(
        &self,
        _: &RepositoryHandle,
        _: &ContributionId,
        _: &mut App,
    ) -> Result<(), HostError> {
        self.unsupported()
    }
    fn close_bottom_panel(
        &self,
        _: &RepositoryHandle,
        _: &ContributionId,
        _: &mut App,
    ) -> Result<(), HostError> {
        self.unsupported()
    }
    fn is_bottom_panel_open(&self, _: &RepositoryHandle, _: &ContributionId, _: &App) -> bool {
        false
    }
    fn watch_repository(
        &self,
        _: &RepositoryHandle,
        _: &App,
    ) -> Result<RepositoryWatch, HostError> {
        self.unsupported()
    }
    fn watch_worktree(
        &self,
        _: &RepositoryHandle,
        _: &std::path::Path,
        _: &App,
    ) -> Result<RepositoryWatch, HostError> {
        self.unsupported()
    }
    fn highlight_line(
        &self,
        path: &std::path::Path,
        text: &str,
        _: &App,
    ) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        crate::view::rows::diff_syntax_language_for_path(path)
            .map(|language| {
                crate::view::rows::syntax_highlights_for_line(
                    self.theme.get(),
                    text,
                    language,
                    crate::view::rows::DiffSyntaxMode::HeuristicOnly,
                )
            })
            .unwrap_or_default()
    }
    fn is_current(&self, _: &RepositoryHandle, _: &App) -> bool {
        false
    }
    fn dispatch(&self, _: Msg, _: &mut App) -> Result<(), HostError> {
        self.unsupported()
    }
    fn open_dialog(
        &self,
        title: SharedString,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.present(title, None, content, cx)
    }
    fn open_popover(
        &self,
        title: SharedString,
        anchor: gpui::Point<gpui::Pixels>,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.present(title, Some(anchor), content, cx)
    }
    fn open_menu(
        &self,
        anchor: gpui::Point<gpui::Pixels>,
        items: Vec<HostedMenuItem>,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        let theme = self.theme.get();
        let parent = self.view.clone();
        self.present(
            "".into(),
            Some(anchor),
            Box::new(move |_, cx| {
                cx.new(|_| SettingsMenu {
                    items,
                    theme,
                    parent,
                })
                .into()
            }),
            cx,
        )
    }
    fn toast(
        &self,
        kind: NotificationKind,
        message: SharedString,
        actions: Vec<HostedAction>,
        cx: &mut App,
    ) -> Result<(), HostError> {
        self.live()?;
        let view = self.view.clone();
        cx.defer(move |cx| {
            let _ = view.update(cx, |view, cx| {
                view.extension_notice = Some((kind, message, actions));
                cx.notify();
            });
        });
        Ok(())
    }
    fn open_window(
        &self,
        title: SharedString,
        content: WindowContent,
        on_closed: OnWindowClosed,
        cx: &mut App,
    ) -> Result<PopOutWindow, HostError> {
        crate::view::pop_out::open(
            self.handle()?,
            self.window.window_id(),
            title,
            content,
            on_closed,
            cx,
        )
    }
    fn notify(&self, message: SharedString, cx: &mut App) -> Result<(), HostError> {
        self.toast(NotificationKind::Success, message, Vec::new(), cx)
    }
    fn open_url(&self, url: &str, cx: &mut App) -> Result<(), HostError> {
        self.launch(crate::view::platform_open::Launch::Url(url.to_string()), cx)
    }
    fn open_path(&self, path: &std::path::Path, cx: &mut App) -> Result<(), HostError> {
        self.launch(
            crate::view::platform_open::Launch::Path(path.to_path_buf()),
            cx,
        )
    }

    fn workspace_state(
        &self,
        _: &ExtensionId,
        _: &App,
    ) -> Result<Option<serde_json::Value>, HostError> {
        self.unsupported()
    }
    fn set_workspace_state(
        &self,
        _: &ExtensionId,
        _: serde_json::Value,
        _: &mut App,
    ) -> Result<Result<(), storage::StorageError>, HostError> {
        self.unsupported()
    }
}

struct SettingsMenu {
    items: Vec<HostedMenuItem>,
    theme: AppTheme,
    parent: gpui::WeakEntity<SettingsWindowView>,
}
impl Render for SettingsMenu {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        div()
            .flex()
            .flex_col()
            .children(self.items.iter().enumerate().map(|(index, item)| {
                match item {
                    HostedMenuItem::Header(label) => {
                        div().px_2().child(label.clone()).into_any_element()
                    }
                    HostedMenuItem::Separator => div()
                        .h(px(1.0))
                        .bg(self.theme.colors.stroke.subtle)
                        .into_any_element(),
                    HostedMenuItem::Action {
                        action,
                        icon,
                        disabled,
                        ..
                    } => {
                        let action = action.clone();
                        let parent = self.parent.clone();
                        let mut button = components::Button::new(
                            format!("settings_hosted_action_{index}"),
                            action.label().clone(),
                        );
                        if let Some(icon) = icon.clone() {
                            button = button.start_slot(crate::view::icons::svg_icon(
                                icon,
                                self.theme.colors.foreground.secondary,
                                ui_scale.px(14.0),
                            ));
                        }
                        button
                            .disabled(*disabled)
                            .on_click(self.theme, cx, move |_, _, window, cx| {
                                let _ = parent
                                    .update(cx, |view, cx| view.close_hosted_dialog(window, cx));
                                action.invoke(cx);
                            })
                            .into_any_element()
                    }
                    _ => gpui::Empty.into_any_element(),
                }
            }))
    }
}

pub(super) struct SettingsDialog {
    pub id: u64,
    pub title: SharedString,
    pub content: gpui::AnyView,
    pub anchor: Option<gpui::Point<gpui::Pixels>>,
    pub focus: gpui::FocusHandle,
    previous_focus: Option<gpui::FocusHandle>,
}
impl SettingsHost {
    fn present(
        &self,
        title: SharedString,
        anchor: Option<gpui::Point<gpui::Pixels>>,
        content: DialogContent,
        cx: &mut App,
    ) -> Result<DialogHandle, HostError> {
        self.live()?;
        let id = crate::view::extension_host::next_dialog_id();
        let view = self.view.clone();
        let window = self.window;
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let content = content(window, cx);
                let _ = view.update(cx, |view, cx| {
                    let focus = cx.focus_handle();
                    let previous_focus = view
                        .extension_dialog
                        .take()
                        .and_then(|dialog| dialog.previous_focus)
                        .or_else(|| window.focused(cx));
                    window.focus(&focus, cx);
                    view.extension_dialog = Some(SettingsDialog {
                        id,
                        title,
                        content,
                        anchor,
                        focus,
                        previous_focus,
                    });
                    cx.notify();
                });
            });
        });
        let view = self.view.clone();
        Ok(DialogHandle::new(move |cx| {
            cx.defer(move |cx| {
                let _ = window.update(cx, |_, window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        if view
                            .extension_dialog
                            .as_ref()
                            .is_some_and(|dialog| dialog.id == id)
                        {
                            view.close_hosted_dialog(window, cx);
                        }
                    });
                });
            })
        }))
    }
}
impl SettingsWindowView {
    pub(super) fn close_hosted_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(dialog) = self.extension_dialog.take() {
            if let Some(focus) = dialog.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }
    pub(super) fn hosted_dialog_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(dialog) = &self.extension_dialog else {
            return;
        };
        match event.keystroke.key.as_str() {
            "escape" => self.close_hosted_dialog(window, cx),
            "tab" => {
                let focus = dialog.focus.clone();
                if event.keystroke.modifiers.shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                if !focus.contains_focused(window, cx) {
                    window.focus(&focus, cx);
                    window.focus_next(cx);
                    if !focus.contains_focused(window, cx) {
                        window.focus(&focus, cx);
                    }
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}
