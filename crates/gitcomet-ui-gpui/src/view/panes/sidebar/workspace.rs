//! The Workspace view: virtual branches as stacks over a target branch, and
//! the working tree grouped by the branch each file belongs to.
//!
//! This is an alternative to the branch workflow, not a replacement. The
//! Branches and Files tabs are unchanged; this tab is simply a third place to
//! work from, for someone who wants several pieces of work in one directory
//! without checking any of them out.
//!
//! The rows are computed from `RepoState::workspace` and cached behind a
//! fingerprint, the same way the branch tree is, because the pane re-renders on
//! every store notification and rebuilding stacks per frame would show up as a
//! stutter in a long stack list.

use super::*;
use crate::view::panels::popover::workspace_prompt::{
    assign_file_prompt, commit_file_prompt, default_commit_message,
};
use gitcomet_core::workspace::{BranchApplyState, VirtualBranch};
use gitcomet_state::model::{Loadable, WorkspaceEdit};
use gpui::{Div, Stateful};
use std::path::PathBuf;
use std::sync::Arc;

/// How much narrower each stacked branch is indented. Small on purpose: a deep
/// stack should still leave room for the branch name.
const STACK_INDENT_PX: f32 = 10.0;
/// File rows sit a little further in than a stacked branch, so the two kinds of
/// row are distinguishable by position alone.
const FILE_INDENT_PX: f32 = 14.0;
const ROW_GAP_PX: f32 = 6.0;

/// The branches a row can be swapped with, in the same stack.
///
/// Carried on the row rather than looked up while rendering because the arrows
/// have to be disabled at the ends of a stack, and a row that cannot move has to
/// say so before it is clicked rather than after.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct WorkspaceSiblings {
    /// The branch one row up in the stack.
    pub(in crate::view) above: Option<String>,
    /// The branch one row down in the stack.
    pub(in crate::view) below: Option<String>,
}

/// One row of the workspace list.
///
/// The list interleaves stack headers, branch rows, and file rows, so a single
/// enum keeps the index space of the uniform list simple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum WorkspaceRow {
    /// The target branch every independent branch is built on.
    Target { name: String },
    /// The first branch of a stack: the line it stands on.
    StackBase {
        name: String,
        applied: bool,
        file_count: usize,
        siblings: WorkspaceSiblings,
    },
    /// A branch stacked on another, indented by its depth.
    Branch {
        name: String,
        depth: usize,
        applied: bool,
        file_count: usize,
        description: Option<String>,
        siblings: WorkspaceSiblings,
    },
    /// The count line above the file list.
    Summary { text: String },
    /// A changed file, labelled with the branch it is assigned to.
    File {
        path: PathBuf,
        /// The branch the whole file is on; `None` for a split or unassigned one.
        branch: Option<String>,
        /// The branches a split file's hunks are on.
        split: Vec<String>,
        indented: bool,
    },
    Placeholder { message: String },
    /// The conflict banner, when the last apply failed.
    Conflict { message: String },
}

/// The whole workspace list, computed once per change rather than per frame.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct WorkspacePresentation {
    pub rows: Arc<Vec<WorkspaceRow>>,
}

/// Identifies the workspace data a presentation was built from. A new
/// fingerprint means the cached rows are stale.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct WorkspaceFingerprint {
    workspace_rev: u64,
    worktree_status_rev: u64,
    busy: bool,
}

impl WorkspaceFingerprint {
    pub(in crate::view) fn from_repo(repo: &RepoState) -> Self {
        Self {
            workspace_rev: repo.workspace.rev,
            worktree_status_rev: repo.worktree_status_rev,
            busy: repo.workspace.busy.any(),
        }
    }
}

impl WorkspacePresentation {
    /// Build the rows for `repo`, or a placeholder when there is none.
    pub(in crate::view) fn build(repo: Option<&RepoState>) -> Self {
        let Some(repo) = repo else {
            return Self::placeholder("No repository selected.");
        };
        Self {
            rows: Arc::new(build_rows(repo)),
        }
    }

    fn placeholder(message: &str) -> Self {
        Self {
            rows: Arc::new(vec![WorkspaceRow::Placeholder {
                message: message.into(),
            }]),
        }
    }
}

/// Changed paths for the repository, falling back to nothing while loading so
/// the stacks still render above an empty file list.
fn changed_paths(repo: &RepoState) -> Arc<Vec<PathBuf>> {
    match &repo.worktree_status {
        Loadable::Ready(status) => {
            Arc::new(status.iter().map(|entry| entry.path.clone()).collect())
        }
        _ => Arc::new(Vec::new()),
    }
}

fn build_rows(repo: &RepoState) -> Vec<WorkspaceRow> {
    let mut rows = Vec::new();

    if let Some(conflict) = &repo.workspace.conflict {
        rows.push(WorkspaceRow::Conflict {
            message: conflict.summary(),
        });
    }

    if matches!(repo.workspace.state, Loadable::NotLoaded) {
        rows.push(WorkspaceRow::Placeholder {
            message: "Loading workspace...".into(),
        });
        return rows;
    }
    if let Loadable::Error(error) = &repo.workspace.state {
        rows.push(WorkspaceRow::Placeholder {
            message: format!("Could not load the workspace: {error}"),
        });
        return rows;
    }

    let Some(workspace) = repo.workspace.workspace() else {
        rows.push(WorkspaceRow::Placeholder {
            message: "No workspace yet. Create a branch to start one.".into(),
        });
        return rows;
    };

    let paths = changed_paths(repo);
    let assignments = repo.workspace.files_by_branch(&paths);
    let mut counts: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for assignment in &assignments {
        // A split file is committed by every branch holding part of it, so it
        // counts under each of them. See `WorkspaceRepoState::summary_counts`.
        match &assignment.branch {
            Some(branch) => *counts.entry(branch.clone()).or_default() += 1,
            None => {
                for branch in &assignment.split {
                    *counts.entry(branch.clone()).or_default() += 1;
                }
            }
        }
    }

    // Opening this tab is supposed to move the working directory onto
    // `gitcomet/workspace`, which is the only reason an applied branch becomes
    // visible in the files. When that did not happen — a refused checkout, or a
    // workspace restored after a restart — the list below describes branches the
    // user is not actually working in, and saying so is the difference between a
    // confusing tab and an honest one.
    if !repo.workspace.active {
        rows.push(WorkspaceRow::Placeholder {
            message: "Your working directory is not on the workspace, so applied \
                      branches are not in your files."
                .into(),
        });
    }

    rows.push(WorkspaceRow::Target {
        name: workspace.target.clone(),
    });

    // The stacks were derived once when the workspace was loaded, and the
    // fingerprint below already covers that revision, so re-deriving them per
    // build would only cost time on a long stack list.
    for stack in repo.workspace.stacks.iter() {
        for (index, member) in stack.branches.iter().enumerate() {
            let count = counts.get(&member.branch.name).copied().unwrap_or(0);
            let siblings = WorkspaceSiblings {
                above: index
                    .checked_sub(1)
                    .and_then(|above| stack.branches.get(above))
                    .map(|above| above.branch.name.clone()),
                below: stack
                    .branches
                    .get(index + 1)
                    .map(|below| below.branch.name.clone()),
            };
            if index == 0 {
                rows.push(WorkspaceRow::StackBase {
                    name: member.branch.name.clone(),
                    applied: member.branch.is_applied(),
                    file_count: count,
                    siblings,
                });
            } else {
                rows.push(WorkspaceRow::Branch {
                    name: member.branch.name.clone(),
                    depth: member.depth,
                    applied: member.branch.is_applied(),
                    file_count: count,
                    description: member.branch.description.clone(),
                    siblings,
                });
            }
        }
    }

    if workspace.branches.is_empty() {
        rows.push(WorkspaceRow::Placeholder {
            message: "Create a branch to start working in the workspace.".into(),
        });
        return rows;
    }

    let summary = repo.workspace.summary_counts(&paths);
    rows.push(WorkspaceRow::Summary {
        text: format!(
            "{} of {} applied · {} unassigned",
            summary.applied, summary.total, summary.unassigned
        ),
    });

    for assignment in assignments {
        // Asked before the struct moves any of it out: a file indented under its
        // branch reads as belonging to it, an unassigned file sits at the margin
        // so the gap is visible, and a split file is indented because it *is*
        // assigned — just to more than one branch.
        let indented = assignment.is_assigned();
        rows.push(WorkspaceRow::File {
            path: assignment.path,
            branch: assignment.branch,
            split: assignment.split,
            indented,
        });
    }

    rows
}

/// What the apply control reads for a branch.
pub(in crate::view) fn apply_label(applied: BranchApplyState) -> &'static str {
    if applied.is_applied() {
        "Unapply"
    } else {
        "Apply"
    }
}

/// The apply control's tooltip, phrased in terms of what happens to the
/// working directory rather than to a branch, since that is the effect the
/// user actually sees.
pub(in crate::view) fn apply_tooltip(branch: &VirtualBranch) -> SharedString {
    if branch.is_applied() {
        SharedString::from(format!(
            "Unapply {}: remove its changes from the workspace",
            branch.name
        ))
    } else {
        SharedString::from(format!(
            "Apply {}: add its changes to the workspace",
            branch.name
        ))
    }
}

impl SidebarPaneView {
    /// The workspace rows, recomputed only when the workspace or the working
    /// tree has actually moved.
    pub(in crate::view) fn workspace_presentation_cached(&mut self) -> WorkspacePresentation {
        let fingerprint = self
            .active_repo()
            .map(WorkspaceFingerprint::from_repo)
            .unwrap_or_default();
        if fingerprint != self.workspace_fingerprint {
            self.workspace_fingerprint = fingerprint;
            // Built in its own statement so the borrow of the active repo ends
            // before the cache field is written.
            let presentation = WorkspacePresentation::build(self.active_repo());
            self.workspace_presentation = presentation;
        }
        self.workspace_presentation.clone()
    }

    /// One row of the workspace list. Registered as the uniform-list processor
    /// so a long stack list scrolls without building every row.
    pub(in crate::view) fn render_workspace_rows(
        this: &mut Self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale_percent = ui_scale::current(cx).percent;
        let row_height = workspace_row_height(theme, ui_scale_percent);
        let presentation = this.workspace_presentation_cached();
        let Some(repo_id) = this.active_repo_id() else {
            // No repository means there is nothing to dispatch an action to, so
            // only the placeholder row survives; the stack rows are never built.
            return presentation
                .rows
                .iter()
                .map(|row| match row {
                    WorkspaceRow::Placeholder { message } => {
                        render_placeholder_row(theme, row_height, message.clone())
                    }
                    _ => div().h(row_height).into_any_element(),
                })
                .collect();
        };
        let busy = this
            .active_repo()
            .is_some_and(|repo| repo.workspace.busy.any());
        let rows = presentation.rows.clone();
        let store = Arc::clone(&this.store);

        range
            .map(|ix| {
                let Some(row) = rows.get(ix) else {
                    return div().h(row_height).into_any_element();
                };
                match row {
                    WorkspaceRow::Target { name } => render_target_row(
                        theme,
                        row_height,
                        ui_scale_percent,
                        ix,
                        name.clone(),
                        busy,
                        repo_id,
                        cx,
                    ),
                    WorkspaceRow::StackBase {
                        name,
                        applied,
                        file_count,
                        siblings,
                    } => render_branch_row(
                        theme,
                        row_height,
                        ui_scale_percent,
                        ix,
                        0,
                        name.clone(),
                        *applied,
                        *file_count,
                        None,
                        siblings.clone(),
                        busy,
                        repo_id,
                        Arc::clone(&store),
                        cx,
                    ),
                    WorkspaceRow::Branch {
                        name,
                        depth,
                        applied,
                        file_count,
                        description,
                        siblings,
                    } => render_branch_row(
                        theme,
                        row_height,
                        ui_scale_percent,
                        ix,
                        *depth,
                        name.clone(),
                        *applied,
                        *file_count,
                        description.clone(),
                        siblings.clone(),
                        busy,
                        repo_id,
                        Arc::clone(&store),
                        cx,
                    ),
                    WorkspaceRow::Summary { text } => render_summary_row(
                        theme,
                        row_height,
                        ix,
                        text.clone(),
                        busy,
                        repo_id,
                        cx,
                    ),
                    WorkspaceRow::File {
                        path,
                        branch,
                        split,
                        indented,
                    } => render_file_row(
                        theme,
                        row_height,
                        ui_scale_percent,
                        ix,
                        path.clone(),
                        branch.clone(),
                        split.clone(),
                        *indented,
                        busy,
                        repo_id,
                        cx,
                    ),
                    WorkspaceRow::Placeholder { message } => {
                        render_placeholder_row(theme, row_height, message.clone())
                    }
                    WorkspaceRow::Conflict { message } => render_conflict_row(
                        theme,
                        row_height,
                        ix,
                        message.clone(),
                        busy,
                        repo_id,
                        Arc::clone(&store),
                        cx,
                    ),
                }
            })
            .collect()
    }
}

/// The shared frame every workspace row sits in, so labels and controls share
/// one vertical rhythm with the branch tree beside them.
///
/// The id is the row's index rather than anything about the row's content: two
/// rows can look identical (two branches with no description, two unassigned
/// files) and element ids have to stay unique within a frame.
fn row_frame(
    theme: AppTheme,
    row_height: gpui::Pixels,
    index: usize,
    indent: gpui::Pixels,
    surface: gpui::Rgba,
) -> Stateful<Div> {
    div()
        .id(("workspace_row", index))
        .relative()
        .h(row_height)
        .w_full()
        .flex()
        .items_center()
        .gap(px(ROW_GAP_PX))
        .pl(indent)
        .pr(px(8.0))
        .bg(surface)
        .interactive_row(
            components::InteractiveRowStyle::new(theme, surface),
            components::InteractiveRowState::default(),
        )
}

fn render_target_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    ui_scale_percent: u32,
    index: usize,
    name: String,
    busy: bool,
    repo_id: RepoId,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    let set_target = components::Button::new("workspace_set_target", "Change")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, {
            // Cloned for the prompt: the tooltip and the row label below both
            // need the name after the closure has taken its copy.
            let prompt_name = name.clone();
            move |this, event, window, cx| {
                this.open_popover_at(
                    PopoverKind::WorkspacePrompt {
                        repo_id,
                        prompt: WorkspacePrompt {
                            kind: WorkspacePromptKind::SetTarget,
                            branch: String::new(),
                            path: None,
                            hunk: None,
                            // Opens on the current target so the common case is an
                            // edit rather than a retype.
                            value: prompt_name.clone(),
                        },
                    },
                    event.position(),
                    window,
                    cx,
                );
            }
        })
        .gitcomet_tooltip(
            theme,
            SharedString::from(format!(
                "Rebase the whole workspace onto a different branch than {name}"
            )),
        );

    row_frame(
        theme,
        row_height,
        index,
        px(0.0),
        theme.colors.surface.chrome,
    )
        .child(
            components::FadingText::new(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(name),
                theme.colors.surface.chrome,
            )
            .render(ui_scale_percent)
            .flex_1(),
        )
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child("target"),
        )
        .child(set_target)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_branch_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    ui_scale_percent: u32,
    index: usize,
    depth: usize,
    name: String,
    applied: bool,
    file_count: usize,
    description: Option<String>,
    siblings: WorkspaceSiblings,
    busy: bool,
    repo_id: RepoId,
    store: std::sync::Arc<AppStore>,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    let state = if applied {
        BranchApplyState::Applied
    } else {
        BranchApplyState::Unapplied
    };
    let branch = VirtualBranch::new(name.clone()).with_applied(state);
    let tooltip = apply_tooltip(&branch);

    let toggle_store = Arc::clone(&store);
    let toggle_name = name.clone();
    let apply = components::Button::new(format!("workspace_apply_{name}"), apply_label(state))
    .style(components::ButtonStyle::Subtle)
    .disabled(busy)
    .on_click(theme, cx, move |_, _, _, _| {
        toggle_store.dispatch(Msg::SetWorkspaceBranchApplied {
            repo_id,
            name: toggle_name.clone(),
            applied: if state.is_applied() {
                BranchApplyState::Unapplied
            } else {
                BranchApplyState::Applied
            },
        });
    })
    .gitcomet_tooltip(theme, tooltip);

    let push_store = Arc::clone(&store);
    let push_name = name.clone();
    let push = components::Button::new(format!("workspace_push_{name}"), "Push")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, move |_, _, _, _| {
            push_store.dispatch(Msg::PushWorkspaceBranch {
                repo_id,
                name: push_name.clone(),
            });
        })
        .gitcomet_tooltip(
            theme,
            SharedString::from(format!("Push {name} to its remote")),
        );

    let up = reorder_button(
        theme,
        format!("workspace_up_{name}"),
        ReorderDirection::Up,
        siblings.above.clone(),
        name.clone(),
        busy,
        repo_id,
        Arc::clone(&store),
        cx,
    );
    let down = reorder_button(
        theme,
        format!("workspace_down_{name}"),
        ReorderDirection::Down,
        siblings.below.clone(),
        name.clone(),
        busy,
        repo_id,
        Arc::clone(&store),
        cx,
    );

    // Stacking, re-parenting, joining another stack and removing are four
    // questions about the same branch, and a row already carries push and
    // apply; the actions go behind one button rather than four more.
    let actions = components::Button::new(format!("workspace_actions_{name}"), "⋯")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, {
            // Cloned for the prompt: the tooltip and the row label below both
            // need the name after the closure has taken its copy.
            let prompt_name = name.clone();
            move |this, event, window, cx| {
                this.open_popover_at(
                    PopoverKind::WorkspacePrompt {
                        repo_id,
                        prompt: WorkspacePrompt {
                            kind: WorkspacePromptKind::BranchActions,
                            branch: prompt_name.clone(),
                            path: None,
                            hunk: None,
                            value: String::new(),
                        },
                    },
                    event.position(),
                    window,
                    cx,
                );
            }
        })
        .gitcomet_tooltip(theme, SharedString::from(format!("Actions for {name}")));

    let label = match &description {
        Some(description) => format!("{name} — {description}"),
        None => name.clone(),
    };
    let color = if applied {
        theme.colors.foreground.primary
    } else {
        theme.colors.foreground.secondary
    };
    let surface = theme.colors.surface.chrome;

    let indent = ui_scale::design_px_from_percent(
        depth as f32 * STACK_INDENT_PX,
        ui_scale_percent,
    );
    row_frame(theme, row_height, index, indent, surface)
        .child(
            components::FadingText::new(
                div().text_size(theme.ui_text(14.0)).text_color(color).child(label),
                surface,
            )
            .render(ui_scale_percent)
            .flex_1(),
        )
    .child(
        div()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(if file_count > 0 {
                SharedString::from(file_count.to_string())
            } else {
                SharedString::from("—")
            }),
    )
    .child(push)
    .child(apply)
    .child(up)
    .child(down)
    .child(actions)
    .into_any_element()
}

/// Which way a branch's arrow moves it within its stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReorderDirection {
    Up,
    Down,
}

impl ReorderDirection {
    /// The label on the arrow.
    pub(in crate::view) fn label(self) -> &'static str {
        match self {
            Self::Up => "↑",
            Self::Down => "↓",
        }
    }

    /// The pair of branches to swap so that `name` ends up on `this` side of
    /// `partner`.
    ///
    /// The edit is a swap of two siblings rather than a direction, so it is the
    /// same edit a drag would produce and the model decides what the result
    /// looks like.
    pub(in crate::view) fn swap(self, name: &str, partner: &str) -> (String, String) {
        let (name, partner) = (name.to_string(), partner.to_string());
        match self {
            Self::Up => (partner, name),
            Self::Down => (name, partner),
        }
    }
}

/// One of a branch's two reordering arrows.
///
/// The arrow at the end of a stack is disabled rather than offered and refused,
/// because "there is nothing above this" is something the row can already show.
#[allow(clippy::too_many_arguments)]
fn reorder_button(
    theme: AppTheme,
    id: String,
    direction: ReorderDirection,
    partner: Option<String>,
    name: String,
    busy: bool,
    repo_id: RepoId,
    store: Arc<AppStore>,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> Stateful<Div> {
    let tooltip = partner.as_ref().map(|partner| {
        SharedString::from(match direction {
            ReorderDirection::Up => format!("Move {name} above {partner}"),
            ReorderDirection::Down => format!("Move {name} below {partner}"),
        })
    });
    let button = components::Button::new(id, direction.label())
        .style(components::ButtonStyle::Subtle)
        .disabled(busy || partner.is_none())
        .on_click(theme, cx, move |_, _, _, _| {
            let Some(partner) = partner.clone() else {
                return;
            };
            let (first, second) = direction.swap(&name, &partner);
            store.dispatch(Msg::ApplyWorkspaceEdit {
                repo_id,
                edit: WorkspaceEdit::Reorder { first, second },
            });
        });
    match tooltip {
        Some(tooltip) => button.gitcomet_tooltip(theme, tooltip),
        None => button,
    }
}

fn render_summary_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    index: usize,
    text: String,
    busy: bool,
    repo_id: RepoId,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    // Creating a branch is the one workspace action with no row of its own to
    // hang off — there is nothing yet to hang it off — so it lives on the
    // summary line that closes the branch list.
    let create = components::Button::new("workspace_create_branch", "New branch")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, move |this, event, window, cx| {
            this.open_popover_at(
                PopoverKind::WorkspacePrompt {
                    repo_id,
                    prompt: WorkspacePrompt {
                        kind: WorkspacePromptKind::Create,
                        branch: String::new(),
                        path: None,
                        hunk: None,
                        value: String::new(),
                    },
                },
                event.position(),
                window,
                cx,
            );
        })
        .gitcomet_tooltip(theme, SharedString::from("Add a branch to the workspace"));

    row_frame(
        theme,
        row_height,
        index,
        px(0.0),
        theme.colors.surface.chrome,
    )
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text)
                .flex_1(),
        )
        .child(create)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_file_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    ui_scale_percent: u32,
    index: usize,
    path: PathBuf,
    branch: Option<String>,
    split: Vec<String>,
    indented: bool,
    busy: bool,
    repo_id: RepoId,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    // An unassigned file sits one level out from its assigned siblings, so the
    // "which branch does this belong to" question is visible in the layout.
    let indent = if indented {
        ui_scale::design_px_from_percent(FILE_INDENT_PX, ui_scale_percent)
    } else {
        px(0.0)
    };
    let label = path.display().to_string();
    let surface = theme.colors.surface.canvas;

    // An unassigned file has nowhere to go, so the control is not offered at all
    // rather than offered and refused. The message is written in the dialog the
    // button opens, pre-filled with what the quick commit used to synthesise, so
    // accepting it is still one press.
    //
    // A split file gets no commit button: it is committed by every branch
    // holding part of it, so there is no single branch for the button to name.
    // The branch row's *Commit its assigned files* is the one that covers it.
    let commit = branch.as_ref().map(|branch| {
        let commit_path = path.clone();
        let message = default_commit_message(&commit_path);
        components::Button::new(
            format!("workspace_commit_{}", commit_path.display()),
            "Commit",
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, {
            // Cloned for the prompt: the tooltip below still names the path.
            let prompt_path = commit_path.clone();
            move |this, event, window, cx| {
                this.open_popover_at(
                    PopoverKind::WorkspacePrompt {
                        repo_id,
                        prompt: commit_file_prompt(&prompt_path, &message),
                    },
                    event.position(),
                    window,
                    cx,
                );
            }
        })
        .gitcomet_tooltip(
            theme,
            SharedString::from(format!(
                "Commit {} to {branch}",
                commit_path.display()
            )),
        )
    });

    // Assignment is what the whole tab is for, so it is offered on every file
    // row and the label follows the file: "Assign" when it has nowhere to go,
    // "Change" when it already does. Assigning a split file whole is a real
    // answer to a real question, so the button is not disabled for one.
    let assigned = branch.is_some() || !split.is_empty();
    let assign_label_text = if assigned { "Change" } else { "Assign" };
    let assign_tooltip = match (&branch, split.as_slice()) {
        (Some(branch), _) => format!("Commit {label} to a different branch than {branch}"),
        (None, [only]) => format!("Commit all of {label} to {only} instead of only part of it"),
        (None, []) => format!("Say which branch commits {label}"),
        (None, many) => format!("Commit all of {label} to one branch instead of {}", many.join(" + ")),
    };
    let assign_path = path.clone();
    let assign_branch = branch.clone();
    let assign = components::Button::new(
        format!("workspace_assign_{}", assign_path.display()),
        assign_label_text,
    )
    .style(components::ButtonStyle::Subtle)
    .disabled(busy)
    .on_click(theme, cx, move |this, event, window, cx| {
        this.open_popover_at(
            PopoverKind::WorkspacePrompt {
                repo_id,
                prompt: assign_file_prompt(&assign_path, assign_branch.as_deref()),
            },
            event.position(),
            window,
            cx,
        );
    })
    .gitcomet_tooltip(theme, SharedString::from(assign_tooltip));

    let mut row = row_frame(theme, row_height, index, indent, surface).child(
        components::FadingText::new(
            div()
                .text_size(theme.ui_text(13.0))
                .text_color(theme.colors.foreground.primary)
                .child(label),
            surface,
        )
        .render(ui_scale_percent)
        .flex_1(),
    );

    row = row.child(
        div()
            .text_size(theme.ui_text(12.0))
            .text_color(match (&branch, split.is_empty()) {
                (Some(_), _) | (None, false) => theme.colors.foreground.secondary,
                // Unassigned is the one label worth pulling attention to: it is
                // the file that will not be committed anywhere.
                (None, true) => theme.colors.status.warning.foreground,
            })
            .child(match (&branch, split.as_slice()) {
                (Some(branch), _) => branch.clone(),
                // "api + ui" rather than a count, because the count is not what
                // the user needs to know to go and look at a hunk.
                (None, [only]) => only.clone(),
                (None, []) => "Unassigned".into(),
                (None, many) => many.join(" + "),
            }),
    );

    row = row.child(assign);
    if let Some(commit) = commit {
        row = row.child(commit);
    }
    row.into_any_element()
}

fn render_placeholder_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    message: String,
) -> AnyElement {
    div()
        .h(row_height)
        .w_full()
        .flex()
        .items_center()
        .px_2()
        .text_size(theme.ui_text(13.0))
        .text_color(theme.colors.foreground.secondary)
        .child(message)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_conflict_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    index: usize,
    message: String,
    busy: bool,
    repo_id: RepoId,
    store: Arc<AppStore>,
    cx: &mut gpui::Context<SidebarPaneView>,
) -> AnyElement {
    let dismiss_store = Arc::clone(&store);
    let dismiss = components::Button::new("workspace_conflict_dismiss", "Dismiss")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .on_click(theme, cx, move |_, _, _, _| {
            dismiss_store.dispatch(Msg::DismissWorkspaceConflict { repo_id });
        })
        .gitcomet_tooltip(theme, SharedString::from("Hide this conflict"));

    row_frame(
        theme,
        row_height,
        index,
        px(0.0),
        theme.colors.surface.chrome,
    )
        .row_accent(theme.colors.status.warning.foreground)
        .child(
            div()
                .text_size(theme.ui_text(13.0))
                .text_color(theme.colors.status.warning.foreground)
                .child(message),
        )
        .child(dismiss)
        .into_any_element()
}

/// Row height for the workspace list, matching the branch tree's rhythm so the
/// two panes do not visibly change size when the user switches tabs.
pub(in crate::view) fn workspace_row_height(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Pixels {
    crate::view::rows::sidebar::sidebar_list_row_height(theme, ui_scale_percent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::workspace::{VirtualBranch, WorkspaceState};
    use gitcomet_state::model::{RepoId, RepoState, WorkspaceRepoState};

    fn repo_with(workspace: WorkspaceState) -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: std::path::PathBuf::from("/repo"),
            },
        );
        let mut state = WorkspaceRepoState::default();
        state.set_state(workspace);
        // Most tests are about the stack layout, and the tab normally has the
        // working directory. The notice for when it does not has its own test.
        state.active = true;
        repo.workspace = state;
        repo
    }

    fn names(presentation: &WorkspacePresentation) -> Vec<String> {
        presentation
            .rows
            .iter()
            .map(|row| match row {
                WorkspaceRow::Target { name }
                | WorkspaceRow::StackBase { name, .. }
                | WorkspaceRow::Branch { name, .. } => name.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn rows_start_with_the_target_then_the_stacks() {
        let repo = repo_with(
            WorkspaceState::new("main").with_branches(vec![
                VirtualBranch::new("api"),
                VirtualBranch::new("ui").with_parent("api"),
            ]),
        );
        let presentation = WorkspacePresentation::build(Some(&repo));
        assert_eq!(
            names(&presentation),
            ["main", "api", "ui", ""],
            "target, stack base, stacked branch, then the summary row and no files"
        );
    }

    #[test]
    fn a_workspace_the_working_directory_is_not_on_says_so() {
        // Without this the tab happily lists applied branches while the user's
        // files contain none of them, which reads as the feature being broken.
        let repo = repo_with(WorkspaceState::new("main").with_branches(vec![VirtualBranch::new("api")]));
        let mut repo = repo;
        repo.workspace.active = false;

        let presentation = WorkspacePresentation::build(Some(&repo));
        let first = match &presentation.rows[0] {
            WorkspaceRow::Placeholder { message } => message.clone(),
            other => panic!("expected a notice, got {other:?}"),
        };
        assert!(
            first.contains("not on the workspace"),
            "the notice says what is wrong, not just that something is"
        );
    }

    #[test]
    fn a_workspace_the_working_directory_is_on_says_nothing_about_it() {
        // The other half of the notice: a workspace in its normal state must not
        // carry a permanent warning at the top of the tab.
        let repo = repo_with(WorkspaceState::new("main").with_branches(vec![VirtualBranch::new("api")]));
        let presentation = WorkspacePresentation::build(Some(&repo));
        assert!(
            presentation.rows.iter().all(|row| !matches!(
                row,
                WorkspaceRow::Placeholder { message } if message.contains("workspace")
            )),
            "the notice is about being off the workspace, not about being in it"
        );
        assert!(
            matches!(presentation.rows.first(), Some(WorkspaceRow::Target { .. })),
            "and the target row still comes first"
        );
    }

    #[test]
    fn a_stacked_branch_reports_its_depth() {
        let repo = repo_with(
            WorkspaceState::new("main").with_branches(vec![
                VirtualBranch::new("api"),
                VirtualBranch::new("ui").with_parent("api"),
                VirtualBranch::new("e2e").with_parent("ui"),
            ]),
        );
        let presentation = WorkspacePresentation::build(Some(&repo));
        let depths: Vec<_> = presentation
            .rows
            .iter()
            .filter_map(|row| match row {
                WorkspaceRow::Branch { depth, .. } => Some(*depth),
                _ => None,
            })
            .collect();
        assert_eq!(depths, [1, 2]);
    }

    #[test]
    fn an_unapplied_branch_is_marked() {
        let repo = repo_with(WorkspaceState::new("main").with_branches(vec![
            VirtualBranch::new("api"),
            VirtualBranch::new("parked").with_applied(BranchApplyState::Unapplied),
        ]));
        let presentation = WorkspacePresentation::build(Some(&repo));
        let unapplied: Vec<_> = presentation
            .rows
            .iter()
            .filter_map(|row| match row {
                WorkspaceRow::StackBase {
                    name,
                    applied: false,
                    ..
                } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(unapplied, ["parked"]);
    }

    #[test]
    fn an_empty_workspace_invites_the_user_to_create_a_branch() {
        let repo = repo_with(WorkspaceState::new("main"));
        let presentation = WorkspacePresentation::build(Some(&repo));
        assert!(
            presentation
                .rows
                .iter()
                .any(|row| matches!(row, WorkspaceRow::Placeholder { message } if message.contains("Create a branch")))
        );
    }

    #[test]
    fn no_repository_renders_a_placeholder() {
        let presentation = WorkspacePresentation::build(None);
        assert_eq!(presentation.rows.len(), 1);
        assert!(matches!(
            presentation.rows[0],
            WorkspaceRow::Placeholder { .. }
        ));
    }

    #[test]
    fn a_conflict_is_shown_before_the_branches() {
        let mut repo = repo_with(WorkspaceState::new("main").with_branches(vec![VirtualBranch::new(
            "api",
        )]));
        repo.workspace.conflict = Some(gitcomet_state::model::WorkspaceConflict {
            branch: "api".into(),
            against: Some("ui".into()),
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        });
        let presentation = WorkspacePresentation::build(Some(&repo));
        assert!(matches!(
            presentation.rows.first(),
            Some(WorkspaceRow::Conflict { .. })
        ));
    }

    #[test]
    fn the_fingerprint_moves_with_the_workspace_and_the_worktree() {
        let mut repo = repo_with(WorkspaceState::new("main"));
        let before = WorkspaceFingerprint::from_repo(&repo);
        repo.workspace.bump_rev();
        assert_ne!(WorkspaceFingerprint::from_repo(&repo), before);

        let mut repo = repo_with(WorkspaceState::new("main"));
        let before = WorkspaceFingerprint::from_repo(&repo);
        repo.worktree_status_rev = 7;
        assert_ne!(WorkspaceFingerprint::from_repo(&repo), before);
    }

    #[test]
    fn a_branch_knows_the_siblings_it_can_swap_with() {
        let repo = repo_with(
            WorkspaceState::new("main").with_branches(vec![
                VirtualBranch::new("api"),
                VirtualBranch::new("ui").with_parent("api"),
                VirtualBranch::new("docs").with_parent("ui"),
            ]),
        );
        let presentation = WorkspacePresentation::build(Some(&repo));
        let siblings: Vec<_> = presentation
            .rows
            .iter()
            .filter_map(|row| match row {
                WorkspaceRow::StackBase { name, siblings, .. }
                | WorkspaceRow::Branch { name, siblings, .. } => Some((name.clone(), siblings.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(siblings.len(), 3);
        assert_eq!(siblings[0].1.above, None, "the stack base has nothing above it");
        assert_eq!(siblings[0].1.below.as_deref(), Some("ui"));
        assert_eq!(siblings[1].1.above.as_deref(), Some("api"));
        assert_eq!(siblings[1].1.below.as_deref(), Some("docs"));
        assert_eq!(siblings[2].1.below, None, "the top of the stack has nothing below it");
    }

    #[test]
    fn an_arrow_swaps_the_branch_with_the_neighbour_it_points_at() {
        // Moving up puts the named branch first, so it lands above its partner.
        assert_eq!(
            ReorderDirection::Up.swap("ui", "api"),
            ("api".to_string(), "ui".to_string())
        );
        assert_eq!(
            ReorderDirection::Down.swap("api", "ui"),
            ("api".to_string(), "ui".to_string())
        );
        // The two directions only differ in which branch ends up first.
        assert_eq!(ReorderDirection::Up.label(), "↑");
        assert_eq!(ReorderDirection::Down.label(), "↓");
    }

    #[test]
    fn the_apply_label_follows_the_branch_state() {
        assert_eq!(apply_label(BranchApplyState::Applied), "Unapply");
        assert_eq!(apply_label(BranchApplyState::Unapplied), "Apply");

        let applied = VirtualBranch::new("feature/api");
        assert!(apply_tooltip(&applied).contains("Unapply"));
        let unapplied = VirtualBranch::new("feature/api")
            .with_applied(BranchApplyState::Unapplied);
        assert!(apply_tooltip(&unapplied).contains("Apply"));
    }
}
