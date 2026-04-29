use super::element::TextElement;
use super::state::*;
use super::*;

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.style;
        let focus = self.focus_handle.clone();
        let entity_id = cx.entity().entity_id();
        let chromeless = self.chromeless;
        let multiline = self.multiline;
        let pad_x = if chromeless { px(0.0) } else { px(8.0) };
        let pad_y = if chromeless || !multiline {
            px(0.0)
        } else {
            px(8.0)
        };
        let is_focused = focus.is_focused(window);

        if self.interaction.has_focus != is_focused {
            self.interaction.has_focus = is_focused;
            self.interaction.cursor_blink_visible = true;
            if !is_focused {
                self.interaction.cursor_blink_task.take();
                self.interaction.context_menu = None;
            }
        }

        if is_focused
            && self.interaction.cursor_blink_task.is_none()
            && crate::ui_runtime::current().uses_cursor_blink()
        {
            let task = cx.spawn(
                async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| {
                    loop {
                        smol::Timer::after(Duration::from_millis(800)).await;
                        let should_continue = input
                            .update(cx, |input, cx| {
                                if !input.interaction.has_focus {
                                    input.interaction.cursor_blink_visible = true;
                                    input.interaction.cursor_blink_task = None;
                                    cx.notify();
                                    return false;
                                }

                                if input.selection.range.is_empty() {
                                    input.interaction.cursor_blink_visible =
                                        !input.interaction.cursor_blink_visible;
                                } else {
                                    input.interaction.cursor_blink_visible = true;
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);

                        if !should_continue {
                            break;
                        }
                    }
                },
            );
            self.interaction.cursor_blink_task = Some(task);
        }

        let text_surface = div()
            .w_full()
            .min_w(px(0.0))
            .px(pad_x)
            .py(pad_y)
            .overflow_hidden()
            .child(TextElement { input: cx.entity() });

        let mut input = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .track_focus(&focus)
            .key_context("TextInput")
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::shift_enter))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::page_up))
            .on_action(cx.listener(Self::select_page_up))
            .on_action(cx.listener(Self::page_down))
            .on_action(cx.listener(Self::select_page_down))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::show_character_palette))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down_right))
            .line_height(self.effective_line_height(window))
            .text_size(crate::ui_scale::design_px_from_window(13.0, window))
            .when(!multiline && !chromeless, |d| {
                d.h(crate::ui_scale::design_px_from_window(
                    CONTROL_HEIGHT_PX,
                    window,
                ))
            })
            .when(!multiline, |d| d.items_center())
            .when(multiline, |d| d.items_start())
            .child(text_surface);

        if !chromeless {
            input = input
                .bg(style.background)
                .border_1()
                .rounded(px(style.radius));

            if is_focused {
                input = input.border_color(style.focus_border);
            } else {
                input = input
                    .border_color(style.border)
                    .hover(move |s| s.border_color(style.hover_border));
            }

            input = input.focus(move |s| s.border_color(style.focus_border));
        }

        let render_id = ElementId::from(("text_input_root", entity_id));
        let render_id =
            ElementId::from((render_id, if is_focused { "focused" } else { "blurred" }));
        let mut outer = div()
            // Focus changes toggle GPUI platform input handler registration during paint.
            // Key the subtree by focus state so GPUI doesn't reuse a stale unfocused paint
            // range that contains no input handlers when the field becomes focused.
            .id(render_id)
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(input);

        if let Some(state) = self.interaction.context_menu {
            outer = outer.child(
                deferred(
                    anchored()
                        .position(state.anchor)
                        .offset(point(px(4.0), px(4.0)))
                        .child(self.render_context_menu(state, cx)),
                )
                .priority(10_000),
            );
        }

        outer
    }
}
