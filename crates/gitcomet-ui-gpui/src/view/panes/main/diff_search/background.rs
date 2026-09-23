//! Immutable search inputs. File reads, matching and query refinement run
//! on the background executor; the UI only captures a generation and publishes
//! its finished matches. A document is reused while its projection is unchanged.
use super::super::preview::worktree_preview_materialized_line_raw_text;
use super::*;
use gitcomet_core::domain::DiffRowProvider;
use gitcomet_core::file_diff::FileDiffLineText;
use gitcomet_core::services::CancellationToken;
use std::sync::Mutex;

/// Includes content generations and the visible row projection. A wrap, mode or
/// viewport change can move matches even when the bytes are unchanged. Extend this
/// key when adding a search surface; omitting its generation can publish stale rows.
#[derive(Clone, PartialEq, Eq)]
pub(in crate::view) struct SearchDocumentKey {
    repo: Option<(RepoId, u64)>,
    patch: (Option<RepoId>, u64),
    file: (Option<RepoId>, u64, u64),
    projection: (u64, DiffViewMode, bool, Option<DiffWrapVisibleCacheKey>),
    preview: u64,
    editor: Option<(u64, u64)>,
    markdown: u64,
    markdown_wrap: [Option<MarkdownPreviewWrapKey>; 4],
    conflict: (u64, Option<u64>, u64, ConflictResolverViewMode, bool),
    surface: (bool, bool, bool, bool),
    viewport: (u32, u32),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn background_search_reuses_rows_and_preserves_query_semantics() {
        let texts = ["needle alpha", "Needle beta", "gamma", "猫 alpha", "tail"];
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let document = SearchDocument(DocumentSource::Rows(RowDocument {
            len: texts.len(),
            columns: 1,
            text: Box::new(move |row, _| {
                counter.fetch_add(1, Ordering::Relaxed);
                Some(texts[row].into())
            }),
            wrapped: Vec::new(),
            streamed: false,
            previous: Mutex::new(VecDeque::new()),
        }));
        let search = |query, options| {
            document
                .search(query, options, CancellationToken::new())
                .matches
        };
        assert_eq!(search("needle", DiffSearchOptions::default()), vec![0, 1]);
        assert_eq!(
            reads.load(Ordering::Relaxed),
            texts.len(),
            "the first query scans once without building an eager index"
        );
        reads.store(0, Ordering::Relaxed);
        assert_eq!(search("needle beta", DiffSearchOptions::default()), vec![1]);
        assert_eq!(
            reads.load(Ordering::Relaxed),
            2,
            "query refinement only checks previous matches"
        );
        reads.store(0, Ordering::Relaxed);
        assert_eq!(search("needle", DiffSearchOptions::default()), vec![0, 1]);
        assert_eq!(
            reads.load(Ordering::Relaxed),
            2,
            "backspacing reuses the broader cached query, preserving its matches"
        );
        for (query, options) in [
            ("alpha\nNeedle", DiffSearchOptions::default()),
            ("猫", DiffSearchOptions::default()),
            (
                "^Needle",
                DiffSearchOptions {
                    regex: true,
                    match_case: true,
                    ..Default::default()
                },
            ),
            (
                "alpha",
                DiffSearchOptions {
                    whole_word: true,
                    ..Default::default()
                },
            ),
        ] {
            let mut expected = Vec::new();
            collect_stream_match_visible_rows(
                texts.iter().enumerate(),
                &DiffSearchMatcher::new(query, options),
                &mut expected,
            );
            assert_eq!(search(query, options), expected);
        }
    }

    // `slice_text` returns "" for a range that splits a character, which made
    // both scanners skip the whole 32 KiB chunk the match was in.
    #[test]
    fn long_line_match_survives_a_chunk_ending_inside_a_character() {
        let mut line = "x".repeat(100);
        line.push_str("needle");
        line.push_str(&"x".repeat(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES - line.len() - 1));
        line.push('é');
        line.push_str(&"y".repeat(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES));
        assert!(!line.is_char_boundary(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES));
        let text: FileDiffLineText = line.into();

        let matcher = DiffSearchMatcher::new("NEEDLE", DiffSearchOptions::default());
        assert!(RowDocument::matches_text(&matcher, &text));
        let needle = AsciiCaseInsensitiveNeedle::new("NEEDLE").unwrap();
        assert!(resolved_output_line_ix_matches_query(&text, needle));
        // A match in the chunk after the split character is still found.
        let matcher = DiffSearchMatcher::new("xéy", DiffSearchOptions::default());
        assert!(RowDocument::matches_text(&matcher, &text));
    }

    #[test]
    fn background_search_cancellation_does_not_cache_partial_results() {
        let token = CancellationToken::new();
        let cancel = token.clone();
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = reads.clone();
        let rows = RowDocument {
            len: 100_000,
            columns: 1,
            text: Box::new(move |_, _| {
                if counter.fetch_add(1, Ordering::Relaxed) == 2 {
                    cancel.cancel();
                }
                Some("needle".into())
            }),
            wrapped: Vec::new(),
            streamed: false,
            previous: Mutex::new(VecDeque::new()),
        };
        let mut matcher = DiffSearchMatcher::new("needle", DiffSearchOptions::default());
        matcher.cancellation = Some(token);
        assert!(rows.search(&matcher).is_empty());
        assert_eq!(reads.load(Ordering::Relaxed), 3);
        assert!(rows.previous.lock().unwrap().is_empty());
    }
}

type RowText = Box<dyn Fn(usize, usize) -> Option<FileDiffLineText> + Send + Sync>;
type CustomSearch = Box<dyn Fn(&DiffSearchMatcher) -> Vec<usize> + Send + Sync>;

struct RowDocument {
    len: usize,
    columns: usize,
    text: RowText,
    wrapped: Vec<DiffWrapVisualRow>,
    streamed: bool,
    previous: Mutex<VecDeque<SearchCandidates>>,
}

struct SearchCandidates {
    query: String,
    options: DiffSearchOptions,
    rows: Vec<usize>,
}

const QUERY_CACHE_ENTRIES: usize = 8;
const QUERY_CACHE_MAX_MATCHES: usize = 16_384;
const QUERY_CACHE_MAX_QUERY_BYTES: usize = 4096;

enum DocumentSource {
    Rows(RowDocument),
    Editor(TextModelSnapshot),
    Custom(CustomSearch),
}

pub(in crate::view) struct SearchDocument(DocumentSource);

#[derive(Default)]
pub(super) struct SearchResult {
    pub(super) matches: Vec<usize>,
    pub(super) editor_ranges: Vec<Range<usize>>,
    pub(super) regex_error: Option<String>,
}

impl SearchDocument {
    pub(super) fn search(
        &self,
        query: &str,
        options: DiffSearchOptions,
        cancellation: CancellationToken,
    ) -> SearchResult {
        let mut matcher = DiffSearchMatcher::new(query, options);
        matcher.cancellation = Some(cancellation);
        let mut result = SearchResult {
            regex_error: matcher.regex_error().map(str::to_owned),
            ..Default::default()
        };
        if matcher.is_empty() || matcher.regex_error().is_some() || matcher.is_cancelled() {
            return result;
        }
        match &self.0 {
            DocumentSource::Editor(snapshot) => {
                result.editor_ranges =
                    file_editor_search_ranges(snapshot, &matcher, FILE_EDITOR_SEARCH_MAX_MATCHES);
                result.matches = result
                    .editor_ranges
                    .iter()
                    .map(|range| snapshot.row_for_offset(range.start))
                    .collect();
            }
            DocumentSource::Custom(search) => result.matches = search(&matcher),
            DocumentSource::Rows(rows) => result.matches = rows.search(&matcher),
        }
        result
    }
}

impl RowDocument {
    fn matches_text(matcher: &DiffSearchMatcher, text: &FileDiffLineText) -> bool {
        line_text_chunks_any(
            text,
            matcher.query().len().saturating_sub(1),
            || matcher.is_cancelled(),
            |chunk| matcher.is_match(chunk),
        )
    }

    fn visual_row(&self, source: usize, column: usize, offset: usize) -> usize {
        let first = self
            .wrapped
            .partition_point(|row| row.source_visible_ix < source);
        let mut boundary = None;
        for (ix, row) in self
            .wrapped
            .iter()
            .enumerate()
            .skip(first)
            .take_while(|(_, row)| row.source_visible_ix == source)
        {
            let range = if column == 0 {
                row.primary_range
            } else {
                row.secondary_range
            };
            if range.start <= offset && offset < range.end
                || range.start == range.end && offset == range.start
            {
                return ix;
            }
            if offset == range.end {
                boundary = Some(ix);
            }
        }
        boundary
            .or_else(|| {
                self.wrapped
                    .get(first)
                    .filter(|row| row.source_visible_ix == source)
                    .map(|_| first)
            })
            .unwrap_or(source)
    }

    fn search(&self, matcher: &DiffSearchMatcher) -> Vec<usize> {
        let mut out = Vec::new();
        if self.wrapped.is_empty() && matcher.can_use_ascii_case_insensitive_fast_path() {
            let mut previous = self.previous.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(candidates) = previous
                .iter()
                .filter(|entry| {
                    entry.options == matcher.options
                        && diff_search_query_reuse(&entry.query, matcher.query())
                            != DiffSearchQueryReuse::None
                })
                .min_by_key(|entry| entry.rows.len())
            {
                out.extend(
                    candidates
                        .rows
                        .iter()
                        .copied()
                        .take_while(|_| !matcher.is_cancelled())
                        .filter(|&ix| {
                            (0..self.columns).any(|column| {
                                (self.text)(ix, column)
                                    .is_some_and(|text| Self::matches_text(matcher, &text))
                            })
                        }),
                );
            } else {
                // An eager trigram index delayed the first debug search by
                // hundreds of milliseconds on large previews. Scan once and
                // retain small result sets instead. Keeping broader queries
                // makes both refinement and backspacing cheap.
                for ix in (0..self.len).take_while(|_| !matcher.is_cancelled()) {
                    if (0..self.columns).any(|column| {
                        (self.text)(ix, column)
                            .is_some_and(|text| Self::matches_text(matcher, &text))
                    }) {
                        out.push(ix);
                    }
                }
            }
            if matcher.is_cancelled() {
                return Vec::new();
            }
            out.sort_unstable();
            out.dedup();
            if out.len() <= QUERY_CACHE_MAX_MATCHES
                && matcher.query().len() <= QUERY_CACHE_MAX_QUERY_BYTES
            {
                previous.retain(|entry| {
                    entry.query != matcher.query() || entry.options != matcher.options
                });
                previous.push_front(SearchCandidates {
                    query: matcher.query().to_owned(),
                    options: matcher.options,
                    rows: out.clone(),
                });
                previous.truncate(QUERY_CACHE_ENTRIES);
            }
            return out;
        }
        for column in 0..self.columns {
            let rows = (0..self.len)
                .take_while(|_| !matcher.is_cancelled())
                .map(|ix| (ix, (self.text)(ix, column).unwrap_or_else(|| "".into())));
            if self.wrapped.is_empty() {
                if self.streamed {
                    collect_file_diff_line_text_stream_match_visible_rows(rows, matcher, &mut out);
                } else {
                    collect_stream_match_visible_rows(rows, matcher, &mut out);
                }
            } else {
                let mut offsets = Vec::new();
                collect_stream_match_row_offsets(rows, matcher, &mut offsets);
                out.extend(
                    offsets
                        .into_iter()
                        .map(|(ix, offset)| self.visual_row(ix, column, offset)),
                );
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl MainPaneView {
    pub(super) fn diff_search_document_key(&self) -> SearchDocumentKey {
        SearchDocumentKey {
            repo: self
                .active_repo()
                .map(|repo| (repo.id, repo.diff_state.diff_target_rev)),
            patch: (self.diff_cache_repo_id, self.diff_cache_rev),
            file: (
                self.file_diff_cache_repo_id,
                self.file_diff_cache_rev,
                self.file_diff_cache_seq,
            ),
            projection: (
                self.diff_visible_projection_rev,
                self.diff_view,
                self.diff_word_wrap,
                self.diff_wrap_visible_cache_key,
            ),
            preview: self.worktree_preview_content_rev,
            editor: self
                .file_editor_search_source
                .as_ref()
                .map(|s| (s.model_id(), s.revision())),
            markdown: self.file_markdown_preview_seq,
            markdown_wrap: self.markdown_preview_wrap.keys(),
            conflict: (
                self.conflict_resolver.conflict_rev,
                self.conflict_resolver.source_hash,
                self.conflict_resolver.resolver_pending_recompute_seq,
                self.conflict_resolver.view_mode,
                self.conflict_resolver.hide_resolved,
            ),
            surface: (
                self.is_file_editor_active(),
                self.is_file_preview_active(),
                self.rendered_markdown_preview_owns_view(),
                self.is_collapsed_diff_projection_active(),
            ),
            viewport: (
                f32::from(self.last_window_size.width).to_bits(),
                f32::from(self.last_window_size.height).to_bits(),
            ),
        }
    }

    pub(super) fn capture_search_document(&self) -> SearchDocument {
        if self.is_file_editor_active() {
            return SearchDocument(DocumentSource::Editor(
                self.file_editor_search_source.clone().unwrap_or_default(),
            ));
        }
        if self.rendered_markdown_preview_owns_view() {
            let documents: Vec<_> = self
                .markdown_search_surface()
                .into_iter()
                .flat_map(|surface| self.markdown_search_documents(surface))
                .map(|(list, doc)| {
                    (
                        doc.rows
                            .iter()
                            .map(|row| row.text.clone())
                            .collect::<Vec<_>>(),
                        list.and_then(|list| self.markdown_preview_wrap_plan(list))
                            .cloned(),
                    )
                })
                .collect();
            return SearchDocument(DocumentSource::Custom(Box::new(move |matcher| {
                let mut out = Vec::new();
                for (rows, plan) in &documents {
                    out.extend(
                        rows.iter()
                            .enumerate()
                            .take_while(|_| !matcher.is_cancelled())
                            .filter(|(_, text)| matcher.is_match(text.as_ref()))
                            .map(|(ix, _)| {
                                plan.as_ref().map_or(ix, |plan| plan.visual_ix_for_row(ix))
                            }),
                    );
                }
                out.sort_unstable();
                out.dedup();
                out
            })));
        }
        if self.active_conflict_target().is_some() {
            // Only search inputs are copied; syntax/image/layout caches remain
            // owned by the live view. Deferred line indexes stay shared.
            let source = &self.conflict_resolver;
            let snapshot = ConflictResolverUiState {
                view_mode: source.view_mode,
                marker_segments: source.marker_segments.clone(),
                mode_state: source.mode_state.clone(),
                three_way_text: source.three_way_text.clone(),
                three_way_line_starts: source.three_way_line_starts.clone(),
                three_way_aligned: source.three_way_aligned.clone(),
                ..Default::default()
            };
            return SearchDocument(DocumentSource::Custom(Box::new(move |matcher| {
                let context = ConflictResolverSearchContext::from_conflict_resolver(&snapshot);
                conflict_resolver_visible_match_indices_with_matcher(matcher, &context)
            })));
        }
        let (len, columns, text): (usize, usize, RowText) = if self.is_file_preview_active() {
            let starts = self.worktree_preview_line_starts.clone();
            let flags = self.worktree_preview_line_flags.clone();
            let text = self.worktree_preview_text.clone();
            let path = self.worktree_preview_source_path.clone().map(Arc::new);
            let source_len = self.worktree_preview_source_len;
            (
                self.worktree_preview_line_count().unwrap_or(0),
                1,
                Box::new(move |ix, _| {
                    let range = indexed_line_byte_range(&starts, source_len, ix)?;
                    if source_len > 0 && text.is_empty() {
                        let flags = flags.get(ix).copied().unwrap_or_default();
                        Some(FileDiffLineText::file_slice(
                            path.clone()?,
                            range,
                            preview_line_is_ascii_without_loading(flags),
                            preview_line_has_tabs_without_loading(flags),
                        ))
                    } else {
                        Some(worktree_preview_materialized_line_raw_text(&text, range))
                    }
                }),
            )
        } else {
            let file_view = self.is_file_diff_view_active();
            let view = self.diff_view;
            let indices = self.diff_visible_indices.clone();
            let map = self.diff_visible_inline_map.clone();
            let collapsed = self
                .is_collapsed_diff_projection_active()
                .then(|| self.collapsed_diff_visible_rows.clone());
            let headers = if collapsed.is_some() {
                self.collapsed_diff_header_display_cache.clone()
            } else {
                self.diff_header_display_cache.clone()
            };
            let patch = self.diff_row_provider.clone();
            let patch_rows = self.diff_cache.clone();
            let split = self.diff_split_row_provider.clone();
            let split_rows = self.diff_split_cache.clone();
            let file = self.file_diff_row_provider.clone();
            let file_rows = self.file_diff_cache_rows.clone();
            let inline = self.file_diff_inline_row_provider.clone();
            let inline_rows = self.file_diff_inline_cache.clone();
            let wrapped = self.diff_word_wrap;
            (
                self.diff_source_visible_len(),
                if view == DiffViewMode::Inline { 1 } else { 2 },
                Box::new(move |visible, column| {
                    let ix = if let Some(collapsed) = &collapsed {
                        let row = *collapsed.get(visible)?;
                        if let Some(header) = row.header_display_src_ix() {
                            return headers.get(&header).map(|text| text.as_ref().into());
                        }
                        row.row_ix()?
                    } else if let Some(map) = &map {
                        map.src_ix_for_visible_ix(visible)?
                    } else {
                        *indices.get(visible)?
                    };
                    let raw = if file_view {
                        if view == DiffViewMode::Inline {
                            if let Some(provider) = &inline {
                                provider.render_data(ix).map(|row| row.text)
                            } else {
                                inline_rows
                                    .get(ix)
                                    .map(crate::view::diff_utils::diff_content_line_text)
                            }
                        } else if let Some(provider) = &file {
                            provider
                                .split_row_texts(ix)
                                .and_then(|(left, right)| if column == 0 { left } else { right })
                        } else {
                            file_rows.get(ix).and_then(|row| {
                                if column == 0 {
                                    row.old.clone()
                                } else {
                                    row.new.clone()
                                }
                            })
                        }
                    } else if view == DiffViewMode::Inline {
                        if let Some(header) = headers.get(&ix) {
                            return Some(header.as_ref().into());
                        }
                        patch
                            .as_ref()
                            .and_then(|provider| provider.row(ix))
                            .or_else(|| patch_rows.get(ix).cloned())
                            .map(|row| row.text.into())
                    } else {
                        match split
                            .as_ref()
                            .and_then(|provider| provider.row(ix))
                            .or_else(|| split_rows.get(ix).cloned())?
                        {
                            PatchSplitRow::Raw { src_ix, .. } => {
                                if let Some(header) = headers.get(&src_ix) {
                                    return Some(header.as_ref().into());
                                }
                                patch
                                    .as_ref()
                                    .and_then(|provider| provider.row(src_ix))
                                    .or_else(|| patch_rows.get(src_ix).cloned())
                                    .map(|row| row.text.into())
                            }
                            PatchSplitRow::Aligned { row, .. } => {
                                if column == 0 {
                                    row.old
                                } else {
                                    row.new
                                }
                            }
                        }
                    }?;
                    if wrapped || view == DiffViewMode::Split || !file_view {
                        Some(expand_tabs_to_string(raw.as_ref()).into())
                    } else {
                        Some(raw)
                    }
                }),
            )
        };
        // Preview search uses source-line navigation, including when wrapped.
        let wrapped = if self.diff_word_wrap && !self.is_file_preview_active() {
            self.diff_wrap_visible_rows.clone()
        } else {
            Vec::new()
        };
        let streamed = self.is_file_preview_active()
            && self.worktree_preview_source_len > 0
            && self.worktree_preview_text.is_empty()
            || self.is_file_diff_view_active() && self.diff_view == DiffViewMode::Inline;
        SearchDocument(DocumentSource::Rows(RowDocument {
            len,
            columns,
            text,
            wrapped,
            streamed,
            previous: Mutex::new(VecDeque::new()),
        }))
    }

    pub(super) fn diff_search_start_background(&mut self, cx: &mut gpui::Context<Self>) {
        if self.diff_search_worker_running || self.diff_search_pending_previous_query.is_none() {
            return;
        }
        self.diff_search_pending_previous_query.take();
        if !self.diff_search_active {
            return;
        }
        if self.diff_search_query.is_empty() {
            self.diff_search_regex_error = None;
            self.diff_search_matches.clear();
            self.diff_search_match_ix = None;
            self.file_editor_search_clear();
            self.diff_search_probe_render = self.diff_search_probe_action;
            crate::ui_probe::action_phase(
                self.diff_search_probe_action,
                "applied",
                || serde_json::json!({"revision":self.diff_search_debounce_seq,"matches":0}),
            );
            cx.notify();
            return;
        }
        if !self.is_file_preview_active() && self.active_conflict_target().is_none() {
            self.ensure_diff_visible_indices();
        }
        let key = self.diff_search_document_key();
        if self
            .diff_search_document
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key)
        {
            self.diff_search_document =
                Some((key.clone(), Arc::new(self.capture_search_document())));
        }
        let document = self.diff_search_document.as_ref().unwrap().1.clone();
        let cancellation = CancellationToken::new();
        self.diff_search_cancellation = Some(cancellation.clone());
        self.diff_search_worker_running = true;
        let sequence = self.diff_search_debounce_seq;
        self.diff_search_worker_seq = sequence;
        let action = self.diff_search_probe_action;
        let finalize = self.diff_search_pending_finalize;
        let query = self.diff_search_query.clone();
        let options = self.diff_search_options;
        let work = cx
            .background_executor()
            .spawn(async move { document.search(query.as_ref(), options, cancellation) });
        cx.spawn(async move |view, cx| {
            let result = work.await;
            let _ = view.update(cx, |this, cx| {
                this.diff_search_worker_running = false;
                this.diff_search_cancellation = None;
                // Cancellation saves work but can race with completion. Check
                // both query sequence and document projection before publishing;
                // only this UI callback may replace the visible match list.
                if this.diff_search_active && this.diff_search_debounce_seq == sequence && this.diff_search_document_key() == key {
                    this.diff_search_matches = result.matches;
                    this.diff_search_probe_render = action;
                    crate::ui_probe::action_phase(action, "applied", || serde_json::json!({"revision":sequence, "matches":this.diff_search_matches.len(),
                        "query_bytes":this.diff_search_query.len(), "target":this.rendered_diff_target().map(|target| format!("{target:?}"))}));
                    this.diff_search_regex_error = result.regex_error.map(Into::into);
                    this.file_editor_search_matches = result.editor_ranges;
                    this.file_editor_search_rev = this.file_editor_search_rev.wrapping_add(1);
                    this.diff_search_finalize_matches(finalize);
                    let step = std::mem::take(&mut this.diff_search_pending_navigation);
                    let len = this.diff_search_matches.len();
                    if len > 0 && step != 0 {
                        let current = this.diff_search_match_ix.unwrap_or(0).min(len - 1);
                        let offset = step.rem_euclid(len as isize) as usize;
                        let ix = (current + offset) % len;
                        this.diff_search_match_ix = Some(ix);
                        this.diff_search_scroll_to_visible_ix(this.diff_search_matches[ix]);
                    }
                    cx.notify();
                } else if this.diff_search_active && this.diff_search_debounce_seq == sequence {
                    // A resize/projection change can happen while the worker
                    // runs without generating another keystroke.
                    this.diff_search_pending_previous_query = Some(this.diff_search_query.clone());
                }
                this.diff_search_start_background(cx);
            });
        }).detach();
    }
}
