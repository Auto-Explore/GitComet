//! The virtual branches edit prompt's behaviour: narrowing the action list down to a
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
use gitcomet_state::model::VirtualBranchesEdit;

impl PopoverHost {
    /// Move from a branch's action list to the text field of one action.
    ///
    /// The field is seeded rather than emptied: re-parenting wants the current
    /// base so the user edits it instead of retyping it, while moving into a
    /// stacked branch has nothing sensible to prefill and so gets nothing.
    pub(in crate::view::panels::popover) fn open_virtual_branches_prompt_step(
        &mut self,
        kind: VirtualBranchesPromptKind,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::VirtualBranchesPrompt { repo_id, prompt }) = self.popover.clone()
        else {
            return;
        };
        let seed = match kind {
            VirtualBranchesPromptKind::SetParent => self
                .virtual_branches_state(repo_id)
                .and_then(|state| {
                    state
                        .get(&prompt.branch)
                        .and_then(|branch| branch.parent.clone())
                })
                .unwrap_or_default(),
            // Deliberately empty, unlike the single-file commit. There is no
            // honest one-word summary of an entire branch's changes, and a
            // synthesised default here would invite the user to accept a commit
            // message that describes nothing.
            VirtualBranchesPromptKind::CommitBranch => String::new(),
            _ => String::new(),
        };
        self.virtual_branches_prompt_kind = Some(kind);
        self.create_branch_input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
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
    pub(in crate::view::panels::popover) fn can_submit_virtual_branches_prompt(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some((repo_id, kind, value)) = self.open_virtual_branches_prompt(cx) else {
            return false;
        };
        self.virtual_branches_prompt_edit(repo_id, kind, &value)
            .is_some()
    }

    /// Dispatch the open prompt's edit and close the dialog.
    pub(in crate::view::panels::popover) fn submit_virtual_branches_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((repo_id, kind, value)) = self.open_virtual_branches_prompt(cx) else {
            return;
        };
        let Some(edit) = self.virtual_branches_prompt_edit(repo_id, kind, &value) else {
            return;
        };
        self.store
            .dispatch(Msg::ApplyVirtualBranchesEdit { repo_id, edit });
        self.dismiss_prompt_popover(window, cx);
    }

    /// The open prompt as `(repo, step, typed value)`, or `None` when no
    /// virtual branches prompt is up.
    fn open_virtual_branches_prompt(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> Option<(RepoId, VirtualBranchesPromptKind, String)> {
        let PopoverKind::VirtualBranchesPrompt { repo_id, prompt } = self.popover.as_ref()? else {
            return None;
        };
        let kind = self.virtual_branches_prompt_kind.unwrap_or(prompt.kind);
        let value = if kind.asks_for_text() {
            self.create_branch_input
                .read_with(cx, |input, _| input.text().to_string())
        } else {
            String::new()
        };
        Some((*repo_id, kind, value))
    }

    fn virtual_branches_prompt_edit(
        &self,
        repo_id: RepoId,
        kind: VirtualBranchesPromptKind,
        value: &str,
    ) -> Option<VirtualBranchesEdit> {
        let (branch, path, hunk) = match &self.popover {
            Some(PopoverKind::VirtualBranchesPrompt { prompt, .. }) => {
                (prompt.branch.clone(), prompt.path.clone(), prompt.hunk)
            }
            _ => (String::new(), None, None),
        };
        // A prompt left open across a background load that changed the list must
        // not dispatch an edit against a branch that is gone, so the subject is
        // re-checked here rather than trusted from when the popover opened.
        let state = self.virtual_branches_state(repo_id);
        let in_virtual_branches = |name: &str| state.is_some_and(|state| state.contains(name));
        let free_name = |value: &str| self.free_branch_name(repo_id, value, state);
        let assignment_of = |path: &std::path::Path| self.file_assignment(repo_id, path);
        let paths_for = |branch: &str| self.assigned_paths(repo_id, branch);
        edit_for(
            kind,
            &branch,
            path.as_deref(),
            hunk,
            value.trim(),
            &in_virtual_branches,
            &free_name,
            &|name| self.knows_git_branch(repo_id, name),
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
        state: Option<&gitcomet_core::virtual_branches::VirtualBranchesState>,
    ) -> String {
        let is_taken = |name: &str| {
            state.is_some_and(|state| state.contains(name)) || self.knows_git_branch(repo_id, name)
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

    fn virtual_branches_state(
        &self,
        repo_id: RepoId,
    ) -> Option<&gitcomet_core::virtual_branches::VirtualBranchesState> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.virtual_branches.virtual_branches())
    }

    /// The branch a file is currently assigned to, if that branch still exists.
    fn file_assignment(&self, repo_id: RepoId, path: &std::path::Path) -> Option<String> {
        let repo = self.state.repos.iter().find(|repo| repo.id == repo_id)?;
        let virtual_branches = repo.virtual_branches.virtual_branches()?;
        repo.virtual_branches
            .assignments
            .resolve(path, virtual_branches)
            .map(str::to_string)
    }

    /// The files assigned to a branch, in path order.
    fn assigned_paths(&self, repo_id: RepoId, branch: &str) -> Vec<std::path::PathBuf> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| {
                repo.virtual_branches
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
/// `in_virtual_branches` answers whether a name is one of the repository's virtual
/// branches, `free_name` turns typed text into a name nothing else has taken,
/// `is_git_branch` whether it is a real branch in the repository — the target
/// is a real branch, while a base or a stacked branch is a virtual one, and conflating
/// the two is how a prompt ends up offering a branch that cannot be resolved —
/// and `assignment_of` which branch a file currently belongs to, with
/// `paths_for` the files assigned to a branch.
#[allow(clippy::too_many_arguments)]
fn edit_for(
    kind: VirtualBranchesPromptKind,
    branch: &str,
    path: Option<&std::path::Path>,
    hunk: Option<gitcomet_core::virtual_branches::HunkFingerprint>,
    value: &str,
    in_virtual_branches: &dyn Fn(&str) -> bool,
    free_name: &dyn Fn(&str) -> String,
    is_git_branch: &dyn Fn(&str) -> bool,
    assignment_of: &dyn Fn(&std::path::Path) -> Option<String>,
    paths_for: &dyn Fn(&str) -> Vec<std::path::PathBuf>,
) -> Option<VirtualBranchesEdit> {
    // The action list submits nothing; picking a row is the whole interaction.
    let branch_exists = !branch.is_empty() && in_virtual_branches(branch);

    let edit = match kind {
        VirtualBranchesPromptKind::BranchActions => return None,
        VirtualBranchesPromptKind::Create => VirtualBranchesEdit::Create {
            name: free_name(value),
        },
        VirtualBranchesPromptKind::CreateAbove | VirtualBranchesPromptKind::CreateBelow => {
            if !branch_exists {
                return None;
            }
            VirtualBranchesEdit::InsertRelativeTo {
                name: free_name(value),
                anchor: branch.to_string(),
                below: kind == VirtualBranchesPromptKind::CreateBelow,
            }
        }
        VirtualBranchesPromptKind::SetTarget => {
            // The target is a revision, not a virtual-branches branch, so it is
            // checked against the repository's real branch list instead.
            if !is_submittable_branch_name(value) || !is_git_branch(value) {
                return None;
            }
            VirtualBranchesEdit::SetTarget {
                target: value.to_string(),
            }
        }
        VirtualBranchesPromptKind::SetParent => {
            if !branch_exists {
                return None;
            }
            // An empty field means "sit on the target directly", which is how a
            // branch leaves a stacked branch, so it is an answer here rather than a
            // missing one.
            if !value.is_empty() && (value == branch || !in_virtual_branches(value)) {
                return None;
            }
            VirtualBranchesEdit::SetParent {
                name: branch.to_string(),
                parent: (!value.is_empty()).then(|| value.to_string()),
            }
        }
        VirtualBranchesPromptKind::MoveToStackedBranch => {
            if !branch_exists || value.is_empty() || value == branch || !in_virtual_branches(value)
            {
                return None;
            }
            VirtualBranchesEdit::MoveToStackedBranch {
                name: branch.to_string(),
                stacked_branch_base: value.to_string(),
                relative_to: None,
                below: false,
            }
        }
        VirtualBranchesPromptKind::Remove => {
            if !branch_exists {
                return None;
            }
            VirtualBranchesEdit::Remove {
                name: branch.to_string(),
            }
        }
        VirtualBranchesPromptKind::AssignFile => {
            // An empty field is not a missing answer here: it is how a file
            // goes back to the unassigned bucket.
            let path = path?;
            if !value.is_empty() && !in_virtual_branches(value) {
                return None;
            }
            VirtualBranchesEdit::AssignFile {
                path: path.to_path_buf(),
                branch: (!value.is_empty()).then(|| value.to_string()),
            }
        }
        VirtualBranchesPromptKind::AssignHunk => {
            // An empty field is how a hunk goes back to nobody's, the same as
            // for a whole file.
            let (Some(path), Some(hunk)) = (path, hunk) else {
                return None;
            };
            if !value.is_empty() && !in_virtual_branches(value) {
                return None;
            }
            VirtualBranchesEdit::AssignHunk {
                path: path.to_path_buf(),
                hunk,
                branch: (!value.is_empty()).then(|| value.to_string()),
            }
        }
        VirtualBranchesPromptKind::CommitMessage => {
            let path = path?;
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
            let name = assignment_of(path)?;
            VirtualBranchesEdit::CommitPaths {
                name,
                message: message.to_string(),
                paths: vec![path.to_path_buf()],
            }
        }
        VirtualBranchesPromptKind::CommitBranch => {
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
            VirtualBranchesEdit::CommitPaths {
                name: branch.to_string(),
                message: message.to_string(),
                paths,
            }
        }
    };
    // A creation whose name could not be made free is not an edit at all, and
    // the confirm button stays disabled rather than dispatching an empty name.
    match &edit {
        VirtualBranchesEdit::Create { name }
        | VirtualBranchesEdit::InsertRelativeTo { name, .. }
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

    /// A virtual branches holding `api` on the target, with `ui` stacked on it.
    fn membership() -> impl Fn(&str) -> bool {
        let names = ["api", "ui"].map(str::to_string);
        move |name: &str| names.iter().any(|known| known == name)
    }

    /// `taken` and `api` are spoken for; nothing else is.
    fn taken() -> impl Fn(&str) -> bool {
        let names = ["api", "taken"].map(str::to_string);
        move |name: &str| names.iter().any(|known| known == name)
    }

    /// The name the prompt would create, under the same rule the real one
    /// uses: a name the virtual branches already lists is taken too, not only a
    /// name some other branch holds.
    fn free(value: &str) -> String {
        let taken = taken();
        let in_virtual_branches = membership();
        free_branch_name(value, &|name| taken(name) || in_virtual_branches(name))
    }

    fn edit(
        kind: VirtualBranchesPromptKind,
        branch: &str,
        value: &str,
    ) -> Option<VirtualBranchesEdit> {
        edit_for(
            kind,
            branch,
            None,
            None,
            value,
            &membership(),
            &free,
            &|name| matches!(name, "main" | "develop" | "api"),
            &|_| None,
            &|_| Vec::new(),
        )
    }

    fn assign(value: &str) -> Option<VirtualBranchesEdit> {
        edit_for(
            VirtualBranchesPromptKind::AssignFile,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            None,
            value,
            &membership(),
            &free,
            &|name| matches!(name, "main" | "develop" | "api"),
            &|_| Some("ui".to_string()),
            &|_| Vec::new(),
        )
    }

    fn commit(value: &str) -> Option<VirtualBranchesEdit> {
        edit_for(
            VirtualBranchesPromptKind::CommitMessage,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            None,
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

    fn commit_branch(branch: &str, value: &str) -> Option<VirtualBranchesEdit> {
        edit_for(
            VirtualBranchesPromptKind::CommitBranch,
            branch,
            None,
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
        assert!(edit(VirtualBranchesPromptKind::BranchActions, "api", "").is_none());
    }

    #[test]
    fn creating_uses_the_typed_name() {
        let Some(VirtualBranchesEdit::Create { name }) =
            edit(VirtualBranchesPromptKind::Create, "", "feat/x")
        else {
            panic!("expected a create edit");
        };
        assert_eq!(name, "feat/x");
    }

    #[test]
    fn creating_over_an_existing_branch_gets_suffixed() {
        let Some(VirtualBranchesEdit::Create { name }) =
            edit(VirtualBranchesPromptKind::Create, "", "api")
        else {
            panic!("expected a create edit");
        };
        assert_eq!(name, "api-2");
    }

    #[test]
    fn an_unusable_name_produces_no_edit() {
        assert!(edit(VirtualBranchesPromptKind::Create, "", "   ").is_none());
    }

    #[test]
    fn inserting_below_stacked_branches_on_the_branch_and_inserting_above_takes_its_place() {
        let Some(VirtualBranchesEdit::InsertRelativeTo {
            name,
            anchor,
            below,
        }) = edit(VirtualBranchesPromptKind::CreateBelow, "api", "ui")
        else {
            panic!("expected an insert edit");
        };
        assert_eq!(name, "ui-2", "a taken name is suffixed, not refused");
        assert_eq!(anchor, "api");
        assert!(below);

        let Some(VirtualBranchesEdit::InsertRelativeTo { below, .. }) =
            edit(VirtualBranchesPromptKind::CreateAbove, "api", "web")
        else {
            panic!("expected an insert edit");
        };
        assert!(!below, "above is the other direction");
    }

    #[test]
    fn inserting_needs_a_branch_that_is_still_there() {
        // The branch vanished from the virtual branches while the prompt was open.
        assert!(edit(VirtualBranchesPromptKind::CreateAbove, "gone", "x").is_none());
        assert!(edit(VirtualBranchesPromptKind::CreateBelow, "gone", "x").is_none());
    }

    #[test]
    fn the_target_must_be_a_real_branch() {
        let Some(VirtualBranchesEdit::SetTarget { target }) =
            edit(VirtualBranchesPromptKind::SetTarget, "", "develop")
        else {
            panic!("expected a set-target edit");
        };
        assert_eq!(target, "develop");

        // A virtual branch is not a revision the virtual branches can be built on.
        assert!(edit(VirtualBranchesPromptKind::SetTarget, "", "ui").is_none());
        assert!(edit(VirtualBranchesPromptKind::SetTarget, "", "").is_none());
    }

    #[test]
    fn re_parenting_accepts_a_virtual_base_or_nothing_at_all() {
        let Some(VirtualBranchesEdit::SetParent { name, parent }) =
            edit(VirtualBranchesPromptKind::SetParent, "api", "ui")
        else {
            panic!("expected a set-parent edit");
        };
        assert_eq!(name, "api");
        assert_eq!(parent.as_deref(), Some("ui"));

        // Empty means "back onto the target", which is how a branch leaves a
        // stacked branch.
        let Some(VirtualBranchesEdit::SetParent { parent, .. }) =
            edit(VirtualBranchesPromptKind::SetParent, "ui", "")
        else {
            panic!("expected a set-parent edit");
        };
        assert_eq!(parent, None);
    }

    #[test]
    fn a_branch_cannot_be_stacked_on_itself_or_on_nothing() {
        assert!(edit(VirtualBranchesPromptKind::SetParent, "api", "api").is_none());
        assert!(edit(VirtualBranchesPromptKind::MoveToStackedBranch, "api", "api").is_none());
        assert!(edit(VirtualBranchesPromptKind::MoveToStackedBranch, "api", "").is_none());
        assert!(
            edit(
                VirtualBranchesPromptKind::MoveToStackedBranch,
                "api",
                "gone"
            )
            .is_none()
        );
    }

    #[test]
    fn moving_into_a_stacked_branch_names_the_stacked_branch_base() {
        let Some(VirtualBranchesEdit::MoveToStackedBranch {
            name,
            stacked_branch_base,
            relative_to,
            below,
        }) = edit(VirtualBranchesPromptKind::MoveToStackedBranch, "api", "ui")
        else {
            panic!("expected a move edit");
        };
        assert_eq!(name, "api");
        assert_eq!(stacked_branch_base, "ui");
        assert_eq!(relative_to, None);
        assert!(!below);
    }

    #[test]
    fn removing_needs_a_branch_that_is_there() {
        let Some(VirtualBranchesEdit::Remove { name }) =
            edit(VirtualBranchesPromptKind::Remove, "api", "")
        else {
            panic!("expected a remove edit");
        };
        assert_eq!(name, "api");
        assert!(edit(VirtualBranchesPromptKind::Remove, "gone", "").is_none());
    }

    #[test]
    fn assigning_a_file_names_the_branch_it_goes_to() {
        let Some(VirtualBranchesEdit::AssignFile { path, branch }) = assign("ui") else {
            panic!("expected an assign edit");
        };
        assert_eq!(path, std::path::Path::new("src/lib.rs"));
        assert_eq!(branch.as_deref(), Some("ui"));
    }

    #[test]
    fn an_empty_field_unassigns_rather_than_refusing() {
        let Some(VirtualBranchesEdit::AssignFile { branch, .. }) = assign("") else {
            panic!("an empty field should still produce an edit");
        };
        assert_eq!(branch, None);
    }

    #[test]
    fn a_file_can_only_go_to_a_branch_that_exists() {
        assert!(assign("nope").is_none());
        // A real Git branch is not a virtual-branches branch, so it is not a target.
        assert!(assign("main").is_none());
    }

    #[test]
    fn assigning_needs_a_file() {
        assert!(
            edit_for(
                VirtualBranchesPromptKind::AssignFile,
                "",
                None,
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

    /// Every kind the dialog can be opened for.
    ///
    /// Hand-written rather than derived, because deriving it would need a way to
    /// enumerate a plain enum, and a list that has to be updated by hand is exactly
    /// what makes the test below worth having: adding a kind without adding it here
    /// fails, and adding a kind here without implementing it fails too.
    const ALL_KINDS: [VirtualBranchesPromptKind; 12] = [
        VirtualBranchesPromptKind::BranchActions,
        VirtualBranchesPromptKind::Create,
        VirtualBranchesPromptKind::CreateAbove,
        VirtualBranchesPromptKind::CreateBelow,
        VirtualBranchesPromptKind::SetTarget,
        VirtualBranchesPromptKind::SetParent,
        VirtualBranchesPromptKind::MoveToStackedBranch,
        VirtualBranchesPromptKind::Remove,
        VirtualBranchesPromptKind::AssignFile,
        VirtualBranchesPromptKind::AssignHunk,
        VirtualBranchesPromptKind::CommitMessage,
        VirtualBranchesPromptKind::CommitBranch,
    ];

    #[test]
    fn every_prompt_kind_produces_an_edit_or_says_it_submits_nothing() {
        // The docs claim every `VirtualBranchesEdit` variant is reachable from the
        // view. `edit_for` has no catch-all — adding a kind is a compile error,
        // not a silent no-op — so this pins the other half of the claim: that
        // each kind really does resolve to something, and that the ones that
        // submit nothing are only the ones meant to.
        // Everything the dialog could be handed exists: `x` names a
        // virtual-branches branch, a real branch and a hunk of the file, and the
        // branch has an assigned path to commit.
        let member = membership();
        let in_virtual_branches = |name: &str| name == "x" || member(name);
        let free = |name: &str| name.to_string();

        for kind in ALL_KINDS {
            let edit = edit_for(
                kind,
                "api",
                Some(std::path::Path::new("src/lib.rs")),
                Some(gitcomet_core::virtual_branches::HunkFingerprint::default()),
                "x",
                &in_virtual_branches,
                &free,
                // The repository does hold the branch the field names, so
                // every kind that needs an existing answer gets one.
                &|_| true,
                &|_| Some("api".to_string()),
                &|branch: &str| {
                    if branch == "api" {
                        vec![std::path::PathBuf::from("src/lib.rs")]
                    } else {
                        Vec::new()
                    }
                },
            );
            if kind == VirtualBranchesPromptKind::BranchActions {
                assert!(
                    edit.is_none(),
                    "the action list submits nothing; picking a row is the interaction"
                );
            } else {
                assert!(edit.is_some(), "{kind:?} produced no edit at all");
            }
        }
    }

    #[test]
    fn assigning_a_hunk_names_the_file_the_hunk_and_the_branch() {
        let hunk = gitcomet_core::virtual_branches::HunkFingerprint::of(&["new\n"], 1);
        let Some(VirtualBranchesEdit::AssignHunk {
            path,
            hunk: key,
            branch,
        }) = edit_for(
            VirtualBranchesPromptKind::AssignHunk,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            Some(hunk),
            "ui",
            &membership(),
            &free,
            &|_| true,
            &|_| None,
            &|_| Vec::new(),
        )
        else {
            panic!("expected a hunk assignment");
        };
        assert_eq!(path, std::path::PathBuf::from("src/lib.rs"));
        assert_eq!(key, hunk);
        assert_eq!(branch.as_deref(), Some("ui"));
    }

    #[test]
    fn an_empty_field_unassigns_the_hunk_too() {
        let hunk = gitcomet_core::virtual_branches::HunkFingerprint::default();
        let Some(VirtualBranchesEdit::AssignHunk { branch, .. }) = edit_for(
            VirtualBranchesPromptKind::AssignHunk,
            "",
            Some(std::path::Path::new("src/lib.rs")),
            Some(hunk),
            "",
            &membership(),
            &free,
            &|_| true,
            &|_| None,
            &|_| Vec::new(),
        ) else {
            panic!("an empty field should still produce an edit");
        };
        assert_eq!(branch, None);
    }

    #[test]
    fn assigning_a_hunk_needs_both_a_file_and_a_hunk() {
        let any = edit_for(
            VirtualBranchesPromptKind::AssignHunk,
            "",
            None,
            Some(gitcomet_core::virtual_branches::HunkFingerprint::default()),
            "ui",
            &membership(),
            &free,
            &|_| true,
            &|_| None,
            &|_| Vec::new(),
        );
        assert!(any.is_none(), "no file, no hunk to assign");

        let any = edit_for(
            VirtualBranchesPromptKind::AssignHunk,
            "",
            Some(std::path::Path::new("a.rs")),
            None,
            "ui",
            &membership(),
            &free,
            &|_| true,
            &|_| None,
            &|_| Vec::new(),
        );
        assert!(any.is_none(), "no hunk, nothing to address");
    }

    #[test]
    fn a_hunk_can_only_go_to_a_branch_that_exists() {
        let edit = |value: &str| {
            edit_for(
                VirtualBranchesPromptKind::AssignHunk,
                "",
                Some(std::path::Path::new("src/lib.rs")),
                Some(gitcomet_core::virtual_branches::HunkFingerprint::default()),
                value,
                &membership(),
                &free,
                &|_| true,
                &|_| None,
                &|_| Vec::new(),
            )
        };
        assert!(edit("nope").is_none());
        assert!(edit("ui").is_some());
    }

    #[test]
    fn committing_carries_the_message_and_the_file() {
        let Some(VirtualBranchesEdit::CommitPaths {
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
                VirtualBranchesPromptKind::CommitMessage,
                "",
                Some(std::path::Path::new("src/lib.rs")),
                None,
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
                VirtualBranchesPromptKind::CommitMessage,
                "",
                None,
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
        let Some(VirtualBranchesEdit::CommitPaths {
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
    fn the_suffix_search_stops_at_the_first_free_number() {
        let mut spoken_for = vec!["busy".to_string()];
        for suffix in 2..=6 {
            spoken_for.push(format!("busy-{suffix}"));
        }
        let is_taken = move |name: &str| spoken_for.iter().any(|taken| taken == name);
        assert_eq!(free_branch_name("busy", &is_taken), "busy-7");
    }

    #[test]
    fn a_name_taken_all_the_way_up_answers_nothing_rather_than_a_taken_name() {
        // The search runs to 99 and then gives up, and an empty answer is what
        // the prompt reads as "do not submit". Falling back to the typed name
        // instead would offer something already taken and let the backend be
        // the one to refuse it.
        let everything = |name: &str| name.starts_with("busy");
        assert_eq!(free_branch_name("busy", &everything), "");
    }

    #[test]
    fn every_branch_edit_is_refused_for_a_branch_that_is_gone() {
        for kind in [
            VirtualBranchesPromptKind::CreateAbove,
            VirtualBranchesPromptKind::CreateBelow,
            VirtualBranchesPromptKind::SetParent,
            VirtualBranchesPromptKind::MoveToStackedBranch,
            VirtualBranchesPromptKind::Remove,
        ] {
            assert!(
                edit(kind, "gone", "ui").is_none(),
                "{:?} should refuse a missing subject",
                kind
            );
        }
    }
}
