use super::*;

#[cfg(test)]
thread_local! {
    // Surface resolutions since the last take. Each one stats the previewed
    // path, so a frame should make a handful, not one per row.
    static SURFACE_RESOLUTIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Surface resolutions since the last call, which resets the count.
#[cfg(test)]
pub(in crate::view) fn take_file_preview_active_checks_for_tests() -> usize {
    SURFACE_RESOLUTIONS.with(|checks| checks.replace(0))
}

/// The body the diff panel draws, in the order the panel picks it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MainPaneBody {
    SubmoduleSummary,
    /// An untracked directory: a notice instead of content.
    DirectoryNotice,
    FileEditor,
    /// A whole-file preview: untracked, added, deleted, or opened content.
    FilePreview,
    /// The merge tool, or its compare view.
    Conflict,
    /// The file's diff: text, image, or rendered markdown.
    FileDiff,
    Patch,
}

/// What the main pane shows, resolved in one place so the body, the toolbar,
/// search, and the per-row helpers all agree on it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct MainPaneSurface {
    pub(in crate::view) body: MainPaneBody,
    pub(in crate::view) submodule_summary: bool,
    pub(in crate::view) inline_submodule_diff: bool,
    pub(in crate::view) directory_notice: bool,
    /// A whole-file preview applies to the target, whatever covers it.
    pub(in crate::view) file_preview_target: bool,
    /// ...and nothing covers it, though the editor may still win the body.
    pub(in crate::view) file_preview: bool,
    /// The editor is open on the target and nothing covers it.
    pub(in crate::view) file_editor: bool,
    pub(in crate::view) supports_diff_content_toggle: bool,
    pub(in crate::view) wants_file_diff: bool,
    pub(in crate::view) wants_collapsed_diff: bool,
    /// The Preview/Source toggle the toolbar offers, if any.
    pub(in crate::view) toggle_kind: Option<RenderedPreviewKind>,
    /// The rendered markdown preview — of the file or of its diff — is the body.
    pub(in crate::view) markdown_preview: bool,
}

/// A resolved surface and what it was resolved from. Everything else it reads
/// is in the app state, and the disk is re-read once a frame.
pub(in crate::view) struct MainPaneSurfaceMemo {
    frame: u64,
    state: Arc<AppState>,
    modes: RenderedPreviewModes,
    content_mode: DiffContentMode,
    surface: MainPaneSurface,
}

impl MainPaneView {
    /// What the pane shows. Asked from per-row paint, so it is resolved once a
    /// frame, or again when the state or a mode it depends on changes.
    pub(in crate::view) fn main_pane_surface(&self) -> MainPaneSurface {
        {
            let memo = self.main_pane_surface_memo.borrow();
            if let Some(memo) = memo.as_ref()
                && memo.frame == self.main_pane_surface_frame
                && Arc::ptr_eq(&memo.state, &self.state)
                && memo.modes == self.rendered_preview_modes
                && memo.content_mode == self.diff_content_mode
            {
                return memo.surface;
            }
        }
        let surface = self.resolve_main_pane_surface();
        *self.main_pane_surface_memo.borrow_mut() = Some(MainPaneSurfaceMemo {
            frame: self.main_pane_surface_frame,
            state: Arc::clone(&self.state),
            modes: self.rendered_preview_modes,
            content_mode: self.diff_content_mode,
            surface,
        });
        surface
    }

    fn resolve_main_pane_surface(&self) -> MainPaneSurface {
        #[cfg(test)]
        SURFACE_RESOLUTIONS.with(|checks| checks.set(checks.get() + 1));
        let file_preview_target = self.file_preview_target();
        let inline_submodule_diff = self.is_inline_submodule_diff_active();
        let submodule_summary = self
            .active_repo()
            .is_some_and(|repo| !matches!(repo.diff_state.submodule_summary, Loadable::NotLoaded));
        let directory_notice = !submodule_summary
            && !inline_submodule_diff
            && self.untracked_directory_notice().is_some();
        // What a notice or a submodule covers is not on screen.
        let uncovered = !directory_notice && !submodule_summary && !inline_submodule_diff;
        let file_preview = file_preview_target && uncovered;
        let supports_diff_content_toggle = (inline_submodule_diff || !submodule_summary)
            && self.supports_diff_content_mode_toggle(file_preview);
        let wants_file_diff =
            supports_diff_content_toggle && self.wants_file_diff_view(file_preview);
        let wants_collapsed_diff =
            supports_diff_content_toggle && self.wants_collapsed_diff_view(file_preview);
        let rendered_preview_kind =
            crate::view::diff_target_rendered_preview_kind(self.rendered_diff_target());
        let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
            wants_file_diff,
            wants_collapsed_diff,
            file_preview,
            rendered_preview_kind,
        );

        let file_editor = self.is_file_editor_active() && uncovered;
        let body = if submodule_summary && !inline_submodule_diff {
            MainPaneBody::SubmoduleSummary
        } else if directory_notice {
            MainPaneBody::DirectoryNotice
        } else if file_editor {
            MainPaneBody::FileEditor
        } else if file_preview {
            MainPaneBody::FilePreview
        } else if !inline_submodule_diff && self.conflicted_worktree_target().is_some() {
            MainPaneBody::Conflict
        } else if wants_file_diff || wants_collapsed_diff {
            MainPaneBody::FileDiff
        } else {
            MainPaneBody::Patch
        };
        let markdown_rendered = self
            .rendered_preview_modes
            .get(RenderedPreviewKind::Markdown)
            == RenderedPreviewMode::Rendered;
        let markdown_preview = markdown_rendered
            && match body {
                MainPaneBody::FilePreview => toggle_kind == Some(RenderedPreviewKind::Markdown),
                // As the file diff draws it: the Full diff only.
                MainPaneBody::FileDiff => {
                    self.diff_content_mode == DiffContentMode::Full
                        && rendered_preview_kind == Some(RenderedPreviewKind::Markdown)
                }
                _ => false,
            };

        MainPaneSurface {
            body,
            submodule_summary,
            inline_submodule_diff,
            directory_notice,
            file_preview_target,
            file_preview,
            file_editor,
            supports_diff_content_toggle,
            wants_file_diff,
            wants_collapsed_diff,
            toggle_kind,
            markdown_preview,
        }
    }

    /// Whether a whole-file preview applies to the target: an untracked,
    /// added, or deleted file, or content opened from the file browser.
    fn file_preview_target(&self) -> bool {
        let preview_text_file_available = self.active_repo().is_some_and(|repo| {
            matches!(
                repo.diff_state.diff_preview_text_file,
                Loadable::Loading | Loadable::Error(_) | Loadable::Ready(Some(_))
            )
        });
        let has_untracked_preview = self.untracked_worktree_preview_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p) && p.is_file()
        });
        let has_added_preview = self.added_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p)
                && !p.is_dir()
                && (p.is_file() || preview_text_file_available)
        });
        let has_deleted_preview = self.deleted_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p)
                && !p.is_dir()
                && preview_text_file_available
        });
        // File-browser "open content" forces a full-content preview for any file.
        let has_content_preview = self.content_preview_abs_path().is_some_and(|p| {
            !self.content_preview_is_picture(&p)
                && !p.is_dir()
                && (p.is_file() || preview_text_file_available)
        });
        has_untracked_preview || has_added_preview || has_deleted_preview || has_content_preview
    }

    /// The unstaged working-tree target, when it is a conflicted file.
    pub(in crate::view) fn conflicted_worktree_target(
        &self,
    ) -> Option<(
        std::path::PathBuf,
        Option<gitcomet_core::domain::FileConflictKind>,
    )> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let conflict = repo
            .status_entry_for_path(DiffArea::Unstaged, path.as_path())
            .filter(|entry| entry.kind == FileStatusKind::Conflicted)?;
        Some((path.clone(), conflict.conflict))
    }
}
