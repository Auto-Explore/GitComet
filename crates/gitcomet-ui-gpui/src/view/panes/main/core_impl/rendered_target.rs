use super::*;

impl MainPaneView {
    pub(super) fn rendered_file_target_path(target: &DiffTarget) -> Option<&std::path::Path> {
        match target {
            DiffTarget::WorkingTree { path, .. } => Some(path.as_path()),
            DiffTarget::Commit {
                path: Some(path), ..
            }
            | DiffTarget::CommitRange {
                path: Some(path), ..
            } => Some(path.as_path()),
            DiffTarget::Commit { path: None, .. } | DiffTarget::CommitRange { path: None, .. } => {
                None
            }
        }
    }

    pub(in crate::view) fn rendered_file_diff_loadable(
        &self,
    ) -> Option<&gitcomet_state::model::Loadable<Option<gitcomet_state::model::Shared<FileDiffText>>>>
    {
        if let Some(inline) = self.active_inline_submodule_diff() {
            Some(&inline.diff_file)
        } else {
            self.active_repo().map(|repo| &repo.diff_state.diff_file)
        }
    }

    pub(in crate::view) fn rendered_file_image_diff_loadable(
        &self,
    ) -> Option<
        &gitcomet_state::model::Loadable<Option<gitcomet_state::model::Shared<FileDiffImage>>>,
    > {
        if let Some(inline) = self.active_inline_submodule_diff() {
            Some(&inline.diff_file_image)
        } else {
            self.active_repo()
                .map(|repo| &repo.diff_state.diff_file_image)
        }
    }

    pub(in crate::view) fn rendered_file_diff_rev(&self) -> u64 {
        self.active_inline_submodule_diff()
            .map(|inline| inline.diff_file_rev)
            .or_else(|| self.active_repo().map(|repo| repo.diff_state.diff_file_rev))
            .unwrap_or(0)
    }

    pub(in crate::view) fn rendered_diff_workdir(&self) -> Option<&std::path::Path> {
        self.active_inline_submodule_diff()
            .map(|inline| inline.submodule_repo_path.as_path())
            .or_else(|| self.active_repo().map(|repo| repo.spec.workdir.as_path()))
    }

    pub(in crate::view) fn rendered_file_diff_identity(
        &self,
    ) -> Option<(
        RepoId,
        u64,
        DiffTarget,
        std::path::PathBuf,
        std::path::PathBuf,
    )> {
        let repo_id = self.active_repo_id()?;
        let diff_file_rev = self.rendered_file_diff_rev();
        let diff_target = self.rendered_diff_target()?.clone();
        let workdir = self.rendered_diff_workdir()?.to_path_buf();
        let rel_path = Self::rendered_file_target_path(&diff_target)?;
        let abs_path = workdir.join(rel_path);
        Some((repo_id, diff_file_rev, diff_target, workdir, abs_path))
    }

    pub(in crate::view) fn supports_diff_content_mode_toggle(&self, is_file_preview: bool) -> bool {
        !is_file_preview
            && !self.is_worktree_target_directory()
            && Self::is_file_diff_target(self.rendered_diff_target())
    }

    /// The diff mode actually in effect. Collapsed hides the unchanged parts of
    /// a patch, so a target the state layer loads as whole-file content — an
    /// added, deleted, or untracked file, which has no patch — has nothing to
    /// collapse and stays on Full however the setting is set.
    pub(in crate::view) fn effective_diff_content_mode(&self) -> DiffContentMode {
        if self.diff_content_mode == DiffContentMode::Collapsed
            && matches!(
                self.rendered_patch_diff_loadable(),
                Some(Loadable::NotLoaded)
            )
        {
            return DiffContentMode::Full;
        }
        self.diff_content_mode
    }

    pub(in crate::view) fn wants_file_diff_view(&self, is_file_preview: bool) -> bool {
        self.effective_diff_content_mode() == DiffContentMode::Full
            && self.supports_diff_content_mode_toggle(is_file_preview)
    }

    pub(in crate::view) fn wants_collapsed_diff_view(&self, is_file_preview: bool) -> bool {
        self.effective_diff_content_mode() == DiffContentMode::Collapsed
            && self.supports_diff_content_mode_toggle(is_file_preview)
    }

    pub(super) fn current_main_diff_supports_diff_content_toggle(&self) -> bool {
        let inline_submodule_diff_active = self.is_inline_submodule_diff_active();
        let has_submodule_summary = self
            .active_repo()
            .is_some_and(|repo| !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded));
        let untracked_directory_notice = if has_submodule_summary || inline_submodule_diff_active {
            None
        } else {
            self.untracked_directory_notice()
        };
        let is_file_preview = self.is_file_preview_active()
            && untracked_directory_notice.is_none()
            && !has_submodule_summary
            && !inline_submodule_diff_active;
        (inline_submodule_diff_active || !has_submodule_summary)
            && self.supports_diff_content_mode_toggle(is_file_preview)
    }

    pub(super) fn current_main_diff_wants_file_diff(&self) -> bool {
        let inline_submodule_diff_active = self.is_inline_submodule_diff_active();
        let has_submodule_summary = self
            .active_repo()
            .is_some_and(|repo| !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded));
        let untracked_directory_notice = if has_submodule_summary || inline_submodule_diff_active {
            None
        } else {
            self.untracked_directory_notice()
        };
        let is_file_preview = self.is_file_preview_active()
            && untracked_directory_notice.is_none()
            && !has_submodule_summary
            && !inline_submodule_diff_active;
        self.current_main_diff_supports_diff_content_toggle()
            && self.wants_file_diff_view(is_file_preview)
    }

    pub(super) fn rendered_patch_diff_cache_is_current(&self) -> bool {
        self.active_repo_id().is_some_and(|repo_id| {
            self.diff_cache_repo_id == Some(repo_id)
                && self.diff_cache_rev == self.rendered_patch_diff_rev()
                && self.diff_cache_target == self.rendered_diff_target().cloned()
        })
    }

    pub(super) fn rendered_file_diff_cache_is_current(&self) -> bool {
        let Some((repo_id, diff_file_rev, diff_target, _workdir, abs_path)) =
            self.rendered_file_diff_identity()
        else {
            return false;
        };

        self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_rev == diff_file_rev
            && self.file_diff_cache_target == Some(diff_target)
            && self.file_diff_cache_whitespace_mode == self.diff_whitespace_mode
            && self.file_diff_cache_path.as_ref() == Some(&abs_path)
    }

    pub(in crate::view) fn is_collapsed_diff_projection_active(&self) -> bool {
        self.effective_diff_content_mode() == DiffContentMode::Collapsed
            && self.current_main_diff_supports_diff_content_toggle()
            && self.rendered_patch_diff_cache_is_current()
            && self.rendered_file_diff_cache_is_current()
    }

    pub(in crate::view) fn collapsed_visible_row(
        &self,
        visible_ix: usize,
    ) -> Option<CollapsedDiffVisibleRow> {
        self.collapsed_diff_visible_rows.get(visible_ix).copied()
    }

    pub(in crate::view) fn current_collapsed_diff_projection_identity(
        &self,
    ) -> Option<CollapsedDiffProjectionIdentity> {
        let (repo_id, _diff_file_rev, diff_target, _workdir, abs_path) =
            self.rendered_file_diff_identity()?;
        Some(CollapsedDiffProjectionIdentity {
            repo_id,
            diff_target,
            file_path: abs_path,
            diff_whitespace_mode: self.diff_whitespace_mode,
            patch_content_signature: self.diff_cache_content_signature,
            file_content_signature: self.file_diff_cache_content_signature,
        })
    }

    pub(in crate::view) fn reset_collapsed_diff_projection(&mut self, clear_reveals: bool) {
        self.collapsed_diff_hunks.clear();
        self.collapsed_diff_hunk_ix_by_src_ix.clear();
        if clear_reveals {
            self.collapsed_diff_reveals.clear();
            self.collapsed_diff_projection_identity = None;
        }
        self.collapsed_diff_visible_rows.clear();
        self.collapsed_diff_hunk_visible_indices.clear();
        self.collapsed_diff_header_display_cache.clear();
        self.diff_visible_projection_rev = self.diff_visible_projection_rev.wrapping_add(1);
        self.clear_diff_text_projected_highlights();
        if clear_reveals {
            self.diff_visible_cache_projection_rev = u64::MAX;
        }
    }

    pub(in crate::view) fn invalidate_collapsed_diff_visible_projection(&mut self) {
        self.collapsed_diff_visible_rows.clear();
        self.collapsed_diff_hunk_visible_indices.clear();
        self.collapsed_diff_header_display_cache.clear();
        self.diff_visible_projection_rev = self.diff_visible_projection_rev.wrapping_add(1);
        // Revealing a hunk renumbers every row below it. The click highlights are
        // stored against the row indices they were projected onto, so leaving
        // them would paint the pair and the name's uses over unrelated rows --
        // and, since the spans are per-row display columns, over unrelated
        // characters.
        self.clear_diff_text_projected_highlights();
    }

    // Apply the mode inside the pane first, then sync the root preference
    // without re-entering `main_pane.update(...)`.
    pub(in crate::view) fn set_diff_content_mode_and_persist(
        &mut self,
        next: DiffContentMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_content_mode != next {
            self.set_diff_content_mode(next, cx);
        }
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.sync_diff_content_mode_from_pane(next, cx);
        });
    }

    pub(in crate::view) fn set_diff_whitespace_mode_and_persist(
        &mut self,
        next: DiffWhitespaceMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_whitespace_mode != next {
            self.set_diff_whitespace_mode(next, cx);
        }
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.sync_diff_whitespace_mode_from_pane(next, cx);
        });
    }

    pub(in crate::view) fn set_diff_reveal_whitespace_chars_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.reveal_whitespace_chars != next {
            self.set_diff_reveal_whitespace_chars(next, cx);
        }
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.sync_diff_reveal_whitespace_chars_from_pane(next, cx);
        });
    }

    pub(in crate::view) fn set_diff_word_wrap_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_word_wrap != next {
            self.set_diff_word_wrap(next, cx);
        }
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.sync_diff_word_wrap_from_pane(next, cx);
        });
    }

    pub(in crate::view) fn set_diff_show_line_numbers_and_persist(
        &mut self,
        next: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_show_line_numbers != next {
            self.set_diff_show_line_numbers(next, cx);
        }
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, cx| {
            root.sync_diff_show_line_numbers_from_pane(next, cx);
        });
    }

    pub(super) fn rendered_diff_target_for_state(state: &AppState) -> Option<DiffTarget> {
        let repo_id = state.active_repo?;
        let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
        repo.diff_state
            .inline_submodule_diff
            .as_ref()
            .map(|inline| inline.target.clone())
            .or_else(|| repo.diff_state.diff_target.clone())
    }
}
