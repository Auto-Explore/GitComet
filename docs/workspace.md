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

### Above and below

A branch row offers *New branch above this* and *New branch below this*, and
they are genuinely different edits:

```
target ── api ── ui        insert "ui2" above ui   →  api ── ui ── ui2
                           insert "ui2" below ui   →  api ── ui2 ── ui
```

*Above* stacks the new branch on the anchor: it starts at the anchor's tip and
displaces nothing. *Below* puts it in the anchor's place: it starts at the
anchor's base and the anchor is then replayed on top of it. Either way the
branches that were already stacked on the anchor stay where they were.

The *below* case has to rewrite history, because the anchor's commits were built
on the new branch's commit, not the other way round. That rewrite goes through
`merge-tree` + `commit-tree` rather than `git rebase`, because `git rebase`
refuses outright when the working tree has unstaged changes — which in a
workspace is the normal state, not a mistake. The cost is that the branch's
commits are replayed as one.

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

## How the workspace reaches the working directory

The workspace branch is not decoration: while the user is inside the Workspace
view, the working directory *is* `gitcomet/workspace`. Apply, unapply, create,
commit and re-target all rebuild that branch and then move the working
directory onto it, which is what makes a branch's changes actually appear in the
files.

Rebuilding the ref is plumbing (`merge-tree` + `commit-tree`) and never touches
the files. Moving the files onto it is a separate, deliberate step:

```
git checkout --no-recurse-submodules gitcomet/workspace   # entering
git read-tree -u -m <old-tip> <new-tip>                  # following a rebuild
git checkout --no-recurse-submodules <previous-branch>   # leaving
```

`read-tree -u -m` is the two-way tree switch `git checkout` performs internally.
Its behaviour is the safety property of the whole feature:

- a file the user has edited and that the applied-set change does *not* touch
  **keeps the edit** and takes the new committed version underneath it;
- a file that is both edited *and* changed by the switch is **refused**, with
  the blocking file named — git's own rule, and the correct one, because there
  is no automatic answer to that conflict.

The sync runs *before* the ref is moved, so a refusal leaves the index, the
working tree and the ref all exactly as they were. There is no state in which
they disagree.

A rebuild only touches the working directory when the workspace is actually
checked out; a workspace the user has not entered never changes their files.

### Where the switch is driven from

Opening the Workspace tab enters; switching to any other tab leaves. Both go
through `set_sidebar_mode`, which is the only place that knows the mode
changed. Entering is deferred behind `LoadWorkspace` when the workspace has
never been read: a checkout cannot run before the state says a workspace
exists, and a rejected one would leave the user on a tab describing branches
they are not actually on.

`WorkspaceRepoState` holds the two pieces of session state this needs:

| Field | Meaning |
| --- | --- |
| `active` | the working directory is on `gitcomet/workspace` |
| `checkout_base` | the branch to go back to, captured on entry |

Neither survives a reload as a *decision* — `workspace_loaded` infers `active`
from whether `HEAD` is the workspace branch, because that is the only true
answer after a restart — and a refused switch leaves both untouched, since git
never moved `HEAD` and claiming otherwise would make the next leave try to undo
a switch that never happened.

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

Assignment also works **per hunk**: one file can have its changes split across
several branches. See below.

## Assigning hunks

A file that goes to more than one branch is *split*. In the diff view, right
click a hunk and choose **Assign to a branch…**; in the Workspace tab the file
row then reads `feature/api + feature/ui` instead of a single branch.

The hard part is not recording the assignment but *finding the hunk again* at
commit time, when the same change is diffed against a different base:

```
the view diffs      HEAD          →  the commit diffs    feature/api's base
```

A line range cannot survive that — the same change has different line numbers
on the two sides. So a hunk is identified by a **fingerprint**: how many lines
it adds, how many it removes, and an FNV-1a hash of the text it produces.

FNV rather than the standard library's hasher, because this is written to
disk: a hasher whose output changes between Rust releases would silently
invalidate every assignment a user ever made.

Two consequences worth knowing:

- **Two identical hunks share a fingerprint.** Two edits that add and remove
  exactly the same text are the same fingerprint, and both follow the
  assignment. Deliberate: they are indistinguishable to somebody reading the
  file, and picking one would be a coin flip dressed up as precision.
- **A hunk is what `git diff` calls a hunk.** Two edits three lines apart are
  one `@@` to the user, so they are one unit here too — `HUNK_CONTEXT_LINES`
  records the context the model groups by. If it disagreed with the diff, an
  assignment would match neither span and quietly apply to nothing.

A split file is committed by every branch holding part of it: each takes the
base lines everywhere except the hunks assigned to it, so `feature/api`'s commit
contains the API change and *not* the UI one, even though both are sitting in
the same working tree. Line endings come from the base, so splitting a CRLF
file does not rewrite every line of it.

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

The message names **both** branches, which is the part that takes work:

- `git merge-tree` reports a conflict on **stdout** — the tree it would have
  written, then one `<mode> <oid> <stage>\t<path>` line per conflicted entry —
  and exits non-zero with **stderr empty**. An error built from stderr, which is
  the obvious thing to do, arrives at the user with no detail at all.
- The merge itself cannot name the *other* branch: by the time it runs, every
  already-applied branch has been folded into one tree. So the backend works out
  which applied branch changed the most of the same paths and reports that one.
- When it cannot attribute the conflict to anyone in particular, the banner says
  so rather than guessing — a branch name that is nearly right is worse than
  none, because the user acts on it.

## State on disk

Everything the model needs lives in `.git/gitcomet/`:

| File | Contents |
| --- | --- |
| `workspace.json` | target branch and every virtual branch |
| `assignments.json` | which branch each changed file — or each of its hunks — belongs to |

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

Per branch: apply/unapply, push, a `⋯` menu (new branch above it, new branch
below it, change its base, move it into another stack, commit every file
assigned to it, remove it from the workspace), and `↑`/`↓` to reorder it inside
its stack — the arrows are disabled at the ends of a stack rather than offered
and refused.

Per workspace: `New branch` on the summary line, and `Change` on the target row
to rebase everything onto a different branch.

Per file: `Assign` (or `Change`, when it already has a branch) to say which
branch commits it, and `Commit` once it has one. Clearing the assign field is
how a file goes back to the unassigned bucket — it is an answer, not a missing
one, which is why the prompt's confirm button stays live on an empty field.

Per hunk: right click a hunk in the diff view and choose *Assign to a branch…*.
This is the only place a file gets split, and the split is undone by assigning
the file whole again.

`Commit` opens a dialog pre-filled with `Update <path>`, the message the quick
commit synthesised, so accepting it is still one press while the field can be
replaced with something the user means. The branch it goes to is read *at
submit*, not captured when the dialog opened: a file can be reassigned while its
commit dialog is up, and committing to the branch it used to belong to would
put the change somewhere the user is no longer looking.

Committing a whole branch (`⋯` → *Commit its assigned files*) takes every file
assigned to that branch and nothing else. Its message field opens **empty**,
unlike the single-file one: there is no honest one-word summary of a branch's
changes, and a synthesised default would invite the user to accept a message
that describes nothing. The confirm button stays disabled until something is
typed, and a branch with no files assigned produces no commit at all rather than
an empty one.

A split file has no per-file *Commit* button, because it has no single branch
for the button to name; the branch's *Commit its assigned files* is what
commits it. Its *Assign* button stays live, and assigns the whole file to one
branch — a real answer to a real question, and one that clears the split.

A conflict from a failed apply is shown above the stacks and can be dismissed.

Every `WorkspaceEdit` variant is now reachable from the view, through a single
popover, `PopoverKind::WorkspacePrompt`, with one `WorkspacePromptKind` per
distinct thing the dialog can be asked for:

- the body is the same for all of them, so they share one variant rather than
  one each — what differs is the label on the field and which `WorkspaceEdit`
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
  a name that is already taken is suffixed rather than passed to Git;
- the mapping from `(kind, branch, path, text)` to an edit is a free function
  over predicates (`in_workspace`, `free_name`, `is_git_branch`,
  `assignment_of`, `paths_for`), so every rule above is unit-tested without
  standing up a popover host.

Not exposed anywhere: dragging a file onto a branch as a way to assign it — the
row's `Assign` button, or the diff view's hunk menu, is the only assignment
gesture. Everything else in `WorkspaceEdit` has a control.

## Acceptance criteria for #539

| # | Criterion | State |
| --- | --- | --- |
| 1 | Workspace view | done — third sidebar tab, existing tabs untouched |
| 2 | Existing branch workflow kept | done |
| 3 | Multiple virtual branches in one directory | done — the tab owns the working directory |
| 4 | Apply / unapply | done — rebuild moves the files |
| 5 | Independent virtual branches | done |
| 6 | Stacked / dependent branches | done |
| 7 | Create branches above/below existing ones | done — `WorkspaceEdit::InsertRelativeTo`, two distinct edits |
| 8 | Move branches between stacks | done — `MoveToStack` |
| 9 | Reorder within a stack | done — `Reorder` plus `↑`/`↓` |
| 10 | Assign files to virtual branches | done |
| 11 | Assign individual hunks | done — `HunkFingerprint`, diff hunk menu |
| 12 | Maintain `gitcomet/workspace` | done |
| 13 | Auto-update when the applied set changes | done — and moves the working directory when the workspace is checked out |
| 14 | Prevent direct commits to `gitcomet/workspace` | structurally — see below |
| 15 | Configurable target branch | done |
| 16 | Rebase when the target changes | done |
| 17 | Expose conflicts between branches | done — both branch names and the file, from git's stdout |
| 18 | Compatible with normal Git branches and remotes | done |
| 19 | Push individual virtual branches | done |
| 20 | Model allows stacked PRs later | done — `base_branch`, `virtual_branch_push_target` |
| 21 | Documentation | done — this file |

### On #14

There is no guard that rejects a commit naming `gitcomet/workspace`. Instead the
branch is filtered out of all three places branches are listed — the gix
iterator, the `for-each-ref` fallback, and the packed-refs reader — so no UI can
offer it for checkout, commit, merge, push or delete. That is a stronger
guarantee than a check that can be bypassed, but it does assume no code path
invents a branch name the user typed.

### On #11

The three things that were missing:

- **A hunk identity that survives the file moving.** `HunkFingerprint` hashes
  what a change adds and removes rather than where it sits. It is content-based
  rather than context-based (GitButler anchors to unidiff context), which makes
  it independent of *both* the base it was made against and the surrounding
  code, at the cost of treating two textually identical changes as one. See
  *Assigning hunks* above.
- **A commit path that can build a tree per branch.** `commit_paths_tree` sends
  a file git-add style when it belongs to one branch, and `stage_split_file`
  synthesizes a per-branch version of it when it does not — base lines
  everywhere except the hunks that branch owns, hashed straight into a
  temporary index. The working tree and the user's real index are never
  involved.
- **A control in the diff view.** The hunk context menu now offers *Assign to a
  branch…*, which opens the same `WorkspacePrompt` the sidebar uses.

A file split across branches is reported as `api + ui` rather than as
unassigned: it has no single branch, but it is assigned, and saying otherwise
would send the user looking for a file two branches are already committing.

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
| Hunk fingerprints from a rendered diff | `crates/gitcomet-ui-gpui/src/view/diff_utils.rs` |
| Hunk menu entry | `crates/gitcomet-ui-gpui/src/view/panels/popover/context_menu/diff_hunk.rs` |

## Naming

GitComet already had a use for the word "workspace" — a saved window grouping
several repositories, in `session::workspaces`. The two are unrelated: that one
groups repository *tabs* in a window, this one groups *branches* in a
repository's working tree. The types are named apart (`WorkspaceRepoState`
versus `session::Workspace`) to keep them from being confused.
