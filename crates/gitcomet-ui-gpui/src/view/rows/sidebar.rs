use super::*;
use gitcomet_core::domain::LogScope;

const WORKTREE_ICON_PATH: &str = "icons/git_worktree.svg";

pub(in crate::view) fn listed_workspace_paths_by_branch(
    repo: &RepoState,
) -> HashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return HashMap::default();
    };

    let mut worktree_paths = HashMap::default();
    for worktree in worktrees.iter() {
        if worktree.path == repo.spec.workdir {
            continue;
        }

        let Some(branch) = worktree.branch.clone() else {
            continue;
        };

        worktree_paths
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    worktree_paths
}

fn branch_workspace_badge_path(
    listed_workspace_path: Option<&std::path::Path>,
    active_workspace_path: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    listed_workspace_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| active_workspace_path.map(std::path::Path::to_path_buf))
}

pub(in crate::view) fn active_workspace_paths_by_branch(
    repo: &RepoState,
    open_repos: &[RepoState],
) -> HashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return HashMap::default();
    };

    let mut active_workspaces = HashMap::default();
    for worktree in worktrees.iter() {
        let Some(open_repo) = open_repos
            .iter()
            .find(|open_repo| open_repo.spec.workdir == worktree.path)
        else {
            continue;
        };

        let branch = if open_repo.detached_head_commit.is_some() {
            None
        } else {
            match &open_repo.head_branch {
                Loadable::Ready(head_branch) if head_branch != "HEAD" => Some(head_branch.clone()),
                Loadable::Ready(_) => None,
                _ => worktree.branch.clone(),
            }
        };
        let Some(branch) = branch else {
            continue;
        };

        active_workspaces
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    active_workspaces
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LocalBranchDoubleClickAction {
    CheckoutBranch { name: String },
    OpenWorkspace { path: std::path::PathBuf },
}

fn local_branch_double_click_action(
    branch: &str,
    workspace_path: Option<&std::path::Path>,
) -> LocalBranchDoubleClickAction {
    match workspace_path {
        Some(path) => LocalBranchDoubleClickAction::OpenWorkspace {
            path: path.to_path_buf(),
        },
        None => LocalBranchDoubleClickAction::CheckoutBranch {
            name: branch.to_string(),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchHistoryRevealTarget {
    commit_id: CommitId,
    desired_scope: LogScope,
}

fn branch_commit_id(repo: &RepoState, section: BranchSection, name: &str) -> Option<CommitId> {
    match section {
        BranchSection::Local => match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|branch| branch.name == name)
                .map(|branch| branch.target.clone()),
            _ => None,
        },
        BranchSection::Remote => {
            let (remote, branch_name) = name.split_once('/')?;
            match &repo.remote_branches {
                Loadable::Ready(branches) => branches
                    .iter()
                    .find(|branch| branch.remote == remote && branch.name == branch_name)
                    .map(|branch| branch.target.clone()),
                _ => None,
            }
        }
    }
}

fn branch_click_history_reveal_target(
    repo: &RepoState,
    section: BranchSection,
    name: &str,
    is_head: bool,
) -> Option<BranchHistoryRevealTarget> {
    let commit_id = branch_commit_id(repo, section, name)?;

    let desired_scope = match section {
        BranchSection::Local if is_head => repo.history_state.history_scope,
        BranchSection::Local | BranchSection::Remote => LogScope::AllBranches,
    };

    Some(BranchHistoryRevealTarget {
        commit_id,
        desired_scope,
    })
}

fn branch_row_is_selected(
    selected_branch: Option<&SelectedBranch>,
    repo_id: RepoId,
    section: BranchSection,
    name: &str,
    selected_commit: Option<&CommitId>,
    selected_branch_commit_id: Option<&CommitId>,
) -> bool {
    selected_branch.is_some_and(|selected_branch| {
        selected_branch.repo_id == repo_id
            && selected_branch.section == section
            && selected_branch.name == name
            && selected_branch_commit_id.is_some_and(|commit_id| selected_commit == Some(commit_id))
    })
}

impl SidebarPaneView {
    pub(in super::super) fn render_branch_sidebar_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        const BRANCH_TREE_BASE_PAD_PX: f32 = 8.0;
        const BRANCH_TREE_DEPTH_STEP_PX: f32 = 10.0;
        const BRANCH_TREE_TOGGLE_SLOT_PX: f32 = 12.0;
        const BRANCH_TREE_ICON_SLOT_PX: f32 = 16.0;
        const BRANCH_TREE_GAP_PX: f32 = 6.0;
        const CONTEXT_MENU_INDICATOR_SIZE_PX: f32 = 18.0;
        const CONTEXT_MENU_INDICATOR_RIGHT_PX: f32 = 6.0;

        let Some(repo_id) = this.active_repo_id() else {
            return Vec::new();
        };
        let Some(presentation) = this.branch_sidebar_presentation_cached() else {
            return Vec::new();
        };
        let rows = presentation.rows;
        let workspace_badges = presentation.workspace_badges;
        let repo_workdir = this.active_repo().map(|r| r.spec.workdir.clone());
        let theme = this.theme;
        let icon_primary = theme.colors.accent;
        let icon_muted = with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 });
        let selected_branch = this.selected_branch().cloned();
        let (selected_commit, selected_branch_commit_id) =
            this.active_repo().map_or((None, None), |repo| {
                let selected_commit = repo.history_state.selected_commit.clone();
                let selected_branch_commit_id = selected_branch
                    .as_ref()
                    .filter(|selected| selected.repo_id == repo_id)
                    .and_then(|selected| {
                        branch_commit_id(repo, selected.section, selected.name.as_str())
                    });
                (selected_commit, selected_branch_commit_id)
            });

        let svg_icon = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_icon(path, color, px(size_px))
        };
        let svg_spinner = |id: (&'static str, u64), color: gpui::Rgba, size_px: f32| {
            super::super::icons::svg_spinner(id, color, px(size_px))
        };
        let svg_collapse = |collapsed: bool| {
            svg_icon(
                if collapsed {
                    "icons/arrow_right.svg"
                } else {
                    "icons/chevron_down.svg"
                },
                icon_muted,
                10.0,
            )
        };
        let tree_toggle_slot = |collapsed: Option<bool>| {
            div()
                .w(px(BRANCH_TREE_TOGGLE_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .when_some(collapsed, |this, collapsed| {
                    this.child(svg_collapse(collapsed))
                })
        };
        let tree_icon_slot = |path: &'static str, color: gpui::Rgba, size_px: f32| {
            div()
                .w(px(BRANCH_TREE_ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(path, color, size_px))
        };
        let branch_tree_color = |section: BranchSection| match section {
            BranchSection::Local => theme.colors.text,
            BranchSection::Remote => theme.colors.text_muted,
        };
        let mix_color = |a: gpui::Rgba, b: gpui::Rgba, t: f32| {
            let t = t.clamp(0.0, 1.0);
            gpui::Rgba {
                r: a.r + (b.r - a.r) * t,
                g: a.g + (b.g - a.g) * t,
                b: a.b + (b.b - a.b) * t,
                a: a.a + (b.a - a.a) * t,
            }
        };
        let context_menu_indicator_icon =
            with_alpha(theme.colors.text, if theme.is_dark { 0.82 } else { 0.70 });
        let context_menu_indicator_bg = mix_color(
            theme.colors.window_bg,
            theme.colors.surface_bg,
            if theme.is_dark { 0.64 } else { 0.52 },
        );
        let context_menu_indicator_hover_bg = mix_color(
            context_menu_indicator_bg,
            theme.colors.active_section,
            if theme.is_dark { 0.60 } else { 0.42 },
        );
        let context_menu_indicator_active_bg = mix_color(
            theme.colors.surface_bg,
            theme.colors.accent,
            if theme.is_dark { 0.34 } else { 0.20 },
        );
        let context_menu_indicator_border = mix_color(
            theme.colors.window_bg,
            theme.colors.border,
            if theme.is_dark { 0.92 } else { 0.86 },
        );
        let context_menu_indicator =
            |id: SharedString, row_group: SharedString, visible: bool, menu_active: bool| {
                div()
                    .id(id)
                    .absolute()
                    .right(px(CONTEXT_MENU_INDICATOR_RIGHT_PX))
                    .top_0()
                    .bottom_0()
                    .w(px(CONTEXT_MENU_INDICATOR_SIZE_PX))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.0))
                    .bg(if menu_active {
                        context_menu_indicator_active_bg
                    } else {
                        context_menu_indicator_bg
                    })
                    .border_1()
                    .border_color(context_menu_indicator_border)
                    .cursor(CursorStyle::PointingHand)
                    .invisible()
                    .when(visible, |d| d.visible())
                    .group_hover(row_group, |d| d.visible())
                    .hover(move |s| {
                        if menu_active {
                            s.bg(context_menu_indicator_active_bg)
                        } else {
                            s.bg(context_menu_indicator_hover_bg)
                        }
                    })
                    .active(move |s| s.bg(context_menu_indicator_active_bg))
                    .child(svg_icon(
                        "icons/menu.svg",
                        context_menu_indicator_icon,
                        12.0,
                    ))
            };

        fn indent_px(depth: usize) -> Pixels {
            px(BRANCH_TREE_BASE_PAD_PX + depth as f32 * BRANCH_TREE_DEPTH_STEP_PX)
        }

        fn left_divider(color: gpui::Rgba, radius: Pixels) -> gpui::Div {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .w(px(2.0))
                .rounded_l(radius)
                .bg(color)
        }

        fn top_divider(color: gpui::Rgba) -> gpui::Div {
            div()
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(1.0))
                .bg(color)
        }

        range
            .filter_map(|ix| rows.get(ix).cloned().map(|r| (ix, r)))
            .map(|(ix, row)| match row {
                BranchSidebarRow::SectionHeader {
                    section,
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let (icon_path, label): (&'static str, SharedString) = match section {
                        BranchSection::Local => ("icons/computer.svg", "Local Branches".into()),
                        BranchSection::Remote => ("icons/cloud.svg", "Remote branches".into()),
                    };
                    let tooltip = label.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString =
                        format!("branch_section_menu_{}_{}", repo_id.0, section_key).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("branch_section_row_{}_{}", repo_id.0, section_key).into();

                    div()
                        .id(("branch_section", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(icon_path, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child(label),
                        )
                        .child(
                            context_menu_indicator(
                                format!(
                                    "branch_section_menu_indicator_{}_{}",
                                    repo_id.0, section_key
                                )
                                .into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::BranchSectionMenu { repo_id, section },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchSectionMenu { repo_id, section },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SectionSpacer => div()
                    .id(("branch_section_spacer", ix))
                    .h(px(10.0))
                    .w_full()
                    .into_any_element(),
                BranchSidebarRow::StashHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_stash_spinner = this.active_repo().is_some_and(|r| {
                        matches!(r.stashes, Loadable::Loading)
                            || (!collapsed && matches!(r.stashes, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("stash_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString = format!("stash_section_row_{}", repo_id.0).into();

                    div()
                        .id(("stash_section", ix))
                        .debug_selector(move || format!("stash_section_{ix}"))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Stash"),
                        )
                        .when(show_stash_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || format!("stash_spinner_{}", repo_id.0))
                                    .child(svg_spinner(
                                        ("stash_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .child(
                            context_menu_indicator(
                                format!("stash_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::StashPrompt,
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Stashes (Right-click or use the menu button for actions)".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::StashPrompt,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::StashPlaceholder { message } => div()
                    .id(("stash_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::StashItem {
                    index,
                    message,
                    tooltip,
                    created_at: _,
                } => {
                    let tooltip = tooltip.clone();
                    let stash_message_for_menu = message.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("stash_menu_{}_{}", repo_id.0, index).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let stash_message_for_right_click = stash_message_for_menu.clone();
                    let stash_message_for_indicator = stash_message_for_menu.clone();
                    let row_group: SharedString =
                        format!("stash_row_{}_{}", repo_id.0, index).into();

                    div()
                        .id(("stash_sidebar_row", index))
                        .relative()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr_2()
                        .h(px(24.0))
                        .w_full()
                        .rounded(px(theme.radii.row))
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(message.clone()),
                        )
                        .child(
                            context_menu_indicator(
                                format!("stash_menu_indicator_{}_{}", repo_id.0, index).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::StashMenu {
                                            repo_id,
                                            index,
                                            message: stash_message_for_indicator.clone(),
                                        },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            this.store.dispatch(Msg::ApplyStash { repo_id, index });
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::StashMenu {
                                        repo_id,
                                        index,
                                        message: stash_message_for_right_click.clone(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::Placeholder {
                    section: _,
                    message,
                } => div()
                    .id(("branch_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreesHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_worktrees_spinner = this.active_repo().is_some_and(|r| {
                        r.worktrees_in_flight > 0
                            || matches!(r.worktrees, Loadable::Loading)
                            || (!collapsed && matches!(r.worktrees, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("worktrees_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("worktrees_section_row_{}", repo_id.0).into();

                    div()
                        .id(("worktrees_section", ix))
                        .debug_selector(move || format!("worktrees_section_{ix}"))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Worktrees"),
                        )
                        .when(show_worktrees_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("worktrees_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("worktrees_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .child(
                            context_menu_indicator(
                                format!("worktrees_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::SectionMenu,
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Worktrees (Add / Refresh / Open / Remove)".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::SectionMenu,
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::WorktreePlaceholder { message } => div()
                    .id(("worktree_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::WorktreeItem {
                    path,
                    branch,
                    detached,
                    is_active,
                } => {
                    let branch = branch.clone();
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let branch_for_indicator = branch.as_ref().map(|name| name.to_string());
                    let branch_for_menu = branch.as_ref().map(|name| name.to_string());
                    let path_label = this.cached_path_display(&path);
                    let label = super::super::branch_sidebar::branch_sidebar_worktree_label(
                        branch.as_ref().map(SharedString::as_ref),
                        detached,
                        path_label.as_ref(),
                    );
                    let context_menu_invoker: SharedString =
                        format!("worktree_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let path_for_indicator = path.clone();
                    let row_group: SharedString =
                        format!("worktree_row_{}_{}", repo_id.0, ix).into();

                    div()
                        .id(("worktree_item", ix))
                        .relative()
                        .h(px(22.0))
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_active, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .child(left_divider(
                                with_alpha(theme.colors.accent, 0.90),
                                px(theme.radii.row),
                            ))
                        })
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(WORKTREE_ICON_PATH, icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label.clone()),
                        )
                        .child(
                            context_menu_indicator(
                                format!("worktree_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: path_for_indicator.clone(),
                                                branch: branch_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            this.store.dispatch(Msg::OpenRepo(path_for_open.clone()));
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::worktree(
                                        repo_id,
                                        WorktreePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                            branch: branch_for_menu.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |= this
                                    .set_tooltip_text_if_changed(Some(label.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&label, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulesHeader {
                    top_border,
                    collapsed,
                    collapse_key,
                } => {
                    let show_submodules_spinner = this.active_repo().is_some_and(|r| {
                        matches!(r.submodules, Loadable::Loading)
                            || (!collapsed && matches!(r.submodules, Loadable::NotLoaded))
                    });
                    let context_menu_invoker: SharedString =
                        format!("submodules_section_menu_{}", repo_id.0).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let row_group: SharedString =
                        format!("submodules_section_row_{}", repo_id.0).into();

                    div()
                        .id(("submodules_section", ix))
                        .debug_selector(move || format!("submodules_section_{ix}"))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .bg(theme.colors.surface_bg_elevated)
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .when(top_border, |d| d.child(top_divider(theme.colors.border)))
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.colors.text)
                                .child("Submodules"),
                        )
                        .when(show_submodules_spinner, |d| {
                            d.child(
                                div()
                                    .debug_selector(move || {
                                        format!("submodules_spinner_{}", repo_id.0)
                                    })
                                    .child(svg_spinner(
                                        ("submodules_spinner", repo_id.0),
                                        icon_muted,
                                        12.0,
                                    )),
                            )
                        })
                        .child(
                            context_menu_indicator(
                                format!("submodules_section_menu_indicator_{}", repo_id.0).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::submodule(
                                            repo_id,
                                            SubmodulePopoverKind::SectionMenu,
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_hover(cx.listener(|this, hovering: &bool, _w, cx| {
                            let text: SharedString =
                                "Submodules (Add / Update / Open / Remove)".into();
                            let mut changed = false;
                            if *hovering {
                                changed |= this.set_tooltip_text_if_changed(Some(text), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&text, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::SectionMenu,
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::SubmodulePlaceholder { message } => div()
                    .id(("submodule_placeholder", ix))
                    .h(px(22.0))
                    .w_full()
                    .px_2()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(message)
                    .into_any_element(),
                BranchSidebarRow::SubmoduleItem { path } => {
                    let path_for_open = path.clone();
                    let path_for_menu = path.clone();
                    let repo_workdir_for_open = repo_workdir.clone();
                    let path_label = this.cached_path_display(&path);
                    let tooltip = path_label.clone();
                    let context_menu_invoker: SharedString =
                        format!("submodule_menu_{}_{}", repo_id.0, path.display()).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let path_for_indicator = path.clone();
                    let row_group: SharedString =
                        format!("submodule_row_{}_{}", repo_id.0, ix).into();

                    div()
                        .id(("submodule_item", ix))
                        .relative()
                        .h(px(22.0))
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(0))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot("icons/box.svg", icon_primary, 12.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(path_label),
                        )
                        .child(
                            context_menu_indicator(
                                format!("submodule_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::submodule(
                                            repo_id,
                                            SubmodulePopoverKind::Menu {
                                                path: path_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() < 2 {
                                return;
                            }
                            let Some(base) = repo_workdir_for_open.clone() else {
                                return;
                            };
                            this.store
                                .dispatch(Msg::OpenRepo(base.join(&path_for_open)));
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::submodule(
                                        repo_id,
                                        SubmodulePopoverKind::Menu {
                                            path: path_for_menu.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::RemoteHeader {
                    name,
                    collapsed,
                    collapse_key,
                } => {
                    let remote_color = branch_tree_color(BranchSection::Remote);
                    let remote_name: String = name.as_ref().to_owned();
                    let context_menu_invoker: SharedString =
                        format!("remote_menu_{}_{}", repo_id.0, remote_name).into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let remote_name_for_right_click: String = name.as_ref().to_owned();
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let remote_name_for_indicator: String = name.as_ref().to_owned();
                    let row_group: SharedString =
                        format!("remote_header_row_{}_{}", repo_id.0, remote_name).into();

                    div()
                        .id(("branch_remote", ix))
                        .relative()
                        .h(px(24.0))
                        .w_full()
                        .pl(indent_px(0))
                        .pr_2()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .rounded(px(theme.radii.row))
                        .cursor(CursorStyle::PointingHand)
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| s.bg(theme.colors.active))
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(remote_color)
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/folder.svg", remote_color, 14.0))
                        .child(div().flex_1().min_w(px(0.0)).line_clamp(1).child(name))
                        .child(
                            context_menu_indicator(
                                format!("remote_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::remote(
                                            repo_id,
                                            RemotePopoverKind::Menu {
                                                name: remote_name_for_indicator.clone(),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::remote(
                                        repo_id,
                                        RemotePopoverKind::Menu {
                                            name: remote_name_for_right_click.clone(),
                                        },
                                    ),
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .into_any_element()
                }
                BranchSidebarRow::GroupHeader {
                    label,
                    section,
                    depth,
                    collapsed,
                    collapse_key,
                } => {
                    let group_text_color = branch_tree_color(section);
                    let group_icon_color = match section {
                        BranchSection::Local => icon_primary,
                        BranchSection::Remote => theme.colors.text_muted,
                    };
                    div()
                        .id(("branch_group", ix))
                        .h(px(22.0))
                        .w_full()
                        .pl(indent_px(usize::from(depth)))
                        .pr_2()
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .rounded(px(theme.radii.row))
                        .cursor(CursorStyle::PointingHand)
                        .hover(move |s| s.bg(theme.colors.hover))
                        .active(move |s| s.bg(theme.colors.active))
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(group_text_color)
                        .child(tree_toggle_slot(Some(collapsed)))
                        .child(tree_icon_slot("icons/folder.svg", group_icon_color, 14.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .child(label),
                        )
                        .on_click(cx.listener(move |this, e: &ClickEvent, _w, cx| {
                            if !e.standard_click() || e.click_count() != 1 {
                                return;
                            }
                            this.toggle_active_repo_collapse_key(collapse_key.clone(), cx);
                        }))
                        .into_any_element()
                }
                BranchSidebarRow::Branch {
                    name,
                    section,
                    depth,
                    muted,
                    divergence_ahead,
                    divergence_behind,
                    is_head,
                    is_upstream,
                } => {
                    let full_name_for_checkout: SharedString = name.clone();
                    let full_name_for_reveal: SharedString = name.clone();
                    let full_name_for_menu: SharedString = name.clone();
                    let full_name_for_tooltip: SharedString = name.clone();
                    let section_key = match section {
                        BranchSection::Local => "local",
                        BranchSection::Remote => "remote",
                    };
                    let context_menu_invoker: SharedString = format!(
                        "branch_menu_{}_{}_{}",
                        repo_id.0,
                        section_key,
                        full_name_for_menu.as_ref()
                    )
                    .into();
                    let context_menu_active =
                        this.active_context_menu_invoker.as_ref() == Some(&context_menu_invoker);
                    let context_menu_invoker_for_right_click = context_menu_invoker.clone();
                    let label: SharedString =
                        super::super::branch_sidebar::branch_sidebar_branch_label(name.as_ref())
                            .to_owned()
                            .into();
                    let workspace_path = (section == BranchSection::Local)
                        .then(|| workspace_badges.listed_path(name.as_ref()).cloned())
                        .flatten();
                    let active_workspace_path = (section == BranchSection::Local)
                        .then(|| workspace_badges.active_path(name.as_ref()).cloned())
                        .flatten();
                    let workspace_badge_path = branch_workspace_badge_path(
                        workspace_path.as_deref(),
                        active_workspace_path.as_deref(),
                    );
                    let branch_selected = branch_row_is_selected(
                        selected_branch.as_ref(),
                        repo_id,
                        section,
                        full_name_for_reveal.as_ref(),
                        selected_commit.as_ref(),
                        selected_branch_commit_id.as_ref(),
                    );
                    let has_worktree = workspace_badge_path.is_some();
                    let has_active_workspace = active_workspace_path.is_some();
                    let show_workspace_badge = has_worktree;
                    let show_branch_context_menu_indicator = !has_worktree;
                    let workspace_row_menu_invoker: Option<SharedString> =
                        workspace_badge_path.as_ref().map(|path| {
                            format!("worktree_menu_{}_{}", repo_id.0, path.display()).into()
                        });
                    let workspace_menu_active = workspace_row_menu_invoker
                        .as_ref()
                        .is_some_and(|invoker| this.active_context_menu_invoker.as_ref() == Some(invoker));
                    let context_menu_invoker_for_indicator = context_menu_invoker.clone();
                    let full_name_for_indicator: SharedString = name.clone();
                    let row_group: SharedString = format!("branch_row_{}_{}", repo_id.0, ix).into();
                    let branch_has_right_metadata = has_active_workspace
                        || (is_upstream
                        && section == BranchSection::Remote)
                        || divergence_behind.is_some()
                        || divergence_ahead.is_some();
                    let branch_text_color = if muted {
                        theme.colors.text_muted
                    } else {
                        branch_tree_color(section)
                    };
                    let branch_selected_bg = selected_branch_row_bg(theme);
                    let branch_selected_label_color = if branch_selected {
                        selected_branch_label_color(theme)
                    } else {
                        branch_text_color
                    };
                    let branch_icon_color = match section {
                        BranchSection::Local => {
                            if muted {
                                icon_muted
                            } else {
                                icon_primary
                            }
                        }
                        BranchSection::Remote => theme.colors.text_muted,
                    };
                    let worktree_action_bg = gpui::rgba(0x00000000);
                    let worktree_action_active_bg = gpui::rgba(0x00000000);
                    let worktree_action_border = with_alpha(
                        theme.colors.text_muted,
                        if theme.is_dark { 0.38 } else { 0.28 },
                    );
                    let worktree_action_hover_border = with_alpha(
                        theme.colors.text_muted,
                        if theme.is_dark { 0.55 } else { 0.40 },
                    );
                    let worktree_action_open_border = with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.56 } else { 0.34 },
                    );
                    let worktree_action_open_hover_border = with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.72 } else { 0.46 },
                    );
                    let worktree_action_active_border = with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.84 } else { 0.68 },
                    );
                    let worktree_action_text = theme.colors.text_muted;
                    let worktree_action_hover_text = theme.colors.text;
                    let worktree_action_open_text = theme.colors.accent;
                    let worktree_action_active_text = theme.colors.accent;
                    let mut row = div()
                        .id(("branch_item", ix))
                        .relative()
                        .h(if section == BranchSection::Local {
                            px(24.0)
                        } else {
                            px(22.0)
                        })
                        .w_full()
                        .group(row_group.clone())
                        .flex()
                        .items_center()
                        .gap(px(BRANCH_TREE_GAP_PX))
                        .pl(indent_px(usize::from(depth)))
                        .pr_2()
                        .rounded(px(theme.radii.row))
                        .when(is_head, |d| {
                            d.bg(with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                            .child(left_divider(
                                with_alpha(theme.colors.accent, 0.90),
                                px(theme.radii.row),
                            ))
                        })
                        .when(branch_selected, |d| d.bg(branch_selected_bg))
                        .when(context_menu_active, |d| d.bg(theme.colors.active))
                        .hover(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else if branch_selected {
                                s.bg(branch_selected_bg)
                            } else {
                                s.bg(theme.colors.hover)
                            }
                        })
                        .active(move |s| {
                            if context_menu_active {
                                s.bg(theme.colors.active)
                            } else if branch_selected {
                                s.bg(branch_selected_bg)
                            } else {
                                s.bg(theme.colors.active)
                            }
                        })
                        .text_color(branch_text_color)
                        .child(tree_toggle_slot(None))
                        .child(tree_icon_slot(
                            "icons/git_branch.svg",
                            branch_icon_color,
                            12.0,
                        ))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_sm()
                                .line_clamp(1)
                                .whitespace_nowrap()
                                .text_color(branch_selected_label_color)
                                .child(label),
                        );

                    let mut right = div().flex().items_center().gap_2().ml_auto().when(
                        branch_has_right_metadata && show_branch_context_menu_indicator,
                        |d| {
                            d.pr(px(CONTEXT_MENU_INDICATOR_SIZE_PX
                                + CONTEXT_MENU_INDICATOR_RIGHT_PX
                                + 4.0))
                        },
                    );

                    if is_upstream && section == BranchSection::Remote {
                        right = right.child(
                            div()
                                .px(px(3.0))
                                .py(px(0.0))
                                .rounded(px(2.0))
                                .text_size(px(11.0))
                                .text_color(theme.colors.text_muted)
                                .bg(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.16 } else { 0.10 },
                                ))
                                .border_1()
                                .border_color(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.32 } else { 0.22 },
                                ))
                                .child("Upstream"),
                        );
                    }

                    if divergence_behind.is_some() || divergence_ahead.is_some() {
                        if let Some(behind) = divergence_behind {
                            let color = theme.colors.warning;
                            right = right.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color)
                                    .child(svg_icon("icons/arrow_down.svg", color, 11.0))
                                    .child(super::super::branch_sidebar::branch_sidebar_divergence_label(
                                        behind,
                                    )),
                            );
                        }
                        if let Some(ahead) = divergence_ahead {
                            let color = theme.colors.success;
                            right = right.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color)
                                    .child(svg_icon("icons/arrow_up.svg", color, 11.0))
                                    .child(super::super::branch_sidebar::branch_sidebar_divergence_label(
                                        ahead,
                                    )),
                            );
                        }
                    }

                    if show_workspace_badge {
                        let Some(workspace_badge_path) = workspace_badge_path.clone() else {
                            unreachable!("workspace badge requires a worktree path");
                        };
                        let workspace_menu_invoker_for_click = workspace_row_menu_invoker.clone();
                        let workspace_menu_invoker_for_right_click =
                            workspace_row_menu_invoker.clone();
                        let workspace_path_for_menu = workspace_badge_path.clone();
                        let workspace_path_for_open = workspace_badge_path.clone();
                        let workspace_badge_label =
                            super::super::path_display::repo_path_name(&workspace_badge_path);
                        let worktree_badge_tooltip: SharedString =
                            workspace_badge_path.display().to_string().into();
                        let branch_name_for_click = name.to_string();
                        let branch_name_for_right_click = branch_name_for_click.clone();
                        let badge_border = if workspace_menu_active {
                            worktree_action_active_border
                        } else if has_active_workspace {
                            worktree_action_open_border
                        } else {
                            worktree_action_border
                        };
                        let badge_hover_border = if has_active_workspace {
                            worktree_action_open_hover_border
                        } else {
                            worktree_action_hover_border
                        };
                        let badge_text = if workspace_menu_active {
                            worktree_action_active_text
                        } else if has_active_workspace {
                            worktree_action_open_text
                        } else {
                            worktree_action_text
                        };
                        let badge_hover_text = if has_active_workspace {
                            worktree_action_open_text
                        } else {
                            worktree_action_hover_text
                        };
                        right = right.child(
                            div()
                                .id(("branch_workspace_badge", ix))
                                .debug_selector(move || format!("branch_workspace_badge_{ix}"))
                                .flex()
                                .items_center()
                                .gap(px(3.0))
                                .px(px(4.0))
                                .py(px(0.0))
                                .rounded(px(2.0))
                                .border_1()
                                .border_color(badge_border)
                                .bg(worktree_action_bg)
                                .cursor(CursorStyle::PointingHand)
                                .text_size(px(11.0))
                                .text_color(badge_text)
                                .hover(move |s| {
                                    if workspace_menu_active {
                                        s.bg(worktree_action_active_bg)
                                            .border_color(worktree_action_active_border)
                                            .text_color(worktree_action_active_text)
                                    } else {
                                        s.bg(worktree_action_bg)
                                            .border_color(badge_hover_border)
                                            .text_color(badge_hover_text)
                                    }
                                })
                                .child(svg_icon(WORKTREE_ICON_PATH, badge_text, 9.0))
                                .child(workspace_badge_label.clone())
                                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    if e.click_count() >= 2 {
                                        this.store.dispatch(Msg::OpenRepo(
                                            workspace_path_for_open.clone(),
                                        ));
                                        cx.notify();
                                        return;
                                    }
                                    let Some(invoker) =
                                        workspace_menu_invoker_for_click.clone()
                                    else {
                                        return;
                                    };
                                    this.activate_context_menu_invoker(invoker, cx);
                                    this.open_popover_at(
                                        PopoverKind::worktree(
                                            repo_id,
                                            WorktreePopoverKind::Menu {
                                                path: workspace_path_for_menu.clone(),
                                                branch: Some(branch_name_for_click.clone()),
                                            },
                                        ),
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                }))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        let Some(invoker) =
                                            workspace_menu_invoker_for_right_click.clone()
                                        else {
                                            return;
                                        };
                                        this.activate_context_menu_invoker(invoker, cx);
                                        this.open_popover_at(
                                            PopoverKind::worktree(
                                                repo_id,
                                                WorktreePopoverKind::Menu {
                                                    path: workspace_badge_path.clone(),
                                                    branch: Some(branch_name_for_right_click.clone()),
                                                },
                                            ),
                                            e.position,
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                                .on_hover(cx.listener(
                                    move |this, hovering: &bool, _w, cx| {
                                        let mut changed = false;
                                        if *hovering {
                                            changed |= this.set_tooltip_text_if_changed(
                                                Some(worktree_badge_tooltip.clone()),
                                                cx,
                                            );
                                        } else {
                                            changed |= this.clear_tooltip_if_matches(
                                                &worktree_badge_tooltip,
                                                cx,
                                            );
                                        }
                                        if changed {
                                            cx.notify();
                                        }
                                    },
                                )),
                        );
                    }

                    row = row.child(right);
                    if show_branch_context_menu_indicator {
                        row = row.child(
                            context_menu_indicator(
                                format!("branch_menu_indicator_{}_{}", repo_id.0, ix).into(),
                                row_group.clone(),
                                context_menu_active,
                                context_menu_active,
                            )
                            .on_click(cx.listener(
                                move |this, e: &ClickEvent, window, cx| {
                                    if !e.standard_click() || e.click_count() != 1 {
                                        return;
                                    }
                                    cx.stop_propagation();
                                    this.activate_context_menu_invoker(
                                        context_menu_invoker_for_indicator.clone(),
                                        cx,
                                    );
                                    this.open_popover_at(
                                        PopoverKind::BranchMenu {
                                            repo_id,
                                            section,
                                            name: full_name_for_indicator.as_ref().to_owned(),
                                        },
                                        e.position(),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        );
                    }

                    row = row
                        .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                            if !e.standard_click() {
                                return;
                            }
                            if e.click_count() == 1 {
                                let Some(target) = this.active_repo().and_then(|repo| {
                                    branch_click_history_reveal_target(
                                        repo,
                                        section,
                                        full_name_for_reveal.as_ref(),
                                        is_head,
                                    )
                                }) else {
                                    return;
                                };
                                this.set_selected_branch(
                                    repo_id,
                                    section,
                                    full_name_for_reveal.as_ref(),
                                    cx,
                                );
                                this.reveal_branch_commit_in_history(
                                    repo_id,
                                    section,
                                    full_name_for_reveal.as_ref(),
                                    target.commit_id,
                                    target.desired_scope,
                                    cx,
                                );
                                cx.notify();
                                return;
                            }
                            if e.click_count() < 2 {
                                return;
                            }
                            match section {
                                BranchSection::Local => {
                                    match local_branch_double_click_action(
                                        full_name_for_checkout.as_ref(),
                                        workspace_path.as_deref(),
                                    ) {
                                        LocalBranchDoubleClickAction::CheckoutBranch { name } => {
                                            this.store.dispatch(Msg::CheckoutBranch {
                                                repo_id,
                                                name,
                                            });
                                            this.rebuild_diff_cache(cx);
                                            cx.notify();
                                        }
                                        LocalBranchDoubleClickAction::OpenWorkspace { path } => {
                                            this.store.dispatch(Msg::OpenRepo(path));
                                            cx.notify();
                                        }
                                    }
                                }
                                BranchSection::Remote => {
                                    if let Some((remote, branch)) =
                                        full_name_for_checkout.as_ref().split_once('/')
                                    {
                                        this.open_popover_at(
                                            PopoverKind::CheckoutRemoteBranchPrompt {
                                                repo_id,
                                                remote: remote.to_string(),
                                                branch: branch.to_string(),
                                            },
                                            e.position(),
                                            window,
                                            cx,
                                        );
                                        cx.notify();
                                    }
                                }
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.activate_context_menu_invoker(
                                    context_menu_invoker_for_right_click.clone(),
                                    cx,
                                );
                                this.open_popover_at(
                                    PopoverKind::BranchMenu {
                                        repo_id,
                                        section,
                                        name: full_name_for_menu.as_ref().to_owned(),
                                    },
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                            let branch_tooltip =
                                super::super::branch_sidebar::branch_sidebar_branch_tooltip(
                                    full_name_for_tooltip.as_ref(),
                                    is_upstream,
                                );
                            let mut changed = false;
                            if *hovering {
                                changed |=
                                    this.set_tooltip_text_if_changed(Some(branch_tooltip), cx);
                            } else {
                                changed |= this.clear_tooltip_if_matches(&branch_tooltip, cx);
                            }
                            if changed {
                                cx.notify();
                            }
                        }));

                    row.into_any_element()
                }
            })
            .collect()
    }
}

impl DetailsPaneView {
    pub(in super::super) fn render_commit_file_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return Vec::new();
        };

        let theme = this.theme;
        let repo_id = repo.id;
        let has_active_menu = this.active_context_menu_invoker.is_some();
        let file_rows = this.cached_commit_file_rows(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );

        range
            .filter_map(|ix| {
                details
                    .files
                    .get(ix)
                    .zip(file_rows.get(ix))
                    .map(|(f, row)| (ix, f, row.label.clone(), row.visuals))
            })
            .map(|(ix, f, path_label, visuals)| {
                let commit_id = details.id.clone();
                let icon = Some(visuals.icon);
                let color = visuals.color(&theme);

                let context_menu_active = has_active_menu && {
                    let invoker: SharedString = format!(
                        "commit_file_menu_{}_{}_{}",
                        repo_id.0,
                        commit_id.as_ref(),
                        f.path.display()
                    )
                    .into();
                    this.active_context_menu_invoker.as_ref() == Some(&invoker)
                };
                let selected = repo
                    .diff_state
                    .diff_target
                    .as_ref()
                    .is_some_and(|t| match t {
                        DiffTarget::Commit {
                            commit_id: t_commit_id,
                            path: Some(t_path),
                        } => t_commit_id == &commit_id && t_path == &f.path,
                        _ => false,
                    });
                let commit_id_for_click = commit_id.clone();
                let path_for_click = f.path.clone();
                let commit_id_for_menu = commit_id.clone();
                let path_for_menu = f.path.clone();
                let tooltip = path_label.clone();

                let mut row = div()
                    .id(("commit_file", ix))
                    .h(px(24.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .w_full()
                    .rounded(px(theme.radii.row))
                    .cursor(CursorStyle::PointingHand)
                    .hover(move |s| {
                        if context_menu_active {
                            s.bg(theme.colors.active)
                        } else {
                            s.bg(theme.colors.hover)
                        }
                    })
                    .active(move |s| s.bg(theme.colors.active))
                    .child(
                        div()
                            .w(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(icon, |this, icon| {
                                this.child(svg_icon(icon, color, px(14.0)))
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .line_clamp(1)
                            .whitespace_nowrap()
                            .child(path_label),
                    )
                    .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
                        this.focus_diff_panel(window, cx);
                        this.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::Commit {
                                commit_id: commit_id_for_click.clone(),
                                path: Some(path_for_click.clone()),
                            },
                        });
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |this, hovering: &bool, _w, cx| {
                        let mut changed = false;
                        if *hovering {
                            changed |= this.set_tooltip_text_if_changed(Some(tooltip.clone()), cx);
                        } else {
                            changed |= this.clear_tooltip_if_matches(&tooltip, cx);
                        }
                        if changed {
                            cx.notify();
                        }
                    }));
                row = row.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.store.dispatch(Msg::SelectDiff {
                            repo_id,
                            target: DiffTarget::Commit {
                                commit_id: commit_id_for_menu.clone(),
                                path: Some(path_for_menu.clone()),
                            },
                        });
                        let invoker: SharedString = format!(
                            "commit_file_menu_{}_{}_{}",
                            repo_id.0,
                            commit_id_for_menu.as_ref(),
                            path_for_menu.display()
                        )
                        .into();
                        this.activate_context_menu_invoker(invoker, cx);
                        this.open_popover_at(
                            PopoverKind::CommitFileMenu {
                                repo_id,
                                commit_id: commit_id_for_menu.clone(),
                                path: path_for_menu.clone(),
                            },
                            e.position,
                            window,
                            cx,
                        );
                        cx.notify();
                    }),
                );

                if selected {
                    row = row.bg(with_alpha(
                        theme.colors.accent,
                        if theme.is_dark { 0.16 } else { 0.10 },
                    ));
                }
                if context_menu_active {
                    row = row.bg(theme.colors.active);
                }

                row.into_any_element()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{
        Branch, Commit, CommitId, DiffTarget, LogPage, RemoteBranch, RepoSpec, Worktree,
    };
    use gitcomet_core::services::{GitBackend, GitRepository, Result};
    use gitcomet_state::msg::{InternalMsg, Msg};
    use gitcomet_state::store::AppStore;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime};

    struct BlockingBackend;

    impl GitBackend for BlockingBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            loop {
                std::thread::park();
            }
        }
    }

    fn wait_until(
        cx: &mut gpui::VisualTestContext,
        description: &str,
        ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            if ready(cx) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for {description}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn sync_view_for_tests(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    fn commit_id(id: &str) -> CommitId {
        CommitId(id.into())
    }

    fn commit(id: &str) -> Commit {
        Commit {
            id: commit_id(id),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: id.into(),
            author: "author".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn listed_workspace_paths_by_branch_includes_closed_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo"),
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-detached"),
                head: None,
                branch: None,
                detached: true,
            },
        ]));

        let paths = listed_workspace_paths_by_branch(&repo);

        assert_eq!(
            paths.get("feature"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
        assert!(!paths.contains_key("main"));
        assert!(!paths.contains_key("repo-detached"));
    }

    #[test]
    fn listed_workspace_paths_by_branch_prefers_first_branch_match() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-a"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-b"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
        ]));

        let paths = listed_workspace_paths_by_branch(&repo);

        assert_eq!(
            paths.get("feature/shared"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
        );
    }

    #[test]
    fn active_workspace_paths_by_branch_only_includes_open_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo"),
                head: None,
                branch: Some("main".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
                head: None,
                branch: Some("feature".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-detached"),
                head: None,
                branch: None,
                detached: true,
            },
        ]));

        let mut open_main = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        open_main.head_branch = Loadable::Ready("main".to_string());
        let mut open_feature = RepoState::new_opening(
            RepoId(3),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_feature.head_branch = Loadable::Ready("feature".to_string());

        let active = active_workspace_paths_by_branch(&repo, &[open_main, open_feature]);

        assert_eq!(
            active.get("main"),
            Some(&std::path::PathBuf::from("/tmp/repo"))
        );
        assert_eq!(
            active.get("feature"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
        assert!(!active.contains_key("repo-detached"));
    }

    #[test]
    fn active_workspace_paths_by_branch_skips_closed_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature".to_string()),
            detached: false,
        }]));

        let active = active_workspace_paths_by_branch(&repo, &[]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_by_branch_uses_open_repo_head_branch_for_live_updates() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));

        let mut open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_worktree.head_branch = Loadable::Ready("feature/new".to_string());
        open_worktree.head_branch_rev = 1;

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert!(!active.contains_key("feature/old"));
        assert_eq!(
            active.get("feature/new"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn active_workspace_paths_by_branch_falls_back_to_listed_branch_while_head_is_loading() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/listed".to_string()),
            detached: false,
        }]));

        let open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert_eq!(
            active.get("feature/listed"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature"))
        );
    }

    #[test]
    fn active_workspace_paths_by_branch_hides_detached_open_worktrees() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
            path: std::path::PathBuf::from("/tmp/repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }]));

        let mut open_worktree = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature"),
            },
        );
        open_worktree.head_branch = Loadable::Ready("HEAD".to_string());
        open_worktree.head_branch_rev = 1;
        open_worktree.detached_head_commit = Some(CommitId("deadbeef".into()));

        let active = active_workspace_paths_by_branch(&repo, &[open_worktree]);

        assert!(active.is_empty());
    }

    #[test]
    fn active_workspace_paths_by_branch_keeps_first_listed_workspace_for_branch() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        );
        repo.worktrees = Loadable::Ready(Arc::new(vec![
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-a"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
            Worktree {
                path: std::path::PathBuf::from("/tmp/repo-feature-b"),
                head: None,
                branch: Some("feature/shared".to_string()),
                detached: false,
            },
        ]));

        let mut open_first = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature-a"),
            },
        );
        open_first.head_branch = Loadable::Ready("feature/shared".to_string());

        let mut open_second = RepoState::new_opening(
            RepoId(3),
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo-feature-b"),
            },
        );
        open_second.head_branch = Loadable::Ready("feature/shared".to_string());

        let active = active_workspace_paths_by_branch(&repo, &[open_first, open_second]);

        assert_eq!(
            active.get("feature/shared"),
            Some(&std::path::PathBuf::from("/tmp/repo-feature-a"))
        );
    }

    #[test]
    fn branch_workspace_badge_path_prefers_listed_workspace_and_falls_back_to_active() {
        assert_eq!(
            branch_workspace_badge_path(
                Some(std::path::Path::new("/tmp/repo-feature-listed")),
                Some(std::path::Path::new("/tmp/repo-feature-open")),
            ),
            Some(std::path::PathBuf::from("/tmp/repo-feature-listed"))
        );
        assert_eq!(
            branch_workspace_badge_path(None, Some(std::path::Path::new("/tmp/repo-feature-open")),),
            Some(std::path::PathBuf::from("/tmp/repo-feature-open"))
        );
    }

    #[test]
    fn local_branch_double_click_checks_out_when_no_workspace_is_open() {
        assert_eq!(
            local_branch_double_click_action("feature/workspace", None),
            LocalBranchDoubleClickAction::CheckoutBranch {
                name: "feature/workspace".to_string(),
            }
        );
    }

    #[test]
    fn local_branch_double_click_opens_workspace_when_branch_has_active_workspace() {
        assert_eq!(
            local_branch_double_click_action(
                "feature/workspace",
                Some(std::path::Path::new("/tmp/repo-feature"))
            ),
            LocalBranchDoubleClickAction::OpenWorkspace {
                path: std::path::PathBuf::from("/tmp/repo-feature"),
            }
        );
    }

    #[test]
    fn branch_row_selection_requires_matching_clicked_branch_identity() {
        let target = commit_id("shared-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&target),
            Some(&target)
        ));
        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Remote,
            "origin/main",
            Some(&target),
            Some(&target)
        ));
    }

    #[test]
    fn branch_row_selection_requires_matching_selected_commit() {
        let target = commit_id("main-tip");
        let other = commit_id("other-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&other),
            Some(&target)
        ));
        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            None,
            Some(&target)
        ));
    }

    #[test]
    fn branch_row_selection_requires_resolved_selected_branch_tip() {
        let target = commit_id("main-tip");
        let selected_branch = SelectedBranch {
            repo_id: RepoId(1),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert!(!branch_row_is_selected(
            Some(&selected_branch),
            RepoId(1),
            BranchSection::Local,
            "main",
            Some(&target),
            None
        ));
    }

    #[test]
    fn branch_click_history_reveal_target_keeps_current_scope_for_head_local_branch() {
        let target = commit_id("main-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]));

        assert_eq!(
            branch_click_history_reveal_target(&repo, BranchSection::Local, "main", true),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                desired_scope: LogScope::CurrentBranch,
            })
        );
    }

    #[test]
    fn branch_click_history_reveal_target_switches_non_head_local_branch_to_all_branches() {
        let target = commit_id("feature-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "feature".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]));

        assert_eq!(
            branch_click_history_reveal_target(&repo, BranchSection::Local, "feature", false),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                desired_scope: LogScope::AllBranches,
            })
        );
    }

    #[test]
    fn branch_click_history_reveal_target_switches_remote_branch_to_all_branches() {
        let target = commit_id("origin-feature-tip");
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.history_state.history_scope = LogScope::CurrentBranch;
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/topic".to_string(),
            target: target.clone(),
        }]));

        assert_eq!(
            branch_click_history_reveal_target(
                &repo,
                BranchSection::Remote,
                "origin/feature/topic",
                false,
            ),
            Some(BranchHistoryRevealTarget {
                commit_id: target,
                desired_scope: LogScope::AllBranches,
            })
        );
    }

    #[gpui::test]
    fn branch_reveal_routes_through_main_pane_and_selects_commit(cx: &mut gpui::TestAppContext) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let sync_view_from_store = |cx: &mut gpui::VisualTestContext| {
            cx.update(|window, app| {
                view.update(app, |this, cx| {
                    crate::view::test_support::sync_store_snapshot(this, cx)
                });
                window.refresh();
                let _ = window.draw(app);
            });
        };

        let repo_id = RepoId(1);
        let target = commit_id("main-tip");
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from("/tmp/repo")));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });
        sync_view_from_store(cx);

        store_for_assert.dispatch(Msg::Internal(InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("main".to_string()),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![Branch {
                name: "main".to_string(),
                target: target.clone(),
                upstream: None,
                divergence: None,
            }]),
        }));
        store_for_assert.dispatch(Msg::Internal(InternalMsg::LogLoaded {
            repo_id,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![commit("main-tip")],
                next_cursor: None,
            }),
        }));
        store_for_assert.dispatch(Msg::SelectDiff {
            repo_id,
            target: DiffTarget::Commit {
                commit_id: commit_id("previous"),
                path: None,
            },
        });
        wait_until(cx, "sidebar repo data", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            matches!(repo.head_branch, Loadable::Ready(ref head) if head == "main")
                && matches!(repo.branches, Loadable::Ready(_))
                && matches!(repo.log, Loadable::Ready(_))
                && repo.diff_state.diff_target.is_some()
        });
        sync_view_from_store(cx);

        wait_until(cx, "history view active repo", |cx| {
            sync_view_for_tests(cx, &view);
            cx.update(|_window, app| {
                let (sidebar_pane, main_pane) = {
                    let root = view.read(app);
                    (root.sidebar_pane.clone(), root.main_pane.clone())
                };
                let history_view = main_pane.read(app).history_view.clone();

                sidebar_pane.read(app).active_repo_id() == Some(repo_id)
                    && main_pane.read(app).active_repo_id() == Some(repo_id)
                    && history_view.read(app).active_repo_id() == Some(repo_id)
            })
        });

        sync_view_for_tests(cx, &view);
        let sidebar_pane = cx.update(|_window, app| view.read(app).sidebar_pane.clone());
        cx.update(|window, app| {
            sidebar_pane.update(app, |pane, cx| {
                pane.reveal_branch_commit_in_history(
                    repo_id,
                    BranchSection::Local,
                    "main",
                    target.clone(),
                    LogScope::CurrentBranch,
                    cx,
                );
            });
            let _ = window.draw(app);
        });

        wait_until(cx, "branch reveal store state", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            repo.diff_state.diff_target.is_none()
                && repo.history_state.history_scope == LogScope::CurrentBranch
                && repo.history_state.selected_commit.as_ref() == Some(&target)
        });
    }
}
