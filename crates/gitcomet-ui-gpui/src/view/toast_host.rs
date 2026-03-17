use super::*;

pub(super) struct ToastHost {
    theme: AppTheme,
    tooltip_host: WeakEntity<TooltipHost>,
    root_view: WeakEntity<GitCometView>,

    toasts: Vec<ToastState>,
    clone_progress_toast_id: Option<u64>,
    clone_progress_last_seq: u64,
    clone_progress_dest: Option<std::path::PathBuf>,
}

fn looks_like_code_message(message: &str) -> bool {
    message.lines().any(|line| line.starts_with("    "))
}

fn strip_code_message_indentation(message: &str) -> String {
    message
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

impl ToastHost {
    pub(super) fn new(
        theme: AppTheme,
        tooltip_host: WeakEntity<TooltipHost>,
        root_view: WeakEntity<GitCometView>,
    ) -> Self {
        Self {
            theme,
            tooltip_host,
            root_view,
            toasts: Vec::new(),
            clone_progress_toast_id: None,
            clone_progress_last_seq: 0,
            clone_progress_dest: None,
        }
    }

    fn route_error_to_banner(&mut self, message: String, cx: &mut gpui::Context<Self>) -> bool {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.push_toast(components::ToastKind::Error, message, cx);
            });
        });
        true
    }

    pub(super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(super) fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error)
            && self.route_error_to_banner(message.clone(), cx)
        {
            return;
        }
        let ttl = match kind {
            components::ToastKind::Error => Duration::from_secs(15),
            components::ToastKind::Warning => Duration::from_secs(10),
            components::ToastKind::Success => Duration::from_secs(6),
        };
        let _ = self.push_toast_inner(kind, message, None, Some(ttl), cx);
    }

    #[cfg(not(test))]
    pub(super) fn push_toast_with_link(
        &mut self,
        kind: components::ToastKind,
        message: String,
        link_url: String,
        link_label: String,
        cx: &mut gpui::Context<Self>,
    ) {
        if matches!(kind, components::ToastKind::Error)
            && self.route_error_to_banner(message.clone(), cx)
        {
            return;
        }
        let ttl = match kind {
            components::ToastKind::Error => Duration::from_secs(15),
            components::ToastKind::Warning => Duration::from_secs(10),
            components::ToastKind::Success => Duration::from_secs(6),
        };
        let _ = self.push_toast_inner(kind, message, Some((link_url, link_label)), Some(ttl), cx);
    }

    pub(super) fn push_persistent_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) -> u64 {
        self.push_toast_inner(kind, message, None, None, cx)
    }

    fn push_toast_inner(
        &mut self,
        kind: components::ToastKind,
        message: String,
        action: Option<(String, String)>,
        ttl: Option<Duration>,
        cx: &mut gpui::Context<Self>,
    ) -> u64 {
        let id = self
            .toasts
            .last()
            .map(|t| t.id.wrapping_add(1))
            .unwrap_or(1);
        let theme = self.theme;
        let is_code_message = looks_like_code_message(&message);
        let display_message = if is_code_message {
            strip_code_message_indentation(&message)
        } else {
            message
        };
        let input = cx.new(|cx| {
            components::TextInput::new_inert(
                components::TextInputOptions {
                    placeholder: "".into(),
                    multiline: true,
                    read_only: true,
                    chromeless: true,
                    soft_wrap: true,
                },
                cx,
            )
        });
        input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(display_message, cx);
            input.set_read_only(true, cx);
        });

        let (action_url, action_label) = action
            .map(|(url, label)| (Some(url), Some(label)))
            .unwrap_or((None, None));
        self.toasts.push(ToastState {
            id,
            kind,
            input,
            is_code_message,
            action_url,
            action_label,
            ttl,
        });
        cx.notify();

        if let Some(ttl) = ttl {
            let lifetime = toast_total_lifetime(ttl);
            cx.spawn(
                async move |view: WeakEntity<ToastHost>, cx: &mut gpui::AsyncApp| {
                    Timer::after(lifetime).await;
                    let _ = view.update(cx, |this, cx| {
                        this.remove_toast(id, cx);
                    });
                },
            )
            .detach();
        }

        id
    }

    pub(super) fn update_toast_text(
        &mut self,
        id: u64,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(toast) = self.toasts.iter_mut().find(|t| t.id == id) else {
            return;
        };
        let theme = self.theme;
        toast.is_code_message = looks_like_code_message(&message);
        let display_message = if toast.is_code_message {
            strip_code_message_indentation(&message)
        } else {
            message
        };
        toast.input.update(cx, |input, cx| {
            input.set_theme(theme, cx);
            input.set_text(display_message, cx);
            input.set_read_only(true, cx);
        });
    }

    pub(super) fn remove_toast(&mut self, id: u64, cx: &mut gpui::Context<Self>) {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.id != id);
        if self.toasts.len() != before {
            cx.notify();
        }
    }

    pub(super) fn sync_clone_progress(
        &mut self,
        next_clone: Option<&CloneOpState>,
        cx: &mut gpui::Context<Self>,
    ) {
        match next_clone {
            Some(op) => match &op.status {
                CloneOpStatus::Running => {
                    let needs_reset = self.clone_progress_toast_id.is_none()
                        || self.clone_progress_dest.as_ref() != Some(&op.dest);
                    if needs_reset {
                        if let Some(id) = self.clone_progress_toast_id.take() {
                            self.remove_toast(id, cx);
                        }
                        self.clone_progress_last_seq = 0;
                        self.clone_progress_dest = Some(op.dest.clone());

                        let id = self.push_persistent_toast(
                            components::ToastKind::Success,
                            format!("Cloning repository…\n{}\n→ {}", op.url, op.dest.display()),
                            cx,
                        );
                        self.clone_progress_toast_id = Some(id);
                    }

                    if let Some(id) = self.clone_progress_toast_id
                        && self.clone_progress_last_seq != op.seq
                    {
                        self.clone_progress_last_seq = op.seq;
                        let tail_lines = op.output_tail.iter().rev().take(12).rev().cloned();
                        let tail = tail_lines.collect::<Vec<_>>().join("\n");
                        let message = if tail.is_empty() {
                            format!("Cloning repository…\n{}\n→ {}", op.url, op.dest.display())
                        } else {
                            format!(
                                "Cloning repository…\n{}\n→ {}\n\n{}",
                                op.url,
                                op.dest.display(),
                                tail
                            )
                        };
                        self.update_toast_text(id, message, cx);
                    }
                }
                CloneOpStatus::FinishedOk => {
                    if self.clone_progress_last_seq != op.seq {
                        if let Some(id) = self.clone_progress_toast_id.take() {
                            self.remove_toast(id, cx);
                        }
                        self.clone_progress_dest = None;
                        self.clone_progress_last_seq = op.seq;
                        self.push_toast(
                            components::ToastKind::Success,
                            format!("Clone finished: {}", op.dest.display()),
                            cx,
                        );
                    }
                }
                CloneOpStatus::FinishedErr(err) => {
                    if self.clone_progress_last_seq != op.seq {
                        if let Some(id) = self.clone_progress_toast_id.take() {
                            self.remove_toast(id, cx);
                        }
                        self.clone_progress_dest = None;
                        self.clone_progress_last_seq = op.seq;
                        self.push_toast(components::ToastKind::Error, err.clone(), cx);
                    }
                }
            },
            None => {
                if let Some(id) = self.clone_progress_toast_id.take() {
                    self.remove_toast(id, cx);
                }
                self.clone_progress_last_seq = 0;
                self.clone_progress_dest = None;
            }
        }
    }

    fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }
}

impl Render for ToastHost {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if self.toasts.is_empty() {
            return div().into_any_element();
        }
        let theme = self.theme;

        let progress_id = self.clone_progress_toast_id;
        let max_other = if progress_id.is_some() { 2 } else { 3 };
        let mut displayed = self
            .toasts
            .iter()
            .rev()
            .filter(|t| Some(t.id) != progress_id)
            .take(max_other)
            .cloned()
            .collect::<Vec<_>>();
        if let Some(id) = progress_id
            && let Some(progress) = self.toasts.iter().find(|t| t.id == id).cloned()
        {
            displayed.push(progress);
        }

        let fade_in = toast_fade_in_duration();
        let fade_out = toast_fade_out_duration();
        let children = displayed.into_iter().map(move |t| {
            let animations = match t.ttl {
                Some(ttl) => vec![
                    Animation::new(fade_in).with_easing(gpui::quadratic),
                    Animation::new(ttl),
                    Animation::new(fade_out).with_easing(gpui::quadratic),
                ],
                None => vec![Animation::new(fade_in).with_easing(gpui::quadratic)],
            };

            let close = components::Button::new(format!("toast_close_{}", t.id), "")
                .start_slot(svg_icon(
                    "icons/generic_close.svg",
                    theme.colors.text_muted,
                    px(12.0),
                ))
                .style(components::ButtonStyle::Transparent)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.remove_toast(t.id, cx);
                })
                .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                    let text: SharedString = "Dismiss notification".into();
                    if *hovering {
                        this.set_tooltip_text_if_changed(Some(text), cx);
                    } else {
                        this.clear_tooltip_if_matches(&text, cx);
                    }
                }));

            let message_scroll = div()
                .id(("toast_message_scroll", t.id))
                .max_h(px(200.0))
                .overflow_y_scroll()
                .child(
                    div()
                        .when(t.is_code_message, |this| {
                            this.font_family("monospace")
                                .bg(with_alpha(
                                    theme.colors.window_bg,
                                    if theme.is_dark { 0.28 } else { 0.75 },
                                ))
                                .rounded(px(theme.radii.row))
                                .px_2()
                                .py_1()
                        })
                        .child(t.input.clone()),
                );

            let action_button =
                t.action_url
                    .clone()
                    .zip(t.action_label.clone())
                    .map(|(url, label)| {
                        components::Button::new(format!("toast_action_{}", t.id), label)
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, cx| {
                                match super::platform_open::open_url(&url) {
                                    Ok(()) => {
                                        this.remove_toast(t.id, cx);
                                    }
                                    Err(err) => {
                                        this.push_toast(
                                            components::ToastKind::Error,
                                            format!("Failed to open link: {err}"),
                                            cx,
                                        );
                                    }
                                }
                            })
                    });

            let message = div()
                .flex()
                .flex_col()
                .gap_1()
                .child(message_scroll)
                .when_some(action_button, |this, button| this.child(button));

            div()
                .relative()
                .child(components::toast(theme, t.kind, message))
                .child(div().absolute().top(px(8.0)).right(px(8.0)).child(close))
                .with_animations(
                    ("toast", t.id),
                    animations,
                    move |toast, animation_ix, delta| {
                        let opacity = match animation_ix {
                            0 => delta,
                            1 => 1.0,
                            2 => 1.0 - delta,
                            _ => 1.0,
                        };
                        let slide_x = match animation_ix {
                            0 => (1.0 - delta) * TOAST_SLIDE_PX,
                            2 => delta * TOAST_SLIDE_PX,
                            _ => 0.0,
                        };
                        toast.opacity(opacity).relative().left(px(slide_x))
                    },
                )
        });

        div()
            .id("toast_layer")
            .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
            .occlude()
            .absolute()
            .right_0()
            .bottom_0()
            .p(px(16.0))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(12.0))
            .children(children)
            .into_any_element()
    }
}
