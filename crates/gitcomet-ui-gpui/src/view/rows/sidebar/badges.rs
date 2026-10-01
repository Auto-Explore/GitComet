//! Worktree and upstream badge palette, labels and chips.

use super::*;

pub(in crate::view) const WORKTREE_ICON_PATH: &str = "icons/git_worktree.svg";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::view) struct WorktreeBadgePalette {
    pub(in crate::view) bg: gpui::Rgba,
    pub(super) active_bg: gpui::Rgba,
    pub(in crate::view) border: gpui::Rgba,
    pub(in crate::view) hover_border: gpui::Rgba,
    pub(super) open_border: gpui::Rgba,
    open_hover_border: gpui::Rgba,
    pub(super) active_border: gpui::Rgba,
    pub(in crate::view) icon: gpui::Rgba,
    pub(super) open_icon: gpui::Rgba,
    pub(super) active_icon: gpui::Rgba,
    pub(in crate::view) text: gpui::Rgba,
    pub(in crate::view) hover_text: gpui::Rgba,
    pub(super) open_text: gpui::Rgba,
    pub(super) active_text: gpui::Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WorktreeBadgeColors {
    pub(super) bg: gpui::Rgba,
    pub(super) border: gpui::Rgba,
    hover_border: gpui::Rgba,
    pub(super) icon: gpui::Rgba,
    pub(super) text: gpui::Rgba,
}

/// Shared chip palette for the sidebar badges (workspace/worktree/upstream),
/// with a transparent body and a quiet border at rest. Badges that refer to an
/// open workspace use the main canvas surface on light themes and a lighter,
/// raised surface on dark themes. Their text stays high-contrast and the icon
/// retains the workspace state color.
pub(in crate::view) fn worktree_badge_palette(theme: AppTheme) -> WorktreeBadgePalette {
    let transparent = gpui::rgba(0x00000000);
    let text = theme.colors.foreground.emphasis;
    WorktreeBadgePalette {
        bg: transparent,
        active_bg: if theme.is_dark {
            theme.colors.surface.raised
        } else {
            theme.colors.surface.canvas
        },
        border: with_alpha(theme.colors.stroke.default, 0.90),
        hover_border: with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.55 } else { 0.40 },
        ),
        open_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.56 } else { 0.34 },
        ),
        open_hover_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.46 },
        ),
        active_border: with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.84 } else { 0.68 },
        ),
        icon: theme.colors.foreground.secondary,
        open_icon: theme.colors.accent.foreground,
        active_icon: theme.colors.accent.foreground,
        text,
        hover_text: text,
        open_text: text,
        active_text: text,
    }
}

pub(super) fn worktree_badge_colors(
    palette: WorktreeBadgePalette,
    is_open: bool,
    menu_active: bool,
) -> WorktreeBadgeColors {
    WorktreeBadgeColors {
        bg: if is_open {
            palette.active_bg
        } else {
            palette.bg
        },
        border: if menu_active {
            palette.active_border
        } else if is_open {
            palette.open_border
        } else {
            palette.border
        },
        hover_border: if is_open {
            palette.open_hover_border
        } else {
            palette.hover_border
        },
        icon: if menu_active {
            palette.active_icon
        } else if is_open {
            palette.open_icon
        } else {
            palette.icon
        },
        text: if menu_active {
            palette.active_text
        } else if is_open {
            palette.open_text
        } else {
            palette.text
        },
    }
}

/// A configured upstream is the active remote relationship, so its status chip
/// uses the same colors as a worktree badge whose workspace is open.
pub(super) fn upstream_badge_colors(palette: WorktreeBadgePalette) -> WorktreeBadgeColors {
    worktree_badge_colors(palette, true, false)
}

pub(in crate::view::rows) fn worktree_branch_badge_label(
    branch: Option<&SharedString>,
    detached: bool,
    open_repo: Option<&RepoState>,
) -> Option<SharedString> {
    if let Some(open_repo) = open_repo {
        if open_repo.detached_head_commit.is_some() {
            return Some("(detached)".into());
        }

        match &open_repo.head_branch {
            Loadable::Ready(head_branch) if head_branch != "HEAD" => {
                return Some(SharedString::new(head_branch.as_str()));
            }
            Loadable::Ready(_) if detached => return Some("(detached)".into()),
            _ => {}
        }
    }

    branch
        .cloned()
        .or_else(|| detached.then(|| "(detached)".into()))
}

/// `"{branch} · {folder}"` — the branch says what is checked out, the folder says
/// which worktree it is checked out in, and only the pair is unambiguous when
/// several worktrees sit on related branches.
///
/// Falls back to the folder alone when there is no branch to name, and to the
/// branch alone when the path has no final component.
pub(in crate::view) fn worktree_origin_label(
    branch: Option<&str>,
    detached: bool,
    path: &std::path::Path,
) -> SharedString {
    let branch =
        worktree_branch_badge_label(branch.map(SharedString::new).as_ref(), detached, None);
    let folder = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    match (branch, folder) {
        (Some(branch), Some(folder)) => SharedString::new(format!("{branch} · {folder}")),
        (Some(branch), None) => branch,
        (None, Some(folder)) => SharedString::new(folder),
        (None, None) => "worktree".into(),
    }
}

/// Height of a worktree badge. Comfortable lifts it with the rows and title
/// bars it sits in.
const WORKTREE_BADGE_HEIGHT_PX: f32 = 18.0;
const WORKTREE_BADGE_COMFORTABLE_HEIGHT_PX: f32 = 24.0;

pub(in crate::view) fn worktree_badge_height(scale: impl Into<ui_scale::UiScale>) -> Pixels {
    scale.into().row_height(
        WORKTREE_BADGE_HEIGHT_PX,
        WORKTREE_BADGE_COMFORTABLE_HEIGHT_PX,
    )
}

/// Marks content belonging to another worktree in history, details and diff
/// headers. Worktree chips share their interaction profile; callers supply the
/// action.
pub(in crate::view) fn worktree_origin_chip(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    label: SharedString,
    icon_size: Pixels,
    height: Pixels,
    max_width: Pixels,
    pad_x: Pixels,
) -> gpui::Stateful<gpui::Div> {
    let palette = worktree_badge_palette(theme);
    div()
        .id(id)
        .control_interaction(
            worktree_badge_interaction(theme),
            controls::InteractionState::default(),
        )
        .flex()
        .items_center()
        .gap_1()
        .flex_shrink_0()
        .max_w(max_width)
        .px(pad_x)
        .h(height)
        .rounded(px(theme.radii.control))
        .border_1()
        .border_color(palette.border)
        .bg(palette.bg)
        .child(svg_icon(WORKTREE_ICON_PATH, palette.icon, icon_size))
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(palette.text)
                .line_clamp(1)
                .whitespace_nowrap()
                .child(label),
        )
}

pub(in crate::view) fn worktree_badge_interaction(theme: AppTheme) -> controls::InteractionStyle {
    let palette = worktree_badge_palette(theme);
    controls::InteractionStyle::accent(theme)
        .hover(
            gpui::StyleRefinement::default()
                .bg(theme.hover_overlay())
                .border_color(palette.hover_border)
                .text_color(palette.hover_text),
        )
        .pressed(
            gpui::StyleRefinement::default()
                .bg(theme.active_overlay())
                .border_color(palette.hover_border)
                .text_color(palette.hover_text),
        )
        .resting_background(palette.bg)
}
