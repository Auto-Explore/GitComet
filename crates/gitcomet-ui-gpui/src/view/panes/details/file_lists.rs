use super::{DetailsPaneView, WorktreeFileListInputs};
use crate::kit::text_truncation::path_alignment_visible_signature;
use crate::view::{
    StatusSection, StatusSectionEntries, components, status_section_content_rev,
    status_section_order_rev,
};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
use gitcomet_state::msg::Msg;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

impl DetailsPaneView {
    pub(in crate::view) fn file_controller(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> crate::view::changed_file_list::SharedFileListController {
        std::rc::Rc::clone(
            &self
                .file_controllers
                .borrow_mut()
                .entry((repo_id, list))
                .or_default()
                .controller,
        )
    }

    fn file_source_key(
        &self,
        repo_id: RepoId,
        revision: u64,
        path: Option<&std::path::Path>,
    ) -> u64 {
        let lifetime = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(RepoState::lifetime);
        let mut hasher = FxHasher::default();
        (repo_id, lifetime, revision, path).hash(&mut hasher);
        hasher.finish()
    }

    pub(in crate::view) fn cached_commit_file_rows(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let key = self.file_source_key(repo_id, commit_details_rev, None);
        self.file_controller(repo_id, crate::view::rows::FileListId::CommitFiles)
            .borrow_mut()
            .presentations
            .rows_for(&key, files)
    }

    pub(in crate::view) fn cached_commit_file_projection(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let list = crate::view::rows::FileListId::CommitFiles;
        let sort = self
            .shared_file_sort_for(repo_id, list)
            .unwrap_or(self.commit_file_sort);
        let filter = self.commit_file_filter;
        let source = self.file_source_key(repo_id, commit_details_rev, None);
        let key = crate::view::rows::file_list_projection_key(repo_id.0, source, sort, filter);
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        controller.sort = sort;
        controller.kind_filter = filter;
        let (query, options) = (controller.query.clone(), controller.search_options);
        controller
            .projection_cache
            .projection_for_query(&key, files, sort, filter, &query, options)
    }

    /// Drop a list's transient layout flip and collapse set, so the next time it
    /// is shown it starts from the global default again.
    pub(super) fn forget_file_list_view_state(&mut self, list: crate::view::rows::FileListId) {
        self.file_list_layout_override
            .retain(|(_, entry), _| *entry != list);
        self.file_controllers
            .borrow_mut()
            .retain(|(_, entry), _| *entry != list);
        // The filter too: a stale "Renamed" empties the list under a header
        // still counting changes.
        self.list_filter.retain(|(_, entry), _| *entry != list);
        self.file_list_selection
            .retain(|(_, entry), _| *entry != list);
    }

    pub(in crate::view) fn file_list_layout_for(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> crate::view::FileListLayout {
        self.file_list_layout_override
            .get(&(repo_id, list))
            .copied()
            .unwrap_or(self.file_list_layout)
    }

    /// A click on the list's layout icon: the next layout, for this list
    /// until its selection changes.
    pub(in crate::view) fn toggle_file_list_layout(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = self.file_list_layout_for(repo_id, list).next();
        self.set_list_file_layout(repo_id, list, next, cx);
    }

    /// `layout` for this list until its selection changes, as the icon's
    /// menu chooses it.
    pub(in crate::view) fn set_list_file_layout(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        layout: crate::view::FileListLayout,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.file_list_layout_for(repo_id, list) == layout {
            return;
        }
        self.file_list_layout_override
            .insert((repo_id, list), layout);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in crate::view) fn file_list_collapsed_for(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> std::borrow::Cow<'_, crate::view::rows::CollapsedDirs> {
        std::borrow::Cow::Owned(
            self.file_controller(repo_id, list)
                .borrow()
                .collapsed
                .clone(),
        )
    }

    pub(in crate::view) fn toggle_file_list_dir(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        key: Arc<std::path::Path>,
        chain: Arc<[Arc<std::path::Path>]>,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        let entry = &mut controller.collapsed;
        if collapsed {
            entry.expand(&chain);
        } else {
            entry.collapse(key, &chain);
        }
        cx.notify();
    }

    pub(in crate::view) fn cached_commit_file_plan(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let projection = self.cached_commit_file_projection(repo_id, commit_details_rev, files);
        self.file_list_plan_from_projection(
            crate::view::rows::FileListId::CommitFiles,
            repo_id,
            commit_details_rev,
            None,
            &projection,
            files,
        )
    }

    pub(in crate::view) fn cached_worktree_file_projection(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let list = crate::view::rows::FileListId::WorktreeFiles;
        let sort = self.file_list_sort_for(list);
        let filter = self.file_list_filter_for(list);
        let source = self.file_source_key(repo_id, worktree_dirty_rev, Some(worktree_path));
        let key = crate::view::rows::file_list_projection_key(repo_id.0, source, sort, filter);
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        controller.sort = sort;
        controller.kind_filter = filter;
        let (query, options) = (controller.query.clone(), controller.search_options);
        controller
            .projection_cache
            .projection_for_query(&key, files, sort, filter, &query, options)
    }

    pub(in crate::view) fn cached_range_file_projection(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::CommitFileProjection> {
        let list = crate::view::rows::FileListId::RangeFiles;
        let sort = self.file_list_sort_for(list);
        let filter = self.file_list_filter_for(list);
        let source = self.file_source_key(repo_id, range_files_rev, None);
        let key = crate::view::rows::file_list_projection_key(repo_id.0, source, sort, filter);
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        controller.sort = sort;
        controller.kind_filter = filter;
        let (query, options) = (controller.query.clone(), controller.search_options);
        controller
            .projection_cache
            .projection_for_query(&key, files, sort, filter, &query, options)
    }

    fn file_list_plan_from_projection(
        &self,
        list: crate::view::rows::FileListId,
        repo_id: RepoId,
        rev: u64,
        // Separates lists sharing a `rev` — every worktree in one scan does.
        scope: Option<&std::path::Path>,
        projection: &crate::view::rows::CommitFileProjection,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let layout = self.file_list_layout_for(repo_id, list);
        let sort = self.file_list_sort_for(list);
        let key = crate::view::rows::file_list_projection_key_scoped(
            repo_id.0,
            self.file_source_key(repo_id, rev, scope),
            sort,
            self.file_list_filter_for(list),
            scope,
        );
        let key = self.review_projection_key(repo_id, list, key);
        let collapsed = self.file_list_collapsed_for(repo_id, list);
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        let collapsed_groups = controller.collapsed_groups().to_vec();
        let labels = controller.kind_labels();
        controller.plan_cache.plan_for(
            key,
            crate::view::rows::PlanShape {
                layout,
                collapsed: &collapsed,
                collapsed_groups: &collapsed_groups,
                file_count: projection.source_indices.len(),
            },
            || {
                crate::view::rows::FileTree::build(
                    projection.source_indices.iter().filter_map(|source_ix| {
                        files
                            .get(*source_ix)
                            .map(|file| crate::view::rows::FileTreeItem {
                                path: file.path.as_path(),
                                additions: file.line_stats.additions,
                                deletions: file.line_stats.deletions,
                            })
                    }),
                    sort,
                )
            },
            || {
                (
                    projection
                        .source_indices
                        .iter()
                        .map(|source_ix| {
                            files.get(*source_ix).map_or(usize::MAX, |file| {
                                crate::view::file_list_controller::group_of(file.kind)
                            })
                        })
                        .collect(),
                    labels,
                )
            },
        )
    }

    pub(in crate::view) fn cached_worktree_file_plan(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let projection =
            self.cached_worktree_file_projection(repo_id, worktree_dirty_rev, worktree_path, files);
        self.file_list_plan_from_projection(
            crate::view::rows::FileListId::WorktreeFiles,
            repo_id,
            worktree_dirty_rev,
            Some(worktree_path),
            &projection,
            files,
        )
    }

    pub(in crate::view) fn cached_range_file_plan(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<crate::view::rows::FileListPlan> {
        let projection = self.cached_range_file_projection(repo_id, range_files_rev, files);
        self.file_list_plan_from_projection(
            crate::view::rows::FileListId::RangeFiles,
            repo_id,
            range_files_rev,
            None,
            &projection,
            files,
        )
    }

    pub(in crate::view) fn active_commit_file_source_indices(
        &self,
        repo_id: RepoId,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        if repo.history_state.range_selection.is_some() {
            let Loadable::Ready(files) = &repo.history_state.range_files else {
                return None;
            };
            let rev = repo.history_state.range_files_rev;
            let projection = self.cached_range_file_projection(repo_id, rev, files);
            let plan = self.cached_range_file_plan(repo_id, rev, files);
            return Some(if plan.reorders() {
                plan.ordered()
                    .iter()
                    .filter_map(|ordinal| projection.source_indices.get(ordinal).copied())
                    .collect()
            } else {
                projection.source_indices.clone()
            });
        }
        let Loadable::Ready(details) = &repo.history_state.commit_details else {
            return None;
        };
        let projection = self.cached_commit_file_projection(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        let plan = self.cached_commit_file_plan(
            repo_id,
            repo.history_state.commit_details_rev,
            &details.files,
        );
        Some(drawn_file_source_indices(&projection, &plan))
    }

    /// The comparison view's files as source indices in drawn order — the
    /// counterpart to [`Self::active_commit_file_source_indices`].
    pub(in crate::view) fn active_range_file_source_indices(
        &self,
        repo_id: RepoId,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let Loadable::Ready(files) = &repo.history_state.range_files else {
            return None;
        };
        let rev = repo.history_state.range_files_rev;
        let projection = self.cached_range_file_projection(repo_id, rev, files);
        let plan = self.cached_range_file_plan(repo_id, rev, files);
        Some(drawn_file_source_indices(&projection, &plan))
    }

    /// A linked worktree's changed files as source indices in *drawn* order —
    /// the counterpart to [`Self::active_commit_file_source_indices`]. An
    /// inline worktree diff's `selected_ix` is a source index, so prev/next
    /// file has to step through this rather than through source order.
    pub(in crate::view) fn active_worktree_file_source_indices(
        &self,
        repo_id: RepoId,
        worktree_path: &std::path::Path,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let worktree_dirty_rev = repo.worktree_dirty_rev;
        // The list on screen has to be the one the diff was opened from, or its
        // indices belong to another checkout's files.
        let summary = self
            .selected_worktree_summary()
            .filter(|summary| summary.path == worktree_path)?;
        let inputs = self.cached_worktree_file_inputs(repo_id, worktree_dirty_rev, summary);
        let projection = self.cached_worktree_file_projection(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &inputs.files,
        );
        let plan = self.cached_worktree_file_plan(
            repo_id,
            worktree_dirty_rev,
            &summary.path,
            &inputs.files,
        );
        if !plan.reorders() {
            return Some(projection.source_indices.clone());
        }
        // Drawn order, and deliberately every file: a collapsed folder or
        // group hides rows, it does not narrow what prev/next file steps
        // through.
        Some(
            plan.ordered()
                .iter()
                .filter_map(|ordinal| projection.source_indices.get(ordinal).copied())
                .collect(),
        )
    }

    fn shared_file_sort_for(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> Option<crate::view::rows::CommitFileSort> {
        self.state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.shared_preferences.as_ref())
            .map(|snapshot| {
                snapshot
                    .preferences
                    .file_sorts
                    .get(&list.preference_key())
                    .copied()
                    .map(crate::view::rows::CommitFileSort::from)
                    .unwrap_or(self.default_file_list_sort)
            })
    }

    fn publish_file_sort(
        &self,
        list: crate::view::rows::FileListId,
        sort: crate::view::rows::CommitFileSort,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        if self.shared_file_sort_for(repo_id, list).is_none() {
            return false;
        }
        // A previous choice may still be queued behind persistence work.
        // Publish every explicit choice; the hub deduplicates in queue order.
        self.store.dispatch(Msg::UpdateRepositoryPreference {
            repo_id,
            update: gitcomet_state::model::RepositoryPreferenceUpdate::FileSort {
                list: list.preference_key(),
                sort: sort.into(),
            },
        });
        true
    }

    pub(in crate::view) fn set_commit_file_sort(
        &mut self,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.publish_file_sort(crate::view::rows::FileListId::CommitFiles, sort) {
            return;
        }
        if self.commit_file_sort == sort {
            return;
        }
        self.commit_file_sort = sort;
        self.commit_files_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_commit_file_filter(
        &mut self,
        filter: crate::view::rows::CommitFileFilter,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_file_filter == filter {
            return;
        }
        self.commit_file_filter = filter;
        self.commit_files_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    /// The cached main pane reads this projection to render commit-diff file
    /// navigation, so local sort/filter changes must invalidate it explicitly.
    pub(in crate::view) fn notify_commit_file_projection_dependents(
        &self,
        cx: &mut gpui::Context<Self>,
    ) {
        let diff_renderer = self.diff_renderer.clone();
        cx.defer(move |cx| {
            let _ = diff_renderer.update(cx, |_pane, cx| cx.notify());
        });
    }

    pub(in crate::view) fn cached_range_file_rows(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let key = self.file_source_key(repo_id, range_files_rev, None);
        self.file_controller(repo_id, crate::view::rows::FileListId::RangeFiles)
            .borrow_mut()
            .presentations
            .rows_for(&key, files)
    }

    /// The scan entry for the worktree row the history selection is on.
    pub(in crate::view) fn selected_worktree_summary(
        &self,
    ) -> Option<&gitcomet_core::domain::WorktreeDirtySummary> {
        let repo = self.active_repo()?;
        let path = repo.history_state.worktree_selection.as_ref()?;
        let Loadable::Ready(dirty) = &repo.worktree_dirty else {
            return None;
        };
        dirty.iter().find(|summary| &summary.path == path)
    }

    /// The changed files of a linked worktree, in the shape the row builder and
    /// the inline-diff navigation need them.
    ///
    /// Derived once per scan and shared by every frame afterwards. Built inline it
    /// is O(all changed files) — three vectors and three `PathBuf` clones per file
    /// — on every layout pass of a list that only ever shows a screenful of rows.
    pub(in crate::view) fn cached_worktree_file_inputs(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        summary: &gitcomet_core::domain::WorktreeDirtySummary,
    ) -> Arc<WorktreeFileListInputs> {
        let mut cache = self.worktree_file_inputs.borrow_mut();
        // Compared field by field rather than against a freshly built key: the hit
        // path runs every frame, and building the key clones a `PathBuf`.
        if let Some(((cached_repo, cached_rev, cached_path), inputs)) = cache.as_ref()
            && *cached_repo == repo_id
            && *cached_rev == worktree_dirty_rev
            && cached_path == &summary.path
        {
            return Arc::clone(inputs);
        }

        // Every file is an entry so the diff view can step between them with the
        // same navigation submodule diffs get. Built by the shared builder rather
        // than here: the reducer re-resolves an open diff's entries against each
        // new scan, and it has to arrive at the same order these rows are in.
        let entries: Arc<[_]> = gitcomet_state::model::worktree_inline_diff_entries(summary).into();
        let files = entries
            .iter()
            .map(|entry| {
                // The entry names its lane, so read the counts from that one.
                let stats = match &entry.target {
                    gitcomet_core::domain::DiffTarget::WorkingTree { path, area, .. } => summary
                        .line_stats
                        .for_area(*area)
                        .get(path)
                        .copied()
                        .unwrap_or_default(),
                    _ => Default::default(),
                };
                gitcomet_core::domain::CommitFileChange::new(entry.path.clone(), entry.kind)
                    .with_line_stats(stats)
            })
            .collect();

        let inputs = Arc::new(WorktreeFileListInputs { files, entries });
        *cache = Some((
            (repo_id, worktree_dirty_rev, summary.path.clone()),
            Arc::clone(&inputs),
        ));
        inputs
    }

    /// Presentation rows for a linked worktree's changed files. Keyed on the
    /// scan revision, so it rebuilds when the worktree is rescanned and is shared
    /// across renders otherwise.
    pub(in crate::view) fn cached_worktree_file_rows(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        files: &[gitcomet_core::domain::CommitFileChange],
    ) -> Arc<[crate::view::rows::CommitFileRowPresentation]> {
        let key = self.file_source_key(repo_id, worktree_dirty_rev, Some(worktree_path));
        self.file_controller(repo_id, crate::view::rows::FileListId::WorktreeFiles)
            .borrow_mut()
            .presentations
            .rows_for(&key, files)
    }

    /// Path-truncation signature for a linked worktree's file rows.
    ///
    /// The scan revision bumps per repo-wide rescan, not per worktree, so the
    /// worktree's own path has to be in here: two worktrees with the same file
    /// count and the same visible range are otherwise indistinguishable, and the
    /// second one would inherit the first one's measured alignment.
    pub(in crate::view) fn worktree_files_visible_signature(
        &self,
        repo_id: RepoId,
        worktree_dirty_rev: u64,
        worktree_path: &std::path::Path,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            worktree_dirty_rev,
            worktree_path,
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in crate::view) fn range_files_visible_signature(
        &self,
        repo_id: RepoId,
        range_files_rev: u64,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            range_files_rev,
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in crate::view) fn status_path_alignment_group(
        &self,
        section: StatusSection,
    ) -> &components::PathTruncationAlignmentGroup {
        match section {
            StatusSection::CombinedUnstaged | StatusSection::Unstaged => {
                &self.unstaged_path_alignment_group
            }
            StatusSection::Untracked => &self.untracked_path_alignment_group,
            StatusSection::Staged => &self.staged_path_alignment_group,
        }
    }

    pub(in crate::view) fn status_visible_signature(
        &self,
        repo: &RepoState,
        section: StatusSection,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo.id,
            Self::status_section_alignment_key(section),
            status_section_content_rev(repo, section),
            total_rows,
            range.start,
            range.end,
        ))
    }

    pub(in crate::view) fn commit_files_visible_signature(
        &self,
        repo_id: RepoId,
        commit_details_rev: u64,
        range: &Range<usize>,
        total_rows: usize,
    ) -> u64 {
        path_alignment_visible_signature(&(
            repo_id,
            commit_details_rev,
            self.shared_file_sort_for(repo_id, crate::view::rows::FileListId::CommitFiles)
                .unwrap_or(self.commit_file_sort),
            self.commit_file_filter,
            total_rows,
            range.start,
            range.end,
        ))
    }

    /// One entry point for the controls, whichever list they sit above.
    pub(in crate::view) fn file_list_sort_for(
        &self,
        list: crate::view::rows::FileListId,
    ) -> crate::view::rows::CommitFileSort {
        if let crate::view::rows::FileListId::Status(section) = list {
            return self.status_file_sort_for(section);
        }
        if let Some(sort) = self
            .active_repo_id()
            .and_then(|id| self.shared_file_sort_for(id, list))
        {
            return sort;
        }
        match list {
            crate::view::rows::FileListId::CommitFiles => self.commit_file_sort,
            other => self
                .active_repo_id()
                .and_then(|repo_id| self.list_sort.get(&(repo_id, other)))
                .copied()
                .unwrap_or(self.default_file_list_sort),
        }
    }

    pub(in crate::view) fn set_file_list_sort(
        &mut self,
        list: crate::view::rows::FileListId,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        match list {
            crate::view::rows::FileListId::Status(section) => {
                self.set_status_file_sort(section, sort, cx)
            }
            crate::view::rows::FileListId::CommitFiles => self.set_commit_file_sort(sort, cx),
            other => {
                if self.publish_file_sort(other, sort) {
                    return;
                }
                let Some(repo_id) = self.active_repo_id() else {
                    return;
                };
                if self.file_list_sort_for(other) == sort {
                    return;
                }
                self.list_sort.insert((repo_id, other), sort);
                self.notify_commit_file_projection_dependents(cx);
                cx.notify();
            }
        }
    }

    pub(in crate::view) fn file_list_filter_for(
        &self,
        list: crate::view::rows::FileListId,
    ) -> crate::view::rows::CommitFileFilter {
        match list {
            crate::view::rows::FileListId::CommitFiles => self.commit_file_filter,
            other => self
                .active_repo_id()
                .and_then(|repo_id| self.list_filter.get(&(repo_id, other)))
                .copied()
                .unwrap_or_default(),
        }
    }

    pub(in crate::view) fn set_file_list_filter(
        &mut self,
        list: crate::view::rows::FileListId,
        filter: crate::view::rows::CommitFileFilter,
        cx: &mut gpui::Context<Self>,
    ) {
        if list == crate::view::rows::FileListId::CommitFiles {
            self.set_commit_file_filter(filter, cx);
            return;
        }
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        if self.file_list_filter_for(list) == filter {
            return;
        }
        self.list_filter.insert((repo_id, list), filter);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    pub(in crate::view) fn status_file_sort_for(
        &self,
        section: StatusSection,
    ) -> crate::view::rows::CommitFileSort {
        if let Some(sort) = self.active_repo_id().and_then(|id| {
            self.shared_file_sort_for(id, crate::view::rows::FileListId::Status(section))
        }) {
            return sort;
        }
        self.status_file_sort
            .get(&section)
            .copied()
            .unwrap_or(self.default_file_list_sort)
    }

    pub(in crate::view) fn set_status_file_sort(
        &mut self,
        section: StatusSection,
        sort: crate::view::rows::CommitFileSort,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.publish_file_sort(crate::view::rows::FileListId::Status(section), sort) {
            return;
        }
        if self.status_file_sort_for(section) == sort {
            return;
        }
        self.status_file_sort.insert(section, sort);
        self.notify_commit_file_projection_dependents(cx);
        cx.notify();
    }

    /// A status section's entries in the backing slice's index space, filtered
    /// and in display order. Everything that renders, selects or navigates the
    /// section must come through here, or it walks a different order than the
    /// rows do.
    pub(in crate::view) fn status_section_order(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> Option<Arc<[usize]>> {
        let sort = self.status_file_sort_for(section);
        let key = crate::view::rows::file_list_projection_key(
            repo.id.0,
            self.file_source_key(repo.id, status_section_order_rev(repo, section, sort), None),
            sort,
            crate::view::rows::CommitFileFilter::All,
        );
        let key = self.review_projection_key(
            repo.id,
            crate::view::rows::FileListId::Status(section),
            key,
        );
        let slot = Self::status_section_alignment_key(section) as usize;
        let mut cache = self.status_section_order.borrow_mut();
        if let Some((cached_key, indexes)) = cache[slot].as_ref()
            && *cached_key == key
        {
            return Some(Arc::clone(indexes));
        }
        let base = StatusSectionEntries::source_order_indexes(repo, section)?;
        #[cfg(test)]
        Self::record_status_footer_work(|work| work.projection_builds += 1);
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries()?,
            _ => repo.worktree_status_entries()?,
        };
        let ordered = crate::view::rows::status_section_sorted_indexes(
            entries,
            &base,
            sort,
            repo.line_stats_for_area(section.diff_area())
                .filter(|_| sort.needs_line_stats()),
        );
        let matcher = {
            let controller =
                self.file_controller(repo.id, crate::view::rows::FileListId::Status(section));
            let controller = controller.borrow();
            crate::view::rows::path_query_matcher(&controller.query, controller.search_options)
        };
        let ordered = match matcher {
            None => ordered,
            Some(matcher) => ordered
                .iter()
                .copied()
                .filter(|&ix| crate::view::rows::path_matches_query(&entries[ix].path, &matcher))
                .collect(),
        };
        cache[slot] = Some((key, Arc::clone(&ordered)));
        Some(ordered)
    }

    pub(in crate::view) fn status_path_display_position(
        &self,
        section: StatusSection,
        path: &std::path::Path,
    ) -> Option<usize> {
        let repo = self.active_repo()?;
        let order = self.active_status_section_order(repo.id, section)?;
        let entries = StatusSectionEntries::from_repo_with_order(repo, section, order)?;
        entries.iter().position(|entry| entry.path == path)
    }

    /// Expand whatever hides the file at drawn-order `position`, and return its
    /// row.
    pub(in crate::view) fn reveal_status_row(
        &mut self,
        section: StatusSection,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        let repo_id = self.active_repo_id()?;
        let list = crate::view::rows::FileListId::Status(section);
        let plan = {
            let repo = self.active_repo()?;
            self.status_file_plan(repo, section)
        };
        if !plan.reorders() {
            return Some(position);
        }
        let ordinal = crate::view::rows::FileOrdinal(plan.ordered().iter().nth(position)?);
        if self.expand_hiding(repo_id, list, &plan, ordinal) {
            cx.notify();
            let repo = self.active_repo()?;
            let plan = self.status_file_plan(repo, section);
            return plan.row_ix_for_ordinal(ordinal).map(|row| row.0);
        }
        plan.row_ix_for_ordinal(ordinal).map(|row| row.0)
    }

    /// Every file under `key`, including any hidden by a nested collapse —
    /// letting collapse narrow a stage would silently leave work behind.
    ///
    /// By prefix rather than the row's `subtree` range, which would have to be
    /// captured per row per frame and could go stale between paint and click.
    /// `Path::starts_with` is component-wise, so `src` never captures `src2`.
    /// The section's paths in drawn order. Shift-click spans a run of *rows*,
    /// and a tree's row order is not its projection order — resolving the range
    /// against the projection skips files displayed between the clicks.
    pub(in crate::view) fn status_display_order_paths(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> Vec<std::path::PathBuf> {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries(),
            _ => repo.worktree_status_entries(),
        };
        let (Some(entries), Some(order)) =
            (entries, self.active_status_section_order(repo_id, section))
        else {
            return Vec::new();
        };
        order
            .iter()
            .filter_map(|source_ix| entries.get(*source_ix).map(|entry| entry.path.clone()))
            .collect()
    }

    /// Identity of the display order, for validating a cached shift-click
    /// anchor. Everything that reorders rows belongs here: sort and layout both
    /// do so without moving any status or line-stats rev.
    pub(in crate::view) fn status_anchor_order_rev(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let list = crate::view::rows::FileListId::Status(section);
        let sort = self.status_file_sort_for(section);
        let mut hasher = rustc_hash::FxHasher::default();
        status_section_order_rev(repo, section, sort).hash(&mut hasher);
        sort.hash(&mut hasher);
        self.file_list_layout_for(repo.id, list)
            .key()
            .hash(&mut hasher);
        self.file_list_collapsed_for(repo.id, list)
            .rev()
            .hash(&mut hasher);
        hasher.finish()
    }

    pub(in crate::view) fn status_folder_subtree_paths(
        &self,
        repo_id: RepoId,
        section: StatusSection,
        key: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let Some(entries) = self.status_section_entries(repo, section) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|entry| entry.path.starts_with(key))
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// The commit or comparison list's files in drawn order, with whether each
    /// is a submodule, for shift-click ranges and actions on the selection.
    fn commit_list_files_in_drawn_order(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> Vec<(std::path::PathBuf, bool)> {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return Vec::new();
        };
        let (files, order) = match list {
            crate::view::rows::FileListId::CommitFiles => {
                let Loadable::Ready(details) = &repo.history_state.commit_details else {
                    return Vec::new();
                };
                (
                    &details.files[..],
                    self.active_commit_file_source_indices(repo_id),
                )
            }
            crate::view::rows::FileListId::RangeFiles => {
                let Loadable::Ready(files) = &repo.history_state.range_files else {
                    return Vec::new();
                };
                (&files[..], self.active_range_file_source_indices(repo_id))
            }
            _ => return Vec::new(),
        };
        order
            .iter()
            .flat_map(|order| order.iter())
            .filter_map(|source_ix| files.get(*source_ix))
            .map(|file| (file.path.clone(), file.is_submodule))
            .collect()
    }

    /// Identity of a commit or comparison list's drawn order, validating a
    /// cached shift-click anchor like [`Self::status_anchor_order_rev`].
    fn commit_list_anchor_order_rev(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let mut hasher = rustc_hash::FxHasher::default();
        match list {
            crate::view::rows::FileListId::CommitFiles => {
                repo.history_state.commit_details_rev.hash(&mut hasher)
            }
            _ => repo.history_state.range_files_rev.hash(&mut hasher),
        }
        self.file_list_sort_for(list).hash(&mut hasher);
        self.file_list_filter_for(list).hash(&mut hasher);
        self.file_list_layout_for(repo_id, list)
            .key()
            .hash(&mut hasher);
        self.file_list_collapsed_for(repo_id, list)
            .rev()
            .hash(&mut hasher);
        Some(hasher.finish())
    }

    /// Applies a click on a commit or comparison file row to its selection.
    pub(in crate::view) fn commit_list_selection_apply_click(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        clicked_path: std::path::PathBuf,
        display_position: Option<usize>,
        modifiers: gpui::Modifiers,
    ) {
        let order_rev = self.commit_list_anchor_order_rev(repo_id, list);
        let entries = modifiers.shift.then(|| {
            self.commit_list_files_in_drawn_order(repo_id, list)
                .into_iter()
                .map(|(path, _)| path)
                .collect::<Vec<_>>()
        });
        crate::view::rows::apply_file_list_selection_click(
            self.file_list_selection.entry((repo_id, list)).or_default(),
            clicked_path,
            display_position,
            modifiers,
            order_rev,
            entries.as_deref(),
        );
    }

    pub(in crate::view) fn commit_list_selected_paths(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> &[std::path::PathBuf] {
        self.file_list_selection
            .get(&(repo_id, list))
            .map(|selection| selection.paths.as_slice())
            .unwrap_or(&[])
    }

    /// The files an action on `clicked_path` covers: the whole selection when
    /// it holds more than one file and includes the clicked one, else just
    /// that file. Submodules carry no file change and are left out.
    pub(in crate::view) fn commit_list_paths_for_action(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        clicked_path: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        let selection = self.commit_list_selected_paths(repo_id, list);
        if selection.len() < 2 || !selection.iter().any(|path| path == clicked_path) {
            return vec![clicked_path.to_path_buf()];
        }
        let selected: rustc_hash::FxHashSet<&std::path::Path> =
            selection.iter().map(std::path::PathBuf::as_path).collect();
        // Drawn order, so the dialog and the commit list files as shown.
        self.commit_list_files_in_drawn_order(repo_id, list)
            .into_iter()
            .filter(|(path, is_submodule)| !is_submodule && selected.contains(path.as_path()))
            .map(|(path, _)| path)
            .collect()
    }

    /// Every file a list shows, in drawn order.
    fn file_list_paths(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
    ) -> Vec<std::path::PathBuf> {
        use crate::view::rows::FileListId;
        match list {
            FileListId::Status(section) => self.status_display_order_paths(repo_id, section),
            FileListId::CommitFiles | FileListId::RangeFiles => self
                .commit_list_files_in_drawn_order(repo_id, list)
                .into_iter()
                .map(|(path, _)| path)
                .collect(),
            FileListId::WorktreeFiles => {
                let (Some(repo), Some(summary)) = (
                    self.active_repo().filter(|repo| repo.id == repo_id),
                    self.selected_worktree_summary(),
                ) else {
                    return Vec::new();
                };
                self.cached_worktree_file_inputs(repo_id, repo.worktree_dirty_rev, summary)
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect()
            }
        }
    }

    /// The files under a folder row, in drawn order, without submodules —
    /// including any a nested collapse hides, as the folder's Stage does.
    pub(in crate::view) fn file_list_folder_files(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        key: &std::path::Path,
    ) -> Vec<std::path::PathBuf> {
        use crate::view::rows::FileListId;
        match list {
            FileListId::Status(section) => self.status_folder_subtree_paths(repo_id, section, key),
            FileListId::CommitFiles | FileListId::RangeFiles => self
                .commit_list_files_in_drawn_order(repo_id, list)
                .into_iter()
                .filter(|(path, is_submodule)| !is_submodule && path.starts_with(key))
                .map(|(path, _)| path)
                .collect(),
            FileListId::WorktreeFiles => self
                .file_list_paths(repo_id, list)
                .into_iter()
                .filter(|path| path.starts_with(key))
                .collect(),
        }
    }

    /// Expands or collapses a folder row; `recursive` takes every folder
    /// below it along.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn set_file_list_folder_collapsed(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        key: Arc<std::path::Path>,
        chain: &[Arc<std::path::Path>],
        collapse: bool,
        recursive: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        // Every directory between the folder and its files has a row, or is
        // a segment of one; marking each collapses them all.
        let below: Vec<Arc<std::path::Path>> = if collapse && recursive {
            let mut dirs = rustc_hash::FxHashSet::default();
            for path in self.file_list_paths(repo_id, list) {
                let mut dir = path.parent();
                while let Some(parent) = dir {
                    if parent == key.as_ref() || !parent.starts_with(&key) {
                        break;
                    }
                    if !dirs.insert(Arc::<std::path::Path>::from(parent)) {
                        break;
                    }
                    dir = parent.parent();
                }
            }
            dirs.into_iter().collect()
        } else {
            Vec::new()
        };
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        let entry = &mut controller.collapsed;
        match (collapse, recursive) {
            (true, false) => entry.collapse(key, chain),
            (false, false) => entry.expand(chain),
            (false, true) => entry.expand_under(&key, chain),
            (true, true) => entry.collapse_all(std::iter::once(key).chain(below)),
        }
        cx.notify();
    }

    pub(in crate::view) fn clear_commit_list_selections(&mut self, repo_id: RepoId) {
        self.file_list_selection
            .retain(|(selection_repo, _), _| *selection_repo != repo_id);
    }

    /// Expand whatever hides the commit file at `position` and return its row.
    /// Mirrors [`Self::reveal_status_row`]: in a tree the position among files
    /// is not the row index, and a collapsed file has no row at all.
    pub(in crate::view) fn reveal_commit_file_row(
        &mut self,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        self.reveal_file_list_row(crate::view::rows::FileListId::CommitFiles, position, cx)
    }

    /// [`Self::reveal_commit_file_row`] for the comparison view's list.
    pub(in crate::view) fn reveal_range_file_row(
        &mut self,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        self.reveal_file_list_row(crate::view::rows::FileListId::RangeFiles, position, cx)
    }

    /// The plan of a commit-diff file list: commit details or comparison.
    fn commit_diff_file_plan(
        &self,
        list: crate::view::rows::FileListId,
    ) -> Option<Arc<crate::view::rows::FileListPlan>> {
        let repo = self.active_repo()?;
        match list {
            crate::view::rows::FileListId::RangeFiles => {
                let Loadable::Ready(files) = &repo.history_state.range_files else {
                    return None;
                };
                Some(self.cached_range_file_plan(
                    repo.id,
                    repo.history_state.range_files_rev,
                    files,
                ))
            }
            _ => {
                let Loadable::Ready(details) = &repo.history_state.commit_details else {
                    return None;
                };
                Some(self.cached_commit_file_plan(
                    repo.id,
                    repo.history_state.commit_details_rev,
                    &details.files,
                ))
            }
        }
    }

    /// Expand the folders or the group hiding `ordinal`'s row; whether any
    /// was collapsed.
    fn expand_hiding(
        &self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        plan: &crate::view::rows::FileListPlan,
        ordinal: crate::view::rows::FileOrdinal,
    ) -> bool {
        let chains = plan.reveal(ordinal);
        let group = plan.reveal_group(ordinal);
        if chains.is_empty() && group.is_none() {
            return false;
        }
        let controller = self.file_controller(repo_id, list);
        let mut controller = controller.borrow_mut();
        for chain in chains {
            controller.collapsed.expand(&chain);
        }
        if let Some(group) = group {
            controller.toggle_group(group);
        }
        true
    }

    /// Collapse or expand a group of a built-in list.
    pub(in crate::view) fn toggle_file_list_group(
        &mut self,
        repo_id: RepoId,
        list: crate::view::rows::FileListId,
        group: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.file_controller(repo_id, list)
            .borrow_mut()
            .toggle_group(group);
        cx.notify();
    }

    fn reveal_file_list_row(
        &mut self,
        list: crate::view::rows::FileListId,
        position: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Option<usize> {
        let repo_id = self.active_repo_id()?;
        let plan = self.commit_diff_file_plan(list)?;
        if !plan.reorders() {
            return Some(position);
        }
        // `position` indexes the drawn-order list navigation walks.
        let ordinal = crate::view::rows::FileOrdinal(plan.ordered().iter().nth(position)?);
        if self.expand_hiding(repo_id, list, &plan, ordinal) {
            cx.notify();
            let plan = self.commit_diff_file_plan(list)?;
            return plan.row_ix_for_ordinal(ordinal).map(|row| row.0);
        }
        plan.row_ix_for_ordinal(ordinal).map(|row| row.0)
    }

    /// The section's backing-slice indexes in *drawn* order, for prev/next-file
    /// navigation. A tree hoists directories above files, so its rows are a
    /// permutation of the projection and stepping the projection would jump
    /// around the list.
    pub(in crate::view) fn active_status_section_order(
        &self,
        repo_id: RepoId,
        section: StatusSection,
    ) -> Option<Arc<[usize]>> {
        let repo = self.active_repo().filter(|repo| repo.id == repo_id)?;
        let projection = self.status_section_order(repo, section)?;
        let plan = self.status_file_plan(repo, section);
        if !plan.reorders() {
            return Some(projection);
        }
        #[cfg(test)]
        Self::record_status_footer_work(|work| work.order_materializations += 1);
        Some(
            plan.ordered()
                .iter()
                .filter_map(|ordinal| projection.get(ordinal).copied())
                .collect(),
        )
    }

    pub(in crate::view) fn status_section_entries<'a>(
        &self,
        repo: &'a RepoState,
        section: StatusSection,
    ) -> Option<StatusSectionEntries<'a>> {
        let order = self.status_section_order(repo, section)?;
        StatusSectionEntries::from_repo_with_order(repo, section, order)
    }

    pub(in crate::view) fn status_file_plan(
        &self,
        repo: &RepoState,
        section: StatusSection,
    ) -> Arc<crate::view::rows::FileListPlan> {
        let list = crate::view::rows::FileListId::Status(section);
        let layout = self.file_list_layout_for(repo.id, list);
        let sort = self.status_file_sort_for(section);
        let order = self.status_section_order(repo, section).unwrap_or_default();
        let key = crate::view::rows::file_list_projection_key(
            repo.id.0,
            self.file_source_key(repo.id, status_section_content_rev(repo, section), None),
            sort,
            crate::view::rows::CommitFileFilter::All,
        );
        let key = self.review_projection_key(repo.id, list, key);
        let collapsed = self.file_list_collapsed_for(repo.id, list);
        let entries = match section {
            StatusSection::Staged => repo.staged_status_entries(),
            _ => repo.worktree_status_entries(),
        };
        let line_stats = repo.line_stats_for_area(section.diff_area());
        let controller = self.file_controller(repo.id, list);
        let mut controller = controller.borrow_mut();
        let collapsed_groups = controller.collapsed_groups().to_vec();
        let labels = controller.kind_labels();
        #[cfg(test)]
        let previous_builds = controller.plan_cache.builds();
        let plan = controller.plan_cache.plan_for(
            key,
            crate::view::rows::PlanShape {
                layout,
                collapsed: &collapsed,
                collapsed_groups: &collapsed_groups,
                file_count: order.len(),
            },
            || {
                crate::view::rows::FileTree::build(
                    order.iter().filter_map(|source_ix| {
                        entries
                            .and_then(|entries| entries.get(*source_ix))
                            .map(|entry| {
                                let stats = line_stats
                                    .and_then(|stats| stats.get(&entry.path))
                                    .copied()
                                    .unwrap_or_default();
                                crate::view::rows::FileTreeItem {
                                    path: entry.path.as_path(),
                                    additions: stats.additions,
                                    deletions: stats.deletions,
                                }
                            })
                    }),
                    sort,
                )
            },
            || {
                (
                    order
                        .iter()
                        .map(|source_ix| {
                            entries
                                .and_then(|entries| entries.get(*source_ix))
                                .map_or(usize::MAX, |entry| {
                                    crate::view::file_list_controller::group_of(entry.kind)
                                })
                        })
                        .collect(),
                    labels,
                )
            },
        );
        #[cfg(test)]
        Self::record_status_footer_work(|work| {
            work.plan_builds += controller.plan_cache.builds() - previous_builds;
        });
        plan
    }

    pub(super) fn status_section_alignment_key(section: StatusSection) -> u8 {
        match section {
            StatusSection::CombinedUnstaged => 0,
            StatusSection::Untracked => 1,
            StatusSection::Unstaged => 2,
            StatusSection::Staged => 3,
        }
    }
}

/// A file list's source indices in drawn order, including files under collapsed folders.
fn drawn_file_source_indices(
    projection: &crate::view::rows::CommitFileProjection,
    plan: &crate::view::rows::FileListPlan,
) -> Arc<[usize]> {
    if !plan.reorders() {
        return projection.source_indices.clone();
    }
    plan.ordered()
        .iter()
        .filter_map(|ordinal| projection.source_indices.get(ordinal).copied())
        .collect()
}
