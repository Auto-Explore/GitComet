# git-annex

GitComet works with repositories that manage large files with [git-annex](https://git-annex.branchable.com). GitComet does not move annexed content itself. Content commands run `git annex`, so location tracking, the numcopies check and special remotes behave exactly as they do in a terminal. Ordinary Git commands still go through Git.

**Settings → Executables** shows whether Git can run `git annex`. When it is **Not found**, GitComet still recognises annexed files, but every git-annex command is listed disabled with "(install git-annex)". Pull and push through git-annex need git-annex 10.20230626 or later.

## What GitComet shows

| Where | What |
| --- | --- |
| Changed-file rows and commit details | An **annex** chip. It turns to the warning colour when the content is not in this clone. |
| Diff pane | A card with the key and size of each side. When the content is here, the real diff sits below the card, including images and older versions of unlocked files. **Where is it?** lists the repositories git-annex has recorded as holding the content. It reads git-annex's location log and does not contact the remotes. |
| Sidebar | A **git-annex** section with this clone, other repositories and special remotes, their type and trust level, and the numcopies setting or adjusted-branch mode. The section appears only in repositories that use git-annex. With the sidebar collapsed, the same section opens from its icon in the rail. |
| Branch lists | The `git-annex` and `synced/*` branches are hidden, locally and on remotes: in the sidebar, the branch badge's picker, the delete, rebase-onto and branch-from lists, and the upstream picker. A checked-out bookkeeping branch stays listed. |
| Branch badge | An adjusted branch reads as its base branch, for example **main · adjusted (unlocked)**. |
| Hook activity | Transfer progress for get, copy and move. |

## Commands

| Command | Where |
| --- | --- |
| Get content, Drop local content, Copy to *remote*, Move to *remote*, Unlock, Lock | Changed-file row menu, for annexed files |
| Drop even without other copies… | Changed-file row menu, behind a confirmation |
| Add to git-annex | Changed-file row menu, for untracked files |
| Get content, Where is it? | Diff card |
| Pull with git-annex, Get all annexed content | **Pull** menu |
| Push with git-annex, Sync with git-annex | **Push** menu |
| Sync, get everything, switch to or leave an adjusted branch, add or enable a special remote, set numcopies, describe this repository, check content | Right-click the **git-annex** sidebar header |
| Get from, copy to, move to or drop from one repository; mark as trusted, semitrusted or untrusted; describe; enable a special remote | Right-click a repository in the **git-annex** section |
| Initialize git-annex in this clone | **git-annex** section, until the clone is initialized |
| Refresh annexed files left stale | **git-annex** section when a cancelled or failed command left files reading as modified; also the header menu |
| Find unused content…, then Drop or Drop without verified copies | Right-click the **git-annex** sidebar header |
| Open git-annex webapp…, Stop git-annex assistant | Right-click the **git-annex** sidebar header; Stop appears while the assistant runs |
| Sync, pull, push, get all, check, adjust, leave adjusted, initialize, refresh, find unused content, open the webapp | Command palette, under **git-annex** |

git-annex refuses to drop content when it cannot verify enough copies elsewhere. GitComet shows that refusal with git-annex's reason and suggests copying first or using **Move to**. **Drop even without other copies…** passes `--force` only after you confirm.

GitComet never lets git-annex commit. Sync runs with `--no-commit`, so your changes stay in the working tree until you commit them. The one exception is the webapp, which you start on purpose: it runs the git-annex assistant, which adds and commits changes by itself until you stop it. GitComet asks before starting it.

**Find unused content…** lists old versions no branch or tag uses any more, corrupt copies `fsck` set aside, and partial downloads, with their sizes. **Drop** keeps anything git-annex cannot verify in another repository; **Drop without verified copies** deletes it anyway. Both refresh the list first, so they drop what is unused at that moment.

To add a special remote, enter the name, the type and its `key=value` parameters, for example:

```
backup directory directory=/mnt/usb encryption=none
```

A special remote another clone added is listed with its type but is not usable here until you enable it. Right-click it and choose **Enable *name* in this clone…**. Most types need only the name; a `directory` remote needs its path on this computer again:

```
backup directory=/media/usb/annex
```

## Settings

**Settings → Large files** has three options:

- **Hide git-annex bookkeeping branches** is on by default.
- **Use git-annex pull and push on adjusted branches** is on by default. On an adjusted branch, the Pull and Push buttons run `git annex pull` and `git annex push`. A plain merge into an adjusted branch would commit the adjusted files to the wrong branch.
- **git-annex pull, push and sync also move file content** is off by default, so these commands only exchange branches.

## Troubleshooting

- **A file shows "presence unknown".** The clone is not initialized or git-annex is not installed. Unlocked files keep their content under a path only git-annex can compute.
- **A drop was refused.** Copy the file to another repository first, raise trust in a repository that has it, or lower numcopies with **Set number of copies…**.
- **Files read as modified after a cancelled download.** git-annex updates Git's index at the end of a command, which a cancel skips. GitComet refreshes it after every git-annex command it runs; for files left by other tools, use **Refresh** in the git-annex section.
- **The git-annex section lists no repositories.** git-annex is not installed, or `git annex info` failed. Run `git annex info` in a terminal to see why.
