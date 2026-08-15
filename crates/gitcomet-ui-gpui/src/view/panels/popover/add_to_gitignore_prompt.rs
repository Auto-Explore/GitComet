use super::*;

use gitcomet_core::gitignore::GitignoreScope;

/// How many paths to spell out before collapsing the rest into a count.
const MAX_LISTED_PATHS: usize = 8;

fn scope_label(scope: GitignoreScope) -> &'static str {
    match scope {
        GitignoreScope::File => "File",
        GitignoreScope::Folder => "Folder",
        GitignoreScope::Extension => "Extension",
    }
}

fn scope_button_id(scope: GitignoreScope) -> &'static str {
    match scope {
        GitignoreScope::File => "add_to_gitignore_scope_file",
        GitignoreScope::Folder => "add_to_gitignore_scope_folder",
        GitignoreScope::Extension => "add_to_gitignore_scope_extension",
    }
}

/// The File | Folder | Extension pill.
///
/// Built inline rather than from a shared component because none exists — this
/// mirrors the conflict resolver's Text/Preview toggle.
fn scope_picker(
    theme: AppTheme,
    scopes: &[GitignoreScope],
    selected: GitignoreScope,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let scaled_px = super::popover_scaled_px_fn(cx);
    let mut pill = div()
        .id("add_to_gitignore_scope")
        .flex()
        .items_center()
        .rounded(px(theme.radii.row))
        .border_1()
        .border_color(theme.colors.stroke.default)
        .overflow_hidden()
        .p(px(1.0));

    for (ix, scope) in scopes.iter().copied().enumerate() {
        if ix > 0 {
            pill = pill.child(div().h_full().w(px(1.0)).bg(theme.colors.stroke.default));
        }
        pill = pill.child(
            components::Button::new(scope_button_id(scope), scope_label(scope))
                .borderless()
                .style(components::ButtonStyle::Subtle)
                .selected(scope == selected)
                .selected_bg(theme.colors.interaction.pressed_background)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.set_add_to_gitignore_scope(scope, cx);
                }),
        );
    }

    div()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child("Ignore"),
        )
        .child(pill)
        .min_h(scaled_px(24.0))
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    area: DiffArea,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scaled_px = super::popover_scaled_px_fn(cx);
    let patterns_scroll = this.gitignore_patterns_scroll.clone();

    let paths = this.gitignore_paths.clone();
    let scopes = this
        .gitignore_suggestions
        .as_ref()
        .map(|s| s.applicable_scopes())
        .unwrap_or_default();
    let selected_scope = this.gitignore_scope;
    let can_submit = this.can_submit_add_to_gitignore(cx);

    // The editor writes whole buffers, so an unsaved `.gitignore` would be
    // written back over this append the moment the user saves it or the 800 ms
    // autosave fires. Warn rather than block: blocking would strand the user in
    // a dialog with no way forward.
    let editor_holds_gitignore = this.main_pane.read(cx).file_edits_are_unsaved_for(
        repo_id,
        std::path::Path::new(gitcomet_core::gitignore::FILE_NAME),
    );

    let heading = match paths.len() {
        0 | 1 => "Ignore this file:".to_string(),
        n => format!("Ignore these {n} files:"),
    };

    let mut dialog = ConfirmDialog::new("Add to .gitignore", DIALOG_440_WIDTH).text(theme, heading);

    // Naming what is about to be ignored is the point of the dialog: the status
    // row that triggered it is behind the popover by the time it opens.
    let listed = paths.iter().take(MAX_LISTED_PATHS);
    for listed_path in listed {
        dialog = dialog.mono_value(theme, listed_path.display().to_string());
    }
    if paths.len() > MAX_LISTED_PATHS {
        dialog = dialog.note(
            theme,
            format!("…and {} more", paths.len() - MAX_LISTED_PATHS),
        );
    }

    dialog = dialog.divider(theme);

    // A single applicable scope means there is nothing to choose between, and a
    // one-segment pill reads as a broken control.
    if scopes.len() > 1 {
        dialog = dialog.section(scope_picker(theme, &scopes, selected_scope, cx));
    }

    dialog = dialog
        .section(input_label(theme, "Pattern"))
        .section(
            div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                components::ScrollContainer::vertical(
                    "add_to_gitignore_pattern_scroll_surface",
                    "add_to_gitignore_pattern_scrollbar",
                    patterns_scroll,
                    scaled_px(120.0),
                )
                .render(theme, this.gitignore_patterns_input.clone()),
            ),
        )
        .note(
            theme,
            format!(
                "One pattern per line, appended to {} in the repository root.",
                gitcomet_core::gitignore::FILE_NAME
            ),
        );

    if editor_holds_gitignore {
        dialog = dialog.section(
            div()
                .px_2()
                .pb_1()
                .text_xs()
                .text_color(theme.colors.status.danger.foreground)
                .child(format!(
                    "{} has unsaved edits open in the editor. Saving them will overwrite this change.",
                    gitcomet_core::gitignore::FILE_NAME
                )),
        );
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "add_to_gitignore_cancel",
            "add_to_gitignore_cancel_hint",
            theme,
            cx,
        ),
        components::Button::new("add_to_gitignore_go", "Add")
            .style(components::ButtonStyle::Filled)
            .disabled(!can_submit)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.submit_add_to_gitignore(repo_id, area, path.clone(), cx);
            }),
        cx,
    )
}
