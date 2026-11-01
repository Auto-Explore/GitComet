use super::shaping::{primary_modifier_label, with_alpha};
use super::state::*;
use super::*;
use crate::components::{ContextMenuEntry, ContextMenuText, ContextMenuTone};

impl TextInput {
    pub(super) fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    pub(super) fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if let Some(text) = crate::clipboard::read_text(cx) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    pub(super) fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_with_source(crate::clipboard::CopySource::TextInputShortcut, cx);
    }

    fn copy_with_source(&self, source: crate::clipboard::CopySource, cx: &mut Context<Self>) {
        if !self.selection.range.is_empty() {
            crate::clipboard::write_text(
                cx,
                self.content[self.selection.range.clone()].to_string(),
                source,
            );
        }
    }

    pub(super) fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        self.cut_with_source(crate::clipboard::CopySource::TextInputShortcut, window, cx);
    }

    fn cut_with_source(
        &mut self,
        source: crate::clipboard::CopySource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection.range.is_empty() {
            self.copy_with_source(source, cx);
            if !self.read_only {
                self.replace_text_in_range(None, "", window, cx)
            }
        }
    }

    pub(super) fn on_mouse_down_right(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Before the suppression guard: when an ancestor renders this input's
        // menu (the conflict resolver's output pane) the press is still ours.
        // Adopt rather than preserve -- the menu belongs to this input.
        self.selection_owner.adopt(window, cx);
        self.interaction.took_press = true;
        if self.interaction.suppress_right_click {
            return;
        }

        crate::press_gesture::claim_press(cx);
        cx.stop_propagation();
        window.focus(&self.focus_handle, cx);
        self.show_cursor();
        self.end_mouse_drag();
        self.interaction.vertical_motion_x = None;

        let index = self.index_for_mouse_position(event.position);
        let click_inside_selection = !self.selection.range.is_empty()
            && index >= self.selection.range.start
            && index <= self.selection.range.end;
        if !click_inside_selection {
            self.move_to(index, cx);
        }

        cx.notify();
    }

    pub(super) fn on_right_click(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.interaction.suppress_right_click {
            return;
        }
        self.interaction.context_menu = Some(TextInputContextMenuState {
            can_paste: crate::clipboard::read_text(cx).is_some(),
            anchor: event.position,
        });
        cx.notify();
    }

    pub(super) fn render_context_menu(
        &mut self,
        state: TextInputContextMenuState,
        cx: &mut Context<Self>,
    ) -> Div {
        let menu_ui_scale_percent = crate::ui_scale::current(cx).percent;
        let ui_scale = crate::ui_scale::UiScale::current(cx);
        let mut menu_theme = self.style.menu_theme;
        menu_theme.metrics = self.appearance_metrics;
        let primary = primary_modifier_label();
        let undo_disabled = self.read_only || self.selection.undo_stack.is_empty();
        let redo_disabled = self.read_only || self.selection.redo_stack.is_empty();
        let cut_disabled = self.read_only || self.selection.range.is_empty();
        let copy_disabled = self.selection.range.is_empty();
        let paste_disabled = self.read_only || !state.can_paste;
        let delete_disabled = self.read_only || self.selection.range.is_empty();
        let select_all_disabled = self.content.is_empty();

        let item = |label: &'static str,
                    shortcut: SharedString,
                    disabled: bool,
                    action: fn(&mut Self, &mut Window, &mut Context<Self>),
                    cx: &mut Context<Self>| {
            ContextMenuEntry::new(label, ContextMenuText::new(label).max_lines(1))
                .tone(ContextMenuTone::Neutral)
                .shortcut(Some(shortcut))
                .disabled(disabled)
                .on_select(menu_theme, ui_scale, cx, move |this, _, window, cx| {
                    this.interaction.context_menu = None;
                    action(this, window, cx);
                    cx.notify();
                })
                .w_full()
        };
        let undo_row = item(
            "Undo",
            format!("{primary}+Z").into(),
            undo_disabled,
            |this, window, cx| this.undo(&Undo, window, cx),
            cx,
        );
        let redo_row = item(
            "Redo",
            format!("{primary}+Shift+Z").into(),
            redo_disabled,
            |this, window, cx| this.redo(&Redo, window, cx),
            cx,
        );
        let cut_row = item(
            "Cut",
            format!("{primary}+X").into(),
            cut_disabled,
            |this, window, cx| {
                this.cut_with_source(
                    crate::clipboard::CopySource::TextInputContextMenu,
                    window,
                    cx,
                )
            },
            cx,
        )
        .debug_selector(|| "text_input_context_cut".to_string());
        let copy_row = item(
            "Copy",
            format!("{primary}+C").into(),
            copy_disabled,
            |this, _, cx| {
                this.copy_with_source(crate::clipboard::CopySource::TextInputContextMenu, cx)
            },
            cx,
        )
        .debug_selector(|| "text_input_context_copy".to_string());
        let paste_row = item(
            "Paste",
            format!("{primary}+V").into(),
            paste_disabled,
            |this, window, cx| this.paste(&Paste, window, cx),
            cx,
        )
        .debug_selector(|| "text_input_context_paste".to_string());
        let delete_row = item(
            "Delete",
            "Del".into(),
            delete_disabled,
            |this, window, cx| {
                if !this.selection.range.is_empty() && !this.read_only {
                    this.replace_text_in_range(None, "", window, cx);
                }
            },
            cx,
        );
        let select_all_row = item(
            "Select all",
            format!("{primary}+A").into(),
            select_all_disabled,
            |this, window, cx| this.select_all(&SelectAll, window, cx),
            cx,
        )
        .debug_selector(|| "text_input_context_select_all".to_string());

        div()
            .w(crate::ui_scale::design_px_from_percent(
                188.0,
                menu_ui_scale_percent,
            ))
            .p_1()
            .flex()
            .flex_col()
            .gap_0p5()
            .bg(self.style.menu_theme.colors.surface.raised)
            .border_1()
            .border_color(self.style.menu_theme.colors.stroke.default)
            .rounded(px(self.style.menu_theme.radii.popover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|_this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                }),
            )
            .child(undo_row)
            .child(redo_row)
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(with_alpha(self.style.border, 0.6)),
            )
            .child(cut_row)
            .child(copy_row)
            .child(paste_row)
            .child(delete_row)
            .child(
                div()
                    .h(px(1.0))
                    .w_full()
                    .bg(with_alpha(self.style.border, 0.6)),
            )
            .child(select_all_row)
    }
}
