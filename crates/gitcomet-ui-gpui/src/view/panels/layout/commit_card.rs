//! The commit card at the top of the commit details: author, dates and the
//! short SHA on one row, and the parent with the refs on this commit below.

use super::*;
use crate::theme::with_alpha;
use gitcomet_core::domain::CommitDetails;
use std::cell::Cell;
use std::rc::Rc;

const MAX_REF_PILLS: usize = 4;
const SHA_CHIP_HEIGHT_PX: f32 = 22.0;

/// The card's two SHA chips. A click copies the commit's own SHA but reveals
/// the parent; both chevrons open the same commit menu.
#[derive(Clone, Copy)]
enum ShaChip {
    /// Washed when the history find query matched the SHA.
    Commit {
        find_matched: bool,
    },
    Parent,
}

impl ShaChip {
    fn id(self) -> &'static str {
        match self {
            Self::Commit { .. } => "commit_card_sha",
            Self::Parent => "commit_card_parent",
        }
    }

    fn menu_id(self) -> &'static str {
        match self {
            Self::Commit { .. } => "commit_card_sha_menu",
            Self::Parent => "commit_card_parent_menu",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Commit { .. } => "Copy commit SHA",
            Self::Parent => "Go to parent commit",
        }
    }

    fn icon(self) -> Option<&'static str> {
        match self {
            Self::Commit { .. } => Some("icons/copy.svg"),
            Self::Parent => None,
        }
    }

    fn washed(self) -> bool {
        matches!(self, Self::Commit { find_matched: true })
    }
}

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

    /// A short SHA whose body runs the chip's click and whose chevron opens
    /// the commit menu under the whole chip. While the card still shows the
    /// previous selection (`!current`) only copying stays live.
    fn commit_card_sha_chip(
        &self,
        chip: ShaChip,
        repo_id: RepoId,
        commit_id: CommitId,
        current: bool,
        cx: &mut gpui::Context<Self>,
    ) -> Div {
        let theme = self.theme;
        let body_enabled = current || matches!(chip, ShaChip::Commit { .. });
        let ui_scale = self.ui_scale();
        let menu_id = chip.menu_id();
        let menu_open = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|invoker| invoker.as_ref() == menu_id);
        let chip_bounds: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();

        let body = div()
            .id(chip.id())
            .debug_selector(move || chip.id().to_string())
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .h_full()
            .control_interaction(
                components::InteractionStyle::new(theme),
                components::InteractionState::default().disabled(!body_enabled),
            )
            .child(crate::view::caches::short_commit_sha(commit_id.as_ref()).to_owned())
            .when_some(chip.icon(), |d, icon| {
                d.child(svg_icon(
                    icon,
                    theme.colors.foreground.secondary,
                    ui_scale.px(12.0),
                ))
            })
            .on_activate(
                !body_enabled,
                components::ControlActivation::Action,
                cx.listener({
                    let commit_id = commit_id.clone();
                    move |this, e: &ClickEvent, _w, cx| {
                        if !e.standard_click() {
                            return;
                        }
                        match chip {
                            ShaChip::Commit { .. } => this.copy_commit_sha(commit_id.clone(), cx),
                            ShaChip::Parent => {
                                this.reveal_commit_in_history(repo_id, commit_id.clone(), cx)
                            }
                        }
                    }
                }),
            )
            .gitcomet_tooltip(theme, chip.tooltip().into());

        let menu = div()
            .id(menu_id)
            .debug_selector(move || menu_id.to_string())
            .flex()
            .items_center()
            .justify_center()
            .px_1()
            .h_full()
            .control_interaction(
                components::InteractionStyle::new(theme),
                components::InteractionState::default()
                    .open(menu_open)
                    .disabled(!current),
            )
            .child(svg_icon(
                "icons/chevron_down.svg",
                if menu_open {
                    theme.colors.accent.foreground
                } else {
                    theme.colors.foreground.secondary
                },
                ui_scale.px(12.0),
            ))
            .on_activate(
                !current,
                components::ControlActivation::Action,
                cx.listener({
                    let chip_bounds = Rc::clone(&chip_bounds);
                    move |this, e: &ClickEvent, window, cx| {
                        let Some(bounds) = chip_bounds.get().filter(|_| e.standard_click()) else {
                            return;
                        };
                        this.open_popover_for_bounds(
                            PopoverKind::CommitShaLinkMenu {
                                repo_id,
                                commit_id: commit_id.clone(),
                            }
                            .invoked_by(menu_id.into()),
                            bounds,
                            window,
                            cx,
                        );
                    }
                }),
            )
            .gitcomet_tooltip(theme, "Commit actions".into());

        div()
            .flex_none()
            .on_children_prepainted(move |children, _window, _cx| {
                chip_bounds.set(children.first().copied());
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(ui_scale.px(SHA_CHIP_HEIGHT_PX))
                    .rounded(px(theme.radii.control))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .when(chip.washed(), |d| {
                        d.debug_selector(|| "commit_card_sha_find_wash".to_string())
                    })
                    .bg(if chip.washed() {
                        with_alpha(theme.colors.accent.foreground, 0.18)
                    } else {
                        theme.colors.surface.raised
                    })
                    .overflow_hidden()
                    .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.primary)
                    .child(body)
                    .child(div().w(px(1.0)).h_full().bg(theme.colors.stroke.default))
                    .child(menu),
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
            .gap(px(6.0))
            .px_2()
            .h(px(24.0))
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
        let parent = details.parent_ids.first().cloned();
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

        let sha_chip = self.commit_card_sha_chip(
            ShaChip::Commit {
                find_matched: sha_matched,
            },
            repo_id,
            details.id.clone(),
            current,
            cx,
        );

        // The author block asks for its natural width, so a narrow pane wraps
        // the badges onto the next line instead of truncating name and date.
        let author = div()
            .debug_selector(|| "commit_card_author".to_string())
            .flex()
            .flex_auto()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(components::author_avatar(theme, ui_scale, &display_name))
            .child(
                div()
                    .flex()
                    .flex_auto()
                    .flex_col()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                gpui::StyledText::new(display_name)
                                    .with_highlights(name_highlights),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(when),
                    ),
            );
        let badges = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .when_some(signature, |row, signature| {
                row.child(commit_details_signature_badge(theme, signature))
            })
            .child(sha_chip);
        let top = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .child(author)
            .child(badges);

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
                    .child("Parent"),
            );
        bottom = match parent {
            Some(parent) => bottom.child(self.commit_card_sha_chip(
                ShaChip::Parent,
                repo_id,
                parent,
                current,
                cx,
            )),
            None => bottom.child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child("none (root commit)"),
            ),
        };
        if !pills.is_empty() {
            bottom = bottom.child(div().w(px(1.0)).h(px(14.0)).bg(theme.colors.stroke.default));
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
