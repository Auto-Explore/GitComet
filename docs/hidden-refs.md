# Hidden Refs

**All branches** walks every ref under `refs/` except tags, the same set as
`git log --all`. Coding agents, review systems and Git's own bookkeeping write
refs there that are not branches. GitComet hides those namespaces from the log by
default.

## Settings

In **Settings → Git log → Hidden refs**, **Hide tool and bookkeeping refs** is on
by default. Turn it off to walk every ref again. With it on, each namespace has
its own switch:

| Namespace | Written by |
|---|---|
| `refs/t3` | T3 Code checkpoints, one per agent turn |
| `refs/claude` | Claude Code checkpoints |
| `refs/gitcomet-pro` | GitComet Pro review refs |
| `refs/jj/keep` | Jujutsu, keeping hidden commits alive |
| `refs/wip` | Magit work-in-progress snapshots |
| `refs/prefetch` | `git maintenance` background prefetch |
| `refs/original` | `git filter-branch` backups |
| `refs/notes` | Git notes history |
| `refs/pull` | GitHub pull request heads and test merges |
| `refs/changes` | Gerrit patch sets |

Hover a switch to see what writes those refs and why they are hidden. Every
namespace starts hidden. Add your own below the list: type a namespace such
as `refs/review` and press Enter or **Add**. Typing alone changes nothing. A
namespace must start with `refs/`. Wildcards are not supported; a namespace
covers everything under it at a `/` boundary, so `refs/t3` hides
`refs/t3/checkpoints/…` but not `refs/t3code`. Branches, remote-tracking branches
and tags have their own controls and cannot be added. To hide a remote, choose
**Hide from history graph** in its sidebar context menu.

The setting applies to every open workspace window. Each repository showing
**All branches** reloads its history once when the setting changes.

## What hiding does

A hidden ref does not start the walk. HEAD is always walked, and a commit that a
shown branch reaches stays visible. Other history modes walk only HEAD, so the
setting does not affect them.
