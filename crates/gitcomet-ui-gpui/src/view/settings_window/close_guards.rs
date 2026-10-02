use super::*;
use gitcomet_extension_api::{CloseDecision, CloseRequest, CloseScope};

impl SettingsWindowView {
    fn close_guard_reasons(&self, scope: CloseScope, cx: &App) -> Vec<SharedString> {
        let Some(extension) = &self.extension_window else {
            return Vec::new();
        };
        let Some(registry) = crate::view::extension_host::registry(cx) else {
            return Vec::new();
        };
        let request = CloseRequest {
            scope,
            window: extension.host(),
            repository: None,
        };
        let mut reasons = Vec::new();
        for (_, guard) in registry.close_guards() {
            crate::view::perf::extension_dispatch();
            if let CloseDecision::Confirm { reason } = guard(&request, cx)
                && !reasons.contains(&reason)
            {
                reasons.push(reason);
            }
        }
        reasons
    }

    pub(super) fn request_close(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let reasons = self.close_guard_reasons(CloseScope::Window, cx);
        if reasons.is_empty() {
            crate::app::mark_clean_shutdown_if_last_window_from_view(cx);
            window.remove_window();
        } else {
            self.confirm_close(CloseScope::Window, reasons, cx);
        }
    }

    fn confirm_close(
        &self,
        scope: CloseScope,
        reasons: Vec<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(extension) = &self.extension_window else {
            return;
        };
        let parent = cx.weak_entity();
        let theme = self.theme;
        let _ = extension.host().open_dialog(
            if scope == CloseScope::Application {
                "Quit application?"
            } else {
                "Close Settings?"
            },
            move |_, cx| {
                cx.new(|_| SettingsClosePrompt {
                    scope,
                    reasons,
                    theme,
                    parent,
                })
                .into()
            },
            cx,
        );
    }
}

struct SettingsClosePrompt {
    scope: CloseScope,
    reasons: Vec<SharedString>,
    theme: AppTheme,
    parent: gpui::WeakEntity<SettingsWindowView>,
}
impl Render for SettingsClosePrompt {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let cancel = self.parent.clone();
        let parent = self.parent.clone();
        let scope = self.scope;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(
                self.reasons
                    .iter()
                    .cloned()
                    .map(|reason| div().child(reason)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        components::Button::new("settings_close_cancel", "Cancel").on_click(
                            self.theme,
                            cx,
                            move |_, _, window, cx| {
                                let _ = cancel
                                    .update(cx, |view, cx| view.close_hosted_dialog(window, cx));
                            },
                        ),
                    )
                    .child(
                        components::Button::new(
                            "settings_close_confirm",
                            if scope == CloseScope::Application {
                                "Quit"
                            } else {
                                "Close"
                            },
                        )
                        .on_click(
                            self.theme,
                            cx,
                            move |_, _, window, cx| {
                                let _ = parent
                                    .update(cx, |view, cx| view.close_hosted_dialog(window, cx));
                                if scope == CloseScope::Application {
                                    cx.defer(|cx| {
                                        crate::app::mark_clean_shutdown_requested(cx);
                                        cx.quit();
                                    });
                                } else {
                                    crate::app::mark_clean_shutdown_if_last_window(cx);
                                    window.remove_window();
                                }
                            },
                        ),
                    ),
            )
    }
}

pub(crate) fn quit_reasons(cx: &App) -> Vec<SharedString> {
    cx.windows()
        .into_iter()
        .filter_map(|window| window.downcast::<SettingsWindowView>())
        .filter_map(|window| {
            window
                .read_with(cx, |view, cx| {
                    view.close_guard_reasons(CloseScope::Application, cx)
                })
                .ok()
        })
        .flatten()
        .collect()
}

pub(crate) fn request_settings_only_quit(cx: &mut App) -> bool {
    let reasons = quit_reasons(cx);
    if reasons.is_empty() {
        return false;
    }
    let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<SettingsWindowView>())
    else {
        return false;
    };
    let _ = window.update(cx, |view, window, cx| {
        window.activate();
        view.confirm_close(CloseScope::Application, reasons, cx);
    });
    true
}

pub(crate) fn request_native_close(window: &mut Window, cx: &mut App) {
    if let Some(Some(view)) = window.root::<SettingsWindowView>() {
        view.update(cx, |view, cx| view.request_close(window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_extension_api::*;
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn settings_close_and_quit_include_extension_guards(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        struct Guard(Rc<Cell<usize>>);
        impl Extension for Guard {
            fn id(&self) -> ExtensionId {
                ExtensionId::new("com.example.settings-guard").unwrap()
            }
            fn register(&self, r: &mut Registrar) {
                let calls = self.0.clone();
                r.close_guard(
                    "pending",
                    Rc::new(move |request, _| {
                        assert_eq!(
                            request.window.kind(),
                            gitcomet_core::identity::WindowKind::Settings
                        );
                        calls.set(calls.get() + 1);
                        CloseDecision::Confirm {
                            reason: "A settings task is still running.".into(),
                        }
                    }),
                );
            }
        }
        let calls = Rc::new(Cell::new(0));
        cx.update(|cx| {
            crate::view::extension_host::install(
                Registry::build(vec![Box::new(Guard(calls.clone()))]).unwrap(),
                cx,
            )
        });
        let (view, cx) = cx.add_window_view(SettingsWindowView::new);
        cx.run_until_parked();
        cx.update(|window, app| view.update(app, |view, cx| view.request_close(window, cx)));
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        assert_eq!(calls.get(), 1);
        assert!(cx.update(|_, app| view.read(app).extension_dialog.is_some()));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.update(|_, app| view.read(app).extension_dialog.is_none()));
        assert_eq!(cx.cx.update(|app| quit_reasons(app)).len(), 1);
        assert!(cx.cx.update(request_settings_only_quit));
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                view.read(app).extension_dialog.as_ref().unwrap().title,
                "Quit application?"
            )
        });
    }
}
