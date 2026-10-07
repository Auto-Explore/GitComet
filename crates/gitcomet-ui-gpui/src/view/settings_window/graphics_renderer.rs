use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::windows_renderer::{RendererPreference, RendererSession};

impl SettingsWindowView {
    pub(super) fn graphics_renderer_rows(
        &self,
        mut card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let Some(session) = cx.try_global::<RendererSession>().cloned() else {
            return card;
        };
        let state = session.0.borrow();
        let disabled = state.overridden || state.saving;
        card = card.child(
            self.summary_row(
                "settings_graphics_renderer",
                "Graphics renderer",
                state.saved.label().into(),
                self.expanded_section == Some(SettingsSection::GraphicsRenderer),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.toggle_section(SettingsSection::GraphicsRenderer, cx)
                }),
            ),
        );
        if self.expanded_section != Some(SettingsSection::GraphicsRenderer) {
            return card;
        }
        let mut detail = self.detail_container("settings_graphics_renderer_options", theme);
        for (value, id, description) in [
            (
                RendererPreference::Auto,
                "settings_graphics_renderer_auto",
                "DirectX 12 when available, otherwise DirectX 11",
            ),
            (
                RendererPreference::Dx11,
                "settings_graphics_renderer_dx11",
                "Use DirectX 11 directly",
            ),
        ] {
            detail = detail.child(
                self.option_row(
                    id,
                    value.label(),
                    Some(description.into()),
                    state.saved == value,
                    theme,
                )
                .on_activate(
                    disabled,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_graphics_renderer(value, window, cx)
                    }),
                ),
            );
        }
        card.child(
            detail.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(state.status_message()),
            ),
        )
    }

    fn set_graphics_renderer(
        &mut self,
        value: RendererPreference,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(session) = cx.try_global::<RendererSession>().cloned() else {
            return;
        };
        {
            let mut state = session.0.borrow_mut();
            if state.overridden || state.saving || state.saved == value {
                return;
            }
            state.saving = true;
        }
        cx.notify();
        let writer = session.0.borrow().writer.clone();
        let sequence = writer.next();
        let write = cx.background_spawn(async move {
            writer.persist(sequence, || {
                session::persist_ui_settings(crate::windows_renderer::preference_settings(value))
            })
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |view, cx| {
            let result = write.await;
            // Complete process state even if the settings window was closed during the write.
            let ask = cx.update(|_| {
                session
                    .0
                    .borrow_mut()
                    .complete_manual_save(sequence, value, result)
            });
            let _ = view.update(cx, |_, cx| cx.notify());
            let answer = if ask {
                window_handle
                    .update(cx, |_, window, cx| {
                        view.upgrade()?;
                        Some(window.prompt(
                            gpui::PromptLevel::Info,
                            "Restart GitComet?",
                            Some("The saved graphics renderer takes effect after restarting."),
                            &[
                                gpui::PromptButton::new("Restart now"),
                                gpui::PromptButton::cancel("Later"),
                            ],
                            cx,
                        ))
                    })
                    .ok()
                    .flatten()
            } else {
                None
            };
            if let Some(answer) = answer
                && answer.await.ok() == Some(0)
            {
                cx.update(|cx| cx.defer(crate::app::request_restart));
            }
        })
        .detach();
    }
}
