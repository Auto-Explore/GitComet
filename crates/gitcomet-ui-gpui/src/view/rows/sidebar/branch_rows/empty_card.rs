//! The dashed card an empty Worktrees / Submodules / Stashes section shows,
//! with a short pitch and a quick action. The sidebar list has one row height,
//! so the card is drawn as `EMPTY_CARD_ROWS` slices that join into one outline.

use super::*;
use crate::view::branch_sidebar::{EMPTY_CARD_ROWS, EmptySection};
use crate::view::commands::Command;

const EMPTY_CARD_TOP_GAP_PX: f32 = 8.0;

impl SidebarPaneView {
    pub(super) fn sidebar_empty_card_row(
        this: &Self,
        ctx: &SidebarRowContext,
        ix: usize,
        section: EmptySection,
        part: u8,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let _ = this;
        let theme = ctx.theme;
        let repo_id = ctx.repo_id;
        let (icon, title, detail, action) = match section {
            EmptySection::Branches => (
                "icons/git_branch.svg",
                "No branches yet",
                "Create one to start working.",
                "New branch",
            ),
            EmptySection::Remotes => (
                "icons/cloud.svg",
                "Nothing to sync with",
                "Add a remote to push and pull.",
                "Add remote",
            ),
            EmptySection::Worktrees => (
                WORKTREE_ICON_PATH,
                "Two branches, two folders",
                "Fix a bug beside your work.",
                "New worktree",
            ),
            EmptySection::Submodules => (
                "icons/box.svg",
                "Repositories inside repositories",
                "Track another repo in this one.",
                "Add submodule",
            ),
            EmptySection::Stashes => (
                STASH_ICON_PATH,
                "Set changes aside",
                "Come back to them later.",
                "New stash",
            ),
        };
        let first = part == 0;
        let last = part + 1 == EMPTY_CARD_ROWS;
        let radius = ctx.px(6.0);

        let body = match part {
            0 => div()
                .flex()
                .items_center()
                .gap(ctx.px(BRANCH_TREE_GAP_PX))
                .child(ctx.tree_icon_slot(icon, ctx.icon_primary, 14.0))
                .child(
                    div()
                        .min_w(px(0.0))
                        .line_clamp(1)
                        .whitespace_nowrap()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.colors.foreground.primary)
                        .child(title),
                ),
            1 => div().child(div().line_clamp(1).whitespace_nowrap().child(detail)),
            _ => div().flex().items_center().child(
                components::Button::new(format!("empty_card_action_{}", ix), action)
                    .start_slot(ctx.svg_icon(
                        "icons/plus.svg",
                        theme.colors.foreground.primary,
                        12.0,
                    ))
                    .on_click(theme, cx, move |this, e, window, cx| {
                        let command = match section {
                            EmptySection::Branches => Some(Command::CreateBranch),
                            EmptySection::Remotes => Some(Command::AddRemote),
                            _ => None,
                        };
                        if let Some(command) = command {
                            this.run_command(command.id(), window, cx);
                            return;
                        }
                        let kind = match section {
                            EmptySection::Branches | EmptySection::Remotes => return,
                            EmptySection::Worktrees => {
                                PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt)
                            }
                            EmptySection::Submodules => {
                                PopoverKind::submodule(repo_id, SubmodulePopoverKind::AddPrompt)
                            }
                            EmptySection::Stashes => PopoverKind::StashPrompt,
                        };
                        this.open_popover_at(kind, e.position(), window, cx);
                    })
                    .w_full(),
            ),
        };

        div()
            .id(("empty_section_card", ix))
            .h(ctx.row_height())
            .w_full()
            .px(ctx.px(ctx.content_inset + BRANCH_TREE_BASE_PAD_PX))
            // Air between the section header and the card's outline.
            .when(first, |d| d.pt(ctx.px(EMPTY_CARD_TOP_GAP_PX)))
            .child(
                body.size_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .border_l_1()
                    .border_r_1()
                    .when(first, |d| d.border_t_1().rounded_t(radius))
                    .when(last, |d| d.border_b_1().rounded_b(radius))
                    .border_dashed()
                    .border_color(theme.colors.stroke.default)
                    .text_size(theme.ui_text(13.0))
                    .text_color(theme.colors.foreground.secondary),
            )
            .into_any_element()
    }
}
