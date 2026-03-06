# libgit2 Resource Repositories

This directory vendors merge-related repositories from libgit2:

- `merge-recursive`
- `merge-resolve`
- `merge-whitespace`
- `mergedrepo`
- `redundant.git`
- `twowaymerge.git`

Worktree-style fixtures keep git metadata in `.gitted` (matching libgit2's
resource layout). Tests rehydrate those by renaming `.gitted` to `.git` inside
a temporary copy before running git commands.

The integration test `tests/libgit2_git_resources.rs` validates that these
fixtures are structurally usable and runs extraction/merge checks directly
against real git repositories.
