//! Commit metadata in the details pane: author row, SHA/date/parent fields,
//! and the linkified message.

use super::*;
use crate::view::commit_message_text::{
    TextHighlights, commit_link_style, commit_message_summary_highlights,
};
use crate::view::panes::history::find::{
    CommitDetailsFindHighlights, history_find_detail_highlights,
};

type MessageLinks = Arc<[components::MessageLink]>;
type CommitMessageLinkHighlights = (TextHighlights, MessageLinks);

/// The name the author row shows: the author's name, or the email without one.
pub(super) fn commit_details_author_display_name(
    details: &gitcomet_core::domain::CommitDetails,
) -> &str {
    if details.author_name.is_empty() {
        &details.author_email
    } else {
        &details.author_name
    }
}

/// Author identity block: avatar + name + muted email, with the authored date
/// as a relative label (absolute date lives in the "Commit date" row below).
/// `name_matches` are history find matches in the shown name.
pub(super) fn commit_details_author_row(
    theme: AppTheme,
    ui_scale: crate::ui_scale::UiScale,
    details: &gitcomet_core::domain::CommitDetails,
    signature: Option<&gitcomet_core::domain::CommitSignature>,
    name_matches: &[std::ops::Range<usize>],
) -> Option<Div> {
    if details.author_name.is_empty() && details.author_email.is_empty() {
        return None;
    }
    let display_name = commit_details_author_display_name(details).to_owned();
    let wash = crate::view::rows::query_highlight_style(theme);
    let mut name_highlights: TextHighlights = name_matches
        .iter()
        .map(|range| (range.clone(), wash))
        .collect();
    crate::text_runs::sanitize_highlights(&display_name, &mut name_highlights);
    let authored_relative = (details.authored_at_unix != 0).then(|| {
        crate::view::date_time::format_relative_time(
            details.authored_at_unix,
            std::time::SystemTime::now(),
        )
    });

    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .child(components::author_avatar(theme, ui_scale, &display_name))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(
                                gpui::StyledText::new(display_name)
                                    .with_highlights(name_highlights),
                            ),
                    )
                    .when(!details.author_email.is_empty(), |column| {
                        column.child(
                            div()
                                .text_size(theme.ui_text(12.0))
                                .text_color(theme.colors.foreground.secondary)
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(details.author_email.clone()),
                        )
                    }),
            )
            .when_some(signature, |row, signature| {
                row.child(commit_details_signature_badge(theme, signature))
            })
            .when_some(authored_relative, |row, relative| {
                row.child(
                    div()
                        .flex_none()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(relative),
                )
            }),
    )
}

/// The signature chip shown beside the commit author.
///
/// Needs its own `.id()`: a stateless div computes the hover style and throws
/// it away, and the tooltip would never attach.
fn commit_details_signature_badge(
    theme: AppTheme,
    signature: &gitcomet_core::domain::CommitSignature,
) -> gpui::Stateful<Div> {
    let badge = crate::view::commit_signature::signature_badge(theme, signature);
    div()
        .id("commit_details_signature_badge")
        .debug_selector(|| "commit_details_signature_badge".to_string())
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1()
        .rounded(px(theme.radii.control))
        .border_1()
        .border_color(badge.palette.border)
        .bg(badge.palette.background)
        .child(
            gpui::svg()
                .path(badge.icon)
                .size(theme.ui_text(12.0))
                .flex_shrink_0()
                .text_color(badge.palette.foreground)
                .debug_selector(|| "commit_details_signature_icon".to_string()),
        )
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(badge.palette.foreground)
                .whitespace_nowrap()
                .child(badge.label),
        )
        .gitcomet_tooltip(theme, badge.tooltip)
}

pub(super) fn commit_details_selectable_row(
    theme: AppTheme,
    key: &'static str,
    value: AnyElement,
) -> Div {
    components::selectable_field(theme, key, value)
}

pub(super) fn commit_details_monospace_value(input: Entity<components::TextInput>) -> AnyElement {
    commit_details_monospace_element(input.into_any_element())
}

pub(super) fn commit_details_monospace_element(value: AnyElement) -> AnyElement {
    div()
        .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
        .child(value)
        .into_any_element()
}

fn commit_message_link_highlights(
    message: &str,
    theme: AppTheme,
    max_id_len: usize,
) -> CommitMessageLinkHighlights {
    use crate::text_selection::MessageLinkKind;

    let style = commit_link_style(theme);
    let found = crate::text_selection::commit_message_link_ranges(message, max_id_len);
    let highlights = found
        .iter()
        .map(|link| (link.range.clone(), style))
        .collect::<Vec<_>>();
    let links = found
        .into_iter()
        .map(|link| {
            let text = &message[link.range.clone()];
            let target = match link.kind {
                MessageLinkKind::CommitSha => components::LinkTarget::Commit {
                    commit_id: CommitId(text.to_ascii_lowercase().into()),
                    allow_navigate: true,
                },
                MessageLinkKind::Url => components::LinkTarget::Url(text.to_owned().into()),
            };
            components::MessageLink {
                range: link.range,
                target,
            }
        })
        .collect::<Vec<_>>();

    (highlights, Arc::from(links))
}

/// The whole of a SHA field is one link, without scanning: the field holds
/// nothing but the id.
fn commit_sha_field_links(sha: &str, interactive: bool, allow_navigate: bool) -> MessageLinks {
    if interactive {
        Arc::from([components::MessageLink {
            range: 0..sha.len(),
            target: components::LinkTarget::Commit {
                commit_id: CommitId(sha.to_string().into()),
                allow_navigate,
            },
        }])
    } else {
        Arc::<[components::MessageLink]>::from([])
    }
}

fn commit_sha_field_highlights(value: &str, theme: AppTheme) -> TextHighlights {
    if value.is_empty() || value == "—" {
        Vec::new()
    } else {
        vec![(0..value.len(), commit_link_style(theme))]
    }
}

/// The "Commit SHA" value, followed by the abbreviation the history find query
/// matched, if it was one, so the typed short form can be seen in the full id.
pub(super) fn commit_details_sha_value(
    theme: AppTheme,
    sha_field: AnyElement,
    sha: &str,
    find: &CommitDetailsFindHighlights,
) -> AnyElement {
    let short = find
        .short_sha_len
        .and_then(|len| sha.get(..len))
        .filter(|short| !short.is_empty());
    let Some(short) = short else {
        return commit_details_monospace_element(sha_field);
    };
    let label = format!("({short})");
    let wash = vec![(
        1..1 + short.len(),
        crate::view::rows::query_highlight_style(theme),
    )];
    commit_details_monospace_element(
        div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .min_w(px(0.0))
            .child(div().flex_1().min_w(px(0.0)).child(sha_field))
            .child(
                div()
                    .debug_selector(|| "commit_details_sha_find_short".to_string())
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.colors.foreground.secondary)
                    .child(gpui::StyledText::new(label).with_highlights(wash)),
            )
            .into_any_element(),
    )
}

impl DetailsPaneView {
    /// What the history find query matched in `details`. The row is matched on
    /// its displayed summary, so the ranges are found there and mapped back
    /// into the raw message the pane shows.
    pub(super) fn commit_details_find_highlights(
        &self,
        details: &gitcomet_core::domain::CommitDetails,
    ) -> CommitDetailsFindHighlights {
        let summary = details.message.split('\n').next().unwrap_or_default();
        let listed = self.active_repo().and_then(|repo| match &repo.stashes {
            Loadable::Ready(stashes) => stashes
                .iter()
                .find(|stash| stash.id == details.id)
                .map(|stash| stash.message.as_ref()),
            _ => None,
        });
        let row_summary = if listed.is_some()
            || gitcomet_core::history_find::is_probable_stash_summary(
                details.parent_ids.len(),
                summary,
            ) {
            gitcomet_core::history_find::stash_row_summary(listed, summary)
        } else {
            summary
        };
        history_find_detail_highlights(
            self.history_find_query.as_ref(),
            details.id.as_ref(),
            &details.message,
            commit_details_author_display_name(details),
            row_summary,
        )
        .unwrap_or_default()
    }

    pub(super) fn sync_commit_details_input_value(
        input: &Entity<components::TextInput>,
        value: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        if input.read(cx).text() != value {
            input.update(cx, |input, cx| {
                input.set_text(value.to_string(), cx);
            });
        }
    }

    pub(super) fn sync_commit_details_message_input(
        &mut self,
        message: &str,
        max_id_len: usize,
        theme: AppTheme,
        repo_id: RepoId,
        find_matches: &[std::ops::Range<usize>],
        cx: &mut gpui::Context<Self>,
    ) {
        let (mut highlights, links) = commit_message_link_highlights(message, theme, max_id_len);
        let mut merged = commit_message_summary_highlights(message, theme, &highlights);
        merged.append(&mut highlights);
        merged.sort_by_key(|(range, _)| range.start);
        let merged = crate::text_runs::overlay_highlights(
            merged,
            find_matches,
            crate::view::rows::query_highlight_style(theme),
        );
        self.commit_details_message_input.update(cx, |input, cx| {
            if input.text() != message {
                input.set_text(message.to_string(), cx);
            }
            input.set_highlights(merged, cx);
        });
        self.commit_details_message_link_menu
            .update(cx, |menu, cx| {
                menu.sync(
                    self.commit_details_message_input.clone(),
                    repo_id,
                    links,
                    "commit_details_message_link_menu",
                    cx,
                );
            });
    }

    pub(super) fn sync_commit_details_parent_input(
        &mut self,
        parent: &str,
        repo_id: RepoId,
        interactive: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) {
        Self::sync_commit_details_input_value(&self.commit_details_parent_input, parent, cx);
        self.commit_details_parent_input.update(cx, |input, cx| {
            input.set_highlights(commit_sha_field_highlights(parent, theme), cx);
        });
        let parent_links = commit_sha_field_links(parent, interactive, true);
        self.commit_details_parent_link_menu.update(cx, |menu, cx| {
            menu.sync(
                self.commit_details_parent_input.clone(),
                repo_id,
                parent_links,
                "commit_details_parent_link_menu",
                cx,
            );
        });
    }

    /// The SHA field's text and link style, washed whole when the history find
    /// query matched it.
    pub(super) fn sync_commit_details_sha_input(
        &mut self,
        sha: &str,
        find_matched: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) {
        Self::sync_commit_details_input_value(&self.commit_details_sha_input, sha, cx);
        let whole = find_matched.then_some(0..sha.len());
        let highlights = crate::text_runs::overlay_highlights(
            commit_sha_field_highlights(sha, theme),
            whole.as_slice(),
            crate::view::rows::query_highlight_style(theme),
        );
        self.commit_details_sha_input.update(cx, |input, cx| {
            input.set_highlights(highlights, cx);
        });
    }

    pub(super) fn sync_commit_details_sha_menu(
        &mut self,
        sha: &str,
        repo_id: RepoId,
        interactive: bool,
        find_matched: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) {
        self.sync_commit_details_sha_input(sha, find_matched, theme, cx);
        // A commit's own SHA has nothing to reveal.
        let sha_links = commit_sha_field_links(sha, interactive, false);
        self.commit_details_sha_link_menu.update(cx, |menu, cx| {
            menu.sync(
                self.commit_details_sha_input.clone(),
                repo_id,
                sha_links,
                "commit_details_sha_link_menu",
                cx,
            );
        });
    }

    pub(super) fn sync_retained_commit_details_message_input(
        &mut self,
        message: &str,
        find_matches: &[std::ops::Range<usize>],
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        let highlights = crate::text_runs::overlay_highlights(
            commit_message_summary_highlights(message, theme, &[]),
            find_matches,
            crate::view::rows::query_highlight_style(theme),
        );
        self.commit_details_message_input.update(cx, |input, cx| {
            if input.text() != message {
                input.set_text(message.to_string(), cx);
            }
            input.set_highlights(highlights, cx);
        });
        self.commit_details_message_link_menu
            .update(cx, |menu, cx| {
                menu.sync(
                    self.commit_details_message_input.clone(),
                    RepoId(0),
                    Arc::<[components::MessageLink]>::from([]),
                    "commit_details_message_link_menu",
                    cx,
                );
            });
    }

    /// Commit message shown in the details pane: a scrollable block whose
    /// summary line is emphasized and whose SHA references are linkified.
    pub(super) fn commit_details_message_view(
        &self,
        theme: AppTheme,
        repo_id: RepoId,
    ) -> AnyElement {
        components::ScrollContainer::vertical(
            ("commit_details_message_scroll_surface", repo_id.0),
            ("commit_details_message_scrollbar", repo_id.0),
            self.commit_scroll.clone(),
            self.ui_scale().px(COMMIT_DETAILS_MESSAGE_MAX_HEIGHT_PX),
        )
        .container_id(("commit_details_message_container", repo_id.0))
        .debug_selector("commit_details_message_scroll_surface")
        .render(theme, self.commit_details_message_link_menu.clone())
    }
}
