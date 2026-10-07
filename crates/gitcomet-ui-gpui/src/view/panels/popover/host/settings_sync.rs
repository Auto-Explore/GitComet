//! Settings the host mirrors from the root view and persists: pins,
//! filters, collapsed items, date display, and diff presentation.

use super::*;

impl PopoverHost {
    pub(in crate::view) fn set_pinned_branches(
        &mut self,
        pinned: std::collections::BTreeMap<std::path::PathBuf, std::collections::BTreeSet<String>>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.pinned_branches_by_repo == pinned {
            return;
        }
        self.pinned_branches_by_repo = pinned;
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::view) fn set_branch_filter_query(
        &mut self,
        query: String,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_branch_search(query, self.branch_search.matcher.options(), cx);
    }

    pub(in crate::view) fn set_branch_search(
        &mut self,
        query: String,
        options: gitcomet_core::text_search::TextSearchOptions,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.branch_filter_query == query && self.branch_search.matcher.options() == options {
            return;
        }
        self.branch_search = crate::view::sidebar_search::SidebarSearch::new(&query, options);
        self.branch_filter_query = query;
        cx.notify();
    }

    /// The active branch filter, or `None` when it matches everything.
    ///
    /// Mirrors `matches_branch_filter`, which treats a blank query as "no
    /// filter" — so a lone space must not read as a filter that hides
    /// everything.
    pub(in crate::view) fn active_branch_filter(&self) -> Option<&str> {
        let query = self.branch_filter_query.trim();
        (!query.is_empty()).then_some(query)
    }

    pub(in crate::view) fn set_collapsed_items(
        &mut self,
        collapsed: std::collections::BTreeMap<
            std::path::PathBuf,
            std::collections::BTreeSet<String>,
        >,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.collapsed_items_by_repo == collapsed {
            return;
        }
        self.collapsed_items_by_repo = collapsed;
        cx.notify();
    }

    /// Whether a sidebar collapse key is currently collapsed, going through
    /// `branch_sidebar::is_collapsed` so default-collapsed sections and the
    /// inverted `expanded:` storage are read the same way the tree reads them.
    pub(in crate::view) fn sidebar_collapse_key_is_collapsed(
        &self,
        repo_id: RepoId,
        collapse_key: &str,
    ) -> bool {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        // A repo with nothing stored reads the same as one with an empty set:
        // `is_collapsed` answers from the key's own default in both cases.
        static EMPTY: std::sync::LazyLock<std::collections::BTreeSet<String>> =
            std::sync::LazyLock::new(std::collections::BTreeSet::new);
        let items = self
            .collapsed_items_by_repo
            .get(&repo.spec.workdir)
            .or_else(|| {
                repo.shared_preferences
                    .as_ref()
                    .map(|snapshot| &snapshot.preferences.collapsed_items)
            })
            .unwrap_or(&EMPTY);
        crate::view::branch_sidebar::is_collapsed(items, collapse_key)
    }

    /// Visible pinned roots for the branch section's "Unpin all (N)".
    ///
    /// Counting raw pin keys would overcount: the row builder skips a pin whose
    /// branch no longer exists, and skips one filtered out by the branch
    /// filter, so "Unpin all (3)" could sit above a single row.
    pub(in crate::view) fn pinned_branch_count(
        &self,
        repo_id: RepoId,
        section: BranchSection,
    ) -> usize {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return 0;
        };
        repo.shared_preferences.as_ref().map(|snapshot| &snapshot.preferences.pinned_items)
            .or_else(|| self.pinned_branches_by_repo.get(&repo.spec.workdir)).map_or(0, |items| {
            crate::view::branch_sidebar::matching_pinned_roots(repo, items, &self.branch_search).iter()
                .filter(|row| matches!(row, BranchSidebarRow::Branch { section: candidate, .. } | BranchSidebarRow::GroupHeader { section: candidate, .. } if *candidate == section)).count()
        })
    }

    pub(in crate::view) fn is_branch_pinned(
        &self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
    ) -> bool {
        let key = crate::view::branch_sidebar::branch_pin_storage_key(section, name);
        self.is_sidebar_item_pinned(repo_id, &key)
    }

    pub(in crate::view) fn is_sidebar_item_pinned(&self, repo_id: RepoId, key: &str) -> bool {
        let Some(repo) = self.state.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        repo.shared_preferences
            .as_ref()
            .map(|snapshot| &snapshot.preferences.pinned_items)
            .or_else(|| self.pinned_branches_by_repo.get(&repo.spec.workdir))
            .is_some_and(|items| items.contains(key))
    }

    pub(in crate::view) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_date_time_format(next, cx));
        self.sync_pane_date_settings(cx);
        cx.notify();
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        self.main_pane
            .update(cx, |pane, cx| pane.set_timezone(next, cx));
        self.sync_pane_date_settings(cx);
        cx.notify();
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.show_timezone == enabled {
            return;
        }
        self.show_timezone = enabled;
        self.main_pane
            .update(cx, |pane, cx| pane.set_show_timezone(enabled, cx));
        self.sync_pane_date_settings(cx);
        cx.notify();
        self.schedule_ui_settings_persist(cx);
    }

    pub(in crate::view) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_relative_dates != enabled {
            self.history_relative_dates = enabled;
            cx.notify();
        }
    }

    pub(in crate::view::panels::popover) fn sync_pane_date_settings(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let (format, timezone, show_timezone) =
            (self.date_time_format, self.timezone, self.show_timezone);
        self.details_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx);
        });
        self.reflog_pane.update(cx, |pane, cx| {
            pane.set_date_settings(format, timezone, show_timezone, cx);
        });
    }

    /// Track the global theme preference, which `schedule_ui_settings_persist`
    /// writes back. The visual theme arrives separately through `set_theme`.
    pub(in crate::view) fn sync_global_theme_mode(&mut self, next: ThemeMode) {
        self.theme_mode = next;
    }

    pub(in crate::view::panels::popover) fn schedule_ui_settings_persist(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let mode = self.theme_mode.clone();
        let fmt = self.date_time_format;
        let tz = self.timezone;
        let show_tz = self.show_timezone;
        let root_view = self.root_view.clone();
        cx.spawn(
            async move |_host: WeakEntity<Self>, cx: &mut gpui::AsyncApp| {
                let _ = root_view.update(cx, |root, cx| {
                    root.theme_mode = mode;
                    root.date_time_format = fmt;
                    root.timezone = tz;
                    root.show_timezone = show_tz;
                    root.schedule_ui_settings_persist(cx);
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn sync_change_tracking_view(
        &mut self,
        next: ChangeTrackingView,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.change_tracking_view == next {
            return;
        }

        self.change_tracking_view = next;
        if matches!(self.popover, Some(PopoverKind::ChangeTrackingSettings)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_commit_push_after_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_push_after_enabled == enabled {
            return;
        }

        self.commit_push_after_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_commit_amend_enabled(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.commit_amend_enabled == enabled {
            return;
        }

        self.commit_amend_enabled = enabled;
        if matches!(self.popover, Some(PopoverKind::CommitOptionsMenu { .. })) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_content_mode(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode == next {
            return;
        }

        self.diff_content_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffContentModeSettings)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_whitespace_mode(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode == next {
            return;
        }

        self.diff_whitespace_mode = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_reveal_whitespace_chars(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_reveal_whitespace_chars == next {
            return;
        }

        self.diff_reveal_whitespace_chars = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_word_wrap(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap == next {
            return;
        }

        self.diff_word_wrap = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }

    pub(in crate::view) fn sync_diff_show_line_numbers(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_show_line_numbers == next {
            return;
        }

        self.diff_show_line_numbers = next;
        if matches!(self.popover, Some(PopoverKind::DiffActionMenu)) {
            cx.notify();
        }
    }
}
