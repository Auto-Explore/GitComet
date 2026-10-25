use super::*;
use gitcomet_core::ref_comparison::{RefComparisonCounts, RefIntegration};
use gitcomet_core::worktree_scan::WorktreeCleanliness;

fn checkout_cleanliness(
    repo: &RepoState,
    path: &std::path::Path,
    head: Option<&CommitId>,
    is_current: bool,
) -> WorktreeCleanliness {
    if !is_current {
        return repo
            .worktree_observations
            .get(path)
            .filter(|observation| observation.head.as_ref() == head)
            .map(|observation| observation.cleanliness.clone())
            .unwrap_or_else(|| {
                WorktreeCleanliness::Unknown(
                    "Waiting for a filesystem scan of this checkout.".into(),
                )
            });
    }
    let mut unknown = None;
    for (lane, staged) in [(&repo.worktree_status, false), (&repo.staged_status, true)] {
        let entries = match lane {
            Loadable::Ready(entries) => Some(entries.as_slice()),
            Loadable::NotLoaded => match &repo.status {
                Loadable::Ready(status) => Some(if staged {
                    status.staged.as_slice()
                } else {
                    status.unstaged.as_slice()
                }),
                Loadable::Error(error) => {
                    unknown = Some(error.as_str());
                    None
                }
                Loadable::Loading | Loadable::NotLoaded => None,
            },
            Loadable::Error(error) => {
                unknown = Some(error.as_str());
                None
            }
            Loadable::Loading => None,
        };
        match entries {
            Some(entries) if !entries.is_empty() => return WorktreeCleanliness::Dirty,
            None => {
                unknown.get_or_insert("Waiting for a filesystem scan of this checkout.");
            }
            Some(_) => {}
        }
    }
    unknown.map_or(WorktreeCleanliness::Clean, |error| {
        WorktreeCleanliness::Unknown(error.into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoSpec};
    use gitcomet_core::worktree_scan::ObservedWorktreeStatus;

    #[test]
    fn cleanliness_uses_recorded_head_when_comparison_metadata_is_unavailable() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/repo".into(),
            },
        );
        let path = std::path::Path::new("/linked");
        let old = CommitId("old".into());
        let new = CommitId("new".into());
        Arc::make_mut(&mut repo.worktree_observations).insert(
            path.into(),
            ObservedWorktreeStatus {
                scan_id: 1,
                head: Some(old.clone()),
                cleanliness: WorktreeCleanliness::Clean,
            },
        );
        assert!(matches!(
            checkout_cleanliness(&repo, path, Some(&new), false),
            WorktreeCleanliness::Unknown(_)
        ));
        assert_eq!(
            checkout_cleanliness(&repo, path, Some(&old), false),
            WorktreeCleanliness::Clean
        );
        Arc::make_mut(&mut repo.worktree_observations)
            .get_mut(path)
            .unwrap()
            .head = Some(new.clone());
        assert_eq!(
            checkout_cleanliness(&repo, path, Some(&new), false),
            WorktreeCleanliness::Clean
        );
        assert!(matches!(
            checkout_cleanliness(&repo, std::path::Path::new("/missing"), Some(&new), false),
            WorktreeCleanliness::Unknown(_)
        ));
    }

    #[test]
    fn active_checkout_cleanliness_uses_both_live_status_lanes() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/repo".into(),
            },
        );
        let path = std::path::Path::new("/repo");
        repo.status = Loadable::Ready(Arc::new(gitcomet_core::domain::RepoStatus {
            staged: Arc::new(Vec::new()),
            unstaged: Arc::new(Vec::new()),
        }));
        assert_eq!(
            checkout_cleanliness(&repo, path, None, true),
            WorktreeCleanliness::Clean
        );
        repo.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
            path: "new.txt".into(),
            kind: FileStatusKind::Untracked,
            conflict: None,
        }]));
        assert_eq!(
            checkout_cleanliness(&repo, path, None, true),
            WorktreeCleanliness::Dirty
        );
        repo.worktree_status = Loadable::Ready(Arc::new(Vec::new()));
        repo.staged_status = Loadable::Error("unavailable".into());
        assert!(matches!(
            checkout_cleanliness(&repo, path, None, true),
            WorktreeCleanliness::Unknown(_)
        ));
        repo.staged_status = Loadable::Loading;
        assert!(matches!(
            checkout_cleanliness(&repo, path, None, true),
            WorktreeCleanliness::Unknown(_)
        ));
        repo.staged_status = Loadable::Ready(Arc::new(Vec::new()));
        assert_eq!(
            checkout_cleanliness(&repo, path, None, true),
            WorktreeCleanliness::Clean
        );
    }
}

impl SidebarPaneView {
    pub(super) fn worktree_cleanliness_badge(
        &self,
        ctx: &SidebarRowContext,
        path: &std::path::Path,
        head: Option<&CommitId>,
        is_current: bool,
    ) -> AnyElement {
        let current = self
            .active_repo()
            .map(|repo| checkout_cleanliness(repo, path, head, is_current));
        let (label, detail) = match current {
            Some(WorktreeCleanliness::Clean) => ("Clean", "The latest filesystem scan found no staged, unstaged, or untracked changes.".to_owned()),
            Some(WorktreeCleanliness::Dirty) => ("Dirty", "The latest filesystem scan found uncommitted changes. Committed-history evidence is separate.".to_owned()),
            Some(WorktreeCleanliness::Unknown(error)) => ("Unknown", format!("Filesystem status is unknown: {error}")),
            None => ("Unknown", "Waiting for a filesystem scan of this checkout.".to_owned()),
        };
        div()
            .id(SharedString::from(format!(
                "worktree_cleanliness_{}",
                path.display()
            )))
            .flex_none()
            .text_size(ctx.theme.ui_text(11.0))
            .text_color(ctx.theme.colors.foreground.secondary)
            .child(label)
            .gitcomet_tooltip(ctx.theme, detail.into())
            .into_any_element()
    }

    pub(super) fn comparison_heading(&self, label: &str) -> (SharedString, SharedString) {
        let Some(repo) = self.active_repo() else {
            return (label.to_owned().into(), label.to_owned().into());
        };
        match &repo.ref_comparisons.target {
            Loadable::Ready(target) => (
                format!("{label} · {}", target.reference).into(),
                format!("Comparing with locally available {} ({}). Fetch updates local remote refs. Right-click to choose a comparison ref or run a deeper check.", target.reference, target.oid).into(),
            ),
            Loadable::Error(error) => (format!("{label} · comparison unavailable").into(), error.clone().into()),
            Loadable::Loading => (label.to_owned().into(), "Resolving the locally available comparison target…".into()),
            Loadable::NotLoaded => (label.to_owned().into(), label.to_owned().into()),
        }
    }

    pub(super) fn comparison_badge(
        &self,
        ctx: &SidebarRowContext,
        subject: Option<&CommitId>,
    ) -> Option<AnyElement> {
        let repo = self.active_repo()?;
        let subject = subject?;
        let Loadable::Ready(target) = &repo.ref_comparisons.target else {
            return None;
        };
        let (label, detail) = match repo.ref_comparisons.decorations.get(subject) {
            None => match repo.ref_comparisons.deferred {
                Some(reason) => (
                    "Not confirmed".to_string(),
                    reason.description().to_string(),
                ),
                None => ("Comparing…".to_string(), "Comparison pending.".to_string()),
            },
            Some(result) => {
                let (counts, count_detail) = match result.counts {
                    RefComparisonCounts::Exact { ahead, behind } => (
                        format!("↑{ahead} ↓{behind}"),
                        format!(
                            "{ahead} ahead, {behind} behind by ancestry. These counts can remain nonzero after a squash or rebase patch match."
                        ),
                    ),
                    RefComparisonCounts::Pending => (String::new(), "Counts pending.".to_string()),
                    RefComparisonCounts::Unavailable(reason) => (
                        String::new(),
                        format!("Exact counts unavailable. {}", reason.description()),
                    ),
                };
                let (status, evidence) = match result.integration {
                    RefIntegration::Pending => match repo.ref_comparisons.deferred {
                        Some(reason) => ("Not confirmed", reason.description()),
                        None => ("Checking…", "Integration evidence pending."),
                    },
                    RefIntegration::Confirmed(evidence) => {
                        (evidence.label(), evidence.description())
                    }
                    RefIntegration::NotConfirmed(reason) => ("Not confirmed", reason.description()),
                };
                let label = if counts.is_empty() {
                    status.to_string()
                } else {
                    format!("{counts} · {status}")
                };
                (label, format!("{count_detail}\n{evidence}"))
            }
        };
        let tooltip = format!(
            "Compared with locally available {} ({}).\nSubject: {}\n{}\nUncommitted files are separate from committed-history evidence.",
            target.reference, target.oid, subject, detail
        );
        Some(
            div()
                .id(SharedString::from(format!("ref_comparison_{subject}")))
                .flex_none()
                .max_w_1_2()
                .min_w(px(0.0))
                .overflow_hidden()
                .text_size(ctx.theme.ui_text(11.0))
                .text_color(ctx.theme.colors.foreground.secondary)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label)
                .gitcomet_tooltip(ctx.theme, tooltip.into())
                .into_any_element(),
        )
    }
}
