//! Commit metadata in the details pane: author row, SHA/date/parent fields,
//! and the linkified message.

use super::*;
use crate::view::commit_message_text::{
    TextHighlights, commit_link_style, commit_message_summary_highlights,
};

type MessageLinks = Arc<[components::MessageLink]>;
type CommitMessageLinkHighlights = (TextHighlights, MessageLinks);

/// Author identity block: avatar + name + muted email, with the authored date
/// as a relative label (absolute date lives in the "Commit date" row below).
pub(super) fn commit_details_author_row(
    theme: AppTheme,
    ui_scale: crate::ui_scale::UiScale,
    details: &gitcomet_core::domain::CommitDetails,
    signature: Option<&gitcomet_core::domain::CommitSignature>,
) -> Option<Div> {
    if details.author_name.is_empty() && details.author_email.is_empty() {
        return None;
    }
    let display_name = if details.author_name.is_empty() {
        details.author_email.clone()
    } else {
        details.author_name.clone()
    };
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
                            .child(display_name),
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

fn commit_message_link_highlights(message: &str, theme: AppTheme) -> CommitMessageLinkHighlights {
    use crate::text_selection::MessageLinkKind;

    let style = commit_link_style(theme);
    let found = crate::text_selection::commit_message_link_ranges(message);
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

impl DetailsPaneView {
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
        theme: AppTheme,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        let (mut highlights, links) = commit_message_link_highlights(message, theme);
        let mut merged = commit_message_summary_highlights(message, theme, &highlights);
        merged.append(&mut highlights);
        merged.sort_by_key(|(range, _)| range.start);
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

    pub(super) fn sync_commit_details_sha_menu(
        &mut self,
        sha: &str,
        repo_id: RepoId,
        interactive: bool,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) {
        Self::sync_commit_details_input_value(&self.commit_details_sha_input, sha, cx);
        self.commit_details_sha_input.update(cx, |input, cx| {
            input.set_highlights(commit_sha_field_highlights(sha, theme), cx);
        });
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
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        self.commit_details_message_input.update(cx, |input, cx| {
            if input.text() != message {
                input.set_text(message.to_string(), cx);
            }
            input.set_highlights(commit_message_summary_highlights(message, theme, &[]), cx);
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
