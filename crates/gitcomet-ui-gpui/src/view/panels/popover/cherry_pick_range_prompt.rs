use super::*;
use gpui::StatefulInteractiveElement;

/// One picker row for the Cherry-pick prompt: a chromeless search input that
/// expands into a branch/ref picker while focused, exactly like the create
/// branch prompt's source row (see `create_branch_from_ref_prompt`). Rows
/// share the picker's own row-part matcher and `rows_cache`-backed layout
/// rather than a self-contained picker component — that component was
/// retired along with the plain-string `match_branches` matcher it used.
fn picker_row(
    this: &mut PopoverHost,
    theme: AppTheme,
    label: &'static str,
    input: &Entity<components::TextInput>,
    on_select: impl Fn(
        &mut PopoverHost,
        String,
        &ClickEvent,
        &mut Window,
        &mut gpui::Context<PopoverHost>,
    ) + 'static,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let is_focused = input
        .read_with(cx, |input, _| input.focus_handle())
        .is_focused(window);
    input.update(cx, |input, cx| {
        input.set_chromeless(is_focused, cx);
        // Keep the branch icon visible even when the row isn't focused, so the
        // three rows read as "pick a ref" fields at a glance instead of
        // looking like plain, unlabeled text boxes.
        input.set_leading_icon(Some("icons/git_branch.svg"), cx);
    });

    if is_focused {
        let query = input.read(cx).text().trim().to_string();
        let built =
            branch_picker::ref_rows_cached(this, branch_picker::RefRowsSpec::source_ref(), &query);
        let names = std::rc::Rc::clone(&built.payloads);
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child(label),
            )
            .child(
                div().px_2().pb_1().w_full().min_w(px(0.0)).child(
                    branch_picker::ref_picker_prompt(
                        input.clone(),
                        this.picker_prompt_scroll.clone(),
                        &built,
                        cx,
                    )
                    .tooltip_host(this.tooltip_host.clone())
                    .empty_text("No matching branches or tags")
                    .max_height(scaled_px(240.0))
                    .selected_index(this.branch_picker_selected_index)
                    .select_on_mouse_down()
                    .render(
                        theme,
                        ui_scale_percent,
                        cx,
                        move |this, ix, e, window, cx| {
                            let Some(name) = names.get(ix).cloned() else {
                                return;
                            };
                            on_select(this, name, e, window, cx);
                        },
                    ),
                ),
            )
    } else {
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_sm()
                    .text_color(theme.colors.foreground.secondary)
                    .child(label),
            )
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .w_full()
                    .min_w(px(0.0))
                    .child(input.clone()),
            )
    }
}

/// The `range..source` commit preview: count + subject list once the pair is
/// chosen, a loading row while it loads, or the backend's error (e.g. range
/// not an ancestor of source).
fn preview_section(
    this: &mut PopoverHost,
    theme: AppTheme,
    repo_id: RepoId,
    scaled_px: impl Fn(f32) -> gpui::Pixels + Copy,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let source = this.cherry_pick_source_target.trim().to_string();
    let range = this.cherry_pick_range_target.trim().to_string();
    let incomplete = source.is_empty() || range.is_empty() || source == range;

    let preview = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.cherry_pick_range_preview.as_ref())
        .filter(|p| p.range == range && p.source == source);

    let body = if incomplete {
        div()
            .px_2()
            .py_1()
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .child("Pick a source and a range to preview the commits to cherry-pick.")
    } else {
        match preview.map(|p| &p.commits) {
            Some(gitcomet_state::model::Loadable::Ready(commits)) if !commits.is_empty() => {
                // The wider dialog (see `panel`) has room for more rows than
                // a small anchored popover would.
                let shown = commits.iter().take(12).collect::<Vec<_>>();
                let more = commits.len().saturating_sub(shown.len());
                let mut rows = div().flex().flex_col().gap(scaled_px(2.0)).py_1();
                for commit in &shown {
                    let short = commit.id.as_ref().get(..7).unwrap_or(commit.id.as_ref());
                    let commit_id = commit.id.clone();
                    let row_debug_id =
                        format!("cherry_pick_range_preview_row_{}", commit_id.as_ref());
                    rows = rows.child(
                        div()
                            .id(SharedString::from(row_debug_id.clone()))
                            .debug_selector(move || row_debug_id.clone())
                            .flex()
                            .items_center()
                            .gap(scaled_px(6.0))
                            .px_2()
                            .rounded_md()
                            .hover(move |style| style.bg(theme.hover_overlay()))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _e: &gpui::ClickEvent, _w, cx| {
                                // Open the commit in the history/details
                                // panes, like a reflog entry click.
                                this.store.dispatch(Msg::SelectCommit {
                                    repo_id,
                                    commit_id: commit_id.clone(),
                                });
                                this.close_popover(cx);
                            }))
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .font_family("ui-monospace")
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(short.to_string()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_xs()
                                    .whitespace_nowrap()
                                    .line_clamp(1)
                                    .text_color(theme.colors.foreground.primary)
                                    .child(commit.summary.to_string()),
                            ),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .child(if more > 0 {
                                format!(
                                    "{} commits will be cherry-picked (showing first {} — click one to open it)",
                                    commits.len(),
                                    shown.len()
                                )
                            } else {
                                format!(
                                    "{} commits will be cherry-picked — click one to open it",
                                    commits.len()
                                )
                            }),
                    )
                    .child(
                        div()
                            .id("cherry_pick_range_preview_scroll")
                            .max_h(scaled_px(220.0))
                            .overflow_y_scroll()
                            .child(rows),
                    )
            }
            Some(gitcomet_state::model::Loadable::Ready(_)) => div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child("No commits to cherry-pick in this range."),
            Some(gitcomet_state::model::Loadable::Error(error)) => div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.status.warning.foreground)
                .child(error.clone()),
            Some(gitcomet_state::model::Loadable::NotLoaded)
            | Some(gitcomet_state::model::Loadable::Loading)
            | None => div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child("Loading preview…"),
        }
    };

    div()
        .flex()
        .flex_col()
        .w_full()
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(body)
}

/// Plain-language readout of what submitting the form will do, plus a small
/// text diagram — the maintainer asked for a way to "visualize what is going
/// into the new branch" beyond the raw commit list in [`preview_section`].
fn summary_section(
    this: &PopoverHost,
    theme: AppTheme,
    repo_id: RepoId,
    scaled_px: impl Fn(f32) -> gpui::Pixels + Copy,
    cx: &gpui::Context<PopoverHost>,
) -> gpui::Div {
    let source = this.cherry_pick_source_target.trim().to_string();
    let range = this.cherry_pick_range_target.trim().to_string();
    let base = this.cherry_pick_base_target.trim().to_string();
    let name = this
        .cherry_pick_name_input
        .read(cx)
        .text()
        .trim()
        .to_string();

    if source.is_empty()
        || range.is_empty()
        || base.is_empty()
        || name.is_empty()
        || source == range
    {
        return div()
            .mx_2()
            .my_1()
            .px_2()
            .py_2()
            .rounded(px(theme.radii.panel))
            .border_1()
            .border_color(theme.colors.stroke.subtle)
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .child("Fill in the fields above to preview the branch that will be created.");
    }

    let commit_count = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.cherry_pick_range_preview.as_ref())
        .filter(|p| p.range == range && p.source == source)
        .and_then(|p| match &p.commits {
            gitcomet_state::model::Loadable::Ready(commits) => Some(commits.len()),
            _ => None,
        });

    let count_text = match commit_count {
        Some(0) => "no commits".to_string(),
        Some(1) => "1 commit".to_string(),
        Some(n) => format!("{n} commits"),
        None => "some commits".to_string(),
    };

    let summary = format!(
        "New branch \"{name}\" will be created from {base}, then {count_text} from {source} \
         (everything not already in {range}) will be copied onto it. \
         {base} and {source} are left untouched."
    );
    let diagram = format!("{base}  ──▶  {name} (new)  ◀──  {count_text} from {range}..{source}");

    div()
        .flex()
        .flex_col()
        .gap(scaled_px(4.0))
        .mx_2()
        .my_1()
        .px_2()
        .py_2()
        .rounded(px(theme.radii.panel))
        .bg(theme.colors.surface.raised)
        .border_1()
        .border_color(theme.colors.stroke.subtle)
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.foreground.primary)
                .child("What will be created"),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(summary),
        )
        .child(
            div()
                .text_xs()
                .font_family("ui-monospace")
                .text_color(theme.colors.foreground.secondary)
                .child(diagram),
        )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    window: &Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let scaled_px = super::popover_scaled_px_fn(cx);
    let can_submit = this.cherry_pick_can_submit(cx);
    let source = this.cherry_pick_source_target.trim().to_string();
    let range = this.cherry_pick_range_target.trim().to_string();
    let same_ref_hint = !source.is_empty() && source == range;

    let source_input = this
        .cherry_pick_source_search_input
        .clone()
        .expect("cherry_pick_source_search_input must be initialized");
    let range_input = this
        .cherry_pick_range_search_input
        .clone()
        .expect("cherry_pick_range_search_input must be initialized");
    let base_input = this
        .cherry_pick_base_search_input
        .clone()
        .expect("cherry_pick_base_search_input must be initialized");

    div()
        .flex()
        .flex_col()
        .w(scaled_px(860.0))
        .child(popover_title("Branch extractor"))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(
            div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(theme.colors.foreground.secondary)
                .child(
                    "Copies a range of commits onto a brand-new branch, without touching the \
                     branches they came from. Pick the branch or tag to copy commits from, then \
                     the earlier point they grew from (it needs to already contain that starting \
                     point in its history) — every commit after that, oldest first and skipping \
                     merges, gets copied. Finally, pick which branch the new one should start from.",
                ),
        )
        .child(div().border_t_1().border_color(theme.colors.stroke.subtle))
        .child(picker_row(
            this,
            theme,
            "Copy commits from (A)",
            &source_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_source_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(picker_row(
            this,
            theme,
            "Excluding commits already in (B)",
            &range_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_range_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(picker_row(
            this,
            theme,
            "New branch starts from (D)",
            &base_input,
            |this, name, _e, window, cx| {
                this.handle_cherry_pick_base_select(name, window, cx);
            },
            window,
            cx,
        ))
        .child(preview_section(this, theme, _repo_id, scaled_px, cx))
        .child(input_label(theme, "New branch name (C)"))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.cherry_pick_name_input.clone()),
        )
        .when(same_ref_hint, |this| {
            this.child(
                div()
                    .px_2()
                    .pb_1()
                    .text_sm()
                    .text_color(theme.colors.status.warning.foreground)
                    .child("Source and range are the same — there is nothing to cherry-pick."),
            )
        })
        .child(summary_section(this, theme, _repo_id, scaled_px, cx))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    cancel_button(
                        "cherry_pick_range_cancel",
                        "cherry_pick_range_cancel_hint",
                        theme,
                    )
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("cherry_pick_range_go", "Create & cherry-pick")
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_submit)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_cherry_pick_range(window, cx);
                        }),
                ),
        )
}
