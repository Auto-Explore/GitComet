use super::*;
use gitcomet_state::model::SubmoduleAddProgressState;

pub(super) struct ToastHost {
    theme: AppTheme,
    root_view: WeakEntity<GitCometView>,

    toasts: Vec<ToastState>,
    clone_progress: Option<CloneOpState>,
    clone_progress_last_seq: u64,
    clone_progress_dest: Option<std::sync::Arc<std::path::PathBuf>>,
    submodule_add_progress: Vec<SubmoduleAddProgressState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastViewportCorner {
    BottomRight,
}

#[derive(Debug, Eq, PartialEq)]
struct CloneProgressSyncAction {
    progress_changed: bool,
    notice: Option<(components::ToastKind, String)>,
}

fn clone_progress_shell_border_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.accent, if theme.is_dark { 0.36 } else { 0.28 })
}

fn clone_progress_shell_accent_color(theme: AppTheme) -> gpui::Rgba {
    with_alpha(theme.colors.accent, if theme.is_dark { 0.20 } else { 0.14 })
}

fn toast_viewport_corner() -> ToastViewportCorner {
    ToastViewportCorner::BottomRight
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

fn apply_clone_progress_sync(
    clone_progress: &mut Option<CloneOpState>,
    clone_progress_last_seq: &mut u64,
    clone_progress_dest: &mut Option<std::sync::Arc<std::path::PathBuf>>,
    next_clone: Option<&CloneOpState>,
) -> CloneProgressSyncAction {
    match next_clone {
        Some(op) => match &op.status {
            CloneOpStatus::Running | CloneOpStatus::Cancelling => {
                let needs_reset = clone_progress.is_none()
                    || !matches!(
                        clone_progress_dest.as_ref(),
                        Some(dest) if std::sync::Arc::ptr_eq(dest, &op.dest)
                    );
                if needs_reset {
                    *clone_progress_last_seq = 0;
                    *clone_progress_dest = Some(op.dest.clone());
                }

                if needs_reset || *clone_progress_last_seq != op.seq {
                    *clone_progress_last_seq = op.seq;
                    *clone_progress = Some(op.clone());
                    CloneProgressSyncAction {
                        progress_changed: true,
                        notice: None,
                    }
                } else {
                    CloneProgressSyncAction {
                        progress_changed: false,
                        notice: None,
                    }
                }
            }
            CloneOpStatus::FinishedOk => {
                if *clone_progress_last_seq != op.seq {
                    let had_progress = clone_progress.take().is_some();
                    *clone_progress_dest = None;
                    *clone_progress_last_seq = op.seq;
                    CloneProgressSyncAction {
                        progress_changed: had_progress,
                        notice: Some((
                            components::ToastKind::Success,
                            format!("Clone finished: {}", op.dest.display()),
                        )),
                    }
                } else {
                    CloneProgressSyncAction {
                        progress_changed: false,
                        notice: None,
                    }
                }
            }
            CloneOpStatus::Cancelled => {
                if *clone_progress_last_seq != op.seq {
                    let had_progress = clone_progress.take().is_some();
                    *clone_progress_dest = None;
                    *clone_progress_last_seq = op.seq;
                    CloneProgressSyncAction {
                        progress_changed: had_progress,
                        notice: Some((
                            components::ToastKind::Warning,
                            format!("Clone aborted: {}", op.dest.display()),
                        )),
                    }
                } else {
                    CloneProgressSyncAction {
                        progress_changed: false,
                        notice: None,
                    }
                }
            }
            CloneOpStatus::FinishedErr(err) => {
                if *clone_progress_last_seq != op.seq {
                    let had_progress = clone_progress.take().is_some();
                    *clone_progress_dest = None;
                    *clone_progress_last_seq = op.seq;
                    CloneProgressSyncAction {
                        progress_changed: had_progress,
                        notice: Some((components::ToastKind::Error, err.clone())),
                    }
                } else {
                    CloneProgressSyncAction {
                        progress_changed: false,
                        notice: None,
                    }
                }
            }
        },
        None => {
            let had_progress = clone_progress.take().is_some();
            *clone_progress_last_seq = 0;
            *clone_progress_dest = None;
            CloneProgressSyncAction {
                progress_changed: had_progress,
                notice: None,
            }
        }
    }
}

fn apply_submodule_add_progress_sync(
    submodule_add_progress: &mut Vec<SubmoduleAddProgressState>,
    next_submodule_add_progress: &[SubmoduleAddProgressState],
) -> bool {
    if submodule_add_progress == next_submodule_add_progress {
        return false;
    }
    *submodule_add_progress = next_submodule_add_progress.to_vec();
    true
}

impl ToastHost {
    pub(super) fn new(theme: AppTheme, root_view: WeakEntity<GitCometView>) -> Self {
        Self {
            theme,
            root_view,
            toasts: Vec::new(),
            clone_progress: None,
            clone_progress_last_seq: 0,
            clone_progress_dest: None,
            submodule_add_progress: Vec::new(),
        }
    }

    fn route_error_to_banner(&mut self, message: String, cx: &mut gpui::Context<Self>) -> bool {
        let root_view = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root_view.update(cx, |root, cx| {
                root.show_error_banner(None, message);
                cx.notify();
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

    #[cfg_attr(test, allow(dead_code))]
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

        let ttl = if crate::ui_runtime::current().uses_toast_ttl() {
            ttl
        } else {
            None
        };

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
                    smol::Timer::after(lifetime).await;
                    let _ = view.update(cx, |this, cx| {
                        this.remove_toast(id, cx);
                    });
                },
            )
            .detach();
        }

        id
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
        let action = apply_clone_progress_sync(
            &mut self.clone_progress,
            &mut self.clone_progress_last_seq,
            &mut self.clone_progress_dest,
            next_clone,
        );
        if action.progress_changed {
            cx.notify();
        }
        if let Some((kind, message)) = action.notice {
            self.push_toast(kind, message, cx);
        }
    }

    pub(super) fn sync_submodule_add_progress(
        &mut self,
        next_submodule_add_progress: &[SubmoduleAddProgressState],
        cx: &mut gpui::Context<Self>,
    ) {
        if apply_submodule_add_progress_sync(
            &mut self.submodule_add_progress,
            next_submodule_add_progress,
        ) {
            cx.notify();
        }
    }

    fn render_progress_shell(&self, content: impl IntoElement) -> AnyElement {
        let theme = self.theme;
        let shell_bg = with_alpha(
            theme.colors.surface_bg_elevated,
            if theme.is_dark { 0.96 } else { 0.98 },
        );
        let shell_border = clone_progress_shell_border_color(theme);
        let shell_accent = clone_progress_shell_accent_color(theme);

        div()
            .min_w(px(360.0))
            .max_w(px(900.0))
            .flex()
            .gap(px(12.0))
            .bg(shell_bg)
            .border_1()
            .border_color(shell_border)
            .rounded(px(theme.radii.panel))
            .overflow_hidden()
            .shadow_sm()
            .text_lg()
            .text_color(theme.colors.text)
            .child(div().w(px(5.0)).bg(shell_accent).flex_shrink_0())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .pl(px(16.0))
                    .pr(px(16.0))
                    .py(px(12.0))
                    .child(content),
            )
            .into_any_element()
    }

    fn render_clone_progress_toast(
        &self,
        op: CloneOpState,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let spinner_color = crate::view::clone_progress::clone_progress_color(theme, &op);
        let percent = op.progress.percent.min(100);
        let bar_fill_color = crate::view::clone_progress::clone_progress_bar_fill_color(theme, &op);
        let bar_track = crate::view::clone_progress::clone_progress_bar_track_color(theme);
        let bar_border = crate::view::clone_progress::clone_progress_bar_border_color(theme);
        let (bar_fill_weight, bar_remainder_weight) =
            crate::view::clone_progress::clone_progress_segment_weights(percent);
        let aborting = matches!(op.status, CloneOpStatus::Cancelling);
        let dest = op.dest.as_ref().clone();
        let root_view = self.root_view.clone();

        let mut bar_fill = div()
            .h_full()
            .bg(bar_fill_color)
            .rounded(px(999.0))
            .when(percent > 0, |this| this.min_w(px(2.0)));
        bar_fill.style().flex_grow = Some(bar_fill_weight);
        bar_fill.style().flex_shrink = Some(0.0);
        bar_fill.style().flex_basis = Some(relative(0.0).into());

        let mut bar_remainder = div().h_full();
        bar_remainder.style().flex_grow = Some(bar_remainder_weight);
        bar_remainder.style().flex_shrink = Some(0.0);
        bar_remainder.style().flex_basis = Some(relative(0.0).into());

        let mut abort_button = components::Button::new(
            "clone_progress_abort",
            if aborting { "Aborting…" } else { "Abort" },
        )
        .style(components::ButtonStyle::Transparent)
        .borderless()
        .disabled(aborting);
        if aborting {
            abort_button = abort_button.start_slot(svg_spinner(
                "clone_progress_abort_spinner",
                spinner_color,
                px(10.0),
            ));
        }
        let abort_button = abort_button.on_click(theme, cx, move |_this, _e, _w, cx| {
            let _ = root_view.update(cx, |root, _cx| {
                root.store
                    .dispatch(Msg::AbortCloneRepo { dest: dest.clone() });
            });
        });

        let content = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(svg_spinner(
                        "clone_progress_spinner",
                        spinner_color,
                        px(16.0),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_weight(FontWeight::BOLD)
                                    .child(crate::view::clone_progress::clone_progress_title(&op)),
                            )
                            .child(div().text_sm().text_color(theme.colors.text_muted).child(
                                crate::view::clone_progress::clone_progress_dest_label(
                                    op.dest.as_ref(),
                                ),
                            )),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_sm()
                    .child(
                        div()
                            .text_color(spinner_color)
                            .child(crate::view::clone_progress::clone_progress_phase_label(&op)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{percent}%")),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(8.0))
                    .flex()
                    .rounded(px(999.0))
                    .overflow_hidden()
                    .bg(bar_track)
                    .border_1()
                    .border_color(bar_border)
                    .child(bar_fill)
                    .child(bar_remainder),
            )
            .child(div().pt_1().child(abort_button));

        self.render_progress_shell(content)
    }

    fn render_submodule_add_progress_toast(
        &self,
        ix: u64,
        progress: &SubmoduleAddProgressState,
    ) -> AnyElement {
        let theme = self.theme;
        let spinner_color = crate::view::clone_progress::clone_progress_loading_color(theme);
        let content = div().w_full().flex().flex_col().gap_2().child(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap_2()
                .child(svg_spinner(
                    ("submodule_add_progress_spinner", ix),
                    spinner_color,
                    px(16.0),
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child("Adding submodule…"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(progress.path.display().to_string()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.colors.text_muted)
                                .child(progress.url.clone()),
                        ),
                ),
        );
        self.render_progress_shell(content)
    }
}

impl Render for ToastHost {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if self.toasts.is_empty()
            && self.clone_progress.is_none()
            && self.submodule_add_progress.is_empty()
        {
            return div().into_any_element();
        }
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;

        let mut progress_toasts = Vec::new();
        if let Some(progress) = self.clone_progress.clone() {
            progress_toasts.push(self.render_clone_progress_toast(progress, cx));
        }
        progress_toasts.extend(
            self.submodule_add_progress
                .iter()
                .enumerate()
                .map(|(ix, progress)| {
                    self.render_submodule_add_progress_toast(ix as u64, progress)
                }),
        );
        let has_progress = !progress_toasts.is_empty();
        let max_other = if has_progress { 2 } else { 3 };
        let mut displayed = self
            .toasts
            .iter()
            .rev()
            .take(max_other)
            .cloned()
            .collect::<Vec<_>>();

        let fade_in = toast_fade_in_duration();
        let fade_out = toast_fade_out_duration();
        let mut children = displayed
            .drain(..)
            .map(move |t| {
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
                    .render(theme, ui_scale_percent)
                    .gitcomet_tooltip(theme, "Dismiss notification".into())
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                        this.remove_toast(t.id, cx);
                    }));

                let message_scroll = div()
                    .id(("toast_message_scroll", t.id))
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .when(t.is_code_message, |this| {
                                this.font_family(
                                    crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY,
                                )
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
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        children.extend(progress_toasts);

        let root = div()
            .id("toast_layer")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .p(px(16.0))
            .flex()
            .child(
                div()
                    .id("toast_stack")
                    .on_any_mouse_down(|_e, _w, cx| cx.stop_propagation())
                    .occlude()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(12.0))
                    .children(children),
            );

        match toast_viewport_corner() {
            ToastViewportCorner::BottomRight => root.justify_end().items_end().into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::with_alpha;
    use gitcomet_state::model::{CloneProgressMeter, CloneProgressStage};
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn clone_op(
        dest: Arc<PathBuf>,
        status: CloneOpStatus,
        stage: CloneProgressStage,
        percent: u8,
        seq: u64,
    ) -> CloneOpState {
        CloneOpState {
            url: Arc::<str>::from("file:///tmp/repo.git"),
            dest,
            status,
            progress: CloneProgressMeter { stage, percent },
            seq,
            output_tail: VecDeque::new(),
        }
    }

    fn submodule_add_progress(url: &str, path: &str) -> SubmoduleAddProgressState {
        SubmoduleAddProgressState {
            url: url.to_string(),
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn apply_clone_progress_sync_tracks_running_progress_and_deduplicates_same_seq() {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let op = clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Running,
            CloneProgressStage::Loading,
            32,
            7,
        );
        let mut progress = None;
        let mut last_seq = 0;
        let mut tracked_dest = None;

        let first =
            apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, Some(&op));
        assert_eq!(
            first,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: None,
            }
        );
        assert_eq!(progress.as_ref(), Some(&op));
        assert_eq!(last_seq, 7);
        assert_eq!(tracked_dest.as_ref(), Some(&dest));

        let second =
            apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, Some(&op));
        assert_eq!(
            second,
            CloneProgressSyncAction {
                progress_changed: false,
                notice: None,
            }
        );
        assert_eq!(progress.as_ref(), Some(&op));
        assert_eq!(last_seq, 7);
    }

    #[test]
    fn apply_clone_progress_sync_resets_for_restarted_clone_even_at_same_path() {
        let first_dest = Arc::new(PathBuf::from("/tmp/repo"));
        let second_dest = Arc::new(PathBuf::from("/tmp/repo"));
        let first = clone_op(
            Arc::clone(&first_dest),
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            91,
            9,
        );
        let restarted = clone_op(
            Arc::clone(&second_dest),
            CloneOpStatus::Running,
            CloneProgressStage::Loading,
            4,
            1,
        );
        let mut progress = Some(first.clone());
        let mut last_seq = first.seq;
        let mut tracked_dest = Some(first_dest);

        let action = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&restarted),
        );

        assert_eq!(
            action,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: None,
            }
        );
        assert_eq!(progress.as_ref(), Some(&restarted));
        assert_eq!(last_seq, 1);
        assert_eq!(tracked_dest.as_ref(), Some(&second_dest));
    }

    #[test]
    fn apply_clone_progress_sync_emits_success_notice_once_and_clears_progress() {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let finished = clone_op(
            Arc::clone(&dest),
            CloneOpStatus::FinishedOk,
            CloneProgressStage::RemoteObjects,
            100,
            8,
        );
        let mut progress = Some(clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            84,
            7,
        ));
        let mut last_seq = 7;
        let mut tracked_dest = Some(dest);

        let first = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );
        assert_eq!(
            first,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: Some((
                    components::ToastKind::Success,
                    "Clone finished: /tmp/repo".to_string(),
                )),
            }
        );
        assert!(progress.is_none());
        assert_eq!(last_seq, 8);
        assert!(tracked_dest.is_none());

        let second = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );
        assert_eq!(
            second,
            CloneProgressSyncAction {
                progress_changed: false,
                notice: None,
            }
        );
    }

    #[test]
    fn apply_clone_progress_sync_emits_cancel_notice_and_clears_progress() {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let finished = clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Cancelled,
            CloneProgressStage::Loading,
            12,
            5,
        );
        let mut progress = Some(clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Cancelling,
            CloneProgressStage::Loading,
            12,
            4,
        ));
        let mut last_seq = 4;
        let mut tracked_dest = Some(dest);

        let action = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );

        assert_eq!(
            action,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: Some((
                    components::ToastKind::Warning,
                    "Clone aborted: /tmp/repo".to_string(),
                )),
            }
        );
        assert!(progress.is_none());
        assert_eq!(last_seq, 5);
        assert!(tracked_dest.is_none());
    }

    #[test]
    fn apply_clone_progress_sync_emits_error_notice_and_clears_progress() {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let finished = clone_op(
            Arc::clone(&dest),
            CloneOpStatus::FinishedErr("Clone failed: permission denied".to_string()),
            CloneProgressStage::RemoteObjects,
            57,
            11,
        );
        let mut progress = Some(clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Running,
            CloneProgressStage::RemoteObjects,
            57,
            10,
        ));
        let mut last_seq = 10;
        let mut tracked_dest = Some(dest);

        let action = apply_clone_progress_sync(
            &mut progress,
            &mut last_seq,
            &mut tracked_dest,
            Some(&finished),
        );

        assert_eq!(
            action,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: Some((
                    components::ToastKind::Error,
                    "Clone failed: permission denied".to_string(),
                )),
            }
        );
        assert!(progress.is_none());
        assert_eq!(last_seq, 11);
        assert!(tracked_dest.is_none());
    }

    #[test]
    fn apply_clone_progress_sync_clears_progress_when_clone_disappears() {
        let dest = Arc::new(PathBuf::from("/tmp/repo"));
        let mut progress = Some(clone_op(
            Arc::clone(&dest),
            CloneOpStatus::Running,
            CloneProgressStage::Loading,
            43,
            3,
        ));
        let mut last_seq = 3;
        let mut tracked_dest = Some(dest);

        let action =
            apply_clone_progress_sync(&mut progress, &mut last_seq, &mut tracked_dest, None);

        assert_eq!(
            action,
            CloneProgressSyncAction {
                progress_changed: true,
                notice: None,
            }
        );
        assert!(progress.is_none());
        assert_eq!(last_seq, 0);
        assert!(tracked_dest.is_none());
    }

    #[test]
    fn apply_submodule_add_progress_sync_deduplicates_equal_snapshots() {
        let mut progress = vec![submodule_add_progress(
            "https://example.com/sub.git",
            "mods/sub",
        )];

        assert!(!apply_submodule_add_progress_sync(
            &mut progress,
            &[submodule_add_progress(
                "https://example.com/sub.git",
                "mods/sub",
            )],
        ));
        assert_eq!(
            progress,
            vec![submodule_add_progress(
                "https://example.com/sub.git",
                "mods/sub"
            )]
        );
    }

    #[test]
    fn apply_submodule_add_progress_sync_replaces_and_clears_entries() {
        let mut progress = vec![submodule_add_progress(
            "https://example.com/one.git",
            "mods/one",
        )];

        assert!(apply_submodule_add_progress_sync(
            &mut progress,
            &[
                submodule_add_progress("https://example.com/two.git", "mods/two"),
                submodule_add_progress("https://example.com/three.git", "mods/three"),
            ],
        ));
        assert_eq!(
            progress,
            vec![
                submodule_add_progress("https://example.com/two.git", "mods/two"),
                submodule_add_progress("https://example.com/three.git", "mods/three"),
            ]
        );

        assert!(apply_submodule_add_progress_sync(&mut progress, &[]));
        assert!(progress.is_empty());
    }

    #[test]
    fn clone_progress_shell_uses_subtle_accent_border_and_strip() {
        let dark = AppTheme::gitcomet_dark();
        let light = AppTheme::gitcomet_light();

        assert_eq!(
            clone_progress_shell_border_color(dark),
            with_alpha(dark.colors.accent, 0.36)
        );
        assert_eq!(
            clone_progress_shell_accent_color(dark),
            with_alpha(dark.colors.accent, 0.20)
        );
        assert_eq!(
            clone_progress_shell_border_color(light),
            with_alpha(light.colors.accent, 0.28)
        );
        assert_eq!(
            clone_progress_shell_accent_color(light),
            with_alpha(light.colors.accent, 0.14)
        );
    }

    #[test]
    fn toast_stack_anchor_is_bottom_right() {
        assert_eq!(toast_viewport_corner(), ToastViewportCorner::BottomRight);
    }
}
