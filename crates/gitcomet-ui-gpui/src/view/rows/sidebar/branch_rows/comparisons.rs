use super::*;
use gitcomet_core::ref_comparison::{IntegrationEvidence, RefComparisonCounts, RefIntegration};
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

impl SidebarPaneView {
    /// Uncommitted-change state of a checkout: a dot and "Dirty", a muted
    /// "?" while unknown, nothing when clean. The detail is in the tooltip.
    pub(super) fn worktree_cleanliness_badge(
        &self,
        ctx: &SidebarRowContext,
        path: &std::path::Path,
        head: Option<&CommitId>,
        is_current: bool,
    ) -> Option<AnyElement> {
        let current = self
            .active_repo()
            .map(|repo| checkout_cleanliness(repo, path, head, is_current));
        let theme = ctx.theme;
        let (label, detail) = match current {
            Some(WorktreeCleanliness::Clean) => return None,
            Some(WorktreeCleanliness::Dirty) => ("Dirty", "The latest filesystem scan found uncommitted changes. Committed-history evidence is separate.".to_owned()),
            Some(WorktreeCleanliness::Unknown(error)) => ("?", format!("Filesystem status is unknown: {error}")),
            None => ("?", "Waiting for a filesystem scan of this checkout.".to_owned()),
        };
        let dirty = label == "Dirty";
        let warning = theme.colors.status.warning.foreground;
        Some(
            div()
                .id(SharedString::from(format!(
                    "worktree_cleanliness_{}",
                    path.display()
                )))
                .flex_none()
                .flex()
                .items_center()
                .gap(ctx.px(3.0))
                .text_size(theme.ui_text(11.0))
                .text_color(if dirty {
                    warning
                } else {
                    theme.colors.foreground.secondary
                })
                .when(dirty, |badge| {
                    badge.child(
                        div()
                            .size(ctx.px(6.0))
                            .rounded(px(theme.radii.pill))
                            .bg(warning),
                    )
                })
                .child(label)
                .gitcomet_tooltip(theme, detail.into())
                .into_any_element(),
        )
    }

    /// A section title, the caption that names the comparison target on the
    /// header's right edge, and the header tooltip.
    pub(super) fn comparison_heading(
        &self,
        label: &str,
    ) -> (SharedString, Option<SharedString>, SharedString) {
        let title: SharedString = label.to_owned().into();
        let Some(repo) = self.active_repo() else {
            return (title.clone(), None, title);
        };
        match &repo.ref_comparisons.target {
            Loadable::Ready(target) => (
                title,
                Some(format!("vs {}", target.reference).into()),
                format!("Comparing with locally available {} ({}). Fetch updates local remote refs. Right-click to choose a comparison ref or run a deeper check.", target.reference, target.oid).into(),
            ),
            Loadable::Error(error) => (
                title,
                Some("comparison unavailable".into()),
                error.clone().into(),
            ),
            Loadable::Loading => (
                title,
                None,
                "Resolving the locally available comparison target…".into(),
            ),
            Loadable::NotLoaded => (title.clone(), None, title),
        }
    }

    /// The muted caption a section header shows on its right edge.
    pub(super) fn header_caption(ctx: &SidebarRowContext, caption: SharedString) -> gpui::Div {
        div()
            .flex_none()
            .text_size(ctx.theme.ui_text(11.0))
            .text_color(ctx.theme.colors.foreground.secondary)
            .whitespace_nowrap()
            .child(caption)
    }

    /// The left side of a tall row's details line: whether the subject is
    /// in the comparison target, and if not, by how many commits. Keeps its
    /// width unless the caller lets it shrink; its text then ends in "…".
    pub(super) fn comparison_status(
        &self,
        ctx: &SidebarRowContext,
        subject: Option<&CommitId>,
    ) -> Option<gpui::Stateful<gpui::Div>> {
        let repo = self.active_repo()?;
        let subject = subject?;
        let (status, tooltip) = comparison_summary(&repo.ref_comparisons, subject)?;
        let status = status?;
        let theme = ctx.theme;
        let cell = div()
            .id(SharedString::from(format!("ref_comparison_{subject}")))
            .debug_selector({
                let subject = subject.clone();
                move || format!("ref_comparison_{subject}")
            })
            .flex()
            .flex_none()
            .min_w(px(0.0))
            .overflow_hidden()
            .items_center()
            .gap(ctx.px(3.0))
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(theme.ui_text(11.0))
            .text_color(theme.colors.foreground.secondary);
        let cell = match status {
            RowStatus::UpToDate => cell
                .child(ctx.svg_icon("icons/check.svg", ctx.icon_muted, 11.0))
                .child(shrinkable_text("Up to date")),
            RowStatus::Integrated(evidence) => {
                let color = theme.colors.status.success.foreground;
                cell.text_color(color)
                    .child(ctx.svg_icon("icons/git_merge.svg", color, 11.0))
                    .child(shrinkable_text(integrated_label(evidence)))
            }
            RowStatus::Unmerged { ahead, behind } => cell
                .child(
                    shrinkable_text(format!("↑{ahead}"))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.colors.status.warning.foreground),
                )
                .when(behind > 0, |cell| {
                    cell.child(shrinkable_text(format!("↓{behind}")))
                }),
            RowStatus::NotChecked => cell.child(shrinkable_text("Not checked")),
        };
        Some(cell.gitcomet_tooltip(theme, tooltip.into()))
    }
}

/// Text that a shrinking status cell can truncate; a bare string keeps its
/// full width.
fn shrinkable_text(text: impl IntoElement) -> gpui::Div {
    div().min_w(px(0.0)).overflow_hidden().child(text)
}

/// What a row says about its subject against the comparison target.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RowStatus {
    UpToDate,
    /// The target already holds the subject's changes.
    Integrated(IntegrationEvidence),
    /// Commits the target lacks, and target commits the subject lacks.
    Unmerged {
        ahead: u64,
        behind: u64,
    },
    /// Analysis never ran or reached a limit.
    NotChecked,
}

fn integrated_label(evidence: IntegrationEvidence) -> &'static str {
    match evidence {
        IntegrationEvidence::UpToDate => "Up to date",
        IntegrationEvidence::Merged => "Merged",
        IntegrationEvidence::ChangesMatch => "Same content",
        IntegrationEvidence::SquashPatch => "Squash-merged",
        IntegrationEvidence::IndividualCommits => "Rebase-merged",
        IntegrationEvidence::NoNetChanges => "No changes",
    }
}

/// The row's status, `None` while pending, and the full explanation for its
/// tooltip. `None` overall until a target is known.
fn comparison_summary(
    comparisons: &gitcomet_state::ref_comparisons::RefComparisonsState,
    subject: &CommitId,
) -> Option<(Option<RowStatus>, String)> {
    let Loadable::Ready(target) = &comparisons.target else {
        return None;
    };
    let (status, detail) = match comparisons.decorations.get(subject) {
        // A deferred row was never analysed, which is not the same as
        // checks that ran and failed to confirm incorporation.
        None => match comparisons.deferred {
            Some(reason) => (
                Some(RowStatus::NotChecked),
                reason.description().to_string(),
            ),
            None => (None, "Comparison pending.".to_string()),
        },
        Some(result) => {
            let count_detail = match result.counts {
                RefComparisonCounts::Exact { ahead, behind } => format!(
                    "{ahead} ahead, {behind} behind by ancestry. These counts can remain nonzero after a squash or rebase patch match."
                ),
                RefComparisonCounts::Pending => "Counts pending.".to_string(),
                RefComparisonCounts::Unavailable(reason) => {
                    format!("Exact counts unavailable. {}", reason.description())
                }
            };
            let evidence = match result.integration {
                RefIntegration::Pending => match comparisons.deferred {
                    Some(reason) => reason.description(),
                    None => "Integration evidence pending.",
                },
                RefIntegration::Confirmed(evidence) => evidence.description(),
                RefIntegration::NotConfirmed(reason) => reason.description(),
            };
            let status = match (result.integration, result.counts) {
                // Each tip reaches the other: they are the same commit.
                (RefIntegration::Confirmed(IntegrationEvidence::UpToDate), _)
                | (
                    _,
                    RefComparisonCounts::Exact {
                        ahead: 0,
                        behind: 0,
                    },
                ) => Some(RowStatus::UpToDate),
                (RefIntegration::Confirmed(evidence), _) => Some(RowStatus::Integrated(evidence)),
                // No commit of the subject is missing from the target.
                (_, RefComparisonCounts::Exact { ahead: 0, .. }) => {
                    Some(RowStatus::Integrated(IntegrationEvidence::Merged))
                }
                (_, RefComparisonCounts::Exact { ahead, behind }) => {
                    Some(RowStatus::Unmerged { ahead, behind })
                }
                (RefIntegration::NotConfirmed(_), _) => Some(RowStatus::NotChecked),
                (RefIntegration::Pending, _) if comparisons.deferred.is_some() => {
                    Some(RowStatus::NotChecked)
                }
                (RefIntegration::Pending, _) => None,
            };
            (status, format!("{count_detail}\n{evidence}"))
        }
    };
    let tooltip = format!(
        "Compared with locally available {} ({}).\nSubject: {}\n{}\nUncommitted files are separate from committed-history evidence.",
        target.reference, target.oid, subject, detail
    );
    Some((status, tooltip))
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

    /// A deferred row was never analysed. It used to read "Not confirmed",
    /// the label for checks that ran and failed to establish incorporation.
    #[test]
    fn deferred_rows_without_results_are_not_checked() {
        let mut comparisons = gitcomet_state::ref_comparisons::RefComparisonsState::default();
        comparisons.target = Loadable::Ready(gitcomet_core::ref_comparison::RefComparisonTarget {
            reference: "origin/main".into(),
            oid: CommitId("target".into()),
            context: 1,
        });
        comparisons.deferred = Some(gitcomet_core::ref_comparison::RefComparisonReason::Budget);
        let (status, tooltip) = comparison_summary(&comparisons, &CommitId("tip".into())).unwrap();
        assert_eq!(status, Some(RowStatus::NotChecked));
        assert!(tooltip.contains("resource or history limit"), "{tooltip}");
    }

    #[test]
    fn status_reads_merged_from_ancestry_and_hides_unconfirmed_labels() {
        use gitcomet_core::ref_comparison::{RefComparison, RefComparisonReason};
        let mut comparisons = gitcomet_state::ref_comparisons::RefComparisonsState::default();
        comparisons.target = Loadable::Ready(gitcomet_core::ref_comparison::RefComparisonTarget {
            reference: "origin/main".into(),
            oid: CommitId("target".into()),
            context: 1,
        });
        let not_confirmed = RefIntegration::NotConfirmed(RefComparisonReason::NoMatch);
        let cases = [
            (
                "same",
                RefComparisonCounts::Exact {
                    ahead: 0,
                    behind: 0,
                },
                RefIntegration::Pending,
                Some(RowStatus::UpToDate),
            ),
            (
                "merged",
                RefComparisonCounts::Exact {
                    ahead: 0,
                    behind: 9,
                },
                RefIntegration::Pending,
                Some(RowStatus::Integrated(IntegrationEvidence::Merged)),
            ),
            (
                "squashed",
                RefComparisonCounts::Exact {
                    ahead: 4,
                    behind: 9,
                },
                RefIntegration::Confirmed(IntegrationEvidence::SquashPatch),
                Some(RowStatus::Integrated(IntegrationEvidence::SquashPatch)),
            ),
            (
                "open",
                RefComparisonCounts::Exact {
                    ahead: 3,
                    behind: 120,
                },
                not_confirmed,
                Some(RowStatus::Unmerged {
                    ahead: 3,
                    behind: 120,
                }),
            ),
            (
                "limited",
                RefComparisonCounts::Unavailable(RefComparisonReason::Budget),
                not_confirmed,
                Some(RowStatus::NotChecked),
            ),
            (
                "pending",
                RefComparisonCounts::Pending,
                RefIntegration::Pending,
                None,
            ),
        ];
        comparisons
            .decorations
            .publish(
                &Arc::new(gitcomet_core::ref_comparison_memory::ComparisonMemory::default()),
                cases
                    .iter()
                    .map(|(subject, counts, integration, _)| RefComparison {
                        subject: CommitId((*subject).into()),
                        counts: *counts,
                        integration: *integration,
                    })
                    .collect(),
            )
            .unwrap();
        for (subject, _, _, expected) in cases {
            let (status, _) = comparison_summary(&comparisons, &CommitId(subject.into())).unwrap();
            assert_eq!(status, expected, "{subject}");
        }
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
