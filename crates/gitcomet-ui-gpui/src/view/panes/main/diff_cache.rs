use super::*;

fn decode_file_image_diff_bytes(
    format: gpui::ImageFormat,
    bytes: &[u8],
    svg_temp_path: Option<&mut Option<std::path::PathBuf>>,
) -> Option<Arc<gpui::Image>> {
    match format {
        gpui::ImageFormat::Svg => {
            if let Some(path) = svg_temp_path {
                *path = Some(svg_diff_cache_path(bytes)?);
            }
            None
        }
        _ => Some(Arc::new(gpui::Image::from_bytes(format, bytes.to_vec()))),
    }
}

fn svg_diff_cache_path(bytes: &[u8]) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let key = hasher.finish();

    let dir = std::env::temp_dir().join("gitcomet").join("svg_diff_cache");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{key:016x}.svg"));
    if !path.exists() {
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path)
}

impl MainPaneView {
    pub(in super::super::super) fn ensure_file_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        struct Rebuild {
            file_path: Option<std::path::PathBuf>,
            language: Option<rows::DiffSyntaxLanguage>,
            rows: Vec<FileDiffRow>,
            inline_rows: Vec<AnnotatedDiffLine>,
            inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
            split_word_highlights_old: Vec<Option<Vec<Range<usize>>>>,
            split_word_highlights_new: Vec<Option<Vec<Range<usize>>>>,
        }

        let clear_cache = |this: &mut Self| {
            this.file_diff_cache_repo_id = None;
            this.file_diff_cache_target = None;
            this.file_diff_cache_rev = 0;
            this.file_diff_cache_inflight = None;
            this.file_diff_cache_path = None;
            this.file_diff_cache_language = None;
            this.file_diff_cache_rows.clear();
            this.file_diff_inline_cache.clear();
            this.file_diff_inline_word_highlights.clear();
            this.file_diff_split_word_highlights_old.clear();
            this.file_diff_split_word_highlights_new.clear();
        };

        let Some((repo_id, diff_file_rev, diff_target, workdir, file)) = (|| {
            let repo = self.active_repo()?;
            if !Self::is_file_diff_target(repo.diff_target.as_ref()) {
                return None;
            }

            let file = match &repo.diff_file {
                Loadable::Ready(Some(file)) => Some(Arc::clone(file)),
                _ => None,
            };

            Some((
                repo.id,
                repo.diff_file_rev,
                repo.diff_target.clone(),
                repo.spec.workdir.clone(),
                file,
            ))
        })() else {
            clear_cache(self);
            return;
        };

        let diff_target_for_task = diff_target.clone();

        if self.file_diff_cache_repo_id == Some(repo_id)
            && self.file_diff_cache_rev == diff_file_rev
            && self.file_diff_cache_target == diff_target
        {
            return;
        }

        self.file_diff_cache_repo_id = Some(repo_id);
        self.file_diff_cache_rev = diff_file_rev;
        self.file_diff_cache_target = diff_target;
        self.file_diff_cache_inflight = None;
        self.file_diff_cache_path = None;
        self.file_diff_cache_language = None;
        self.file_diff_cache_rows.clear();
        self.file_diff_inline_cache.clear();
        self.file_diff_inline_word_highlights.clear();
        self.file_diff_split_word_highlights_old.clear();
        self.file_diff_split_word_highlights_new.clear();

        // Reset the segment cache to avoid mixing patch/file indices.
        self.diff_text_segments_cache.clear();

        let Some(file) = file else {
            return;
        };

        self.file_diff_cache_seq = self.file_diff_cache_seq.wrapping_add(1);
        let seq = self.file_diff_cache_seq;
        self.file_diff_cache_inflight = Some(seq);

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let rebuild = smol::unblock(move || {
                    let old_text = file.old.as_deref().unwrap_or("");
                    let new_text = file.new.as_deref().unwrap_or("");
                    let rows = gitcomet_core::file_diff::side_by_side_rows(old_text, new_text);

                    // Store the file path for syntax highlighting.
                    let file_path = Some(if file.path.is_absolute() {
                        file.path.clone()
                    } else {
                        workdir.join(&file.path)
                    });
                    let language = file_path.as_ref().and_then(|p| {
                        rows::diff_syntax_language_for_path(p.to_string_lossy().as_ref())
                    });

                    // Precompute word highlights and inline rows.
                    let mut split_word_highlights_old: Vec<Option<Vec<Range<usize>>>> =
                        vec![None; rows.len()];
                    let mut split_word_highlights_new: Vec<Option<Vec<Range<usize>>>> =
                        vec![None; rows.len()];

                    let mut inline_rows: Vec<AnnotatedDiffLine> =
                        Vec::with_capacity(rows.len().saturating_mul(2));
                    let mut inline_word_highlights: Vec<Option<Vec<Range<usize>>>> =
                        Vec::with_capacity(rows.len().saturating_mul(2));
                    for (row_ix, row) in rows.iter().enumerate() {
                        use gitcomet_core::file_diff::FileDiffRowKind as K;
                        match row.kind {
                            K::Context => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Context,
                                    text: format!(" {}", row.old.as_deref().unwrap_or("")).into(),
                                    old_line: row.old_line,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Add => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Add,
                                    text: format!("+{}", row.new.as_deref().unwrap_or("")).into(),
                                    old_line: None,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Remove => {
                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                                    text: format!("-{}", row.old.as_deref().unwrap_or("")).into(),
                                    old_line: row.old_line,
                                    new_line: None,
                                });
                                inline_word_highlights.push(None);
                            }
                            K::Modify => {
                                let old = row.old.as_deref().unwrap_or("");
                                let new = row.new.as_deref().unwrap_or("");
                                let (old_ranges, new_ranges) = capped_word_diff_ranges(old, new);
                                let old_ranges_opt = (!old_ranges.is_empty()).then_some(old_ranges);
                                let new_ranges_opt = (!new_ranges.is_empty()).then_some(new_ranges);

                                split_word_highlights_old[row_ix] = old_ranges_opt.clone();
                                split_word_highlights_new[row_ix] = new_ranges_opt.clone();

                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Remove,
                                    text: format!("-{}", old).into(),
                                    old_line: row.old_line,
                                    new_line: None,
                                });
                                inline_word_highlights.push(old_ranges_opt);

                                inline_rows.push(AnnotatedDiffLine {
                                    kind: gitcomet_core::domain::DiffLineKind::Add,
                                    text: format!("+{}", new).into(),
                                    old_line: None,
                                    new_line: row.new_line,
                                });
                                inline_word_highlights.push(new_ranges_opt);
                            }
                        }
                    }

                    Rebuild {
                        file_path,
                        language,
                        rows,
                        inline_rows,
                        inline_word_highlights,
                        split_word_highlights_old,
                        split_word_highlights_new,
                    }
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.file_diff_cache_inflight != Some(seq) {
                        return;
                    }
                    if this.file_diff_cache_repo_id != Some(repo_id)
                        || this.file_diff_cache_rev != diff_file_rev
                        || this.file_diff_cache_target != diff_target_for_task
                    {
                        return;
                    }

                    this.file_diff_cache_inflight = None;
                    this.file_diff_cache_path = rebuild.file_path;
                    this.file_diff_cache_language = rebuild.language;
                    this.file_diff_cache_rows = rebuild.rows;
                    this.file_diff_inline_cache = rebuild.inline_rows;
                    this.file_diff_inline_word_highlights = rebuild.inline_word_highlights;
                    this.file_diff_split_word_highlights_old = rebuild.split_word_highlights_old;
                    this.file_diff_split_word_highlights_new = rebuild.split_word_highlights_new;

                    // Reset the segment cache to avoid mixing patch/file indices.
                    this.diff_text_segments_cache.clear();
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn image_format_for_path(path: &std::path::Path) -> Option<gpui::ImageFormat> {
        image_format_for_path(path)
    }

    pub(in super::super::super) fn ensure_file_image_diff_cache(&mut self) {
        struct Rebuild {
            repo_id: RepoId,
            diff_file_rev: u64,
            diff_target: Option<DiffTarget>,
            file_path: Option<std::path::PathBuf>,
            old: Option<Arc<gpui::Image>>,
            new: Option<Arc<gpui::Image>>,
            old_svg_path: Option<std::path::PathBuf>,
            new_svg_path: Option<std::path::PathBuf>,
        }

        enum Action {
            Clear,
            Noop,
            Reset {
                repo_id: RepoId,
                diff_file_rev: u64,
                diff_target: Option<DiffTarget>,
            },
            Rebuild(Rebuild),
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };

            if !Self::is_file_diff_target(repo.diff_target.as_ref()) {
                return Action::Clear;
            }

            if self.file_image_diff_cache_repo_id == Some(repo.id)
                && self.file_image_diff_cache_rev == repo.diff_file_rev
                && self.file_image_diff_cache_target.as_ref() == repo.diff_target.as_ref()
            {
                return Action::Noop;
            }

            let repo_id = repo.id;
            let diff_file_rev = repo.diff_file_rev;
            let diff_target = repo.diff_target.clone();

            let Loadable::Ready(file_opt) = &repo.diff_file_image else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };
            let Some(file) = file_opt.as_ref() else {
                return Action::Reset {
                    repo_id,
                    diff_file_rev,
                    diff_target,
                };
            };

            let format = Self::image_format_for_path(&file.path);
            let mut old_svg_path = None;
            let mut new_svg_path = None;
            let old = file.old.as_ref().and_then(|bytes| {
                format.and_then(|format| {
                    decode_file_image_diff_bytes(format, bytes, Some(&mut old_svg_path))
                })
            });
            let new = file.new.as_ref().and_then(|bytes| {
                format.and_then(|format| {
                    decode_file_image_diff_bytes(format, bytes, Some(&mut new_svg_path))
                })
            });

            let workdir = &repo.spec.workdir;
            let file_path = Some(if file.path.is_absolute() {
                file.path.clone()
            } else {
                workdir.join(&file.path)
            });

            Action::Rebuild(Rebuild {
                repo_id,
                diff_file_rev,
                diff_target,
                file_path,
                old,
                new,
                old_svg_path,
                new_svg_path,
            })
        })();

        match action {
            Action::Noop => {}
            Action::Clear => {
                self.file_image_diff_cache_repo_id = None;
                self.file_image_diff_cache_target = None;
                self.file_image_diff_cache_rev = 0;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Reset {
                repo_id,
                diff_file_rev,
                diff_target,
            } => {
                self.file_image_diff_cache_repo_id = Some(repo_id);
                self.file_image_diff_cache_rev = diff_file_rev;
                self.file_image_diff_cache_target = diff_target;
                self.file_image_diff_cache_path = None;
                self.file_image_diff_cache_old = None;
                self.file_image_diff_cache_new = None;
                self.file_image_diff_cache_old_svg_path = None;
                self.file_image_diff_cache_new_svg_path = None;
            }
            Action::Rebuild(rebuild) => {
                self.file_image_diff_cache_repo_id = Some(rebuild.repo_id);
                self.file_image_diff_cache_rev = rebuild.diff_file_rev;
                self.file_image_diff_cache_target = rebuild.diff_target;
                self.file_image_diff_cache_path = rebuild.file_path;
                self.file_image_diff_cache_old = rebuild.old;
                self.file_image_diff_cache_new = rebuild.new;
                self.file_image_diff_cache_old_svg_path = rebuild.old_svg_path;
                self.file_image_diff_cache_new_svg_path = rebuild.new_svg_path;
            }
        }
    }

    pub(in super::super::super) fn rebuild_diff_cache(&mut self, cx: &mut gpui::Context<Self>) {
        self.diff_cache.clear();
        self.diff_cache_repo_id = None;
        self.diff_cache_rev = 0;
        self.diff_cache_target = None;
        self.diff_file_for_src_ix.clear();
        self.diff_language_for_src_ix.clear();
        self.diff_click_kinds.clear();
        self.diff_header_display_cache.clear();
        self.diff_split_cache.clear();
        self.diff_split_cache_len = 0;
        self.diff_visible_indices.clear();
        self.diff_visible_cache_len = 0;
        self.diff_visible_is_file_view = false;
        self.diff_scrollbar_markers_cache.clear();
        self.diff_word_highlights.clear();
        self.diff_word_highlights_inflight = None;
        self.diff_file_stats.clear();
        self.diff_text_segments_cache.clear();
        self.diff_selection_anchor = None;
        self.diff_selection_range = None;
        self.diff_preview_is_new_file = false;
        self.diff_preview_new_file_lines = Arc::new(Vec::new());

        let (repo_id, diff_rev, diff_target, workdir, annotated) = {
            let Some(repo) = self.active_repo() else {
                return;
            };
            let workdir = repo.spec.workdir.clone();
            let annotated = match &repo.diff {
                Loadable::Ready(diff) => Some(annotate_unified(diff)),
                _ => None,
            };
            (
                repo.id,
                repo.diff_rev,
                repo.diff_target.clone(),
                workdir,
                annotated,
            )
        };

        self.diff_cache_repo_id = Some(repo_id);
        self.diff_cache_rev = diff_rev;
        self.diff_cache_target = diff_target;

        let Some(annotated) = annotated else {
            return;
        };

        self.diff_cache = annotated;
        self.diff_file_for_src_ix = compute_diff_file_for_src_ix(&self.diff_cache);
        self.diff_click_kinds = self
            .diff_cache
            .iter()
            .map(|line| {
                if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Hunk) {
                    DiffClickKind::HunkHeader
                } else if matches!(line.kind, gitcomet_core::domain::DiffLineKind::Header)
                    && line.text.starts_with("diff --git ")
                {
                    DiffClickKind::FileHeader
                } else {
                    DiffClickKind::Line
                }
            })
            .collect();
        for (src_ix, click_kind) in self.diff_click_kinds.iter().enumerate() {
            match click_kind {
                DiffClickKind::FileHeader => {
                    let Some(line) = self.diff_cache.get(src_ix) else {
                        continue;
                    };
                    let display = parse_diff_git_header_path(line.text.as_ref())
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::HunkHeader => {
                    let Some(line) = self.diff_cache.get(src_ix) else {
                        continue;
                    };
                    let display = parse_unified_hunk_header_for_display(line.text.as_ref())
                        .map(|p| {
                            let heading = p.heading.unwrap_or_default();
                            if heading.is_empty() {
                                format!("{} {}", p.old, p.new)
                            } else {
                                format!("{} {}  {heading}", p.old, p.new)
                            }
                        })
                        .unwrap_or_else(|| line.text.as_ref().to_string());
                    self.diff_header_display_cache
                        .insert(src_ix, display.into());
                }
                DiffClickKind::Line => {}
            }
        }
        self.diff_file_stats = compute_diff_file_stats(&self.diff_cache);
        self.diff_word_highlights = vec![None; self.diff_cache.len()];
        self.diff_word_highlights_seq = self.diff_word_highlights_seq.wrapping_add(1);
        let seq = self.diff_word_highlights_seq;
        self.diff_word_highlights_inflight = Some(seq);

        let diff_lines = self.diff_cache.clone();
        let diff_target_for_task = self.diff_cache_target.clone();
        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let highlights =
                    smol::unblock(move || compute_diff_word_highlights(&diff_lines)).await;

                let _ = view.update(cx, |this, cx| {
                    if this.diff_word_highlights_inflight != Some(seq) {
                        return;
                    }
                    if this.diff_cache_repo_id != Some(repo_id)
                        || this.diff_cache_rev != diff_rev
                        || this.diff_cache_target != diff_target_for_task
                    {
                        return;
                    }

                    this.diff_word_highlights_inflight = None;
                    this.diff_word_highlights = highlights;
                    cx.notify();
                });
            },
        )
        .detach();

        let mut current_file: Option<Arc<str>> = None;
        let mut current_language: Option<rows::DiffSyntaxLanguage> = None;
        for (src_ix, line) in self.diff_cache.iter().enumerate() {
            let file = self
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|p| p.as_ref());
            let file_changed = match (&current_file, file) {
                (Some(cur), Some(next)) => !Arc::ptr_eq(cur, next),
                (None, None) => false,
                _ => true,
            };
            if file_changed {
                current_file = file.cloned();
                current_language =
                    file.and_then(|p| rows::diff_syntax_language_for_path(p.as_ref()));
            }

            let language = match line.kind {
                gitcomet_core::domain::DiffLineKind::Add
                | gitcomet_core::domain::DiffLineKind::Remove
                | gitcomet_core::domain::DiffLineKind::Context => current_language,
                gitcomet_core::domain::DiffLineKind::Header
                | gitcomet_core::domain::DiffLineKind::Hunk => None,
            };
            self.diff_language_for_src_ix.push(language);
        }

        if let Some((abs_path, lines)) = build_new_file_preview_from_diff(
            &self.diff_cache,
            &workdir,
            self.diff_cache_target.as_ref(),
        ) {
            self.diff_preview_is_new_file = true;
            self.diff_preview_new_file_lines = Arc::new(lines);
            self.worktree_preview_path = Some(abs_path);
            self.worktree_preview = Loadable::Ready(self.diff_preview_new_file_lines.clone());
            self.worktree_preview_segments_cache_path = None;
            self.worktree_preview_segments_cache.clear();
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
        }
    }

    fn ensure_diff_split_cache(&mut self) {
        if self.diff_split_cache_len == self.diff_cache.len() && !self.diff_split_cache.is_empty() {
            return;
        }
        self.diff_split_cache_len = self.diff_cache.len();
        self.diff_split_cache = build_patch_split_rows(&self.diff_cache);
    }

    fn diff_scrollbar_markers_patch(&self) -> Vec<components::ScrollbarMarker> {
        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_indices.len(), |visible_ix| {
                    let Some(&src_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.diff_cache.get(src_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                scrollbar_markers_from_flags(self.diff_visible_indices.len(), |visible_ix| {
                    let Some(&row_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.diff_split_cache.get(row_ix) else {
                        return 0;
                    };
                    match row {
                        PatchSplitRow::Aligned { row, .. } => match row.kind {
                            gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                            gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                            gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                            gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        },
                        PatchSplitRow::Raw { .. } => 0,
                    }
                })
            }
        }
    }

    fn compute_diff_scrollbar_markers(&self) -> Vec<components::ScrollbarMarker> {
        if !self.is_file_diff_view_active() {
            return self.diff_scrollbar_markers_patch();
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                scrollbar_markers_from_flags(self.diff_visible_indices.len(), |visible_ix| {
                    let Some(&inline_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return 0;
                    };
                    let Some(line) = self.file_diff_inline_cache.get(inline_ix) else {
                        return 0;
                    };
                    match line.kind {
                        gitcomet_core::domain::DiffLineKind::Add => 1,
                        gitcomet_core::domain::DiffLineKind::Remove => 2,
                        _ => 0,
                    }
                })
            }
            DiffViewMode::Split => {
                scrollbar_markers_from_flags(self.diff_visible_indices.len(), |visible_ix| {
                    let Some(&row_ix) = self.diff_visible_indices.get(visible_ix) else {
                        return 0;
                    };
                    let Some(row) = self.file_diff_cache_rows.get(row_ix) else {
                        return 0;
                    };
                    match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                    }
                })
            }
        }
    }

    pub(in super::super::super) fn ensure_diff_visible_indices(&mut self) {
        let is_file_view = self.is_file_diff_view_active();
        let current_len = if is_file_view {
            match self.diff_view {
                DiffViewMode::Inline => self.file_diff_inline_cache.len(),
                DiffViewMode::Split => self.file_diff_cache_rows.len(),
            }
        } else {
            self.diff_cache.len()
        };

        if self.diff_visible_cache_len == current_len
            && self.diff_visible_view == self.diff_view
            && self.diff_visible_is_file_view == is_file_view
        {
            return;
        }

        self.diff_visible_cache_len = current_len;
        self.diff_visible_view = self.diff_view;
        self.diff_visible_is_file_view = is_file_view;
        self.diff_horizontal_min_width = px(0.0);

        if is_file_view {
            self.diff_visible_indices = (0..current_len).collect();
            self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();
            if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
                self.diff_search_recompute_matches_for_current_view();
            }
            return;
        }

        match self.diff_view {
            DiffViewMode::Inline => {
                self.diff_visible_indices = self
                    .diff_cache
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, line)| {
                        (!should_hide_unified_diff_header_line(line)).then_some(ix)
                    })
                    .collect();
            }
            DiffViewMode::Split => {
                self.ensure_diff_split_cache();

                self.diff_visible_indices = self
                    .diff_split_cache
                    .iter()
                    .enumerate()
                    .filter_map(|(ix, row)| match row {
                        PatchSplitRow::Raw { src_ix, .. } => self
                            .diff_cache
                            .get(*src_ix)
                            .is_some_and(|line| !should_hide_unified_diff_header_line(line))
                            .then_some(ix),
                        PatchSplitRow::Aligned { .. } => Some(ix),
                    })
                    .collect();
            }
        }

        self.diff_scrollbar_markers_cache = self.compute_diff_scrollbar_markers();

        if self.diff_search_active && !self.diff_search_query.as_ref().trim().is_empty() {
            self.diff_search_recompute_matches_for_current_view();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn image_format_for_path_detects_known_extensions_case_insensitively() {
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.PNG")),
            Some(gpui::ImageFormat::Png)
        );
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.JpEg")),
            Some(gpui::ImageFormat::Jpeg)
        );
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.webp")),
            Some(gpui::ImageFormat::Webp)
        );
    }

    #[test]
    fn image_format_for_path_returns_none_for_unknown_or_missing_extension() {
        assert_eq!(
            MainPaneView::image_format_for_path(Path::new("x.heic")),
            None
        );
        assert_eq!(MainPaneView::image_format_for_path(Path::new("x")), None);
    }

    #[test]
    fn decode_file_image_diff_bytes_keeps_non_svg_bytes() {
        let bytes = [1_u8, 2, 3, 4, 5];
        let mut svg_path = None;
        let image =
            decode_file_image_diff_bytes(gpui::ImageFormat::Png, &bytes, Some(&mut svg_path))
                .unwrap();
        assert_eq!(image.format(), gpui::ImageFormat::Png);
        assert_eq!(image.bytes(), bytes);
        assert!(svg_path.is_none());
    }

    #[test]
    fn decode_file_image_diff_bytes_writes_svg_cache_file() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">
<rect width="16" height="16" fill="#00aaff"/>
</svg>"##;
        let mut svg_path = None;
        let image = decode_file_image_diff_bytes(gpui::ImageFormat::Svg, svg, Some(&mut svg_path));
        assert!(image.is_none());
        assert!(svg_path.is_some());
        assert!(svg_path.unwrap().exists());
    }
}
