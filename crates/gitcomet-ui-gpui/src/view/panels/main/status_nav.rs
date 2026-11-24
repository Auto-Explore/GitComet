use super::*;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct StatusFooterWork {
    pub(in crate::view) section_resolutions: u64,
    pub(in crate::view) position_lookups: u64,
    pub(in crate::view) projection_builds: u64,
    pub(in crate::view) plan_builds: u64,
    pub(in crate::view) order_materializations: u64,
}

#[cfg(test)]
thread_local! {
    static STATUS_FOOTER_WORK: std::cell::Cell<StatusFooterWork> = Default::default();
}

impl DetailsPaneView {
    #[cfg(test)]
    pub(in crate::view) fn record_status_footer_work(update: impl FnOnce(&mut StatusFooterWork)) {
        STATUS_FOOTER_WORK.with(|work| {
            let mut next = work.get();
            update(&mut next);
            work.set(next);
        });
    }

    #[cfg(test)]
    pub(in crate::view) fn take_status_footer_work() -> StatusFooterWork {
        STATUS_FOOTER_WORK.with(|work| work.replace(StatusFooterWork::default()))
    }

    /// Resolve the footer from the same source and plan as the status rows.
    /// Unchanged renders compare retained identities before locating the file's
    /// section or materializing its drawn order.
    pub(in crate::view) fn status_footer_position(
        &self,
        renderer_repo: &RepoState,
        path: &std::path::Path,
        area: DiffArea,
        view: ChangeTrackingView,
    ) -> Option<Option<(StatusSection, usize, usize)>> {
        use crate::view::status_position_cache::{OrderSnapshot, PositionQuery};

        let repo_id = renderer_repo.id;
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        // The panes observe snapshots independently. Use the retained lookup
        // only when its source is also the source the renderer is navigating.
        let same_source = match (
            repo.status_entries_for_area(area),
            renderer_repo.status_entries_for_area(area),
        ) {
            (Some(displayed), Some(rendered)) => std::ptr::eq(displayed, rendered),
            (None, None) => true,
            _ => false,
        };
        if !same_source {
            return None;
        }
        // Match RepoState::status_entries_for_area, including the legacy
        // Ready status used while a split lane has no ready value of its own.
        let lane = match area {
            DiffArea::Unstaged => &repo.worktree_status,
            DiffArea::Staged => &repo.staged_status,
        };
        let source = match lane {
            Loadable::Ready(source) => Some(source),
            _ => match &repo.status {
                Loadable::Ready(status) => Some(match area {
                    DiffArea::Unstaged => &status.unstaged,
                    DiffArea::Staged => &status.staged,
                }),
                _ => None,
            },
        };
        Some(self.status_footer_position.borrow_mut().lookup(
            PositionQuery {
                repo: repo_id.0,
                source,
                path,
                area,
                view,
            },
            |section| {
                Some(OrderSnapshot {
                    projection: self.status_section_order(repo, section)?,
                    plan: self.status_file_plan(repo, section),
                })
            },
            || status_navigation_section(repo, path, area, view),
            |section, _| {
                let order = self.active_status_section_order(repo_id, section);
                let target = DiffTarget::working_tree(path.to_owned(), area);
                status_navigation_position(repo, &target, view, order.as_deref())
                    .map(|(_, index, count)| (index, count))
            },
        ))
    }
}

#[derive(Debug)]
pub(super) struct StatusNavigationContext<'a> {
    section: StatusSection,
    entries: Vec<&'a gitcomet_core::domain::FileStatus>,
    current_ix: usize,
    /// False once the open file left the list; `current_ix` is then 0.
    open_file_listed: bool,
}

/// The index `delta` rows from `current`, clamped to the list. `None` when the
/// step does not move.
fn step_ix(current: usize, len: usize, delta: isize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let target = current.saturating_add_signed(delta).min(last);
    (delta != 0 && target != current).then_some(target)
}

impl<'a> StatusNavigationContext<'a> {
    pub(super) fn prev_ix(&self) -> Option<usize> {
        self.current_ix.checked_sub(1)
    }

    pub(super) fn next_ix(&self) -> Option<usize> {
        (self.current_ix + 1 < self.entries.len()).then_some(self.current_ix + 1)
    }

    fn adjacent_ix(&self, delta: isize) -> Option<usize> {
        step_ix(self.current_ix, self.entries.len(), delta)
    }

    pub(super) fn section(&self) -> StatusSection {
        self.section
    }

    pub(super) fn next_or_prev_path(&self) -> Option<std::path::PathBuf> {
        self.next_ix()
            .or_else(|| self.prev_ix())
            .and_then(|ix| self.entries.get(ix).map(|entry| entry.path.clone()))
    }
}

#[cfg(test)]
fn status_navigation_section_for_target(
    status: &gitcomet_core::domain::RepoStatus,
    change_tracking_view: ChangeTrackingView,
    path: &std::path::Path,
    area: DiffArea,
) -> Option<StatusSection> {
    match area {
        DiffArea::Staged => Some(StatusSection::Staged),
        DiffArea::Unstaged => match change_tracking_view {
            ChangeTrackingView::Combined => Some(StatusSection::CombinedUnstaged),
            ChangeTrackingView::SplitUntracked => status
                .unstaged
                .iter()
                .find(|entry| entry.path == path)
                .map(|entry| {
                    if entry.kind == gitcomet_core::domain::FileStatusKind::Untracked {
                        StatusSection::Untracked
                    } else {
                        StatusSection::Unstaged
                    }
                }),
        },
    }
}

#[cfg(test)]
fn status_navigation_entries_for_section(
    status: &gitcomet_core::domain::RepoStatus,
    section: StatusSection,
) -> Vec<&gitcomet_core::domain::FileStatus> {
    match section {
        StatusSection::CombinedUnstaged => status.unstaged.iter().collect(),
        StatusSection::Untracked => status
            .unstaged
            .iter()
            .filter(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Untracked)
            .collect(),
        StatusSection::Unstaged => status
            .unstaged
            .iter()
            .filter(|entry| entry.kind != gitcomet_core::domain::FileStatusKind::Untracked)
            .collect(),
        StatusSection::Staged => status.staged.iter().collect(),
    }
}

#[cfg(test)]
pub(super) fn status_navigation_context<'a>(
    status: &'a gitcomet_core::domain::RepoStatus,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section =
        status_navigation_section_for_target(status, change_tracking_view, path.as_path(), *area)?;
    let entries = status_navigation_entries_for_section(status, section);
    let current_ix = entries.iter().position(|entry| entry.path == *path)?;
    Some(StatusNavigationContext {
        section,
        entries,
        current_ix,
        open_file_listed: true,
    })
}

/// Which section a working-tree diff target belongs to. Shared so the order
/// looked up for navigation is the order navigation then walks.
pub(super) fn status_navigation_section(
    repo: &RepoState,
    path: &std::path::Path,
    area: DiffArea,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusSection> {
    #[cfg(test)]
    DetailsPaneView::record_status_footer_work(|work| work.section_resolutions += 1);
    Some(match area {
        DiffArea::Staged => StatusSection::Staged,
        DiffArea::Unstaged => match change_tracking_view {
            ChangeTrackingView::Combined => StatusSection::CombinedUnstaged,
            ChangeTrackingView::SplitUntracked => {
                let entry = repo.status_entry_for_path(DiffArea::Unstaged, path)?;
                if entry.kind == gitcomet_core::domain::FileStatusKind::Untracked {
                    StatusSection::Untracked
                } else {
                    StatusSection::Unstaged
                }
            }
        },
    })
}

/// The section F1/F4 step through: the open file's, or once its last change
/// was staged or discarded and it left the list, the list it was in. The
/// split view cannot tell which that was, so tracked changes come first.
fn status_step_section(
    repo: &RepoState,
    path: &std::path::Path,
    area: DiffArea,
    change_tracking_view: ChangeTrackingView,
) -> Option<StatusSection> {
    status_navigation_section(repo, path, area, change_tracking_view).or_else(|| {
        [StatusSection::Unstaged, StatusSection::Untracked]
            .into_iter()
            .find(|section| {
                StatusSectionEntries::from_repo(repo, *section)
                    .is_some_and(|entries| entries.len() > 0)
            })
    })
}

/// `section_order` is the section's display order in the backing slice's index
/// space. `None` falls back to source order, which is only correct where the
/// caller does not care about position (it is what picks a *neighbouring* file).
pub(super) fn status_navigation_context_for_repo<'a>(
    repo: &'a RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    section_order: Option<&[usize]>,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section = status_navigation_section(repo, path.as_path(), *area, change_tracking_view)?;
    let entries: Vec<_> = match section_order {
        Some(order) => StatusSectionEntries::from_repo_with_order(repo, section, order.into())?
            .iter()
            .collect(),
        None => StatusSectionEntries::from_repo(repo, section)?
            .iter()
            .collect(),
    };
    let current_ix = entries.iter().position(|entry| entry.path == *path)?;
    Some(StatusNavigationContext {
        section,
        entries,
        current_ix,
        open_file_listed: true,
    })
}

/// The list the open working-tree file was in, after its last change was
/// staged or discarded and it left the list. `None` while it is still listed
/// or the status is not loaded.
fn status_navigation_context_without_open_file<'a>(
    repo: &'a RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    section_order: Option<&[usize]>,
) -> Option<StatusNavigationContext<'a>> {
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    if repo.status_entry_for_path(*area, path).is_some() {
        return None;
    }
    let section = status_step_section(repo, path, *area, change_tracking_view)?;
    let entries: Vec<_> = match section_order {
        Some(order) => StatusSectionEntries::from_repo_with_order(repo, section, order.into())?
            .iter()
            .collect(),
        None => StatusSectionEntries::from_repo(repo, section)?
            .iter()
            .collect(),
    };
    (!entries.is_empty()).then_some(StatusNavigationContext {
        section,
        entries,
        current_ix: 0,
        open_file_listed: false,
    })
}

/// The open working-tree target's section, its place in the section's display
/// order and the section's length: the bar's "3 of 43 files" and its arrows.
/// The pane renders on every diff scroll step, so this walks the section once
/// without copying it, where the navigation context collects the whole section.
pub(super) fn status_navigation_position(
    repo: &RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    section_order: Option<&[usize]>,
) -> Option<(StatusSection, usize, usize)> {
    #[cfg(test)]
    DetailsPaneView::record_status_footer_work(|work| work.position_lookups += 1);
    let DiffTarget::WorkingTree { path, area, .. } = diff_target else {
        return None;
    };
    let section = status_navigation_section(repo, path.as_path(), *area, change_tracking_view)?;
    let mut len = 0usize;
    let mut current = None;
    let mut visit = |entry: &gitcomet_core::domain::FileStatus| {
        if current.is_none() && entry.path == *path {
            current = Some(len);
        }
        len += 1;
    };
    match section_order {
        Some(order) => {
            let entries = match section {
                StatusSection::Staged => repo.staged_status_entries()?,
                _ => repo.worktree_status_entries()?,
            };
            // Like the section iterator: an index past the end ends the list.
            order
                .iter()
                .map_while(|ix| entries.get(*ix))
                .for_each(&mut visit);
        }
        None => StatusSectionEntries::from_repo(repo, section)?
            .iter()
            .for_each(&mut visit),
    }
    let current = current?;
    Some((section, current, len))
}

/// Where the open file sits in the list it came from, for the bottom bar's
/// "3 of 43 files": its zero-based place, the list's length, and what the
/// list is, for the count's tooltip.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct DiffFilePlace {
    pub(in crate::view) index: usize,
    pub(in crate::view) count: usize,
    pub(in crate::view) context: SharedString,
}

fn status_section_label(section: StatusSection) -> &'static str {
    match section {
        StatusSection::Staged => "Staged changes",
        StatusSection::Unstaged => "Unstaged changes",
        StatusSection::Untracked => "Untracked files",
        StatusSection::CombinedUnstaged => "Changes",
    }
}

/// `path`'s place among `files` as drawn, and how many are drawn.
/// `drawn_source_indices` is the list's display order; `None` means source
/// order.
fn file_position_in_list(
    files: &[gitcomet_core::domain::CommitFileChange],
    drawn_source_indices: Option<&[usize]>,
    path: &std::path::Path,
) -> Option<(usize, usize)> {
    match drawn_source_indices {
        Some(drawn) => drawn
            .iter()
            .position(|source_ix| files.get(*source_ix).is_some_and(|file| file.path == path))
            .map(|ix| (ix, drawn.len())),
        None => files
            .iter()
            .position(|file| file.path == path)
            .map(|ix| (ix, files.len())),
    }
}

/// The selected inline diff entry's place in the order drawn, and how many
/// are drawn; like [`adjacent_inline_diff_ix`].
fn inline_diff_file_position(
    selected_ix: usize,
    entries_len: usize,
    drawn_order: Option<&[usize]>,
) -> Option<(usize, usize)> {
    match drawn_order {
        Some(order) => order
            .iter()
            .position(|entry| *entry == selected_ix)
            .map(|ix| (ix, order.len())),
        None => (selected_ix < entries_len).then_some((selected_ix, entries_len)),
    }
}

/// Where `diff_target` sits in the list it came from, from the same data the
/// file arrows step through: the status section, the commit's files, or the
/// comparison's files. `None` when it came from no list.
pub(super) fn diff_file_position_for_repo(
    repo: &RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    file_list_source_indices: Option<&[usize]>,
    status_section_order: Option<&[usize]>,
) -> Option<DiffFilePlace> {
    let (index, count, context): (usize, usize, SharedString) = match diff_target {
        DiffTarget::WorkingTree { .. } => {
            let (section, index, count) = status_navigation_position(
                repo,
                diff_target,
                change_tracking_view,
                status_section_order,
            )?;
            (index, count, status_section_label(section).into())
        }
        DiffTarget::Commit {
            commit_id, path, ..
        } => {
            let files = repo.history_state.listed_files(diff_target)?;
            let (index, count) = file_position_in_list(files, file_list_source_indices, path)?;
            let short: String = commit_id.0.chars().take(7).collect();
            (index, count, format!("Files in commit {short}").into())
        }
        DiffTarget::CommitRange {
            path: Some(path), ..
        } => {
            let files = repo.history_state.listed_files(diff_target)?;
            let (index, count) = file_position_in_list(files, file_list_source_indices, path)?;
            (index, count, "Files in comparison".into())
        }
        DiffTarget::CommitRange { path: None, .. } => return None,
    };
    Some(DiffFilePlace {
        index,
        count,
        context,
    })
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum AdjacentDiffFileTarget {
    Range {
        target: DiffTarget,
        target_ix: usize,
    },
    WorkingTree {
        section: StatusSection,
        area: DiffArea,
        target_ix: usize,
        path: std::path::PathBuf,
        is_conflicted: bool,
    },
    Commit {
        commit_id: CommitId,
        target_ix: usize,
        path: std::path::PathBuf,
        old_path: Option<std::path::PathBuf>,
    },
}

/// The file `direction` steps to from `path` in a commit-diff file list, and
/// its position among the files as drawn. `drawn_source_indices` is the list's
/// display order; `None` means source order.
fn adjacent_file_in_list<'a>(
    files: &'a [gitcomet_core::domain::CommitFileChange],
    drawn_source_indices: Option<&[usize]>,
    path: &std::path::Path,
    delta: isize,
) -> Option<(usize, &'a gitcomet_core::domain::CommitFileChange)> {
    let (current_ix, len) = file_position_in_list(files, drawn_source_indices, path)?;
    let target_ix = step_ix(current_ix, len, delta)?;
    let source_ix = match drawn_source_indices {
        Some(drawn) => *drawn.get(target_ix)?,
        None => target_ix,
    };
    Some((target_ix, files.get(source_ix)?))
}

/// The entry an inline foreign diff should open when stepping `direction` from
/// `selected_ix`. Entries are in source order, so a sorted or tree-grouped list
/// passes `drawn_order` -- display position to entry index -- and navigation
/// follows the rows the user sees. An entry the list is not currently showing
/// has no neighbour, like the commit-file path above.
fn adjacent_inline_diff_ix(
    selected_ix: usize,
    entries_len: usize,
    drawn_order: Option<&[usize]>,
    delta: isize,
) -> Option<usize> {
    match drawn_order {
        Some(order) => {
            let display_ix = order.iter().position(|entry| *entry == selected_ix)?;
            order.get(step_ix(display_ix, order.len(), delta)?).copied()
        }
        None => step_ix(selected_ix, entries_len, delta),
    }
}

pub(super) fn adjacent_diff_file_target_for_repo(
    repo: &RepoState,
    diff_target: &DiffTarget,
    change_tracking_view: ChangeTrackingView,
    delta: isize,
    file_list_source_indices: Option<&[usize]>,
    status_section_order: Option<&[usize]>,
) -> Option<AdjacentDiffFileTarget> {
    if delta == 0 {
        return None;
    }

    match diff_target {
        DiffTarget::WorkingTree { .. } => {
            let navigation = match status_navigation_context_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                status_section_order,
            ) {
                Some(navigation) => navigation,
                None => status_navigation_context_without_open_file(
                    repo,
                    diff_target,
                    change_tracking_view,
                    status_section_order,
                )?,
            };
            let target_ix = if navigation.open_file_listed {
                navigation.adjacent_ix(delta)?
            } else if delta == isize::MAX {
                navigation.entries.len().checked_sub(1)?
            } else {
                0
            };
            let entry = navigation.entries.get(target_ix)?;
            let path = entry.path.clone();
            let area = navigation.section.diff_area();
            let is_conflicted = area == DiffArea::Unstaged
                && entry.kind == gitcomet_core::domain::FileStatusKind::Conflicted;

            Some(AdjacentDiffFileTarget::WorkingTree {
                section: navigation.section,
                area,
                target_ix,
                path,
                is_conflicted,
            })
        }
        DiffTarget::Commit {
            commit_id, path, ..
        } => {
            let files = repo.history_state.listed_files(diff_target)?;
            let (target_ix, file) =
                adjacent_file_in_list(files, file_list_source_indices, path, delta)?;

            Some(AdjacentDiffFileTarget::Commit {
                commit_id: commit_id.clone(),
                target_ix,
                path: file.path.clone(),
                old_path: file.old_path.clone(),
            })
        }
        DiffTarget::CommitRange {
            from_commit_id,
            to_commit_id,
            path: Some(path),
            ..
        } => {
            // The file list on screen must be this comparison's.
            let files = repo.history_state.listed_files(diff_target)?;
            let (target_ix, file) =
                adjacent_file_in_list(files, file_list_source_indices, path, delta)?;
            Some(AdjacentDiffFileTarget::Range {
                target: DiffTarget::commit_range(
                    from_commit_id.clone(),
                    to_commit_id.clone(),
                    None,
                )
                .for_change(file),
                target_ix,
            })
        }
        DiffTarget::CommitRange { path: None, .. } => None,
    }
}

/// Selected entry, entry count and drawn order of the open inline diff.
type InlineDiffFileState = (usize, usize, Option<std::sync::Arc<[usize]>>);

impl DiffRenderer {
    /// The display order of the section the open working-tree diff belongs to.
    /// The pane owns the sort, so navigation has to ask it rather than re-derive
    /// an order of its own.
    pub(super) fn active_status_section_order(
        &self,
        repo_id: RepoId,
        change_tracking_view: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) -> Option<std::sync::Arc<[usize]>> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area, .. } =
            self.bound_diff_state(repo).diff_target.as_ref()?
        else {
            return None;
        };
        let section = status_step_section(repo, path.as_path(), *area, change_tracking_view)?;
        self.commands
            .navigation
            .as_ref()?
            .status_order(repo_id, section, cx)
    }

    /// The drawn order of the commit-diff file list the open diff belongs to:
    /// the commit details list, or the comparison list.
    pub(super) fn active_file_list_source_indices(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<std::sync::Arc<[usize]>> {
        let repo = self.active_repo()?;
        let is_range = matches!(
            self.bound_diff_state(repo).diff_target,
            Some(DiffTarget::CommitRange { .. })
        );
        self.commands
            .navigation
            .as_ref()?
            .file_order(repo_id, is_range, cx)
    }

    /// Where the open file sits in the list it came from, for the bottom bar.
    /// Hosted panes have none of the window's lists; their owner says.
    pub(in crate::view) fn diff_file_place(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<DiffFilePlace> {
        if self.hosted_decor.is_some() {
            return None;
        }
        if let Some(inline) = self.active_inline_submodule_diff() {
            let worktree = matches!(
                inline.origin,
                gitcomet_state::model::ForeignDiffOrigin::Worktree { .. }
            );
            let (selected_ix, entries_len) = (inline.selected_ix, inline.entries.len());
            let path = inline.submodule_repo_path.clone();
            let drawn_order = worktree
                .then(|| {
                    self.commands
                        .navigation
                        .as_ref()?
                        .worktree_order(repo_id, &path, cx)
                })
                .flatten();
            let (index, count) =
                inline_diff_file_position(selected_ix, entries_len, drawn_order.as_deref())?;
            let context = if worktree {
                "Files in worktree"
            } else {
                "Files in submodule"
            };
            return Some(DiffFilePlace {
                index,
                count,
                context: context.into(),
            });
        }
        let change_tracking_view = self.active_change_tracking_view(cx);
        if let Some(repo) = self.active_repo()
            && let Some(DiffTarget::WorkingTree { path, area, .. }) =
                self.bound_diff_state(repo).diff_target.as_ref()
            && let Some(position) = self.commands.navigation.as_ref().and_then(|navigation| {
                navigation.status_position(repo, path, *area, change_tracking_view, cx)
            })
        {
            return position.map(|(section, index, count)| DiffFilePlace {
                index,
                count,
                context: status_section_label(section).into(),
            });
        }
        let file_list_source_indices = self.active_file_list_source_indices(repo_id, cx);
        let status_section_order =
            self.active_status_section_order(repo_id, change_tracking_view, cx);
        let repo = self.active_repo()?;
        let diff_target = self.bound_diff_state(repo).diff_target.as_ref()?;
        diff_file_position_for_repo(
            repo,
            diff_target,
            change_tracking_view,
            file_list_source_indices.as_deref(),
            status_section_order.as_deref(),
        )
    }

    /// The previous footer path, retained for equivalence and allocation
    /// measurements through the actual pane/navigation implementation.
    #[cfg(test)]
    pub(in crate::view) fn uncached_diff_file_place_for_test(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<DiffFilePlace> {
        let file_list_source_indices = self.active_file_list_source_indices(repo_id, cx);
        let view = self.active_change_tracking_view(cx);
        let order = self.active_status_section_order(repo_id, view, cx);
        let repo = self.active_repo()?;
        let target = self.bound_diff_state(repo).diff_target.as_ref()?;
        diff_file_position_for_repo(
            repo,
            target,
            view,
            file_list_source_indices.as_deref(),
            order.as_deref(),
        )
    }

    /// The open inline diff's selected entry, entry count and drawn order.
    fn inline_diff_file_state(
        &self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> Option<InlineDiffFileState> {
        let inline = self.active_inline_submodule_diff()?;
        let selected_ix = inline.selected_ix;
        let entries_len = inline.entries.len();
        // Worktree rows can be sorted or grouped into a tree; submodule rows
        // follow source order.
        let worktree_path = matches!(
            inline.origin,
            gitcomet_state::model::ForeignDiffOrigin::Worktree { .. }
        )
        .then(|| inline.submodule_repo_path.clone());
        let drawn_order = worktree_path.and_then(|path| {
            self.commands
                .navigation
                .as_ref()?
                .worktree_order(repo_id, &path, cx)
        });
        Some((selected_ix, entries_len, drawn_order))
    }

    fn try_select_adjacent_diff_file_inner(
        &mut self,
        repo_id: RepoId,
        delta: isize,
        focus_diff_panel: bool,
        review_landing: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.hosted_decor.is_some() {
            if !self.store.policy.file_navigation {
                return false;
            }
            let action = self.hosted_decor.as_ref().and_then(|decor| {
                if delta < 0 {
                    decor.options.file_navigation.previous.clone()
                } else {
                    decor.options.file_navigation.next.clone()
                }
            });
            if let Some(action) = action {
                action.invoke(cx);
                return true;
            }
            return false;
        }
        if let Some((selected_ix, entries_len, drawn_order)) =
            self.inline_diff_file_state(repo_id, cx)
        {
            let Some(next_ix) =
                adjacent_inline_diff_ix(selected_ix, entries_len, drawn_order.as_deref(), delta)
            else {
                return false;
            };
            if focus_diff_panel {
                window.focus(&self.diff_panel_focus_handle, cx);
            }
            if review_landing
                && let Some(target) = self
                    .active_inline_submodule_diff()
                    .and_then(|inline| inline.entries.get(next_ix))
                    .map(|entry| entry.target.clone())
            {
                self.prepare_review_file_jump(repo_id, target, delta < 0);
            }
            self.store.dispatch(Msg::SelectInlineSubmoduleDiff {
                repo_id,
                selected_ix: next_ix,
            });
            return true;
        }

        let file_list_source_indices = self.active_file_list_source_indices(repo_id, cx);
        let change_tracking_view = self.active_change_tracking_view(cx);
        let status_section_order =
            self.active_status_section_order(repo_id, change_tracking_view, cx);
        let Some(target) = (|| {
            let repo = self.active_repo()?;
            let diff_target = self.bound_diff_state(repo).diff_target.as_ref()?;
            adjacent_diff_file_target_for_repo(
                repo,
                diff_target,
                change_tracking_view,
                delta,
                file_list_source_indices.as_deref(),
                status_section_order.as_deref(),
            )
        })() else {
            return false;
        };

        if focus_diff_panel {
            window.focus(&self.diff_panel_focus_handle, cx);
        }
        match target {
            AdjacentDiffFileTarget::Range { target, target_ix } => {
                if review_landing {
                    self.prepare_review_file_jump(repo_id, target.clone(), delta < 0);
                }
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
                self.scroll_range_file_to_ix(target_ix, cx);
            }
            AdjacentDiffFileTarget::WorkingTree {
                section,
                area,
                target_ix: _,
                path,
                is_conflicted,
            } => {
                if review_landing {
                    self.prepare_review_file_jump(
                        repo_id,
                        DiffTarget::working_tree(path.clone(), area),
                        delta < 0,
                    );
                }
                self.open_status_file(repo_id, section, area, path, is_conflicted, cx);
            }
            AdjacentDiffFileTarget::Commit {
                commit_id,
                target_ix,
                path,
                old_path,
            } => {
                let target = DiffTarget::commit(commit_id, path).with_old_path(old_path);
                if review_landing {
                    self.prepare_review_file_jump(repo_id, target.clone(), delta < 0);
                }
                self.store.dispatch(Msg::SelectDiff { repo_id, target });
                self.scroll_commit_details_file_to_ix(target_ix, cx);
            }
        }

        true
    }

    /// Opens a file of a status list, revealing its row.
    pub(super) fn open_status_file(
        &mut self,
        repo_id: RepoId,
        section: StatusSection,
        area: DiffArea,
        path: std::path::PathBuf,
        is_conflicted: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        self.clear_status_multi_selection(repo_id, cx);
        self.scroll_status_section_to_path(section, &path, cx);
        if is_conflicted {
            self.store
                .dispatch(Msg::SelectConflictDiff { repo_id, path });
        } else {
            self.store.dispatch(Msg::SelectDiff {
                repo_id,
                target: DiffTarget::working_tree(path, area),
            });
        }
    }

    pub(in crate::view) fn try_select_adjacent_diff_file(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.try_select_adjacent_diff_file_inner(
            repo_id,
            direction as isize,
            true,
            false,
            window,
            cx,
        )
    }

    pub(in crate::view) fn try_select_adjacent_diff_file_preserving_focus(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.try_select_adjacent_diff_file_inner(
            repo_id,
            direction as isize,
            false,
            false,
            window,
            cx,
        )
    }

    pub(in crate::view) fn try_select_review_file(
        &mut self,
        repo_id: RepoId,
        direction: i8,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.try_select_adjacent_diff_file_inner(
            repo_id,
            direction as isize,
            false,
            true,
            window,
            cx,
        )
    }
}

#[cfg(test)]
mod tests;
