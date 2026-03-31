use super::*;

struct ReadyWorktreePreview {
    text: SharedString,
    line_starts: Arc<[usize]>,
}

impl ReadyWorktreePreview {
    fn from_text(text: String) -> Self {
        let line_starts = Arc::from(build_line_starts(&text));
        Self {
            text: text.into(),
            line_starts,
        }
    }

    fn from_lines(lines: &[String], source_len: usize) -> Self {
        let (text, line_starts) = preview_source_text_and_line_starts_from_lines(lines, source_len);
        Self { text, line_starts }
    }
}

type ConflictPreviewImagePayload = (gpui::ImageFormat, Vec<u8>);

fn conflict_preview_side_bytes(
    file: Option<&gitcomet_state::model::ConflictFile>,
    side: ThreeWayColumn,
    fallback_text: &SharedString,
) -> Option<Vec<u8>> {
    let file_bytes = file.and_then(|file| match side {
        ThreeWayColumn::Base => file.base_bytes.as_deref(),
        ThreeWayColumn::Ours => file.ours_bytes.as_deref(),
        ThreeWayColumn::Theirs => file.theirs_bytes.as_deref(),
    });
    if let Some(bytes) = file_bytes
        && !bytes.is_empty()
    {
        return Some(bytes.to_vec());
    }

    let file_text = file.and_then(|file| match side {
        ThreeWayColumn::Base => file.base.as_deref(),
        ThreeWayColumn::Ours => file.ours.as_deref(),
        ThreeWayColumn::Theirs => file.theirs.as_deref(),
    });
    if let Some(text) = file_text
        && !text.is_empty()
    {
        return Some(text.as_bytes().to_vec());
    }

    (!fallback_text.is_empty()).then(|| fallback_text.as_ref().as_bytes().to_vec())
}

fn ready_conflict_preview_image_from_bytes(
    format: gpui::ImageFormat,
    bytes: Option<Vec<u8>>,
) -> LoadableImagePreview {
    match bytes {
        Some(bytes) => Loadable::Ready(Some(Arc::new(gpui::Image::from_bytes(format, bytes)))),
        None => Loadable::Ready(None),
    }
}

fn loading_conflict_preview_image(has_source: bool) -> LoadableImagePreview {
    if has_source {
        Loadable::Loading
    } else {
        Loadable::Ready(None)
    }
}

fn rasterize_conflict_preview_svg_payload(
    svg_bytes: Option<Vec<u8>>,
) -> Option<ConflictPreviewImagePayload> {
    let svg_bytes = svg_bytes?;
    if let Some(png) = crate::view::diff_utils::rasterize_svg_preview_png(&svg_bytes) {
        return Some((gpui::ImageFormat::Png, png));
    }
    Some((gpui::ImageFormat::Svg, svg_bytes))
}

fn loadable_conflict_preview_svg_image(
    payload: Option<ConflictPreviewImagePayload>,
    had_source: bool,
) -> LoadableImagePreview {
    match payload {
        Some((format, bytes)) => {
            Loadable::Ready(Some(Arc::new(gpui::Image::from_bytes(format, bytes))))
        }
        None if had_source => Loadable::Error("Preview unavailable.".into()),
        None => Loadable::Ready(None),
    }
}

impl MainPaneView {
    /// Clears worktree preview source text, line starts, and the segments
    /// cache. Use this when the preview content is invalidated but the caller
    /// still needs to set identity fields (path, loadable state, syntax
    /// language) separately.
    pub(super) fn reset_worktree_preview_source_state(&mut self) {
        self.worktree_preview_text = SharedString::default();
        self.worktree_preview_line_starts = Arc::default();
        self.worktree_preview_search_trigram_index = None;
        self.worktree_preview_segments_cache_path = None;
        self.worktree_preview_cache_write_blocked_until_rev = None;
        self.worktree_preview_segments_cache.clear();
    }

    pub(in super::super::super) fn is_file_diff_target(target: Option<&DiffTarget>) -> bool {
        matches!(
            target,
            Some(DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. })
        )
    }

    pub(in crate::view) fn is_file_preview_active(&self) -> bool {
        let is_commit_file_target = self.active_repo().is_some_and(|repo| {
            matches!(
                repo.diff_state.diff_target.as_ref(),
                Some(DiffTarget::Commit { path: Some(_), .. })
            )
        });
        let has_untracked_preview = self.untracked_worktree_preview_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p) && p.is_file()
        });
        let has_added_preview = self.added_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p)
                && !p.is_dir()
                && (p.is_file() || is_commit_file_target)
        });
        let has_deleted_preview = self.deleted_file_preview_abs_path().is_some_and(|p| {
            !crate::view::should_bypass_text_file_preview_for_path(&p) && !p.is_dir()
        });
        has_untracked_preview || has_added_preview || has_deleted_preview
    }

    /// Returns `true` when the markdown rendered preview is currently shown
    /// (either single-pane file preview or two-sided diff preview).
    pub(in crate::view) fn is_markdown_preview_active(&self) -> bool {
        let is_file_preview =
            self.is_file_preview_active() && self.untracked_directory_notice().is_none();
        let wants_file_diff = !is_file_preview
            && !self.is_worktree_target_directory()
            && self.active_repo().is_some_and(|repo| {
                Self::is_file_diff_target(repo.diff_state.diff_target.as_ref())
            });
        let rendered_preview_kind = crate::view::diff_target_rendered_preview_kind(
            self.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.as_ref()),
        );
        let toggle_kind = crate::view::main_diff_rendered_preview_toggle_kind(
            wants_file_diff,
            is_file_preview,
            rendered_preview_kind,
        );
        toggle_kind == Some(RenderedPreviewKind::Markdown)
            && self
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Rendered
    }

    /// Returns `true` when the current diff target is a conflicted file and
    /// there is an applicable conflict resolver strategy.
    pub(in crate::view) fn is_conflict_resolver_active(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            let Some(DiffTarget::WorkingTree { path, area }) = repo.diff_state.diff_target.as_ref()
            else {
                return false;
            };
            if *area != DiffArea::Unstaged {
                return false;
            }
            let Loadable::Ready(status) = &repo.status else {
                return false;
            };
            let conflict_kind = status
                .unstaged
                .iter()
                .find(|e| e.path == *path && e.kind == FileStatusKind::Conflicted)
                .and_then(|e| e.conflict);
            Self::conflict_resolver_strategy(conflict_kind, false).is_some()
        })
    }

    pub(in crate::view) fn is_conflict_rendered_preview_active(&self) -> bool {
        self.conflict_resolver.path.as_ref().is_some_and(|path| {
            crate::view::preview_path_rendered_kind(path).is_some()
                && self.conflict_resolver.resolver_preview_mode
                    == ConflictResolverPreviewMode::Preview
        })
    }

    pub(in crate::view) fn ensure_conflict_markdown_preview_cache(&mut self) {
        if self.is_conflict_rendered_preview_active()
            && self
                .request_conflict_file_load_mode(gitcomet_state::model::ConflictFileLoadMode::Full)
        {
            return;
        }

        let Some(source_hash) = self.conflict_resolver.source_hash else {
            self.conflict_resolver.markdown_preview =
                ConflictResolverMarkdownPreviewState::default();
            return;
        };

        let previews = &self.conflict_resolver.markdown_preview;
        let cache_ready = previews.source_hash == Some(source_hash)
            && !matches!(previews.documents.base, Loadable::NotLoaded)
            && !matches!(previews.documents.ours, Loadable::NotLoaded)
            && !matches!(previews.documents.theirs, Loadable::NotLoaded);
        if cache_ready {
            return;
        }

        let _perf_scope = perf::span(ViewPerfSpan::MarkdownPreviewParse);
        self.conflict_resolver.markdown_preview = ConflictResolverMarkdownPreviewState {
            source_hash: Some(source_hash),
            documents: build_conflict_markdown_preview_documents(
                &self.conflict_resolver.three_way_text,
            ),
        };
    }

    pub(in crate::view) fn ensure_conflict_image_preview_cache(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(source_hash) = self.conflict_resolver.source_hash else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };
        let Some(path) = self.conflict_resolver.path.clone() else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };
        let Some(format) = crate::view::diff_utils::image_format_for_path(&path) else {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState::default();
            return;
        };

        let previews = &self.conflict_resolver.image_preview;
        let cache_ready = previews.source_hash == Some(source_hash)
            && previews.path.as_ref() == Some(&path)
            && !matches!(previews.images.base, Loadable::NotLoaded)
            && !matches!(previews.images.ours, Loadable::NotLoaded)
            && !matches!(previews.images.theirs, Loadable::NotLoaded);
        if cache_ready {
            return;
        }

        let loaded_file = self.conflict_resolver.loaded_file.as_ref();
        let base_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Base,
            &self.conflict_resolver.three_way_text.base,
        );
        let ours_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Ours,
            &self.conflict_resolver.three_way_text.ours,
        );
        let theirs_bytes = conflict_preview_side_bytes(
            loaded_file,
            ThreeWayColumn::Theirs,
            &self.conflict_resolver.three_way_text.theirs,
        );

        if format != gpui::ImageFormat::Svg {
            self.conflict_resolver.image_preview = ConflictResolverImagePreviewState {
                source_hash: Some(source_hash),
                path: Some(path),
                images: ThreeWaySides {
                    base: ready_conflict_preview_image_from_bytes(format, base_bytes),
                    ours: ready_conflict_preview_image_from_bytes(format, ours_bytes),
                    theirs: ready_conflict_preview_image_from_bytes(format, theirs_bytes),
                },
            };
            return;
        }

        let base_has_source = base_bytes.is_some();
        let ours_has_source = ours_bytes.is_some();
        let theirs_has_source = theirs_bytes.is_some();
        self.conflict_resolver.image_preview = ConflictResolverImagePreviewState {
            source_hash: Some(source_hash),
            path: Some(path.clone()),
            images: ThreeWaySides {
                base: loading_conflict_preview_image(base_has_source),
                ours: loading_conflict_preview_image(ours_has_source),
                theirs: loading_conflict_preview_image(theirs_has_source),
            },
        };

        cx.spawn(
            async move |view: WeakEntity<MainPaneView>, cx: &mut gpui::AsyncApp| {
                let (base_payload, ours_payload, theirs_payload) = smol::unblock(move || {
                    (
                        rasterize_conflict_preview_svg_payload(base_bytes),
                        rasterize_conflict_preview_svg_payload(ours_bytes),
                        rasterize_conflict_preview_svg_payload(theirs_bytes),
                    )
                })
                .await;

                let _ = view.update(cx, |this, cx| {
                    if this.conflict_resolver.image_preview.source_hash != Some(source_hash)
                        || this.conflict_resolver.image_preview.path.as_ref() != Some(&path)
                    {
                        return;
                    }

                    this.conflict_resolver.image_preview.images.base =
                        loadable_conflict_preview_svg_image(base_payload, base_has_source);
                    this.conflict_resolver.image_preview.images.ours =
                        loadable_conflict_preview_svg_image(ours_payload, ours_has_source);
                    this.conflict_resolver.image_preview.images.theirs =
                        loadable_conflict_preview_svg_image(theirs_payload, theirs_has_source);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    pub(in crate::view) fn is_worktree_target_directory(&self) -> bool {
        self.active_repo().is_some_and(|repo| {
            let Some(DiffTarget::WorkingTree { path, .. }) = repo.diff_state.diff_target.as_ref()
            else {
                return false;
            };
            let abs_path = if path.is_absolute() {
                path.clone()
            } else {
                repo.spec.workdir.join(path)
            };
            abs_path.is_dir()
        })
    }

    pub(in crate::view) fn untracked_directory_notice(&self) -> Option<SharedString> {
        let repo = self.active_repo()?;
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        let abs_path = if path.is_absolute() {
            path.clone()
        } else {
            repo.spec.workdir.join(path)
        };
        if !abs_path.is_dir() {
            return None;
        }

        let is_untracked = *area == DiffArea::Unstaged
            && matches!(&repo.status, Loadable::Ready(status) if status
                .unstaged
                .iter()
                .any(|e| e.kind == FileStatusKind::Untracked && &e.path == path));

        if is_untracked {
            Some(
                "Folder is untracked. Select a file inside it, or stage the folder to inspect tracked changes."
                    .into(),
            )
        } else {
            Some(
                "Selected path is a directory. Select a file inside it to preview its contents."
                    .into(),
            )
        }
    }

    pub(in crate::view) fn worktree_preview_line_count(&self) -> Option<usize> {
        match &self.worktree_preview {
            Loadable::Ready(line_count) => Some(*line_count),
            _ => None,
        }
    }

    pub(in crate::view) fn worktree_preview_line_text(&self, line_ix: usize) -> Option<&str> {
        let line_count = self.worktree_preview_line_count()?;
        if line_ix >= line_count {
            return None;
        }
        Some(rows::resolved_output_line_text(
            self.worktree_preview_text.as_ref(),
            self.worktree_preview_line_starts.as_ref(),
            line_ix,
        ))
    }

    /// Returns the row count for the active markdown preview, taking the
    /// current diff view mode into account.  Returns `None` when no markdown
    /// preview is active.
    pub(in crate::view) fn markdown_preview_row_count(&self) -> Option<usize> {
        if self.is_file_preview_active() {
            if let Loadable::Ready(doc) = &self.worktree_markdown_preview {
                return Some(doc.rows.len());
            }
            return None;
        }
        if let Loadable::Ready(diff) = &self.file_markdown_preview {
            return Some(match self.diff_view {
                DiffViewMode::Inline => diff.inline.rows.len(),
                DiffViewMode::Split => diff.old.rows.len().max(diff.new.rows.len()),
            });
        }
        None
    }

    /// Returns the text of a markdown preview row at `visible_ix` for the
    /// given `region`.  For file preview (added/deleted/untracked) only
    /// `DiffTextRegion::Inline` is meaningful.
    pub(in crate::view) fn markdown_preview_row_text(
        &self,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> SharedString {
        let fallback = SharedString::default();

        if self.is_file_preview_active() {
            let Loadable::Ready(doc) = &self.worktree_markdown_preview else {
                return fallback;
            };
            return doc
                .rows
                .get(visible_ix)
                .map(|r| r.text.clone())
                .unwrap_or(fallback);
        }

        let Loadable::Ready(diff) = &self.file_markdown_preview else {
            return fallback;
        };

        match self.diff_view {
            DiffViewMode::Inline => diff
                .inline
                .rows
                .get(visible_ix)
                .map(|r| r.text.clone())
                .unwrap_or(fallback),
            DiffViewMode::Split => match region {
                DiffTextRegion::SplitLeft | DiffTextRegion::Inline => diff
                    .old
                    .rows
                    .get(visible_ix)
                    .map(|r| r.text.clone())
                    .unwrap_or(fallback),
                DiffTextRegion::SplitRight => diff
                    .new
                    .rows
                    .get(visible_ix)
                    .map(|r| r.text.clone())
                    .unwrap_or(fallback),
            },
        }
    }

    pub(in super::super::super) fn untracked_worktree_preview_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let status = match &repo.status {
            Loadable::Ready(s) => s,
            _ => return None,
        };
        let workdir = repo.spec.workdir.clone();
        let DiffTarget::WorkingTree { path, area } = repo.diff_state.diff_target.as_ref()? else {
            return None;
        };
        if *area != DiffArea::Unstaged {
            return None;
        }
        let is_untracked = status
            .unstaged
            .iter()
            .any(|e| e.kind == FileStatusKind::Untracked && &e.path == path);
        is_untracked.then(|| {
            if path.is_absolute() {
                path.clone()
            } else {
                workdir.join(path)
            }
        })
    }

    pub(in super::super::super) fn added_file_preview_abs_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let target = repo.diff_state.diff_target.as_ref()?;

        match target {
            DiffTarget::WorkingTree { path, area } => {
                if *area != DiffArea::Staged {
                    return None;
                }
                let status = match &repo.status {
                    Loadable::Ready(s) => s,
                    _ => return None,
                };
                let is_added = status
                    .staged
                    .iter()
                    .any(|e| e.kind == FileStatusKind::Added && &e.path == path);
                if !is_added {
                    return None;
                }
                Some(if path.is_absolute() {
                    path.clone()
                } else {
                    workdir.join(path)
                })
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
            } => {
                let details = match &repo.history_state.commit_details {
                    Loadable::Ready(d) => d,
                    _ => return None,
                };
                if &details.id != commit_id {
                    return None;
                }
                let is_added = details
                    .files
                    .iter()
                    .any(|f| f.kind == FileStatusKind::Added && &f.path == path);
                if !is_added {
                    return None;
                }
                Some(workdir.join(path))
            }
            _ => None,
        }
    }

    pub(in super::super::super) fn deleted_file_preview_abs_path(
        &self,
    ) -> Option<std::path::PathBuf> {
        let repo = self.active_repo()?;
        let workdir = repo.spec.workdir.clone();
        let target = repo.diff_state.diff_target.as_ref()?;

        match target {
            DiffTarget::WorkingTree { path, area } => {
                let status = match &repo.status {
                    Loadable::Ready(s) => s,
                    _ => return None,
                };
                let entries = match area {
                    DiffArea::Unstaged => status.unstaged.as_slice(),
                    DiffArea::Staged => status.staged.as_slice(),
                };
                let is_deleted = entries
                    .iter()
                    .any(|e| e.kind == FileStatusKind::Deleted && &e.path == path);
                if !is_deleted {
                    return None;
                }
                Some(if path.is_absolute() {
                    path.clone()
                } else {
                    workdir.join(path)
                })
            }
            DiffTarget::Commit {
                commit_id,
                path: Some(path),
            } => {
                let details = match &repo.history_state.commit_details {
                    Loadable::Ready(d) => d,
                    _ => return None,
                };
                if &details.id != commit_id {
                    return None;
                }
                let is_deleted = details
                    .files
                    .iter()
                    .any(|f| f.kind == FileStatusKind::Deleted && &f.path == path);
                if !is_deleted {
                    return None;
                }
                Some(workdir.join(path))
            }
            _ => None,
        }
    }

    pub(in super::super::super) fn ensure_preview_loading(&mut self, path: std::path::PathBuf) {
        let should_reset = match self.worktree_preview_path.as_ref() {
            Some(p) => p != &path,
            None => true,
        };
        if should_reset {
            self.worktree_preview_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
            self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
            self.worktree_preview_path = Some(path);
            self.worktree_preview = Loadable::Loading;
            self.reset_worktree_preview_source_state();
            self.diff_horizontal_min_width = px(0.0);
        } else if matches!(self.worktree_preview, Loadable::NotLoaded) {
            self.worktree_preview = Loadable::Loading;
            self.reset_worktree_preview_source_state();
            self.diff_horizontal_min_width = px(0.0);
        }
    }

    pub(in super::super::super) fn ensure_worktree_preview_loaded(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        let should_reload = match self.worktree_preview_path.as_ref() {
            Some(p) => p != &path,
            None => true,
        } || matches!(self.worktree_preview, Loadable::NotLoaded);
        if !should_reload {
            return;
        }

        self.worktree_preview_syntax_language = rows::diff_syntax_language_for_path(&path);
        self.worktree_preview_path = Some(path.clone());
        self.worktree_preview = Loadable::Loading;
        self.reset_worktree_preview_source_state();
        self.diff_horizontal_min_width = px(0.0);
        self.worktree_preview_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);

        cx.spawn(async move |view, cx| {
            const MAX_BYTES: u64 = 2 * 1024 * 1024;
            let result = smol::unblock({
                let path_for_task = path.clone();
                move || {
                let meta = std::fs::metadata(&path_for_task).map_err(|e| e.to_string())?;
                if meta.is_dir() {
                    return Err("Selected path is a directory. Select a file inside to preview, or stage the directory to add its contents.".to_string());
                }
                if meta.len() > MAX_BYTES {
                    return Err(format!(
                        "File is too large to preview ({} bytes).",
                        meta.len()
                    ));
                }

                let bytes = std::fs::read(&path_for_task).map_err(|e| e.to_string())?;
                let text = String::from_utf8(bytes).map_err(|_| {
                    "File is not valid UTF-8; binary preview is not supported.".to_string()
                })?;

                Ok::<ReadyWorktreePreview, String>(ReadyWorktreePreview::from_text(text))
                }
            })
            .await;
            let _ = view.update(cx, |this, cx| {
                if this.worktree_preview_path.as_ref() != Some(&path) {
                    return;
                }
                this.worktree_preview_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                match result {
                    Ok(preview) => this.set_worktree_preview_ready_source(
                        path.clone(),
                        preview.text,
                        preview.line_starts,
                        cx,
                    ),
                    Err(e) => {
                        this.worktree_preview = Loadable::Error(e);
                        this.reset_worktree_preview_source_state();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in super::super::super) fn try_populate_worktree_preview_from_diff_file(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((abs_path, preview_result)) = (|| {
            let repo = self.active_repo()?;
            let path_from_target = match repo.diff_state.diff_target.as_ref()? {
                DiffTarget::WorkingTree { path, .. } => Some(path),
                DiffTarget::Commit {
                    path: Some(path), ..
                } => Some(path),
                _ => None,
            }?;

            let abs_path = if path_from_target.is_absolute() {
                path_from_target.clone()
            } else {
                repo.spec.workdir.join(path_from_target)
            };

            let prefer_old = match repo.diff_state.diff_target.as_ref()? {
                DiffTarget::WorkingTree { path, area } => match &repo.status {
                    Loadable::Ready(status) => {
                        let entries = match area {
                            DiffArea::Unstaged => status.unstaged.as_slice(),
                            DiffArea::Staged => status.staged.as_slice(),
                        };
                        entries
                            .iter()
                            .any(|e| e.kind == FileStatusKind::Deleted && &e.path == path)
                    }
                    _ => false,
                },
                DiffTarget::Commit {
                    commit_id,
                    path: Some(path),
                } => match &repo.history_state.commit_details {
                    Loadable::Ready(details) if &details.id == commit_id => details
                        .files
                        .iter()
                        .any(|f| f.kind == FileStatusKind::Deleted && &f.path == path),
                    _ => false,
                },
                _ => false,
            };

            let mut diff_file_error: Option<String> = None;
            let mut preview_result: Option<Result<ReadyWorktreePreview, String>> =
                match &repo.diff_state.diff_file {
                    Loadable::NotLoaded | Loadable::Loading => None,
                    Loadable::Error(e) => {
                        diff_file_error = Some(e.clone());
                        None
                    }
                    Loadable::Ready(file) => file.as_ref().and_then(|file| {
                        let text = if prefer_old {
                            file.old.as_deref()
                        } else {
                            file.new.as_deref()
                        };
                        text.map(|text| Ok(ReadyWorktreePreview::from_text(text.to_owned())))
                    }),
                };

            if preview_result.is_none() {
                match &repo.diff_state.diff {
                    Loadable::Ready(diff) => {
                        let annotated = annotate_unified(diff);
                        if prefer_old {
                            if let Some(preview) = build_deleted_file_preview_from_diff(
                                &annotated,
                                &repo.spec.workdir,
                                repo.diff_state.diff_target.as_ref(),
                            ) {
                                preview_result = Some(Ok(ReadyWorktreePreview::from_lines(
                                    preview.lines.as_slice(),
                                    preview.source_len,
                                )));
                            }
                        } else if let Some(preview) = build_new_file_preview_from_diff(
                            &annotated,
                            &repo.spec.workdir,
                            repo.diff_state.diff_target.as_ref(),
                        ) {
                            preview_result = Some(Ok(ReadyWorktreePreview::from_lines(
                                preview.lines.as_slice(),
                                preview.source_len,
                            )));
                        } else if let Some(e) = diff_file_error {
                            preview_result = Some(Err(e));
                        } else {
                            preview_result =
                                Some(Err("No text preview available for this file.".to_string()));
                        }
                    }
                    Loadable::Error(e) => preview_result = Some(Err(e.clone())),
                    Loadable::NotLoaded | Loadable::Loading => {}
                }
            }

            Some((abs_path, preview_result))
        })() else {
            return;
        };

        if matches!(self.worktree_preview, Loadable::Ready(_))
            && self.worktree_preview_path.as_ref() == Some(&abs_path)
        {
            return;
        }

        let Some(preview_result) = preview_result else {
            return;
        };

        match preview_result {
            Ok(preview) => {
                self.worktree_preview_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                self.set_worktree_preview_ready_source(
                    abs_path,
                    preview.text,
                    preview.line_starts,
                    cx,
                );
                self.diff_horizontal_min_width = px(0.0);
            }
            Err(e) => {
                if self.worktree_preview_path.as_ref() != Some(&abs_path)
                    || matches!(
                        self.worktree_preview,
                        Loadable::NotLoaded | Loadable::Loading
                    )
                {
                    self.worktree_preview_scroll
                        .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                    self.worktree_preview_path = Some(abs_path);
                    self.worktree_preview = Loadable::Error(e);
                    self.worktree_preview_syntax_language = None;
                    self.reset_worktree_preview_source_state();
                    self.diff_horizontal_min_width = px(0.0);
                }
            }
        }
    }
}

fn build_conflict_markdown_preview_documents(
    sources: &ThreeWaySides<SharedString>,
) -> ThreeWaySides<LoadableMarkdownDoc> {
    use crate::view::markdown_preview;

    let build = |source: &str| -> LoadableMarkdownDoc {
        match markdown_preview::parse_markdown(source) {
            Some(document) => Loadable::Ready(Arc::new(document)),
            None => Loadable::Error(
                markdown_preview::single_preview_unavailable_reason(source.len()).to_string(),
            ),
        }
    };
    ThreeWaySides {
        base: build(sources.base.as_ref()),
        ours: build(sources.ours.as_ref()),
        theirs: build(sources.theirs.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_conflict_markdown_preview_documents_parses_each_side() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "# Base\n".into(),
            ours: "- item\n".into(),
            theirs: "plain text".into(),
        });

        assert!(matches!(documents.base, Loadable::Ready(_)));
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn build_conflict_markdown_preview_documents_reports_per_side_size_limits() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "x"
                .repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1)
                .into(),
            ours: "".into(),
            theirs: "".into(),
        });

        let Loadable::Error(message) = documents.base else {
            panic!("expected oversize base preview to error: {documents:?}");
        };
        assert!(
            message.contains("1 MiB"),
            "should mention size limit: {message}"
        );
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn build_conflict_markdown_preview_documents_handles_empty_sources() {
        let documents = build_conflict_markdown_preview_documents(&ThreeWaySides {
            base: "".into(),
            ours: "".into(),
            theirs: "".into(),
        });

        // Empty sources should still produce Ready documents, not errors.
        assert!(matches!(documents.base, Loadable::Ready(_)));
        assert!(matches!(documents.ours, Loadable::Ready(_)));
        assert!(matches!(documents.theirs, Loadable::Ready(_)));
    }

    #[test]
    fn conflict_markdown_preview_state_document_returns_correct_side() {
        let state = ConflictResolverMarkdownPreviewState {
            source_hash: Some(42),
            documents: build_conflict_markdown_preview_documents(&ThreeWaySides {
                base: "# Base".into(),
                ours: "# Ours".into(),
                theirs: "# Theirs".into(),
            }),
        };

        // Each side should have its own document with the expected content.
        let base = state.document(ThreeWayColumn::Base);
        let ours = state.document(ThreeWayColumn::Ours);
        let theirs = state.document(ThreeWayColumn::Theirs);

        let base_doc = match base {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for base"),
        };
        let ours_doc = match ours {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for ours"),
        };
        let theirs_doc = match theirs {
            Loadable::Ready(d) => d,
            _ => panic!("expected Ready for theirs"),
        };

        assert!(base_doc.rows[0].text.contains("Base"));
        assert!(ours_doc.rows[0].text.contains("Ours"));
        assert!(theirs_doc.rows[0].text.contains("Theirs"));
    }
}
