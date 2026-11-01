use gitcomet_core::domain::{HistoryFocus, HistoryFocusSet, RemoteBranch};
use gitcomet_state::model::RepoState;

/// How much of a repository's branch list the history focus keeps in view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryFocusCount {
    /// Branch refs the focus covers.
    pub focused: usize,
    /// Every branch ref that could be focused: local and remote alike.
    pub total: usize,
}

pub(crate) fn history_focus_count(repo: &RepoState) -> HistoryFocusCount {
    let local_branches = repo.branches.ready().map_or(0, |branches| branches.len());
    let remote_branches = repo
        .remote_branches
        .ready()
        .map_or(&[][..], |branches| branches.as_slice());
    count(
        &repo.history_state.history_focus,
        local_branches,
        remote_branches,
    )
}

/// Counted per entry rather than per listed branch, so a focus restored before
/// the branch lists load still counts its own refs. Only a whole remote needs
/// the list, to say how many branches it stands for; a remote branch that its
/// focused remote already covers is not counted twice.
fn count(
    focus: &HistoryFocusSet,
    local_branches: usize,
    remote_branches: &[RemoteBranch],
) -> HistoryFocusCount {
    let focused = focus
        .iter()
        .map(|target| match target {
            HistoryFocus::LocalBranch { .. } => 1,
            HistoryFocus::RemoteBranch { remote, .. } => {
                usize::from(!focus.contains(&HistoryFocus::Remote {
                    name: remote.clone(),
                }))
            }
            HistoryFocus::Remote { name } => remote_branches
                .iter()
                .filter(|branch| branch.remote == name.as_ref())
                .count(),
        })
        .sum();
    HistoryFocusCount {
        focused,
        total: local_branches + remote_branches.len(),
    }
}

/// `1 ref`, `8 refs`.
pub(crate) fn refs_label(count: usize) -> String {
    format!("{count} {}", if count == 1 { "ref" } else { "refs" })
}

/// What the REF header chip says after "Focus: " — the ref's own name when one
/// is focused, otherwise how many are, because a list of names does not fit the
/// chip. `None` while nothing is focused.
pub(crate) fn history_focus_label(repo: &RepoState) -> Option<String> {
    let focus = &repo.history_state.history_focus;
    if focus.is_empty() {
        return None;
    }
    Some(match focus.single() {
        Some(only) => only.label(),
        None => refs_label(history_focus_count(repo).focused),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::CommitId;

    fn remote_branch(remote: &str, name: &str) -> RemoteBranch {
        RemoteBranch {
            remote: remote.to_string(),
            name: name.to_string(),
            target: CommitId("a".repeat(40).into()),
        }
    }

    /// Remote branches can be focused, so they count towards the total next to
    /// the local ones.
    #[test]
    fn the_total_counts_local_and_remote_branches() {
        let focus = HistoryFocusSet::from_iter([
            HistoryFocus::local_branch("main"),
            HistoryFocus::remote_branch("origin", "main"),
        ]);
        let remote_branches = [
            remote_branch("origin", "main"),
            remote_branch("origin", "feature"),
        ];
        assert_eq!(
            count(&focus, 2, &remote_branches),
            HistoryFocusCount {
                focused: 2,
                total: 4
            }
        );
    }

    /// A whole remote counts as the branches it seeds from, and a branch it
    /// already covers is not counted a second time.
    #[test]
    fn a_focused_remote_counts_as_its_branches_once() {
        let focus = HistoryFocusSet::from_iter([
            HistoryFocus::remote("origin"),
            HistoryFocus::remote_branch("origin", "main"),
            HistoryFocus::remote_branch("upstream", "main"),
        ]);
        let remote_branches = [
            remote_branch("origin", "main"),
            remote_branch("origin", "feature"),
            remote_branch("origin", "fix"),
            remote_branch("upstream", "main"),
        ];
        assert_eq!(
            count(&focus, 1, &remote_branches),
            HistoryFocusCount {
                focused: 4,
                total: 5
            }
        );
    }

    #[test]
    fn refs_label_agrees_with_its_count() {
        assert_eq!(refs_label(1), "1 ref");
        assert_eq!(refs_label(8), "8 refs");
    }
}
