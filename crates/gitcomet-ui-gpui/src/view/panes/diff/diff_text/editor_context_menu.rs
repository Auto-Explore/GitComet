//! The right-click menu of the diff and file editor text.

use super::{
    DiffArea, DiffRenderer, DiffTarget, DiffTextRegion, DiffViewMode, Pixels, Point, PopoverKind,
    Window, build_unified_patch_for_hunks, build_unified_patch_for_selected_lines_across_hunks,
    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply, rows,
};

impl DiffRenderer {
    pub(in super::super::super::super) fn open_diff_editor_context_menu(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        // Before every guard, like `TextInput::on_mouse_down_right`: this is a
        // press on the surface that owns the selection, so it keeps it whether
        // or not a menu ends up opening. A right-click inside the highlight
        // must still reach Copy. Gated on there being one, so a pane painting
        // nothing never holds the window's selection.
        if self.diff_text_has_selection() {
            self.diff_text_selection_owner.adopt(window, cx);
        }
        // As a left press does, so the menu hands focus back here and S, U
        // and D then reach the lines it acted on.
        window.focus(&self.diff_panel_focus_handle, cx);
        if self.is_inline_submodule_diff_active() {
            return;
        }
        if self.structural_diff_active() {
            let len = self.diff_visible_len();
            let clicked_visible_ix = visible_ix.min(len.saturating_sub(1));
            self.select_diff_editor_menu_row(self.diff_view, clicked_visible_ix, region, len);
            self.open_structural_line_menu(clicked_visible_ix, region, anchor, window, cx);
            return;
        }
        let Some(workdir) = self
            .rendered_diff_workdir()
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let repo_id = self.active_repo_id();

        let (area, allow_apply) = match self.rendered_diff_target() {
            Some(DiffTarget::WorkingTree { area, .. }) => {
                (*area, repo_id.is_some() && self.store.policy.allow_stage)
            }
            _ => (DiffArea::Unstaged, false),
        };
        let is_file_preview = self.is_file_preview_active();

        let selected_copy_text = self.selected_diff_text_string();
        let copy_target = if selected_copy_text.is_none()
            && self.is_file_preview_active()
            && region == DiffTextRegion::Inline
            && self
                .worktree_preview_line_raw_text(visible_ix)
                .is_some_and(|line| rows::is_streamable_diff_text(&line))
        {
            Some((visible_ix, region))
        } else {
            None
        };
        let copy_text = if copy_target.is_some() {
            selected_copy_text
        } else {
            selected_copy_text.or_else(|| self.diff_text_string_for_region(visible_ix, region))
        };

        let list_len = if is_file_preview {
            self.worktree_preview_line_count().unwrap_or(0)
        } else {
            self.diff_visible_len()
        };
        let clicked_visible_ix = if list_len == 0 {
            visible_ix
        } else {
            visible_ix.min(list_len - 1)
        };

        let menu_view = if is_file_preview {
            DiffViewMode::Inline
        } else {
            self.diff_view
        };
        self.select_diff_editor_menu_row(menu_view, clicked_visible_ix, region, list_len);

        let sources = self.diff_line_sources(&workdir);
        let clicked_src_ix = self
            .diff_line_src_ixs(&sources, clicked_visible_ix)
            .into_iter()
            .next();
        let hunk_src_ix = clicked_src_ix.and_then(|src_ix| self.diff_enclosing_hunk_src_ix(src_ix));

        let path = hunk_src_ix
            .or(clicked_src_ix)
            .and_then(|ix| self.diff_file_for_src_ix.get(ix))
            .and_then(|p| p.as_deref())
            .map(std::path::PathBuf::from);
        let path = path
            .or_else(|| sources.file_rel().map(std::path::Path::to_path_buf))
            .or_else(|| {
                self.worktree_preview_path.as_ref().map(|abs| {
                    let rel = abs.strip_prefix(&workdir).unwrap_or(abs);
                    rel.to_path_buf()
                })
            });

        let allow_patch_actions =
            allow_apply && !is_file_preview && !self.has_large_file_text_diff();

        self.remember_diff_editor_menu_click(clicked_visible_ix, region);
        let selection = self.diff_editor_menu_rows(menu_view, clicked_visible_ix, region, list_len);

        let (hunks_count, hunk_patch, lines_count, lines_patch, discard_lines_patch) =
            if allow_patch_actions && let Some((sel_a, sel_b)) = selection {
                let selected = self.selected_diff_lines(&sources, (sel_a, sel_b));
                let selected_change_src_ixs = &selected.change_src_ixs;

                let mut selected_hunks: Vec<usize> = selected
                    .src_ixs
                    .iter()
                    .filter_map(|&ix| self.diff_enclosing_hunk_src_ix(ix))
                    .collect();
                selected_hunks.sort_unstable();
                selected_hunks.dedup();

                let materialized_diff = self.patch_diff_rows_slice(0, self.patch_diff_row_len());
                let hunk_patch = build_unified_patch_for_hunks(&materialized_diff, &selected_hunks);
                let hunks_count = hunk_patch
                    .as_ref()
                    .map(|_| selected_hunks.len())
                    .unwrap_or(0);

                // "Stage line(s)" applies forward to the index; "Unstage
                // line(s)" applies the same selection in reverse, which needs
                // the opposite treatment of the unselected changes around it or
                // git rejects the patch.
                let lines_patch = match area {
                    DiffArea::Unstaged => build_unified_patch_for_selected_lines_across_hunks(
                        &materialized_diff,
                        selected_change_src_ixs,
                    ),
                    DiffArea::Staged => {
                        build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                            &materialized_diff,
                            selected_change_src_ixs,
                        )
                    }
                };
                let discard_lines_patch = if area == DiffArea::Unstaged {
                    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                        &materialized_diff,
                        selected_change_src_ixs,
                    )
                } else {
                    None
                };
                let lines_count = lines_patch
                    .as_ref()
                    .map(|_| selected_change_src_ixs.len())
                    .unwrap_or(0);

                (
                    hunks_count,
                    hunk_patch.map(Into::into),
                    lines_count,
                    lines_patch.map(Into::into),
                    discard_lines_patch.map(Into::into),
                )
            } else {
                (0, None, 0, None, None)
            };

        self.open_popover_at(
            (PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
                copy_target,
            })
            .invoked_by("diff_editor_menu".into()),
            anchor,
            window,
            cx,
        );
    }

    /// The visible rows of the text selection when a right-click on `region`
    /// of `clicked_visible_ix` lands inside it.
    fn diff_editor_menu_text_rows(
        &self,
        view: DiffViewMode,
        clicked_visible_ix: usize,
        region: DiffTextRegion,
    ) -> Option<(usize, usize)> {
        let clicked_source_visible_ix = self
            .diff_source_visible_ix_for_visible_ix(clicked_visible_ix)
            .unwrap_or(clicked_visible_ix);
        self.diff_text_selection_line_rows_at(view, clicked_source_visible_ix, region)
            .and_then(|(a, b)| self.diff_text_visible_range_for_source_range(a, b))
    }

    /// Selects the right-clicked row, dropping the outline, unless the text
    /// selection or the selected rows already take it: the menu acts on what
    /// the diff shows selected.
    fn select_diff_editor_menu_row(
        &mut self,
        view: DiffViewMode,
        clicked_visible_ix: usize,
        region: DiffTextRegion,
        list_len: usize,
    ) {
        if list_len == 0
            || self
                .diff_editor_menu_text_rows(view, clicked_visible_ix, region)
                .is_some()
            || self
                .diff_selection_range
                .is_some_and(|(a, b)| (a.min(b)..=a.max(b)).contains(&clicked_visible_ix))
        {
            return;
        }
        self.diff_selection_anchor = Some(clicked_visible_ix);
        self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
        self.diff_focused_change_block = None;
    }

    /// The visible rows the diff editor menu opened by that right-click acts
    /// on: the text selection it landed in, else the selected rows, else the
    /// clicked row, within the `list_len` rows listed.
    pub(super) fn diff_editor_menu_rows(
        &self,
        view: DiffViewMode,
        clicked_visible_ix: usize,
        region: DiffTextRegion,
        list_len: usize,
    ) -> Option<(usize, usize)> {
        self.diff_editor_menu_text_rows(view, clicked_visible_ix, region)
            .or_else(|| self.diff_selection_range.map(|(a, b)| (a.min(b), a.max(b))))
            .or_else(|| (list_len > 0).then_some((clicked_visible_ix, clicked_visible_ix)))
            .map(|(a, b)| {
                if list_len == 0 {
                    (0, 0)
                } else {
                    (a.min(list_len - 1), b.min(list_len - 1))
                }
            })
    }
}
