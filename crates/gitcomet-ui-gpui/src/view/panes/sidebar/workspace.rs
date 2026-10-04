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
use crate::kit::interaction::ControlInteractionExt as _;
use crate::view::components::InteractiveRowExt as _;
use gitcomet_core::workspace::{BranchApplyState, VirtualBranch};
use gitcomet_state::model::Loadable;
use std::path::PathBuf;
use std::sync::Arc;

/// How much narrower each stacked branch is indented. Small on purpose: a deep
/// stack should still leave room for the branch name.
const STACK_INDENT_PX: f32 = 10.0;
/// File rows sit a little further in than a stacked branch, so the two kinds of
/// row are distinguishable by position alone.
const FILE_INDENT_PX: f32 = 14.0;
const ROW_GAP_PX: f32 = 6.0;

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
    },
    /// A branch stacked on another, indented by its depth.
    Branch {
        name: String,
        depth: usize,
        applied: bool,
        file_count: usize,
        description: Option<String>,
    },
    /// The count line above the file list.
    Summary { text: String },
    /// A changed file, labelled with the branch it is assigned to.
    File {
        path: PathBuf,
        /// `None` is the unassigned bucket.
        branch: Option<String>,
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
        if let Some(branch) = &assignment.branch {
            *counts.entry(branch.clone()).or_default() += 1;
        }
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
            if index == 0 {
                rows.push(WorkspaceRow::StackBase {
                    name: member.branch.name.clone(),
                    applied: member.branch.is_applied(),
                    file_count: count,
                });
            } else {
                rows.push(WorkspaceRow::Branch {
                    name: member.branch.name.clone(),
                    depth: member.depth,
                    applied: member.branch.is_applied(),
                    file_count: count,
                    description: member.branch.description.clone(),
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
        rows.push(WorkspaceRow::File {
            path: assignment.path,
            // A file indented under its branch reads as belonging to it; an
            // unassigned file sits at the margin so the gap is visible.
            indented: assignment.branch.is_some(),
            branch: assignment.branch,
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
                    ),
                    WorkspaceRow::StackBase {
                        name,
                        applied,
                        file_count,
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
                        busy,
                        repo_id,
                        Arc::clone(&store),
                        cx,
                    ),
                    WorkspaceRow::Summary { text } => {
                        render_summary_row(theme, row_height, ix, text.clone())
                    }
                    WorkspaceRow::File {
                        path,
                        branch,
                        indented,
                    } => render_file_row(
                        theme,
                        row_height,
                        ui_scale_percent,
                        ix,
                        path.clone(),
                        branch.clone(),
                        *indented,
                        busy,
                        repo_id,
                        Arc::clone(&store),
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
) -> AnyElement {
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
            .render(ui_scale_percent),
        )
        .child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child("target"),
        )
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
    .gitcomet_tooltip(theme, tooltip)
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
    });

    let push_store = Arc::clone(&store);
    let push_name = name.clone();
    let push = components::Button::new(format!("workspace_push_{name}"), "Push")
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .gitcomet_tooltip(
            theme,
            SharedString::from(format!("Push {name} to its remote")),
        )
        .on_click(theme, cx, move |_, _, _, _| {
            push_store.dispatch(Msg::PushWorkspaceBranch {
                repo_id,
                name: push_name.clone(),
            });
        });

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
    .into_any_element()
}

fn render_summary_row(
    theme: AppTheme,
    row_height: gpui::Pixels,
    index: usize,
    text: String,
) -> AnyElement {
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
                .child(text),
        )
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
    indented: bool,
    busy: bool,
    repo_id: RepoId,
    store: Arc<AppStore>,
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
    // rather than offered and refused.
    let commit = branch.as_ref().map(|_| {
        let commit_store = Arc::clone(&store);
        let commit_path = path.clone();
        components::Button::new(
            format!("workspace_commit_{}", commit_path.display()),
            "Commit",
        )
        .style(components::ButtonStyle::Subtle)
        .disabled(busy)
        .gitcomet_tooltip(
            theme,
            SharedString::from(format!("Commit {} to its branch", commit_path.display())),
        )
        .on_click(theme, cx, move |_, _, _, _| {
            commit_store.dispatch(Msg::CommitWorkspaceFile {
                repo_id,
                path: commit_path.clone(),
            });
        })
    });

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
            .text_color(match &branch {
                Some(_) => theme.colors.foreground.secondary,
                // Unassigned is the one label worth pulling attention to: it is
                // the file that will not be committed anywhere.
                None => theme.colors.status.warning.foreground,
            })
            .child(branch.clone().unwrap_or_else(|| "Unassigned".into())),
    );
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
        .gitcomet_tooltip(theme, SharedString::from("Hide this conflict"))
        .on_click(theme, cx, move |_, _, _, _| {
            dismiss_store.dispatch(Msg::DismissWorkspaceConflict { repo_id });
        });

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
