//! The workspace edit prompt's behaviour: narrowing the action list down to a
//! single edit, validating what was typed, and dispatching it.
//!
//! Validation is deliberately stricter than the model requires. The model would
//! accept a base branch that does not exist and only fail later, on a
//! background thread, with a rebase error against a revision nobody asked for;
//! the prompt refuses to submit instead, so the mistake is caught while the
//! branch it was made on is still on screen.
//!
//! The mapping from `(step, branch, text)` to an edit is a free function over
//! two membership predicates rather than a method, so the rules can be tested
//! without standing up a popover host.

use super::*;
use gitcomet_state::model::WorkspaceEdit;

impl PopoverHost {
    /// Move from a branch's action list to the text field of one action.
    ///
    /// The field is seeded rather than emptied: re-parenting wants the current
    /// base so the user edits it instead of retyping it, while moving into a
    /// stack has nothing sensible to prefill and so gets nothing.
    pub(in crate::view::panels::popover) fn open_workspace_prompt_step(
        &mut self,
        kind: WorkspacePromptKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::WorkspacePrompt { repo_id, prompt }) = self.popover.clone() else {
            return;
        };
        let theme = self.theme;
        let seed = match kind {
            WorkspacePromptKind::SetParent => self
                .workspace_state(repo_id)
                .and_then(|state| state.get(&prompt.branch).and_then(|branch| branch.parent.clone()))
                .unwrap_or_default(),
            // Deliberately empty, unlike the single-file commit. There is no
            // honest one-word summary of an entire branch's changes, and a
            // synthesised default here would invite the user to accept a commit
            // message that describes nothing.
            WorkspacePromptKind::CommitBranch => String::new(),
            _ => String::new(),
        };
        self.workspace_prompt_kind = Some(kind);
        self.create_branch_input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.set_theme(theme, cx);
            input.set_text(seed, cx);
            cx.notify();
        });
        if kind.asks_for_text() {
            let focus = self
                .create_branch_input
                .read_with(cx, |input, _| input.focus_handle());
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Whether the open prompt currently describes a valid edit.
    pub(in crate::view::panels::popover) fn can_submit_workspace_prompt(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some((repo_id, kind, value)) = self.open_workspace_prompt(cx) else {
            return false;
        };
        self.workspace_prompt_edit(repo_id, kind, &value).is_some()
    }

    /// Dispatch the open prompt's edit and close the dialog.
    pub(in crate::view::panels::popover) fn submit_workspace_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((repo_id, kind, value)) = self.open_workspace_prompt(cx) else {
            return;
        };
        let Some(edit) = self.workspace_prompt_edit(repo_id, kind, &value) else {
            return;
        };
        self.store.dispatch(Msg::ApplyWorkspaceEdit { repo_id, edit });
        self.dismiss_prompt_popover(window, cx);
    }

    /// The open prompt as `(repo, step, typed value)`, or `None` when no
    /// workspace prompt is up.
    fn open_workspace_prompt(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<(RepoId, WorkspacePromptKind, String)> {
        let PopoverKind::WorkspacePrompt { repo_id, prompt } = self.popover.as_ref()? else {
            return None;
        };
        let kind = self.workspace_prompt_kind.unwrap_or(prompt.kind);
        let value = if kind.asks_for_text() {
            self.create_branch_input
                .read_with(cx, |input, _| input.text().to_string())
        } else {
            String::new()
        };
        Some((*repo_id, kind, value))
    }

    fn workspace_prompt_edit(
        &self,
        repo_id: RepoId,
        kind: WorkspacePromptKind,
        value: &str,
    ) -> Option<WorkspaceEdit> {
        let (branch, path) = match &self.popover {
            Some(PopoverKind::WorkspacePrompt { prompt, .. }) => {
                (prompt.branch.clone(), prompt.path.clone())
            }
            _ => (String::new(), None),
        };
        // A prompt left open across a background load that changed the list must
        // not dispatch an edit against a branch that is gone, so the subject is
        // re-checked here rather than trusted from when the popover opened.
        let state = self.workspace_state(repo_id);
        let in_workspace = |name: &str| state.is_some_and(|state| state.contains(name));
        let free_name = |value: &str| self.free_branch_name(repo_id, value, state);
        let assignment_of = |path: &std::path::Path| self.file_assignment(repo_id, path);
        let paths_for = |branch: &str| self.assigned_paths(repo_id, branch);
        edit_for(
            kind,
            &branch,
            path.as_deref(),
            value.trim(),
            &in_workspace,
            &free_name,
            |name| self.knows_git_branch(repo_id, name),
            &assignment_of,
            &paths_for,
        )
    }

    /// The name to create, suffixed until it is free.
    ///
    /// Creating a branch whose name is taken fails on the backend with a Git
    /// error the user can do nothing about, and the likeliest way to get there
    /// is typing a name that is visible in the list beside the dialog.
    fn free_branch_name(
        &self,
        repo_id: RepoId,
        value: &str,
        state: Option<&gitcomet_core::workspace::WorkspaceState>,
    ) -> String {
        let is_taken = |name: &str| {
            state.is_some_and(|state| state.contains(name))
                || self.knows_git_branch(repo_id, name)
        };
        free_branch_name(value.trim(), &is_taken)
    }

    /// Whether `name` is a branch in `repo_id`'s real branch list.
    fn knows_git_branch(&self, repo_id: RepoId, name: &str) -> bool {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| match &repo.branches {
                Loadable::Ready(branches) => {
                    branches.iter().any(|branch| branch.name == name.trim())
                }
                _ => false,
            })
    }

    fn workspace_state(
        &self,
        repo_id: RepoId,
    ) -> Option<&gitcomet_core::workspace::WorkspaceState> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.workspace.workspace())
    }

    /// The branch a file is currently assigned to, if that branch still exists.
    fn file_assignment(&self, repo_id: RepoId, path: &std::path::Path) -> Option<String> {
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;
        let workspace = repo.workspace.workspace()?;
        repo.workspace
            .assignments
            .resolve(path, workspace)
            .map(str::to_string)
    }

    /// The files assigned to a branch, in path order.
    fn assigned_paths(&self, repo_id: RepoId, branch: &str) -> Vec<std::path::PathBuf> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| {
                repo.workspace
                    .assignments
                    .paths_for(branch)
                    .into_iter()
                    .map(std::path::PathBuf::from)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The edit a prompt at `kind` with `branch`, `path` and `value` produces.
///
/// `in_workspace` answers whether a name is one of the repository's virtual
/// branches, `free_name` turns typed text into a name nothing else has taken,
/// `is_git_branch` whether it is a real branch in the repository — the target
/// is a real branch, while a base or a stack is a virtual one, and conflating
/// the two is how a prompt ends up offering a branch that cannot be resolved —
/// and `assignment_of` which branch a file currently belongs to, with
/// `paths_for` the files assigned to a branch.
#[allow(clippy::too_many_arguments)]
fn edit_for(
    kind: WorkspacePromptKind,
    branch: &str,
    path: Option<&std::path::Path>,
    value: &str,
    in_workspace: &dyn Fn(&str) -> bool,
    free_name: &dyn Fn(&str) -> String,
    is_git_branch: &dyn Fn(&str) -> bool,
    assignment_of: &dyn Fn(&std::path::Path) -> Option<String>,
    paths_for: &dyn Fn(&str) -> Vec<std::path::PathBuf>,
) -> Option<WorkspaceEdit> {
    // The action list submits nothing; picking a row is the whole interaction.
    let branch_exists = !branch.is_empty() && in_workspace(branch);

    let edit = match kind {
        WorkspacePromptKind::BranchActions => return None,
        WorkspacePromptKind::Create => WorkspaceEdit::Create {
            name: free_name(value),
        },
        WorkspacePromptKind::CreateStacked => {
            if !branch_exists {
                return None;
            }
            WorkspaceEdit::CreateStacked {
                name: free_name(value),
                parent: Some(branch.to_string()),
            }
        }
        WorkspacePromptKind::SetTarget => {
            // The target is a revision, not a workspace branch, so it is
            // checked against the repository's real branch list instead.
            if !is_submittable_branch_name(value) || !is_git_branch(value) {
                return None;
            }
            WorkspaceEdit::SetTarget {
                target: value.to_string(),
            }
        }
        WorkspacePromptKind::SetParent => {
            if !branch_exists {
                return None;
            }
            // An empty field means "sit on the target directly", which is how a
            // branch leaves a stack, so it is an answer here rather than a
            // missing one.
            if !value.is_empty() && (value == branch || !in_workspace(value)) {
                return None;
            }
            WorkspaceEdit::SetParent {
                name: branch.to_string(),
                parent: (!value.is_empty()).then(|| value.to_string()),
            }
        }
        WorkspacePromptKind::MoveToStack => {
            if !branch_exists || value.is_empty() || value == branch || !in_workspace(value) {
                return None;
            }
            WorkspaceEdit::MoveToStack {
                name: branch.to_string(),
                stack_base: value.to_string(),
                relative_to: None,
                below: false,
            }
        }
        WorkspacePromptKind::Remove => {
            if !branch_exists {
                return None;
            }
            WorkspaceEdit::Remove {
                name: branch.to_string(),
            }
        }
        WorkspacePromptKind::AssignFile => {
            // An empty field is not a missing answer here: it is how a file
            // goes back to the unassigned bucket.
            let Some(path) = path else {
                return None;
            };
            if !value.is_empty() && !in_workspace(value) {
                return None;
            }
            WorkspaceEdit::AssignFile {
                path: path.to_path_buf(),
                branch: (!value.is_empty()).then(|| value.to_string()),
            }
        }
        WorkspacePromptKind::CommitMessage => {
            let Some(path) = path else {
                return None;
            };
            // A commit with no message is one Git will refuse to describe, so
            // the prompt opens on the synthesised default rather than empty and
            // the user can accept it or replace it.
            let message = value.trim();
            if message.is_empty() {
                return None;
            }
            // The destination is read live rather than captured when the prompt
            // opened: a file can be reassigned while its commit dialog is up,
            // and committing to the branch it used to belong to would put the
            // change somewhere the user is no longer looking.
            let Some(name) = assignment_of(path) else {
                return None;
            };
            WorkspaceEdit::CommitPaths {
                name,
                message: message.to_string(),
                paths: vec![path.to_path_buf()],
            }
        }
        WorkspacePromptKind::CommitBranch => {
            if !branch_exists {
                return None;
            }
            // A commit Git cannot describe is refused, so an empty message is
            // not a commit with a blank subject — it is nothing to submit.
            let message = value.trim();
            if message.is_empty() {
                return None;
            }
            // A branch with no files assigned has nothing to commit. Saying so
            // at the prompt beats opening an empty commit dialog.
            let paths = paths_for(branch);
            if paths.is_empty() {
                return None;
            }
            WorkspaceEdit::CommitPaths {
                name: branch.to_string(),
                message: message.to_string(),
                paths,
            }
        }
    };
    // A creation whose name could not be made free is not an edit at all, and
    // the confirm button stays disabled rather than dispatching an empty name.
    match &edit {
        WorkspaceEdit::Create { name } | WorkspaceEdit::CreateStacked { name, .. }
            if name.is_empty() =>
        {
            None
        }
        _ => Some(edit),
    }
}

/// `base`, suffixed with the lowest number that is free.
///
/// An empty answer means no usable name could be produced, which the caller
/// treats as "do not submit".
fn free_branch_name(base: &str, is_taken: &dyn Fn(&str) -> bool) -> String {
    if !is_submittable_branch_name(base) {
        return String::new();
    }
    if !is_taken(base) {
        return base.to_string();
    }
    (2..100)
        .map(|suffix| format!("{base}-{suffix}"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A workspace holding `api` on the target, with `ui` stacked on it.
    fn membership() -> impl Fn(&str) -> bool {
        let names = ["api", "ui"].map(str::to_string);
        move |name: &str| names.iter().any(|known| known == name)
    }

    /// `taken` and `api` are spoken for; nothing else is.
    fn taken() -> impl Fn(&str) -> bool {
        let names = ["api", "taken"].map(str::to_string);
        move |name: &str| names.iter().any(|known| known == name)
    }

    fn free(value: &str) -> String {
        free_branch_name(value, &taken())
    }

    fn edit(
        kind: WorkspacePromptKind,
        branch: &str,
        value: &str,
    ) -> Option<WorkspaceEdit> {
        edit_for(
            kind,
            branch,
            None,
            value,
            &membership(),
            &free,
            &|name| matches!(name, "main" | "develop" | "api"),
            &|_| None,
            &|_| Vec::new(),
        )
    }

    fn assign(value: &str) -> Option<WorkspaceEdit> {
        edit_for(
            WorkspacePromptKind::AssignFile,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            value,
            &membership(),
            &free,
            &|name| matches!(name, "main" | "develop" | "api"),
            &|_| Some("ui".to_string()),
            &|_| Vec::new(),
        )
    }

    fn commit(value: &str) -> Option<WorkspaceEdit> {
        edit_for(
            WorkspacePromptKind::CommitMessage,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            value,
            &membership(),
            &free,
            &|name| matches!(name, "main" | "develop" | "api"),
            &|_| Some("ui".to_string()),
            &|_| Vec::new(),
        )
    }

    /// `api` has two files assigned; `ui` has none.
    fn assigned_paths(branch: &str) -> Vec<std::path::PathBuf> {
        match branch {
            "api" => vec![
                std::path::PathBuf::from("src/api.rs"),
                std::path::PathBuf::from("src/api/client.rs"),
            ],
            _ => Vec::new(),
        }
    }

    fn commit_branch(branch: &str, value: &str) -> Option<WorkspaceEdit> {
        edit_for(
            WorkspacePromptKind::CommitBranch,
            branch,
            None,
            value,
            &membership(),
            &free,
            &|_| true,
            &|_| None,
            &assigned_paths,
        )
    }

    #[test]
    fn the_action_list_produces_no_edit() {
        assert!(edit(WorkspacePromptKind::BranchActions, "api", "").is_none());
    }

    #[test]
    fn creating_uses_the_typed_name() {
        let Some(WorkspaceEdit::Create { name }) = edit(WorkspacePromptKind::Create, "", "feat/x")
        else {
            panic!("expected a create edit");
        };
        assert_eq!(name, "feat/x");
    }

    #[test]
    fn creating_over_an_existing_branch_gets_suffixed() {
        let Some(WorkspaceEdit::Create { name }) = edit(WorkspacePromptKind::Create, "", "api")
        else {
            panic!("expected a create edit");
        };
        assert_eq!(name, "api-2");
    }

    #[test]
    fn an_unusable_name_produces_no_edit() {
        assert!(edit(WorkspacePromptKind::Create, "", "   ").is_none());
    }

    #[test]
    fn stacking_needs_a_subject_and_stacks_on_it() {
        let Some(WorkspaceEdit::CreateStacked { name, parent }) =
            edit(WorkspacePromptKind::CreateStacked, "api", "ui")
        else {
            panic!("expected a stacked edit");
        };
        assert_eq!(name, "ui-2");
        assert_eq!(parent.as_deref(), Some("api"));

        // The branch vanished from the workspace while the prompt was open.
        assert!(edit(WorkspacePromptKind::CreateStacked, "gone", "x").is_none());
    }

    #[test]
    fn the_target_must_be_a_real_branch() {
        let Some(WorkspaceEdit::SetTarget { target }) =
            edit(WorkspacePromptKind::SetTarget, "", "develop")
        else {
            panic!("expected a set-target edit");
        };
        assert_eq!(target, "develop");

        // A virtual branch is not a revision the workspace can be built on.
        assert!(edit(WorkspacePromptKind::SetTarget, "", "ui").is_none());
        assert!(edit(WorkspacePromptKind::SetTarget, "", "").is_none());
    }

    #[test]
    fn re_parenting_accepts_a_virtual_base_or_nothing_at_all() {
        let Some(WorkspaceEdit::SetParent { name, parent }) =
            edit(WorkspacePromptKind::SetParent, "api", "ui")
        else {
            panic!("expected a set-parent edit");
        };
        assert_eq!(name, "api");
        assert_eq!(parent.as_deref(), Some("ui"));

        // Empty means "back onto the target", which is how a branch leaves a
        // stack.
        let Some(WorkspaceEdit::SetParent { parent, .. }) =
            edit(WorkspacePromptKind::SetParent, "ui", "")
        else {
            panic!("expected a set-parent edit");
        };
        assert_eq!(parent, None);
    }

    #[test]
    fn a_branch_cannot_be_stacked_on_itself_or_on_nothing() {
        assert!(edit(WorkspacePromptKind::SetParent, "api", "api").is_none());
        assert!(edit(WorkspacePromptKind::MoveToStack, "api", "api").is_none());
        assert!(edit(WorkspacePromptKind::MoveToStack, "api", "").is_none());
        assert!(edit(WorkspacePromptKind::MoveToStack, "api", "gone").is_none());
    }

    #[test]
    fn moving_into_a_stack_names_the_stack_base() {
        let Some(WorkspaceEdit::MoveToStack {
            name,
            stack_base,
            relative_to,
            below,
        }) = edit(WorkspacePromptKind::MoveToStack, "api", "ui")
        else {
            panic!("expected a move edit");
        };
        assert_eq!(name, "api");
        assert_eq!(stack_base, "ui");
        assert_eq!(relative_to, None);
        assert!(!below);
    }

    #[test]
    fn removing_needs_a_branch_that_is_there() {
        let Some(WorkspaceEdit::Remove { name }) = edit(WorkspacePromptKind::Remove, "api", "")
        else {
            panic!("expected a remove edit");
        };
        assert_eq!(name, "api");
        assert!(edit(WorkspacePromptKind::Remove, "gone", "").is_none());
    }

    #[test]
    fn assigning_a_file_names_the_branch_it_goes_to() {
        let Some(WorkspaceEdit::AssignFile { path, branch }) = assign("ui") else {
            panic!("expected an assign edit");
        };
        assert_eq!(path, std::path::Path::new("src/lib.rs"));
        assert_eq!(branch.as_deref(), Some("ui"));
    }

    #[test]
    fn an_empty_field_unassigns_rather_than_refusing() {
        let Some(WorkspaceEdit::AssignFile { branch, .. }) = assign("") else {
            panic!("an empty field should still produce an edit");
        };
        assert_eq!(branch, None);
    }

    #[test]
    fn a_file_can_only_go_to_a_branch_that_exists() {
        assert!(assign("nope").is_none());
        // A real Git branch is not a workspace branch, so it is not a target.
        assert!(assign("main").is_none());
    }

    #[test]
    fn assigning_needs_a_file() {
        assert!(
            edit_for(
                WorkspacePromptKind::AssignFile,
                "",
                None,
                "ui",
                &membership(),
                &free,
                &|_| true,
                &|_| None,
                &|_| Vec::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn committing_carries_the_message_and_the_file() {
        let Some(WorkspaceEdit::CommitPaths {
            name,
            message,
            paths,
        }) = commit("Fix the thing")
        else {
            panic!("expected a commit edit");
        };
        // The branch is the file's assignment, read at submit rather than
        // captured at open.
        assert_eq!(name, "ui");
        assert_eq!(message, "Fix the thing");
        assert_eq!(paths, [std::path::PathBuf::from("src/lib.rs")]);
    }

    #[test]
    fn a_commit_with_no_message_is_refused() {
        assert!(commit("").is_none());
        assert!(commit("   ").is_none());
    }

    #[test]
    fn a_file_with_no_branch_cannot_be_committed() {
        assert!(
            edit_for(
                WorkspacePromptKind::CommitMessage,
                "",
                Some(std::path::Path::new("src/lib.rs")),
                "Update src/lib.rs",
                &membership(),
                &free,
                &|_| true,
                &|_| None,
                &|_| Vec::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn committing_needs_a_file() {
        assert!(
            edit_for(
                WorkspacePromptKind::CommitMessage,
                "",
                None,
                "Update something",
                &membership(),
                &free,
                &|_| true,
                &|_| Some("ui".to_string()),
                &|_| Vec::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn committing_a_branch_takes_every_file_assigned_to_it() {
        let Some(WorkspaceEdit::CommitPaths {
            name,
            message,
            paths,
        }) = commit_branch("api", "Add the client")
        else {
            panic!("expected a commit edit");
        };
        assert_eq!(name, "api");
        assert_eq!(message, "Add the client");
        assert_eq!(
            paths,
            [
                std::path::PathBuf::from("src/api.rs"),
                std::path::PathBuf::from("src/api/client.rs"),
            ],
            "the whole branch is committed, not just the file last touched"
        );
    }

    #[test]
    fn a_branch_with_nothing_assigned_has_nothing_to_commit() {
        assert!(commit_branch("ui", "Anything").is_none());
        assert!(commit_branch("gone", "Anything").is_none());
    }

    #[test]
    fn committing_a_branch_refuses_an_empty_message() {
        assert!(commit_branch("api", "").is_none());
        assert!(commit_branch("api", "  ").is_none());
    }

    #[test]
    fn a_taken_name_is_suffixed_up_to_the_first_gap() {
        assert_eq!(free_branch_name("new", &taken()), "new");
        assert_eq!(free_branch_name("taken", &taken()), "taken-2");
        assert_eq!(free_branch_name("feat/", &taken()), "");
    }

    #[test]
    fn every_branch_edit_is_refused_for_a_branch_that_is_gone() {
        for kind in [
            WorkspacePromptKind::CreateStacked,
            WorkspacePromptKind::SetParent,
            WorkspacePromptKind::MoveToStack,
            WorkspacePromptKind::Remove,
        ] {
            assert!(
                edit(kind, "gone", "ui").is_none(),
                "{:?} should refuse a missing subject",
                kind
            );
        }
    }
}