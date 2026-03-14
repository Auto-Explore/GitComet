use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    rev: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let title: SharedString = path.display().to_string().into();
    let subtitle: SharedString = rev
        .clone()
        .map(|r| format!("rev: {r}").into())
        .unwrap_or_else(|| "rev: HEAD".into());

    let header = div()
        .px_2()
        .py_1()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.0))
                .child(div().text_sm().font_weight(FontWeight::BOLD).child("Blame"))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .child(subtitle),
                ),
        )
        .child(
            components::Button::new("blame_close", "Close")
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |this, _e, _w, cx| {
                    this.popover = None;
                    this.popover_anchor = None;
                    cx.notify();
                }),
        );

    let body: AnyElement = match repo.map(|r| &r.history_state.blame) {
        None => components::context_menu_label(theme, "No repository").into_any_element(),
        Some(Loadable::Loading) => {
            components::context_menu_label(theme, "Loading").into_any_element()
        }
        Some(Loadable::Error(e)) => {
            components::context_menu_label(theme, e.clone()).into_any_element()
        }
        Some(Loadable::NotLoaded) => {
            components::context_menu_label(theme, "Not loaded").into_any_element()
        }
        Some(Loadable::Ready(lines)) => {
            let count = lines.len();
            let list = uniform_list(
                "blame_popover",
                count,
                cx.processor(render_blame_popover_rows),
            )
            .h(px(360.0))
            .track_scroll(this.blame_scroll.clone());

            div()
                .relative()
                .child(list)
                .child(
                    components::Scrollbar::new(
                        "blame_popover_scrollbar",
                        this.blame_scroll.clone(),
                    )
                        .render(theme),
                )
                .into_any_element()
        }
    };

    div()
        .flex()
        .flex_col()
        .min_w(px(720.0))
        .max_w(px(980.0))
        .child(header)
        .child(div().border_t_1().border_color(theme.colors.border))
        .child(body)
}

fn render_blame_popover_rows(
    this: &mut PopoverHost,
    range: std::ops::Range<usize>,
    _window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) -> Vec<AnyElement> {
    let Some((repo_id, path)) = this.popover.as_ref().and_then(|k| match k {
        PopoverKind::Blame { repo_id, path, .. } => Some((*repo_id, path.clone())),
        _ => None,
    }) else {
        return Vec::new();
    };

    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Loadable::Ready(lines) = &repo.history_state.blame else {
        return Vec::new();
    };

    let theme = this.theme;
    let mut rows = Vec::with_capacity(range.len());
    for ix in range {
        let Some(line) = lines.get(ix) else {
            continue;
        };
        let line_no = ix + 1;
        let sha = line.commit_id.clone();
        let short = sha.get(0..8).unwrap_or(&sha).to_string();
        let author: SharedString = line.author.clone().into();
        let code: SharedString = line.line.clone().into();
        let commit_id = CommitId(sha);
        let path = path.clone();

        rows.push(
            div()
                .id(("blame_row", ix))
                .h(px(20.0))
                .flex()
                .items_center()
                .px_2()
                .gap_2()
                .hover(move |s| s.bg(theme.colors.hover))
                .active(move |s| s.bg(theme.colors.active))
                .child(
                    div()
                        .w(px(44.0))
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .whitespace_nowrap()
                        .child(format!("{line_no:>4}")),
                )
                .child(
                    div()
                        .w(px(76.0))
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .whitespace_nowrap()
                        .child(short),
                )
                .child(
                    div()
                        .w(px(140.0))
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .child(author),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_xs()
                        .font_family("monospace")
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(code),
                )
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    this.store.dispatch(Msg::SelectCommit {
                        repo_id,
                        commit_id: commit_id.clone(),
                    });
                    this.store.dispatch(Msg::SelectDiff {
                        repo_id,
                        target: DiffTarget::Commit {
                            commit_id: commit_id.clone(),
                            path: Some(path.clone()),
                        },
                    });
                    this.popover = None;
                    this.popover_anchor = None;
                    cx.notify();
                }))
                .into_any_element(),
        );
    }

    rows
}
