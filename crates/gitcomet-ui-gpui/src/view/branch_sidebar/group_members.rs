use super::BranchSection;
use crate::view::sidebar_search::SidebarSearch;
use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet};
use gitcomet_state::model::{Loadable, RepoState};

/// Visits the branches a `/`-prefix group row is actually showing.
///
/// Prefix matching is against `"{path}/"` rather than the bare `path`, so a
/// sibling group whose name merely starts with the same characters
/// (`features/` against `feat/`) is never counted as a member. A branch named
/// exactly `path` is a member too: with both `feat` and `feat/a` present the
/// tree renders the `feat/` group header and then `feat` itself as a row
/// *inside* it, so a prefix-only test would skip a branch the user can see in
/// the folder.
///
/// The sidebar's branch filter is applied here too. The tree filters branch
/// names *before* building the group tree, so a filtered `feat/` row lists only
/// its matching members — and a group action counting all 47 while one is on
/// screen would act on 46 branches the user cannot see.
///
/// Names are passed without the remote prefix, matching how the tree stores
/// them and what `git push --delete` expects. Borrowed rather than cloned so
/// the per-repaint callers can count without allocating.
pub(in crate::view) fn for_each_group_member(
    repo: &RepoState,
    search: &SidebarSearch,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
    mut visit: impl FnMut(&str),
) {
    let needle = format!("{path}/");
    match section {
        BranchSection::Local => {
            let Loadable::Ready(branches) = &repo.branches else {
                return;
            };
            for branch in branches.iter() {
                if is_group_member(&branch.name, path, &needle) && search.matches_ref(&branch.name)
                {
                    visit(&branch.name);
                }
            }
        }
        BranchSection::Remote => {
            let Some(remote) = remote else {
                return;
            };
            let Loadable::Ready(branches) = &repo.remote_branches else {
                return;
            };
            for branch in branches.iter() {
                if branch.remote == remote
                    && is_group_member(&branch.name, path, &needle)
                    // `matches_remote` spells the full ref out, which allocates;
                    // with no filter there is nothing to match.
                    && (search.matcher.is_empty() || search.matches_remote(remote, &branch.name))
                {
                    visit(&branch.name);
                }
            }
        }
    }
}

/// Whether `name` is one of the rows the `path` group renders: anything beneath
/// it, plus the branch named exactly `path`, which the tree draws as the group's
/// own first child.
fn is_group_member(name: &str, path: &str, needle: &str) -> bool {
    name == path || name.starts_with(needle)
}

/// How many of a group's members there are, and how many of them the history
/// focus covers.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct GroupFocusState {
    pub members: usize,
    pub focused: usize,
}

impl GroupFocusState {
    pub fn all_focused(self) -> bool {
        self.members > 0 && self.focused == self.members
    }
}

/// Counted in one walk and without building a target per member: the group
/// menu and every visible group row ask this on each repaint.
pub(in crate::view) fn group_focus_state(
    repo: &RepoState,
    search: &SidebarSearch,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> GroupFocusState {
    let focus = &repo.history_state.history_focus;
    let mut state = GroupFocusState::default();
    for_each_group_member(repo, search, section, remote, path, |name| {
        state.members += 1;
        if member_in_history_focus(focus, section, remote, name) {
            state.focused += 1;
        }
    });
    state
}

fn member_in_history_focus(
    focus: &HistoryFocusSet,
    section: BranchSection,
    remote: Option<&str>,
    name: &str,
) -> bool {
    match (section, remote) {
        (BranchSection::Local, _) => focus.covers_local_branch(name),
        (BranchSection::Remote, Some(remote)) => focus.covers_remote_branch(remote, name),
        (BranchSection::Remote, None) => false,
    }
}

/// The group's members as focus targets, resolved once when the user acts on
/// the group.
pub(in crate::view) fn group_focus_targets(
    repo: &RepoState,
    search: &SidebarSearch,
    section: BranchSection,
    remote: Option<&str>,
    path: &str,
) -> Vec<HistoryFocus> {
    let mut targets = Vec::new();
    for_each_group_member(repo, search, section, remote, path, |name| {
        targets.push(match (section, remote) {
            (BranchSection::Remote, Some(remote)) => HistoryFocus::remote_branch(remote, name),
            _ => HistoryFocus::local_branch(name),
        });
    });
    targets
}
