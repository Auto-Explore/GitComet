use super::*;

impl MainPaneView {
    fn diff_text_normalized_selection(&self) -> Option<(DiffTextPos, DiffTextPos)> {
        let a = self.diff_text_anchor?;
        let b = self.diff_text_head?;
        Some(if a.cmp_key() <= b.cmp_key() {
            (a, b)
        } else {
            (b, a)
        })
    }

    pub(in super::super::super) fn diff_text_selection_color(&self) -> gpui::Rgba {
        with_alpha(
            self.theme.colors.accent,
            if self.theme.is_dark { 0.28 } else { 0.18 },
        )
    }

    pub(in super::super::super) fn set_diff_text_hitbox(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        hitbox: DiffTextHitbox,
    ) {
        self.diff_text_hitboxes.insert((visible_ix, region), hitbox);
    }

    fn diff_text_pos_from_hitbox(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) -> Option<DiffTextPos> {
        let hitbox = self.diff_text_hitboxes.get(&(visible_ix, region))?;
        let layout = &self.diff_text_layout_cache.get(&hitbox.layout_key)?.layout;
        let local = hitbox.bounds.localize(&position)?;
        let x = local.x.max(px(0.0));
        let offset = layout
            .closest_index_for_x(x)
            .min(layout.len())
            .min(hitbox.text_len);
        Some(DiffTextPos {
            visible_ix,
            region,
            offset,
        })
    }

    fn diff_text_pos_for_mouse(&self, position: Point<Pixels>) -> Option<DiffTextPos> {
        if self.diff_text_hitboxes.is_empty() {
            return None;
        }

        let restrict_region = self
            .diff_text_selecting
            .then_some(self.diff_text_anchor)
            .flatten()
            .map(|p| p.region)
            .filter(|r| matches!(r, DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight));

        for ((visible_ix, region), hitbox) in &self.diff_text_hitboxes {
            if restrict_region.is_some_and(|restrict| restrict != *region) {
                continue;
            }
            if hitbox.bounds.contains(&position) {
                return self.diff_text_pos_from_hitbox(*visible_ix, *region, position);
            }
        }

        let mut best: Option<((usize, DiffTextRegion), Pixels)> = None;
        for (key, hitbox) in &self.diff_text_hitboxes {
            if restrict_region.is_some_and(|restrict| restrict != key.1) {
                continue;
            }
            let dy = if position.y < hitbox.bounds.top() {
                hitbox.bounds.top() - position.y
            } else if position.y > hitbox.bounds.bottom() {
                position.y - hitbox.bounds.bottom()
            } else {
                px(0.0)
            };
            let dx = if position.x < hitbox.bounds.left() {
                hitbox.bounds.left() - position.x
            } else if position.x > hitbox.bounds.right() {
                position.x - hitbox.bounds.right()
            } else {
                px(0.0)
            };
            let score = dy + dx;
            if best.is_none() || score < best.unwrap().1 {
                best = Some((*key, score));
            }
        }
        let ((visible_ix, region), _) = best?;
        self.diff_text_pos_from_hitbox(visible_ix, region, position)
    }

    pub(in super::super::super) fn begin_diff_text_selection(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
    ) {
        let Some(pos) = self.diff_text_pos_from_hitbox(visible_ix, region, position) else {
            return;
        };
        self.diff_text_selecting = true;
        self.diff_text_anchor = Some(pos);
        self.diff_text_head = Some(pos);
        self.diff_text_last_mouse_pos = position;
        self.diff_suppress_clicks_remaining = 0;
    }

    pub(in super::super::super) fn begin_diff_text_scroll_tracking(
        &mut self,
        position: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.diff_text_selecting {
            return;
        }

        self.diff_text_last_mouse_pos = position;
        self.diff_text_autoscroll_target =
            Some(self.diff_text_autoscroll_target_for_position(position));
        self.diff_text_autoscroll_seq = self.diff_text_autoscroll_seq.wrapping_add(1);

        let autoscroll_seq = self.diff_text_autoscroll_seq;
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| loop {
                Timer::after(Duration::from_millis(16)).await;
                let mut keep_going = false;
                let _ = view.update(cx, |this, cx| {
                    if !this.diff_text_selecting {
                        return;
                    }
                    if this.diff_text_autoscroll_seq != autoscroll_seq {
                        return;
                    }

                    keep_going = true;
                    let changed = this.tick_diff_text_selection_autoscroll(cx);
                    if changed {
                        cx.notify();
                    }
                });

                if !keep_going {
                    break;
                }
            },
        )
        .detach();
    }

    pub(in super::super::super) fn update_diff_text_selection_from_mouse(
        &mut self,
        position: Point<Pixels>,
    ) {
        if !self.diff_text_selecting {
            return;
        }
        self.diff_text_last_mouse_pos = position;
        let Some(pos) = self.diff_text_pos_for_mouse(position) else {
            return;
        };
        if self.diff_text_head != Some(pos) {
            self.diff_text_head = Some(pos);
            if self
                .diff_text_normalized_selection()
                .is_some_and(|(a, b)| a != b)
            {
                self.diff_suppress_clicks_remaining = 1;
            }
        }
    }

    pub(in super::super::super) fn end_diff_text_selection(&mut self) {
        self.diff_text_selecting = false;
        self.diff_text_autoscroll_target = None;
    }

    pub(in super::super::super) fn diff_text_has_selection(&self) -> bool {
        self.diff_text_normalized_selection()
            .is_some_and(|(a, b)| a != b)
    }

    pub(in super::super::super) fn diff_text_local_selection_range(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
        text_len: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end {
            return None;
        }
        if visible_ix < start.visible_ix || visible_ix > end.visible_ix {
            return None;
        }

        let split_region = (self.diff_view == DiffViewMode::Split
            && start.region == end.region
            && matches!(
                start.region,
                DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
            ))
        .then_some(start.region);
        if split_region.is_some_and(|r| r != region) {
            return None;
        }

        let region_order = region.order();
        let start_order = start.region.order();
        let end_order = end.region.order();

        let mut a = 0usize;
        let mut b = text_len;

        if start.visible_ix == end.visible_ix && visible_ix == start.visible_ix {
            if region_order < start_order || region_order > end_order {
                return None;
            }
            if region == start.region {
                a = start.offset.min(text_len);
            }
            if region == end.region {
                b = end.offset.min(text_len);
            }
        } else if visible_ix == start.visible_ix {
            if region_order < start_order {
                return None;
            }
            if region == start.region {
                a = start.offset.min(text_len);
            }
        } else if visible_ix == end.visible_ix {
            if region_order > end_order {
                return None;
            }
            if region == end.region {
                b = end.offset.min(text_len);
            }
        }

        if a >= b {
            return None;
        }
        Some(a..b)
    }

    pub(in super::super::super) fn diff_text_line_for_region(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        let fallback = SharedString::default();
        let expand_tabs = |s: &str| -> SharedString {
            if !s.contains('\t') {
                return s.to_string().into();
            }
            let mut out = String::with_capacity(s.len());
            for ch in s.chars() {
                match ch {
                    '\t' => out.push_str("    "),
                    _ => out.push(ch),
                }
            }
            out.into()
        };

        if self.is_file_preview_active() {
            if region != DiffTextRegion::Inline {
                return fallback;
            }
            return self
                .worktree_preview_line_text(visible_ix)
                .map(expand_tabs)
                .unwrap_or(fallback);
        }

        let Some(mapped_ix) = self.diff_mapped_ix_for_visible_ix(visible_ix) else {
            return fallback;
        };

        if self.diff_view == DiffViewMode::Inline {
            if region != DiffTextRegion::Inline {
                return fallback;
            }
            if self.is_file_diff_view_active() {
                let Some(line) = self.file_diff_inline_row(mapped_ix) else {
                    return fallback;
                };
                let cache_epoch = self.file_diff_inline_style_cache_epoch(&line);
                if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, cache_epoch) {
                    return styled.text.clone();
                }
                return expand_tabs(diff_content_text(&line));
            }

            if let Some(styled) = self.diff_text_segments_cache_get(mapped_ix, 0) {
                return styled.text.clone();
            }
            let Some(line) = self.patch_diff_row(mapped_ix) else {
                return fallback;
            };
            let click_kind = self
                .diff_click_kinds
                .get(mapped_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if matches!(
                click_kind,
                DiffClickKind::HunkHeader | DiffClickKind::FileHeader
            ) && let Some(display) = self.diff_header_display_cache.get(&mapped_ix)
            {
                return display.clone();
            }
            return expand_tabs(line.text.as_ref());
        }

        match region {
            DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => {}
            DiffTextRegion::Inline => return fallback,
        }

        if self.is_file_diff_view_active() {
            let cache_epoch = self.file_diff_split_style_cache_epoch(region);
            if let Some(key) = self.file_diff_split_cache_key(mapped_ix, region)
                && let Some(styled) = self.diff_text_segments_cache_get(key, cache_epoch)
            {
                return styled.text.clone();
            }
            let Some(row) = self.file_diff_split_row(mapped_ix) else {
                return fallback;
            };
            let text = match region {
                DiffTextRegion::SplitLeft => row.old.as_deref().unwrap_or(""),
                DiffTextRegion::SplitRight => row.new.as_deref().unwrap_or(""),
                DiffTextRegion::Inline => unreachable!(),
            };
            return expand_tabs(text);
        }

        let Some(split_row) = self.patch_diff_split_row(mapped_ix) else {
            return fallback;
        };
        match split_row {
            PatchSplitRow::Raw { src_ix, click_kind } => {
                let Some(line) = self.patch_diff_row(src_ix) else {
                    return fallback;
                };
                if matches!(
                    click_kind,
                    DiffClickKind::HunkHeader | DiffClickKind::FileHeader
                ) && let Some(display) = self.diff_header_display_cache.get(&src_ix)
                {
                    return display.clone();
                }
                expand_tabs(line.text.as_ref())
            }
            PatchSplitRow::Aligned { row, .. } => {
                let text = match region {
                    DiffTextRegion::SplitLeft => row.old.as_deref().unwrap_or(""),
                    DiffTextRegion::SplitRight => row.new.as_deref().unwrap_or(""),
                    DiffTextRegion::Inline => unreachable!(),
                };
                expand_tabs(text)
            }
        }
    }

    fn diff_text_combined_offset(&self, pos: DiffTextPos, left_len: usize) -> usize {
        match self.diff_view {
            DiffViewMode::Inline => pos.offset,
            DiffViewMode::Split => match pos.region {
                DiffTextRegion::SplitLeft => pos.offset,
                DiffTextRegion::SplitRight => left_len.saturating_add(1).saturating_add(pos.offset),
                DiffTextRegion::Inline => pos.offset,
            },
        }
    }

    fn selected_diff_text_string(&self) -> Option<String> {
        let (start, end) = self.diff_text_normalized_selection()?;
        if start == end {
            return None;
        }

        let force_inline = self.is_file_preview_active();

        let mut out = String::new();
        for visible_ix in start.visible_ix..=end.visible_ix {
            if force_inline || self.diff_view == DiffViewMode::Inline {
                let text = self.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                let line_len = text.len();
                let a = if visible_ix == start.visible_ix {
                    start.offset.min(line_len)
                } else {
                    0
                };
                let b = if visible_ix == end.visible_ix {
                    end.offset.min(line_len)
                } else {
                    line_len
                };
                if !out.is_empty() {
                    out.push('\n');
                }
                if a < b {
                    out.push_str(&text[a..b]);
                }
                continue;
            }

            let split_region = (start.region == end.region
                && matches!(
                    start.region,
                    DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight
                ))
            .then_some(start.region);

            if let Some(region) = split_region {
                let text = self.diff_text_line_for_region(visible_ix, region);
                let line_len = text.len();
                let a = if visible_ix == start.visible_ix {
                    start.offset.min(line_len)
                } else {
                    0
                };
                let b = if visible_ix == end.visible_ix {
                    end.offset.min(line_len)
                } else {
                    line_len
                };
                if !out.is_empty() {
                    out.push('\n');
                }
                if a < b {
                    out.push_str(&text[a..b]);
                }
            } else {
                let left = self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                let right = self.diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                let combined = format!("{}\t{}", left.as_ref(), right.as_ref());
                let left_len = left.len();
                let combined_len = combined.len();

                let a = if visible_ix == start.visible_ix {
                    self.diff_text_combined_offset(start, left_len)
                        .min(combined_len)
                } else {
                    0
                };
                let b = if visible_ix == end.visible_ix {
                    self.diff_text_combined_offset(end, left_len)
                        .min(combined_len)
                } else {
                    combined_len
                };

                if !out.is_empty() {
                    out.push('\n');
                }
                if a < b {
                    out.push_str(&combined[a..b]);
                }
            }
        }

        if out.is_empty() { None } else { Some(out) }
    }

    pub(in super::super::super) fn copy_selected_diff_text_to_clipboard(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(text) = self.selected_diff_text_string() else {
            return;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
    }

    pub(in super::super::super) fn open_diff_editor_context_menu(
        &mut self,
        visible_ix: usize,
        region: DiffTextRegion,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let repo_id = repo.id;
        let workdir = repo.spec.workdir.clone();

        let (area, allow_apply) = match repo.diff_state.diff_target.as_ref() {
            Some(DiffTarget::WorkingTree { area, .. }) => (*area, true),
            _ => (DiffArea::Unstaged, false),
        };
        let is_file_preview = self.is_file_preview_active();

        let copy_text = self.selected_diff_text_string().or_else(|| {
            let text = self.diff_text_line_for_region(visible_ix, region);
            (!text.is_empty()).then_some(text.to_string())
        });

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

        let text_selection = context_menu_selection_range_from_diff_text(
            self.diff_text_normalized_selection(),
            if is_file_preview {
                DiffViewMode::Inline
            } else {
                self.diff_view
            },
            clicked_visible_ix,
            region,
        );

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
            add_by_new_line: HashMap<u32, usize>,
            remove_by_old_line: HashMap<u32, usize>,
            context_by_old_line: HashMap<u32, usize>,
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
                let mut add_by_new_line: HashMap<u32, usize> =
                    HashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut remove_by_old_line: HashMap<u32, usize> =
                    HashMap::with_capacity_and_hasher(approx_map_len, Default::default());
                let mut context_by_old_line: HashMap<u32, usize> =
                    HashMap::with_capacity_and_hasher(approx_map_len, Default::default());

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
                        let Some(line) = self.file_diff_inline_row(mapped_ix) else {
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
                        let Some(row) = self.file_diff_split_row(mapped_ix) else {
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

        let allow_patch_actions = allow_apply && !is_file_preview;

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
                let mut selected_src_ixs: HashSet<usize> =
                    HashSet::with_capacity_and_hasher(approx_selected, Default::default());
                let mut selected_change_src_ixs: HashSet<usize> =
                    HashSet::with_capacity_and_hasher(approx_selected, Default::default());

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

                let lines_patch = build_unified_patch_for_selected_lines_across_hunks(
                    &materialized_diff,
                    &selected_change_src_ixs,
                );
                let discard_lines_patch = if area == DiffArea::Unstaged {
                    build_unified_patch_for_selected_lines_across_hunks_for_worktree_discard(
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
                    hunk_patch,
                    lines_count,
                    lines_patch,
                    discard_lines_patch,
                )
            } else {
                (0, None, 0, None, None)
            };

        self.activate_context_menu_invoker("diff_editor_menu".into(), cx);
        self.open_popover_at(
            PopoverKind::DiffEditorMenu {
                repo_id,
                area,
                path,
                hunk_patch,
                hunks_count,
                lines_patch,
                discard_lines_patch,
                lines_count,
                copy_text,
            },
            anchor,
            window,
            cx,
        );
    }
}

impl MainPaneView {
    fn tick_diff_text_selection_autoscroll(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        if let Ok(pos) = self.root_view.update(cx, |root, _cx| root.last_mouse_pos) {
            self.diff_text_last_mouse_pos = pos;
        }

        let Some(target) = self.diff_text_autoscroll_target else {
            // Still update selection periodically so it can expand while the user scrolls.
            let before = self.diff_text_head;
            self.update_diff_text_selection_from_mouse(self.diff_text_last_mouse_pos);
            return self.diff_text_head != before;
        };

        let handle = self.scroll_handle_for_diff_text_autoscroll_target(target);
        let bounds = handle.bounds();
        if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
            return false;
        }

        let max_offset = handle.max_offset();
        let old_offset = handle.offset();
        let mouse = self.diff_text_last_mouse_pos;

        let delta_x = autoscroll_delta_for_axis(mouse.x, bounds.left(), bounds.right());
        let delta_y = autoscroll_delta_for_axis(mouse.y, bounds.top(), bounds.bottom());

        let new_x = (old_offset.x + delta_x).clamp(-max_offset.width, px(0.0));
        let new_y = (old_offset.y + delta_y).clamp(-max_offset.height, px(0.0));

        let scrolled = new_x != old_offset.x || new_y != old_offset.y;
        if scrolled {
            handle.set_offset(point(new_x, new_y));
        }

        let before_head = self.diff_text_head;
        self.update_diff_text_selection_from_mouse(mouse);
        let selection_changed = self.diff_text_head != before_head;

        scrolled || selection_changed
    }

    fn diff_text_autoscroll_target_for_position(
        &self,
        position: Point<Pixels>,
    ) -> DiffTextAutoscrollTarget {
        if self.is_file_preview_active() {
            return DiffTextAutoscrollTarget::WorktreePreview;
        }

        if self.is_conflict_resolver_active() {
            return DiffTextAutoscrollTarget::ConflictResolvedPreview;
        }

        if self.diff_view == DiffViewMode::Split {
            let right_bounds = self.diff_split_right_scroll.0.borrow().base_handle.bounds();
            if right_bounds.contains(&position) {
                return DiffTextAutoscrollTarget::DiffSplitRight;
            }
        }

        DiffTextAutoscrollTarget::DiffLeftOrInline
    }

    fn scroll_handle_for_diff_text_autoscroll_target(
        &self,
        target: DiffTextAutoscrollTarget,
    ) -> ScrollHandle {
        match target {
            DiffTextAutoscrollTarget::DiffLeftOrInline => {
                self.diff_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::DiffSplitRight => {
                self.diff_split_right_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::WorktreePreview => {
                self.worktree_preview_scroll.0.borrow().base_handle.clone()
            }
            DiffTextAutoscrollTarget::ConflictResolvedPreview => self
                .conflict_resolved_preview_scroll
                .0
                .borrow()
                .base_handle
                .clone(),
        }
    }
}

fn autoscroll_delta_for_axis(cursor: Pixels, min: Pixels, max: Pixels) -> Pixels {
    fn speed(distance: Pixels) -> Pixels {
        // 2–48px per tick, scaling with how far outside the container the cursor is.
        let min_step = px(2.0);
        let max_step = px(48.0);
        (distance * 0.4).max(min_step).min(max_step)
    }

    if cursor < min {
        speed(min - cursor)
    } else if cursor > max {
        -speed(cursor - max)
    } else {
        px(0.0)
    }
}
