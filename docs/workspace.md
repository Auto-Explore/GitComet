# The Workspace: virtual branches and stacked branches

GitComet has two ways to work. The **Branches** tab is the ordinary Git
workflow: check out a branch, work, commit. The **Workspace** tab is a
GitButler-style alternative, for working on several things at once without
checking any of them out.

This document explains the model behind the Workspace tab: what a virtual
branch is, how stacks work, what `gitcomet/workspace` is for, and which
operations change which piece of it.

## Why another workflow

In a traditional checkout you have one branch at a time. To work on two
unrelated features you either switch back and forth — losing your place and
your build state — or you use separate worktrees, which means separate
directories to keep straight.

A workspace lets several branches be applied to one working directory at the
same time. Each file in the working tree belongs to a branch, so you edit all
of them, and commit each set of changes to the branch it belongs to. Nothing
is checked out except the workspace itself.

The Branches tab is unchanged and remains fully supported. The Workspace is
for people who prefer this model, not a replacement.

## Virtual branches

A **virtual branch** is an ordinary Git branch under `refs/heads/`, plus one
piece of extra information Git has nowhere to store: what it is stacked on,
and whether it is currently applied.

```rust
pub struct VirtualBranch {
    pub name: String,             // the Git branch name
    pub parent: Option<String>,   // the branch it is stacked on, if any
    pub applied: BranchApplyState,// Applied or Unapplied
    pub description: Option<String>,
    pub order: Option<u32>,       // position among siblings
}
```

That is the whole abstraction. A virtual branch is not a special kind of
branch, and it does not need server support: `feature/api` is a normal branch
you can push, review, and merge like any other.

`parent` is what makes stacking work. `None` means the branch sits directly on
the target. `Some("feature/api")` means it sits on `feature/api`, and its pull
request targets `feature/api` rather than the target branch.

## Applied and unapplied

Every branch has an **applied** or **unapplied** state.

- **Applied** — its changes are part of the working directory.
- **Unapplied** — it exists as a branch but contributes nothing to the working
  directory.

Applying and unapplying never involve `git checkout`. Applying merges the
branch's commits into the workspace branch; unapplying rebuilds the workspace
without it.

```
main
├── feature/api     applied
├── feature/ui      applied
└── feature/e2e     unapplied
```

All three exist as branches. The two that are applied are what you see in the
working directory.

## Stacks

A **stack** is a chain of branches, each stacked on the one below it. The
bottom of the stack sits on the target branch.

```
main
└── feature/api
    └── feature/ui
        └── feature/e2e
```

`feature/e2e` is a real branch whose parent is `feature/ui`, which is a real
branch whose parent is `feature/api`. Each can be pushed and reviewed
separately:

```
feature/api → main
feature/ui  → feature/api
feature/e2e → feature/ui
```

Stacks are not stored as a separate structure. They are derived from the
`parent` fields by `WorkspaceState::stacks()`, which means a stack can never
disagree with the branches that make it up.

### Independent and stacked are the same thing

There is no "stacked branch" type and no "independent branch" type. A branch
is independent when `parent` is `None` and stacked when it is `Some`. That is
why every structural edit is the same edit — changing one field:

| Action | What changes |
| --- | --- |
| Stack a branch onto another | `parent = Some(other)` |
| Make a branch independent | `parent = None` |
| Move a branch to another stack | `parent = Some(new_base)` |
| Reorder within a stack | swap two siblings' `order` |

`WorkspaceState` validates every one of these before it is written, so a move
that would close a loop is refused with the state left exactly as it was.

## The target branch

The workspace has a **target**: the base every independent branch is built on.
`main` or `origin/main` — a revision, not necessarily a local branch.

```
origin/main
├── feature/api
│   └── feature/ui
└── fix/logging
```

Changing the target rebases the independent branches onto the new one and
rebuilds the workspace. Branches stacked on another virtual branch are not
rebased at that point; they move when their base does.

## The workspace branch

GitComet keeps a branch called `gitcomet/workspace` that holds the combined
tree of every applied branch:

```
gitcomet/workspace
│  merge of applied branches
├── feature/api
├── feature/ui
└── fix/logging
```

It is an implementation detail, not a branch you work on:

- You never commit to it. Commits belong to the virtual branches.
- It is rebuilt from scratch whenever the applied set changes, so it is a pure
  function of the virtual-branch state plus the target.
- It is filtered out of the ordinary branch list, so it never appears as
  something you can check out or delete by accident.

Rebuilding from scratch rather than patching is deliberate. Applying a branch
replays the same sequence of merges from the target, so the branch can always
be reconstructed, and unapplying one later is a replay rather than an edit of
history that may already have been pushed.

Each merge gets its own commit, so the workspace history shows which branch
contributed what.

## Assigning changes to branches

The point of the workspace is that unrelated changes can coexist in one
working directory and still belong to different branches.

```
src/api/auth.ts        → feature/api
src/api/users.ts       → feature/api
src/components/User.tsx → feature/ui
src/components/List.tsx → feature/ui
tests/e2e/users.test.ts → feature/e2e
README.md              → Unassigned
```

A file's assignment is recorded per repository and kept in
`.git/gitcomet/assignments.json`, rather than derived from the diff. A file
therefore keeps its branch across edits made while it is unmodified — the
alternative would silently move a file to "Unassigned" the moment someone
saved it and nothing changed.

Assigning is currently per file. Assigning individual hunks of one file to
different branches is not implemented.

## Committing

Committing a branch collects the files assigned to it and commits them against
**that branch's own base**, not against the workspace. The commit therefore
contains only the work assigned to that branch.

The commit is built with a temporary index, so your real index and working
tree are untouched — you can have unrelated staged and unstaged changes in the
same directory and still commit one branch cleanly.

After a commit the workspace is rebuilt, since the branch moved.

## Conflicts

Two applied branches that changed the same lines cannot both be applied. The
Workspace reports this in terms of the branches involved rather than as a bare
checkout or rebase conflict:

```
⚠ feature/ui conflicts with feature/api
```

Applying a conflicting branch leaves the workspace branch unchanged and records
the conflict on the branch that caused it. Unapplying one of the two, or
resolving the conflict, clears it.

## State on disk

Everything the model needs lives in `.git/gitcomet/`:

| File | Contents |
| --- | --- |
| `workspace.json` | target branch and every virtual branch |
| `assignments.json` | which branch each changed file belongs to |

Both are written atomically and are ignored if unreadable: a workspace file
written by a newer or hand-edited version makes the Workspace tab show an
empty workspace the user can rebuild, rather than making the repository
unusable.

## Pushing

Virtual branches are ordinary branches, so pushing needs no special server
support. `refs/heads/feature/api` is pushed as `feature/api`.

`GitRepository::push_virtual_branch` pushes the *named ref* rather than `HEAD`.
That is the one thing a workspace has to be explicit about: the user is
usually sitting on an unrelated branch, and `git push` with no arguments would
publish that one instead of the branch whose Push button they clicked. The
backend picks the remote the same way the ordinary push path does and sets
the upstream on the branch it pushed.

What a workspace adds is the *base*: a stacked branch's pull request targets
the branch below it, not the target. `WorkspaceState::base_branch` computes
that base, and `virtual_branch_push_target` exposes it so the UI can say what
a push will do before it does it.

The data model keeps each branch's base explicit, which is what stacked pull
requests will need later; the multi-PR workflow itself is not implemented yet.

## What the Workspace tab can do today

Per branch: apply/unapply, push, a `⋯` menu (stack a new branch on this one,
change its base, move it into another stack, remove it from the workspace), and
`↑`/`↓` to reorder it inside its stack — the arrows are disabled at the ends of
a stack rather than offered and refused.

Per workspace: `New branch` on the summary line, and `Change` on the target row
to rebase everything onto a different branch.

Per file: commit the file to the branch it is assigned to.

A conflict from a failed apply is shown above the stacks and can be dismissed.

Every `WorkspaceEdit` variant is now reachable from the view. The ones that
need a name go through a single popover, `PopoverKind::WorkspacePrompt`:

- the body is the same for all of them, so they share one variant rather than
  seven — what differs is the label on the field and which `WorkspaceEdit`
  confirming builds;
- opened from a branch row it shows that branch's actions first, because
  stacking, re-parenting, joining a stack and removing are the same four
  questions about the same branch and a row has no room for four buttons;
- the step shown is host state rather than part of the kind, so narrowing the
  list to one action does not change which popover is open — a kind swap would
  read to the fingerprint as a different dialog replacing this one;
- validation is stricter than the model. A base that does not exist, or a
  branch that has gone since the prompt opened, is refused at the prompt rather
  than accepted and failed later on a background thread with a rebase error;
  a name that is already taken is suffixed rather than passed to Git.

Still not reachable: `AssignFile` (dragging a file onto a branch) and
`CommitPaths` with a written message (the file row commits with an empty
message). Both are complete below the view.

## Where the code is

| Piece | Location |
| --- | --- |
| Model, validation, stacks | `crates/gitcomet-core/src/workspace.rs` |
| Service contract | `crates/gitcomet-core/src/services.rs` (workspace section) |
| Git plumbing | `crates/gitcomet-git-gix/src/repo/workspace.rs` |
| Per-repository state | `crates/gitcomet-state/src/model/workspace.rs` |
| Edit dialog | `crates/gitcomet-ui-gpui/src/view/panels/popover/workspace_prompt.rs` |
| Dialog behaviour | `crates/gitcomet-ui-gpui/src/view/panels/popover/host/workspace_prompts.rs` |
| Messages and effects | `crates/gitcomet-state/src/store/effects/workspace_effects.rs` |
| Reducers | `crates/gitcomet-state/src/store/reducer/workspace.rs` |
| The view | `crates/gitcomet-ui-gpui/src/view/panes/sidebar/workspace.rs` |

## Naming

GitComet already had a use for the word "workspace" — a saved window grouping
several repositories, in `session::workspaces`. The two are unrelated: that one
groups repository *tabs* in a window, this one groups *branches* in a
repository's working tree. The types are named apart (`WorkspaceRepoState`
versus `session::Workspace`) to keep them from being confused.
