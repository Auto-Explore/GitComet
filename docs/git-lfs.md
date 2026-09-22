# Git LFS

GitComet works with repositories that store large files in [Git LFS](https://git-lfs.com). GitComet does not move LFS content itself. Checkout, staging, commit and push go through Git, and Git runs the `git-lfs` filters and hooks. Downloads, uploads, locks and tracking run `git lfs` directly.

**Settings → Executables** shows whether Git can run `git lfs`. When it is **Not found**, GitComet still shows which files are LFS files, but every LFS command is listed disabled with "(install git-lfs)".

## What GitComet shows

| Where | What |
| --- | --- |
| Changed-file rows | An **LFS** chip. It turns to the warning colour when the content is not downloaded here, and reads **LFS locked** when someone holds a lock on the file. |
| Commit details | The same chip on files whose committed version is an LFS pointer. |
| Diff pane | A card with the size and object id of each side. When the content is here, the real diff sits below the card, including images. When it is not, the card replaces the diff and offers **Download content**. |
| Hook activity | LFS transfer progress (files and bytes) for pushes, pulls, checkouts and LFS commands. |

Content above 16 MB stays behind its pointer in the text diff. Images use the normal 64 MB image limit.

LFS text diffs show changes in the actual content in both Full and Collapsed views. Stage or unstage these files as a whole; line and hunk actions are unavailable because Git stores a pointer to the complete file.

## Commands

| Command | Where |
| --- | --- |
| Download LFS content | Changed-file row menu (for files that are not downloaded), and the diff card |
| Lock file, Unlock, Force unlock | Changed-file row menu, for files matching a `lockable` pattern |
| Track *.ext in Git LFS, Track this file in Git LFS | Changed-file row menu, for files LFS does not manage yet. Tracked files are re-added so they become pointers in the staged changes. |
| Download all LFS content, Fetch LFS objects for all refs | **Pull** menu |
| Push all LFS objects, Refresh LFS locks, Enable Git LFS in this repository | **Push** menu |
| All of the above, plus Prune and Check | Command palette, under **Git LFS** |

GitComet never polls the LFS server for locks. It loads them when the repository has `lockable` patterns, after lock and unlock, and when you choose **Refresh LFS locks**. Many LFS servers have no lock API; the lock entries then stay unavailable without an error banner.

**Download content** in the diff card fetches both displayed revisions without changing your working files. The changed-file row action downloads and checks out that file. These explicit downloads override configured LFS fetch exclusions for the selected path without changing your configuration.

**Track this file in Git LFS** treats the filename literally, including spaces and brackets. When an already tracked file is converted, GitComet stages `.gitattributes` along with its new pointer.

## Long transfers

Git commands time out after five minutes without output (`GITCOMET_GIT_COMMAND_TIMEOUT_SECS` changes this). git-lfs prints no progress when it is not attached to a terminal, so GitComet reads its progress file instead. A transfer that keeps reporting progress is never cut off, however long it takes.

## Troubleshooting

When an LFS step fails, GitComet recognises the common causes and adds a hint to the error:

- **Git LFS is not installed where Git can find it.** Install git-lfs and run `git lfs install`. On macOS, a GUI app can have a shorter PATH than your shell; see [Commit Signature Verification](commit-signatures.md) for the same problem with GPG.
- **An LFS object could not be downloaded.** The remote does not have it, or you cannot authenticate to its LFS server. Run `git lfs fetch --all` in a terminal to see the server's answer.
- **Another user holds an LFS lock.** Ask them to unlock the file, or use **Force unlock** if the server allows it.
- **LFS objects could not be uploaded.** Run `git lfs push --all <remote>`, or check the LFS server's credentials.

To see what Git LFS knows about a repository, run:

```bash
git lfs env
git lfs ls-files --size
```
