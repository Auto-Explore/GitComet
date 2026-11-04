//! The commit card at the top of the commit details: author, dates and the
//! short SHA on one row, and the parent with the refs on this commit below.

use super::*;
use crate::theme::with_alpha;
use gitcomet_core::domain::CommitDetails;

const SHORT_SHA_LEN: usize = 8;
const MAX_REF_PILLS: usize = 4;
/// Narrowest the author block gets before the SHA badge wraps below it.
const AUTHOR_BLOCK_MIN_WIDTH_PX: f32 = 180.0;

enum RefPillKind {
    Local,
    Remote,
    Tag,
}

struct RefPill {
    label: String,
    kind: RefPillKind,
    /// The checked-out branch, tinted like the sidebar's HEAD row.
    is_head: bool,
}

fn short_sha(sha: &str) -> &str {
    sha.get(..SHORT_SHA_LEN).unwrap_or(sha)
}

impl DetailsPaneView {
    /// Branches and tags that point at `commit`: local first, then remote, then tags.
    fn commit_ref_pills(&self, commit: &CommitId) -> Vec<RefPill> {
        let Some(repo) = self.active_repo() else {
            return Vec::new();
        };
        let mut pills = Vec::new();
        let head = match &repo.head_branch {
            Loadable::Ready(head) => Some(head.as_str()),
            _ => None,
        };
        if let Loadable::Ready(branches) = &repo.branches {
            pills.extend(
                branches
                    .iter()
                    .filter(|branch| &branch.target == commit)
                    .map(|branch| RefPill {
                        is_head: head == Some(branch.name.as_str()),
                        label: branch.name.clone(),
                        kind: RefPillKind::Local,
                    }),
            );
        }
        if let Loadable::Ready(branches) = &repo.remote_branches {
            pills.extend(
                branches
                    .iter()
                    .filter(|branch| &branch.target == commit)
                    .map(|branch| RefPill {
                        label: format!("{}/{}", branch.remote, branch.name),
                        kind: RefPillKind::Remote,
                        is_head: false,
                    }),
            );
        }
        if let Loadable::Ready(tags) = &repo.tags {
            pills.extend(
                tags.iter()
                    .filter(|tag| &tag.target == commit)
                    .map(|tag| RefPill {
                        label: tag.name.clone(),
                        kind: RefPillKind::Tag,
                        is_head: false,
                    }),
            );
        }
        pills
    }

    /// The bordered frame of a SHA badge. Its parts (`commit_card_chip_part`)
    /// carry the clicks, so the main action and the menu arrow stay separate.
    fn commit_card_chip(&self, wash: bool) -> Div {
        let theme = self.theme;
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(self.ui_scale().px(22.0))
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .bg(if wash {
                with_alpha(theme.colors.accent.foreground, 0.18)
            } else {
                theme.colors.surface.raised
            })
            .overflow_hidden()
    }

    fn commit_card_chip_part(
        &self,
        id: impl Into<gpui::ElementId>,
        selector: String,
        tooltip: SharedString,
    ) -> gpui::Stateful<Div> {
        let theme = self.theme;
        div()
            .id(id)
            .debug_selector(move || selector)
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .h_full()
            .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.primary)
            .cursor_pointer()
            .control_interaction(InteractionStyle::new(theme), InteractionState::default())
            .gitcomet_tooltip(theme, tooltip)
    }

    /// The arrow that opens the commit menu (reveal, browse files, copy).
    fn commit_card_menu_arrow(
        &self,
        id: gpui::ElementId,
        selector: String,
        repo_id: RepoId,
        commit_id: CommitId,
        allow_navigate: bool,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Stateful<Div> {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        self.commit_card_chip_part(id, selector, "Commit actions".into())
            .border_l_1()
            .border_color(theme.colors.stroke.subtle)
            .px_1()
            .child(svg_icon(
                "icons/chevron_down.svg",
                theme.colors.foreground.secondary,
                ui_scale.px(12.0),
            ))
            .on_activate(
                !enabled,
                components::ControlActivation::Action,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    if e.standard_click() {
                        this.open_popover_at(
                            PopoverKind::CommitShaLinkMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                                allow_navigate,
                            },
                            e.position(),
                            window,
                            cx,
                        );
                    }
                }),
            )
    }

    /// A ref drawn like a sidebar branch row: the same icon, label size and
    /// colours, with the HEAD tint, but sized to its content.
    fn commit_card_ref_pill(&self, pill: RefPill) -> Div {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let secondary = theme.colors.foreground.secondary;
        let (icon, label_color) = match pill.kind {
            RefPillKind::Local => ("icons/git_branch.svg", theme.colors.foreground.primary),
            RefPillKind::Remote => ("icons/git_branch.svg", secondary),
            RefPillKind::Tag => ("icons/tag.svg", secondary),
        };
        let icon_color = if pill.is_head {
            theme.colors.accent.foreground
        } else {
            secondary
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(ui_scale.px(6.0))
            .px_2()
            .h(ui_scale.px(24.0))
            .rounded(px(theme.radii.row))
            .when(pill.is_head, |d| {
                d.bg(with_alpha(
                    theme.colors.accent.foreground,
                    if theme.is_dark { 0.18 } else { 0.12 },
                ))
            })
            .text_size(theme.ui_text(14.0))
            .text_color(label_color)
            .whitespace_nowrap()
            .child(svg_icon(icon, icon_color, ui_scale.px(14.0)))
            .child(pill.label)
    }

    pub(super) fn commit_details_card(
        &mut self,
        details: &CommitDetails,
        signature: Option<&gitcomet_core::domain::CommitSignature>,
        name_matches: &[std::ops::Range<usize>],
        sha_matched: bool,
        repo_id: RepoId,
        current: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let full_sha = details.id.as_ref().to_string();
        let date = self.commit_details_date_display(details);
        let relative = (details.authored_at_unix != 0).then(|| {
            crate::view::date_time::format_relative_time(
                details.authored_at_unix,
                std::time::SystemTime::now(),
            )
        });
        let when = match (relative, date.is_empty()) {
            (Some(relative), false) => format!("{relative} · {date}"),
            (Some(relative), true) => relative,
            (None, _) => date,
        };

        let display_name = commit_details_author_display_name(details).to_owned();
        let wash = crate::view::rows::query_highlight_style(theme);
        let mut name_highlights: crate::view::commit_message_text::TextHighlights = name_matches
            .iter()
            .map(|range| (range.clone(), wash))
            .collect();
        crate::text_runs::sanitize_highlights(&display_name, &mut name_highlights);

        let copy_id = details.id.clone();
        let sha_main = self
            .commit_card_chip_part(
                "commit_card_sha",
                "commit_card_sha".to_string(),
                format!("Copy {full_sha}").into(),
            )
            .child(short_sha(&full_sha).to_string())
            .child(svg_icon(
                "icons/copy.svg",
                theme.colors.foreground.secondary,
                ui_scale.px(12.0),
            ))
            .on_activate(
                false,
                components::ControlActivation::Action,
                cx.listener(move |this, e: &ClickEvent, _w, cx| {
                    if e.standard_click() {
                        this.copy_commit_id(copy_id.clone(), cx);
                    }
                }),
            );
        let sha_arrow = self.commit_card_menu_arrow(
            "commit_card_sha_menu".into(),
            "commit_card_sha_menu".to_string(),
            repo_id,
            details.id.clone(),
            false,
            current,
            cx,
        );
        let sha_chip = self
            .commit_card_chip(sha_matched)
            .when(sha_matched, |chip| {
                chip.debug_selector(|| "commit_card_sha_find_match".to_string())
            })
            .child(sha_main)
            .child(sha_arrow);

        // Wraps instead of squeezing: when the pane is narrow the SHA badge
        // drops to its own row and the author and date stay readable.
        let top = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x_2()
            .gap_y_1()
            .w_full()
            .min_w(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(AUTHOR_BLOCK_MIN_WIDTH_PX))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_none().child(components::author_avatar(
                        theme,
                        ui_scale,
                        &display_name,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(theme.ui_text(14.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        gpui::StyledText::new(display_name)
                                            .with_highlights(name_highlights),
                                    ),
                            )
                            .when(!details.author_email.is_empty(), |column| {
                                column.child(
                                    div()
                                        .debug_selector(|| "commit_card_author_email".to_string())
                                        .text_size(theme.ui_text(12.0))
                                        .text_color(theme.colors.foreground.secondary)
                                        .truncate()
                                        .child(details.author_email.clone()),
                                )
                            })
                            .child(
                                div()
                                    .text_size(theme.ui_text(12.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(when),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .when_some(signature, |row, signature| {
                        row.child(commit_details_signature_badge(theme, signature))
                    })
                    .child(sha_chip),
            );

        let pills = self.commit_ref_pills(&details.id);
        let hidden = pills.len().saturating_sub(MAX_REF_PILLS);
        let mut bottom = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(theme.colors.stroke.subtle)
            .child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(if details.parent_ids.len() > 1 {
                        "Parents"
                    } else {
                        "Parent"
                    }),
            );
        if details.parent_ids.is_empty() {
            bottom = bottom.child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("none (root commit)"),
            );
        }
        // A merge commit lists every parent, first parent first.
        for (ix, parent) in details.parent_ids.iter().enumerate() {
            let parent_sha = parent.as_ref().to_string();
            let select = parent.clone();
            let main = self
                .commit_card_chip_part(
                    ("commit_card_parent", ix),
                    format!("commit_card_parent_{ix}"),
                    format!("Go to parent commit {parent_sha}").into(),
                )
                .child(short_sha(&parent_sha).to_string())
                .on_activate(
                    !current,
                    components::ControlActivation::Action,
                    cx.listener(move |this, e: &ClickEvent, _w, cx| {
                        if e.standard_click() {
                            this.store.dispatch(Msg::SelectCommit {
                                request_id: None,
                                repo_id,
                                commit_id: select.clone(),
                            });
                            cx.notify();
                        }
                    }),
                );
            let arrow = self.commit_card_menu_arrow(
                ("commit_card_parent_menu", ix).into(),
                format!("commit_card_parent_menu_{ix}"),
                repo_id,
                parent.clone(),
                true,
                current,
                cx,
            );
            bottom = bottom.child(self.commit_card_chip(false).child(main).child(arrow));
        }
        if !pills.is_empty() {
            bottom = bottom.child(
                div()
                    .w(px(1.0))
                    .h(ui_scale.px(14.0))
                    .bg(theme.colors.stroke.default),
            );
            for pill in pills.into_iter().take(MAX_REF_PILLS) {
                bottom = bottom.child(self.commit_card_ref_pill(pill));
            }
            if hidden > 0 {
                bottom = bottom.child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(format!("+{hidden}")),
                );
            }
        }

        div()
            .id("commit_details_card")
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .p_2()
            .border_1()
            .border_color(theme.colors.stroke.default)
            .rounded(px(theme.radii.panel))
            .child(top)
            .child(bottom)
    }
}
