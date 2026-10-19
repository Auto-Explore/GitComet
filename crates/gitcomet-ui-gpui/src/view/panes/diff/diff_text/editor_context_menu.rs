//! The right-click menu of the diff and file editor text.

use super::{
    DiffArea, DiffRenderer, DiffTarget, DiffTextRegion, DiffViewMode, FxHashMap, FxHashSet, Pixels,
    Point, PopoverKind, Window, build_unified_patch_for_hunks,
    build_unified_patch_for_selected_lines_across_hunks,
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
        if self.is_inline_submodule_diff_active() {
            return;
        }
        if self.structural_diff_active() {
            self.open_structural_line_menu(visible_ix, region, anchor, window, cx);
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

        let clicked_source_visible_ix = self
            .diff_source_visible_ix_for_visible_ix(clicked_visible_ix)
            .unwrap_or(clicked_visible_ix);
        let menu_view = if is_file_preview {
            DiffViewMode::Inline
        } else {
            self.diff_view
        };
        let text_selection = self
            .diff_text_selection_line_rows_at(menu_view, clicked_source_visible_ix, region)
            .and_then(|(a, b)| self.diff_text_visible_range_for_source_range(a, b));

        if list_len > 0 && text_selection.is_none() {
            let existing = self
                .diff_selection_range
                .map(|(a, b)| (a.min(b), a.max(b)))
                .filter(|(a, b)| clicked_visible_ix >= *a && clicked_visible_ix <= *b);
            if existing.is_none() {
                self.diff_selection_anchor = Some(clicked_visible_ix);
                self.diff_selection_range = Some((clicked_visible_ix, clicked_visible_ix));
            }
        }

        struct FileDiffSrcLookup {
            file_rel: std::path::PathBuf,
            add_by_new_line: FxHashMap<u32, usize>,
            remove_by_old_line: FxHashMap<u32, usize>,
            context_by_old_line: FxHashMap<u32, usize>,
        }

        let file_diff_lookup = if self.is_file_diff_view_active() {
            self.file_diff_cache_path.as_ref().map(|abs| {
                let rel = abs.strip_prefix(&workdir).unwrap_or(abs);
                let file_rel = rel.to_path_buf();
                // Git diffs use forward slashes even on Windows.
                let rel_str = file_rel.to_str().map(|text| text.replace('\\', "/"));

                let approx_map_len = match self.diff_view {
                    DiffViewMode::Inline => self.file_diff_inline_row_len(),
                    DiffViewMode::Split => self.file_diff_split_row_len(),
                };
                let mut add_by_new_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut remove_by_old_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut context_by_old_line: FxHashMap<u32, usize> =
                    FxHashMap::with_capacity_and_hasher(approx_map_len, Default::default());

                for ix in 0..self.patch_diff_row_len() {
                    let Some(line) = self.patch_diff_row(ix) else {
                        continue;
                    };
                    if self.diff_file_for_src_ix.get(ix).and_then(|p| p.as_deref())
                        != rel_str.as_deref()
                    {
                        continue;
                    }
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => {
                            if let Some(n) = line.new_line {
                                add_by_new_line.insert(n, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Remove => {
                            if let Some(o) = line.old_line {
                                remove_by_old_line.insert(o, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Context => {
                            if let Some(o) = line.old_line {
                                context_by_old_line.insert(o, ix);
                            }
                        }
                        gitcomet_core::domain::DiffLineKind::Header
                        | gitcomet_core::domain::DiffLineKind::Hunk => {}
                    }
                }

                FileDiffSrcLookup {
                    file_rel,
                    add_by_new_line,
                    remove_by_old_line,
                    context_by_old_line,
                }
            })
        } else {
            None
        };

        let src_ixs_for_visible_ix = |visible_ix: usize| -> Vec<usize> {
            if let Some(lookup) = file_diff_lookup.as_ref() {
                let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return Vec::new();
                };
                match self.diff_view {
                    DiffViewMode::Inline => {
                        let Some(line) = self.file_diff_inline_render_data(mapped_ix) else {
                            return Vec::new();
                        };
                        match line.kind {
                            gitcomet_core::domain::DiffLineKind::Add => line
                                .new_line
                                .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Remove => line
                                .old_line
                                .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Context => line
                                .old_line
                                .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::domain::DiffLineKind::Header
                            | gitcomet_core::domain::DiffLineKind::Hunk => Vec::new(),
                        }
                    }
                    DiffViewMode::Split => {
                        let Some(row) = self.file_diff_split_render_data(mapped_ix) else {
                            return Vec::new();
                        };
                        match row.kind {
                            gitcomet_core::file_diff::FileDiffRowKind::Context => row
                                .old_line
                                .and_then(|o| lookup.context_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Add => row
                                .new_line
                                .and_then(|n| lookup.add_by_new_line.get(&n).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Remove => row
                                .old_line
                                .and_then(|o| lookup.remove_by_old_line.get(&o).copied())
                                .into_iter()
                                .collect(),
                            gitcomet_core::file_diff::FileDiffRowKind::Modify => {
                                let mut out = Vec::with_capacity(2);
                                if let Some(o) = row.old_line
                                    && let Some(ix) = lookup.remove_by_old_line.get(&o).copied()
                                {
                                    out.push(ix);
                                }
                                if let Some(n) = row.new_line
                                    && let Some(ix) = lookup.add_by_new_line.get(&n).copied()
                                    && !out.contains(&ix)
                                {
                                    out.push(ix);
                                }
                                out
                            }
                        }
                    }
                }
            } else {
                self.diff_src_ixs_for_visible_ix(visible_ix)
            }
        };

        let clicked_src_ix = src_ixs_for_visible_ix(clicked_visible_ix)
            .into_iter()
            .next();
        let hunk_src_ix = clicked_src_ix.and_then(|src_ix| self.diff_enclosing_hunk_src_ix(src_ix));

        let path = hunk_src_ix
            .or(clicked_src_ix)
            .and_then(|ix| self.diff_file_for_src_ix.get(ix))
            .and_then(|p| p.as_deref())
            .map(std::path::PathBuf::from);
        let path = path
            .or_else(|| file_diff_lookup.as_ref().map(|l| l.file_rel.clone()))
            .or_else(|| {
                self.worktree_preview_path.as_ref().map(|abs| {
                    let rel = abs.strip_prefix(&workdir).unwrap_or(abs);
                    rel.to_path_buf()
                })
            });

        let allow_patch_actions =
            allow_apply && !is_file_preview && !self.has_large_file_text_diff();

        let selection = text_selection
            .or_else(|| self.diff_selection_range.map(|(a, b)| (a.min(b), a.max(b))))
            .or_else(|| (list_len > 0).then_some((clicked_visible_ix, clicked_visible_ix)))
            .map(|(a, b)| {
                if list_len == 0 {
                    (0, 0)
                } else {
                    (a.min(list_len - 1), b.min(list_len - 1))
                }
            });

        let (hunks_count, hunk_patch, lines_count, lines_patch, discard_lines_patch) =
            if allow_patch_actions && let Some((sel_a, sel_b)) = selection {
                let approx_selected = sel_b
                    .saturating_sub(sel_a)
                    .saturating_add(1)
                    .saturating_mul(2);
                let mut selected_src_ixs: FxHashSet<usize> =
                    FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());
                let mut selected_change_src_ixs: FxHashSet<usize> =
                    FxHashSet::with_capacity_and_hasher(approx_selected, Default::default());

                for vix in sel_a..=sel_b {
                    for src_ix in src_ixs_for_visible_ix(vix) {
                        let Some(line) = self.patch_diff_row(src_ix) else {
                            continue;
                        };
                        selected_src_ixs.insert(src_ix);
                        if matches!(
                            line.kind,
                            gitcomet_core::domain::DiffLineKind::Add
                                | gitcomet_core::domain::DiffLineKind::Remove
                        ) {
                            selected_change_src_ixs.insert(src_ix);
                        }
                    }
                }

                let mut selected_hunks: Vec<usize> = selected_src_ixs
                    .into_iter()
                    .filter_map(|ix| self.diff_enclosing_hunk_src_ix(ix))
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
                        &selected_change_src_ixs,
                    ),
                    DiffArea::Staged => {
                        build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                            &materialized_diff,
                            &selected_change_src_ixs,
                        )
                    }
                };
                let discard_lines_patch = if area == DiffArea::Unstaged {
                    build_unified_patch_for_selected_lines_across_hunks_for_reverse_apply(
                        &materialized_diff,
                        &selected_change_src_ixs,
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
}
