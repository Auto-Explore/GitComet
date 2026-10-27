# Bare Repositories

GitComet opens bare repositories for history, committed files, remote operations, and linked worktree management.

## Recognizing bare mode

A **Git Bare** badge appears beside the worktree badge, including in extension views. Its tooltip explains that the repository has no active working tree. The branch badge identifies HEAD; checkout from that badge is unavailable.

The changes panel shows **No working tree in bare mode**. History omits the bare repository's uncommitted row, while linked checkouts keep their own change rows. The file browser shows the selected commit or HEAD. An unborn repository has no committed files to show.

Commands that need working files remain discoverable but disabled with **Requires a working tree; this repository is bare.**

| Available | Requires a working tree |
| --- | --- |
| History, reflog, committed files, diffs, blame, stash viewing and patch export | Working-file editing, staging, unstaging, discard, commit and amend |
| Fetch and push | Ordinary Git pull, checkout, merge, rebase, cherry-pick and revert |
| Branch, tag, remote, upstream and linked worktree management | Create-and-checkout, mixed or hard reset, patch application and conflict resolution |
| Soft reset and object-based HEAD squash, subject to linked branch ownership | Submodule checkout changes and stash creation, apply, pop or drop |
| Terminal and document tools | Repository editor saves |

Soft reset and HEAD squash refuse to change a symbolic HEAD branch checked out in another worktree. A detached bare HEAD can still be changed. Documents outside the repository remain editable.

## Disable bare mode

Click **Git Bare → Disable bare mode**. GitComet first checks the repository without changing it.

If the folder is an existing checkout with a conventional `.git` directory and only its bare setting is wrong, GitComet repairs the local configuration directly. It keeps the working files, index and references intact.

For a genuine bare repository, GitComet asks for confirmation. The dialog identifies the selected folder and HEAD. Conversion keeps that folder at the same path, moves Git data into `.git`, and creates working files from HEAD. An unborn branch produces an empty checkout. Git's configured checkout filters may run. Changes a filter makes outside the prepared checkout cannot be rolled back.

Before confirming, stop other Git tools, server jobs and the git-annex assistant using this repository or its linked worktrees. Keep them stopped until conversion finishes. GitComet coordinates its own windows and processes, but cannot stop unrelated tools from writing.

The dialog reports conversion and reopening progress. The badge disappears after verification and reopening succeed. Closing the progress dialog does not cancel a conversion already running; click the badge to reopen it.

## Refusals and recovery

Conversion stops unchanged when HEAD is checked out elsewhere, the repository is busy, or its layout cannot be moved safely. Examples include unexpected files at the bare root, an index that would be replaced, inaccessible or nested linked worktrees, conflicting `.git` entries, and configuration or storage paths that depend on the old metadata location. GitComet reports the specific reason. Resolve that condition and try again; it does not detach HEAD or guess how to rewrite storage paths.

GitComet prepares a replacement beside the original repository and retains a recovery journal and backup through installation and verification. If interrupted, opening the repository resumes recovery. When recovery detects unexpected changes, it preserves the copies and names the recovery location in the error. Keep those copies until recovery is resolved.

## Large files

In [Git LFS](git-lfs.md), fetch for all references, push, prune, check and installation remain available. A committed diff can fetch its displayed content. Download-and-checkout, track, lock and ordinary unlock require a working tree; an explicitly selected force unlock remains available.

In [git-annex](git-annex.md), get by key and repository-wide get, drop, copy and move remain available. Repository-wide actions are labelled **all versions**. A file-specific action is never widened into an all-versions operation. Pull, push and sync remain available with or without content transfer. Initialization, remote configuration, trust, description, copy counts, checks, unused-content cleanup and restage retain their normal prerequisites. Add, lock, unlock, adjusted-branch changes, webapp and assistant controls require a working tree.
