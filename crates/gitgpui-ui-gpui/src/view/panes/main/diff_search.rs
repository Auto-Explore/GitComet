use super::*;

impl MainPaneView {
    fn active_conflict_target(
        &self,
    ) -> Option<(
        std::path::PathBuf,
        Option<gitgpui_core::domain::FileConflictKind>,
    )> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let Loadable::Ready(status) = &repo.status else {
            return None;
        };
        let conflict = status
            .unstaged
            .iter()
            .find(|e| e.path == *path && e.kind == FileStatusKind::Conflicted)?;

        Some((path.clone(), conflict.conflict))
    }

    pub(in super::super::super) fn diff_search_recompute_matches(&mut self) {
        if !self.diff_search_active {
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            return;
        }

        if !self.is_file_preview_active() && self.active_conflict_target().is_none() {
            self.ensure_diff_visible_indices();
        }

        self.diff_search_recompute_matches_for_current_view();
    }

    pub(super) fn diff_search_recompute_matches_for_current_view(&mut self) {
        self.diff_search_matches.clear();
        self.diff_search_match_ix = None;

        let query = self.diff_search_query.as_ref().trim();
        if query.is_empty() {
            return;
        }

        if self.is_file_preview_active() {
            let Loadable::Ready(lines) = &self.worktree_preview else {
                return;
            };
            for (ix, line) in lines.iter().enumerate() {
                if contains_ascii_case_insensitive(line, query) {
                    self.diff_search_matches.push(ix);
                }
            }
        } else if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            let is_conflict_resolver =
                Self::conflict_resolver_strategy(conflict_kind).is_some();

            match (is_conflict_resolver, self.diff_view) {
                (true, _) => match self.conflict_resolver.diff_mode {
                    ConflictDiffMode::Split => {
                        for (ix, row) in self.conflict_resolver.diff_rows.iter().enumerate() {
                            if row
                                .old
                                .as_deref()
                                .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                                || row
                                    .new
                                    .as_deref()
                                    .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                            {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                    ConflictDiffMode::Inline => {
                        for (ix, row) in self.conflict_resolver.inline_rows.iter().enumerate() {
                            if contains_ascii_case_insensitive(row.content.as_str(), query) {
                                self.diff_search_matches.push(ix);
                            }
                        }
                    }
                },
                (false, DiffViewMode::Split) => {
                    for (ix, row) in self.conflict_resolver.diff_rows.iter().enumerate() {
                        if row
                            .old
                            .as_deref()
                            .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                            || row
                                .new
                                .as_deref()
                                .is_some_and(|s| contains_ascii_case_insensitive(s, query))
                        {
                            self.diff_search_matches.push(ix);
                        }
                    }
                }
                (false, DiffViewMode::Inline) => {
                    for (ix, row) in self.conflict_resolver.inline_rows.iter().enumerate() {
                        if contains_ascii_case_insensitive(row.content.as_str(), query) {
                            self.diff_search_matches.push(ix);
                        }
                    }
                }
            }
        } else {
            let total = self.diff_visible_indices.len();
            for visible_ix in 0..total {
                match self.diff_view {
                    DiffViewMode::Inline => {
                        let text =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        if contains_ascii_case_insensitive(text.as_ref(), query) {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                    DiffViewMode::Split => {
                        let left =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                        let right =
                            self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                        if contains_ascii_case_insensitive(left.as_ref(), query)
                            || contains_ascii_case_insensitive(right.as_ref(), query)
                        {
                            self.diff_search_matches.push(visible_ix);
                        }
                    }
                }
            }
        }

        if !self.diff_search_matches.is_empty() {
            self.diff_search_match_ix = Some(0);
            let first = self.diff_search_matches[0];
            self.diff_search_scroll_to_visible_ix(first);
        }
    }

    pub(in super::super::super) fn diff_search_prev_match(&mut self) {
        if !self.diff_search_active {
            return;
        }

        if self.diff_search_matches.is_empty() {
            self.diff_search_recompute_matches();
        }
        let len = self.diff_search_matches.len();
        if len == 0 {
            return;
        }

        let current = self
            .diff_search_match_ix
            .unwrap_or(0)
            .min(len.saturating_sub(1));
        let next_ix = if current == 0 { len - 1 } else { current - 1 };
        self.diff_search_match_ix = Some(next_ix);
        let target = self.diff_search_matches[next_ix];
        self.diff_search_scroll_to_visible_ix(target);
    }

    pub(in super::super::super) fn diff_search_next_match(&mut self) {
        if !self.diff_search_active {
            return;
        }

        if self.diff_search_matches.is_empty() {
            self.diff_search_recompute_matches();
        }
        let len = self.diff_search_matches.len();
        if len == 0 {
            return;
        }

        let current = self
            .diff_search_match_ix
            .unwrap_or(0)
            .min(len.saturating_sub(1));
        let next_ix = (current + 1) % len;
        self.diff_search_match_ix = Some(next_ix);
        let target = self.diff_search_matches[next_ix];
        self.diff_search_scroll_to_visible_ix(target);
    }

    fn diff_search_scroll_to_visible_ix(&mut self, visible_ix: usize) {
        if self.is_file_preview_active() {
            self.worktree_preview_scroll
                .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            return;
        }

        if let Some((_path, conflict_kind)) = self.active_conflict_target() {
            if Self::conflict_resolver_strategy(conflict_kind).is_some() {
                self.conflict_resolver_diff_scroll
                    .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            } else {
                self.diff_scroll
                    .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
            }
            return;
        }

        self.diff_scroll
            .scroll_to_item_strict(visible_ix, gpui::ScrollStrategy::Center);
        self.diff_selection_anchor = Some(visible_ix);
        self.diff_selection_range = Some((visible_ix, visible_ix));
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    'outer: for start in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        for (offset, needle_byte) in needle_bytes.iter().copied().enumerate() {
            let haystack_byte = haystack_bytes[start + offset];
            if !haystack_byte.eq_ignore_ascii_case(&needle_byte) {
                continue 'outer;
            }
        }
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::contains_ascii_case_insensitive;

    #[test]
    fn matches_empty_needle() {
        assert!(contains_ascii_case_insensitive("abc", ""));
    }

    #[test]
    fn matches_case_insensitively() {
        assert!(contains_ascii_case_insensitive("Hello", "he"));
        assert!(contains_ascii_case_insensitive("Hello", "HEL"));
        assert!(contains_ascii_case_insensitive("Hello", "lo"));
    }

    #[test]
    fn does_not_match_absent_substring() {
        assert!(!contains_ascii_case_insensitive("Hello", "world"));
    }
}
