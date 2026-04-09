use super::*;

fn prompt_title(kind: LfsPatternPromptKind) -> &'static str {
    match kind {
        LfsPatternPromptKind::Track => "LFS track",
        LfsPatternPromptKind::Untrack => "LFS untrack",
        LfsPatternPromptKind::MigrateImport => "LFS migrate import",
    }
}

fn prompt_help(kind: LfsPatternPromptKind) -> &'static str {
    match kind {
        LfsPatternPromptKind::Track => "Pattern to pass to `git lfs track`.",
        LfsPatternPromptKind::Untrack => "Pattern to pass to `git lfs untrack`.",
        LfsPatternPromptKind::MigrateImport => {
            "Pattern to pass as `git lfs migrate import --include`."
        }
    }
}

fn submit_label(kind: LfsPatternPromptKind) -> &'static str {
    match kind {
        LfsPatternPromptKind::Track => "Track",
        LfsPatternPromptKind::Untrack => "Untrack",
        LfsPatternPromptKind::MigrateImport => "Migrate",
    }
}

fn hotkey_hint(theme: AppTheme, debug_selector: &'static str, label: &'static str) -> gpui::Div {
    div()
        .debug_selector(move || debug_selector.to_string())
        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child(label)
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    kind: LfsPatternPromptKind,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_submit = this.can_submit_lfs_pattern_prompt(cx);

    div()
        .flex()
        .flex_col()
        .w(px(460.0))
        .child(
            div()
                .px_2()
                .py_1()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(prompt_title(kind)),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(prompt_help(kind)),
        )
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.lfs_pattern_input.clone()),
        )
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    components::Button::new("lfs_pattern_cancel", "Cancel")
                        .separated_end_slot(hotkey_hint(theme, "lfs_pattern_cancel_hint", "Esc"))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.close_popover(cx);
                        }),
                )
                .child(
                    components::Button::new("lfs_pattern_go", submit_label(kind))
                        .separated_end_slot(hotkey_hint(theme, "lfs_pattern_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_submit)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_lfs_pattern_prompt(cx);
                        }),
                ),
        )
}
