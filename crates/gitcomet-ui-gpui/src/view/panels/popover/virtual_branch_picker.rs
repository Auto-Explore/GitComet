use super::*;

/// What picking a branch should do: assign a worktree path to it (file
/// context menu) or move a hunk's patch into it (diff hunk context menu).
#[derive(Clone)]
enum BranchPick {
    Assign { path: std::path::PathBuf },
    Move { patch: String, path: std::path::PathBuf },
}

fn branch_list(
    this: &mut PopoverHost,
    repo_id: RepoId,
    scaled_px: impl Fn(f32) -> gpui::Pixels + Copy,
    pick: BranchPick,
    cx: &mut gpui::Context<PopoverHost>,
) -> AnyElement {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let branches = repo
        .map(|r| &r.virtual_branches)
        .cloned()
        .unwrap_or_default();

    if branches.is_empty() {
        let ui_scale_percent = super::popover_ui_scale(cx).percent();
        return components::context_menu_label(
            theme,
            ui_scale_percent,
            "No virtual branches. Open the Virtual Branches panel to create one.",
            Some(this.tooltip_host.clone()),
            cx,
        )
        .into_any_element();
    }

    let mut list = div().flex().flex_col().w_full();
    for branch in branches.iter() {
        let branch_id = branch.id;
        let name = branch.name.to_string();
        let count = branch.paths.len();
        let pick_for_row = pick.clone();
        let row = div()
            .id(("vb_picker_row", branch_id))
            .debug_selector(move || format!("vb_picker_row_{branch_id}"))
            .h(scaled_px(28.0))
            .w_full()
            .flex()
            .items_center()
            .gap(scaled_px(8.0))
            .px(scaled_px(8.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .hover(move |s| s.bg(theme.hover_overlay()))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_sm()
                    .whitespace_nowrap()
                    .line_clamp(1)
                    .text_color(theme.colors.text)
                    .child(name),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.colors.text_muted)
                    .child(format!("{count} file(s)")),
            )
            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                match &pick_for_row {
                    BranchPick::Assign { path } => {
                        this.store.dispatch(Msg::AssignPathToVirtualBranch {
                            repo_id,
                            branch_id,
                            path: path.clone(),
                        });
                    }
                    BranchPick::Move { patch, path } => {
                        this.store.dispatch(Msg::MoveHunkToVirtualBranch {
                            repo_id,
                            branch_id,
                            patch: patch.clone(),
                            path: path.clone(),
                        });
                    }
                }
                this.close_popover(cx);
            }));
        list = list.child(row);
    }
    list.into_any_element()
}

/// Counts `+`/`-` content lines in a unified patch (excluding the `---`/`+++`
/// file headers and `@@` hunk headers) so the picker can preview the hunk.
fn hunk_change_stats(patch: &str) -> (usize, usize) {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix('+') {
            if !rest.starts_with("++") {
                additions += 1;
            }
        } else if let Some(rest) = line.strip_prefix('-') {
            if !rest.starts_with("--") {
                deletions += 1;
            }
        }
    }
    (additions, deletions)
}

fn picker_shell(
    this: &mut PopoverHost,
    header: gpui::Div,
    body: AnyElement,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let width = super::LARGE_PICKER_WIDTH;
    components::context_menu(
        theme,
        div()
            .flex()
            .flex_col()
            .w(width.preferred_px(ui_scale))
            .child(header)
            .child(div().border_t_1().border_color(theme.colors.border))
            .child(body),
    )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale(cx).percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Assign to virtual branch"),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .line_height(scaled_px(14.0))
                .child(
                    components::TruncatedText::path(path.display().to_string())
                        .id(("vb_picker_path", repo_id.0))
                        .text_color(theme.colors.text_muted)
                        .full_text_tooltip(this.tooltip_host.clone())
                        .render(cx),
                ),
        );

    let body = branch_list(this, repo_id, scaled_px, BranchPick::Assign { path }, cx);
    picker_shell(this, header, body, cx)
}

/// Picker shown from the hunk context menu: previews the hunk that will be
/// moved and lists the virtual branches to move it into.
pub(super) fn move_panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    patch: String,
    path: std::path::PathBuf,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale(cx).percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let (additions, deletions) = hunk_change_stats(&patch);

    let header = div()
        .px(scaled_px(8.0))
        .py(scaled_px(4.0))
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child("Move hunk to virtual branch"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(scaled_px(6.0))
                .min_w(px(0.0))
                .child(
                    components::TruncatedText::path(path.display().to_string())
                        .id(("vb_move_picker_path", repo_id.0))
                        .text_color(theme.colors.text_muted)
                        .full_text_tooltip(this.tooltip_host.clone())
                        .render(cx),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.success)
                        .child(format!("+{additions}")),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(theme.colors.danger)
                        .child(format!("-{deletions}")),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .line_height(scaled_px(14.0))
                .child("The hunk leaves the worktree and is parked in the branch until you apply it."),
        );

    let body = branch_list(this, repo_id, scaled_px, BranchPick::Move { patch, path }, cx);
    picker_shell(this, header, body, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_stats_counts_content_lines_only() {
        let patch = "\
diff --git a/src/lib.rs b/src/lib.rs
index 123..456 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,6 @@
 fn main() {
-    let old = 1;
+    let new = 1;
+    let extra = 2;
 }
";
        assert_eq!(hunk_change_stats(patch), (2, 1));
    }

    #[test]
    fn change_stats_ignores_binary_and_empty_patches() {
        assert_eq!(hunk_change_stats(""), (0, 0));
        assert_eq!(hunk_change_stats("+++ b/x\n--- a/x\n@@ -0,0 +1 @@\n"), (0, 0));
    }
}
