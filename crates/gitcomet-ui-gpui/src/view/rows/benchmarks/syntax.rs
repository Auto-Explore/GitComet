use super::diff_text::{
    benchmark_diff_syntax_cache_drop_payload_timed_step,
    benchmark_diff_syntax_cache_replacement_drop_step,
    benchmark_diff_syntax_prepared_cache_contains_document,
    benchmark_diff_syntax_prepared_cache_metrics,
    benchmark_diff_syntax_prepared_loaded_chunk_count,
    benchmark_flush_diff_syntax_deferred_drop_queue,
    benchmark_reset_diff_syntax_prepared_cache_metrics,
    prepare_diff_syntax_document_in_background_text,
};
use super::*;
use crate::view::markdown_preview::{self, MarkdownPreviewDiff, MarkdownPreviewDocument};

pub struct FileDiffSyntaxPrepareFixture {
    lines: Vec<String>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    budget: DiffSyntaxBudget,
}

impl FileDiffSyntaxPrepareFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
        }
    }

    pub fn new_query_stress(lines: usize, line_bytes: usize, nesting_depth: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_nested_query_stress_lines(lines, line_bytes, nesting_depth),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
        }
    }

    pub fn prewarm(&self) {
        let _ = self.prepare_document(&self.lines);
    }

    pub fn run_prepare_cold(&self, nonce: u64) -> u64 {
        let lines = self
            .lines
            .iter()
            .enumerate()
            .map(|(ix, line)| format!("{line} // cold_{nonce}_{ix}"))
            .collect::<Vec<_>>();
        let document = self.prepare_document(&lines);
        self.hash_prepared(&lines, document)
    }

    pub fn run_prepare_warm(&self) -> u64 {
        let document = self.prepare_document(&self.lines);
        self.hash_prepared(&self.lines, document)
    }

    pub fn run_prepared_syntax_multidoc_cache_hit_rate_step(&self, docs: usize, nonce: u64) -> u64 {
        let docs = docs.clamp(3, 6);
        benchmark_reset_diff_syntax_prepared_cache_metrics();

        let mut prepared = Vec::with_capacity(docs);
        for doc_ix in 0..docs {
            let lines = self
                .lines
                .iter()
                .enumerate()
                .map(|(line_ix, line)| format!("{line} // multidoc_{nonce}_{doc_ix}_{line_ix}"))
                .collect::<Vec<_>>();
            if let Some(document) = self.prepare_document(&lines) {
                prepared.push((lines, document));
            }
        }

        for (lines, document) in &prepared {
            let _ = self.hash_prepared_line(lines, Some(*document), 0);
        }
        for _ in 0..4 {
            for (lines, document) in &prepared {
                let _ = self.hash_prepared_line(lines, Some(*document), 0);
            }
        }

        let metrics = benchmark_diff_syntax_prepared_cache_metrics();
        let total = metrics.hit.saturating_add(metrics.miss);
        let hit_rate_per_mille = if total == 0 {
            0
        } else {
            metrics.hit.saturating_mul(1000) / total
        };

        let mut h = FxHasher::default();
        prepared.len().hash(&mut h);
        metrics.hit.hash(&mut h);
        metrics.miss.hash(&mut h);
        metrics.evict.hash(&mut h);
        metrics.chunk_build_ms.hash(&mut h);
        hit_rate_per_mille.hash(&mut h);
        h.finish()
    }

    pub fn run_prepared_syntax_chunk_miss_cost_step(&self, nonce: u64) -> Duration {
        let lines = self
            .lines
            .iter()
            .enumerate()
            .map(|(ix, line)| {
                if ix == 0 {
                    format!("{line} // chunk_miss_{nonce}")
                } else {
                    line.clone()
                }
            })
            .collect::<Vec<_>>();
        let Some(document) = self.prepare_document(&lines) else {
            return Duration::ZERO;
        };

        benchmark_reset_diff_syntax_prepared_cache_metrics();
        let line_count = lines.len().max(1);
        let chunk_rows = 64usize;
        let chunk_count = line_count.div_ceil(chunk_rows).max(1);
        let chunk_ix = (nonce as usize) % chunk_count;
        let line_ix = chunk_ix
            .saturating_mul(chunk_rows)
            .min(line_count.saturating_sub(1));

        let start = std::time::Instant::now();
        let _ = self.hash_prepared_line(&lines, Some(document), line_ix);
        let elapsed = start.elapsed();

        let metrics = benchmark_diff_syntax_prepared_cache_metrics();
        let _loaded_chunks = benchmark_diff_syntax_prepared_loaded_chunk_count(document);
        let _is_cached = benchmark_diff_syntax_prepared_cache_contains_document(document);
        if metrics.miss == 0 {
            return Duration::ZERO.max(elapsed);
        }
        elapsed
    }

    pub(super) fn prepare_document(
        &self,
        lines: &[String],
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        let text = lines.join("\n");
        prepare_bench_diff_syntax_document(self.language, self.budget, text.as_str(), None)
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> &[String] {
        &self.lines
    }

    fn hash_prepared(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        self.hash_prepared_line(lines, document, 0)
    }

    fn hash_prepared_line(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
        line_ix: usize,
    ) -> u64 {
        let line_ix = line_ix.min(lines.len().saturating_sub(1));
        let text = lines.get(line_ix).map(String::as_str).unwrap_or("");
        let styled =
            super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                self.theme,
                text,
                &[],
                "",
                super::diff_text::DiffSyntaxConfig {
                    language: Some(self.language),
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                super::diff_text::PreparedDiffSyntaxLine { document, line_ix },
            )
            .into_inner();

        let mut h = FxHasher::default();
        lines.len().hash(&mut h);
        line_ix.hash(&mut h);
        styled.text_hash.hash(&mut h);
        styled.highlights_hash.hash(&mut h);
        h.finish()
    }
}

pub struct FileDiffSyntaxReparseFixture {
    lines: Vec<String>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    budget: DiffSyntaxBudget,
    nonce: u64,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl FileDiffSyntaxReparseFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme: AppTheme::zed_ayu_dark(),
            budget: DiffSyntaxBudget::default(),
            nonce: 0,
            prepared_document: None,
        }
    }

    pub fn run_small_edit_step(&mut self) -> u64 {
        self.ensure_prepared_document();
        let mut next_lines = self.lines.clone();
        if next_lines.is_empty() {
            next_lines.push(String::new());
        }
        let line_ix = (self.nonce as usize) % next_lines.len();
        let marker = format!(" tiny_reparse_{}", self.nonce);
        next_lines[line_ix].push_str(marker.as_str());
        self.nonce = self.nonce.wrapping_add(1);

        let next_document = self.prepare_document_with_reuse(&next_lines, self.prepared_document);
        if next_document.is_some() {
            self.lines = next_lines;
            self.prepared_document = next_document;
        }

        self.hash_prepared(&self.lines, self.prepared_document)
    }

    pub fn run_large_edit_step(&mut self) -> u64 {
        self.ensure_prepared_document();
        let mut next_lines = self.lines.clone();
        if next_lines.is_empty() {
            next_lines.push(String::new());
        }

        let total_lines = next_lines.len();
        let changed_lines = total_lines.saturating_mul(3) / 5;
        let changed_lines = changed_lines.max(1).min(total_lines);
        let start = if total_lines == 0 {
            0
        } else {
            (self.nonce as usize).wrapping_mul(13) % total_lines
        };
        for offset in 0..changed_lines {
            let ix = (start + offset) % total_lines;
            next_lines[ix] = format!(
                "pub fn fallback_edit_{}_{offset}() {{ let value = {}; }}",
                self.nonce,
                offset.wrapping_mul(17)
            );
        }
        self.nonce = self.nonce.wrapping_add(1);

        let next_document = self.prepare_document_with_reuse(&next_lines, self.prepared_document);
        if next_document.is_some() {
            self.lines = next_lines;
            self.prepared_document = next_document;
        }

        self.hash_prepared(&self.lines, self.prepared_document)
    }

    fn ensure_prepared_document(&mut self) {
        if self.prepared_document.is_some() {
            return;
        }
        self.prepared_document = self.prepare_document_with_reuse(&self.lines, None);
    }

    fn prepare_document_with_reuse(
        &self,
        lines: &[String],
        old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        let text = lines.join("\n");
        prepare_bench_diff_syntax_document(self.language, self.budget, text.as_str(), old_document)
    }

    fn hash_prepared(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        let text = lines.first().map(String::as_str).unwrap_or("");
        let styled =
            super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                self.theme,
                text,
                &[],
                "",
                super::diff_text::DiffSyntaxConfig {
                    language: Some(self.language),
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                super::diff_text::PreparedDiffSyntaxLine {
                    document,
                    line_ix: 0,
                },
            )
            .into_inner();

        let mut h = FxHasher::default();
        lines.len().hash(&mut h);
        styled.text_hash.hash(&mut h);
        styled.highlights_hash.hash(&mut h);
        h.finish()
    }
}

pub struct FileDiffInlineSyntaxProjectionFixture {
    inline_rows: Vec<AnnotatedDiffLine>,
    inline_word_highlights: Vec<Option<Vec<Range<usize>>>>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    new_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl FileDiffInlineSyntaxProjectionFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        let generated_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(32));

        let mut old_lines = Vec::with_capacity(generated_lines.len());
        let mut new_lines = Vec::with_capacity(generated_lines.len());
        let mut inline_rows = Vec::with_capacity(generated_lines.len().saturating_mul(2));
        let mut inline_word_highlights =
            Vec::with_capacity(generated_lines.len().saturating_mul(2));
        let mut old_line_no = 1u32;
        let mut new_line_no = 1u32;

        for (slot_ix, base_line) in generated_lines.into_iter().enumerate() {
            match slot_ix % 9 {
                0 => {
                    let old_line = format!("{base_line} // inline_remove_{slot_ix}");
                    old_lines.push(old_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Remove,
                        text: format!("-{old_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: None,
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                }
                1 => {
                    let new_line = format!("{base_line} // inline_add_{slot_ix}");
                    new_lines.push(new_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Add,
                        text: format!("+{new_line}").into(),
                        old_line: None,
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    new_line_no = new_line_no.saturating_add(1);
                }
                2 => {
                    let old_line = format!("{base_line} // inline_before_{slot_ix}");
                    let new_line = format!("{base_line} // inline_after_{slot_ix}");
                    old_lines.push(old_line.clone());
                    new_lines.push(new_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Remove,
                        text: format!("-{old_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: None,
                    });
                    inline_word_highlights.push(None);
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Add,
                        text: format!("+{new_line}").into(),
                        old_line: None,
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                    new_line_no = new_line_no.saturating_add(1);
                }
                _ => {
                    old_lines.push(base_line.clone());
                    new_lines.push(base_line.clone());
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Context,
                        text: format!(" {base_line}").into(),
                        old_line: Some(old_line_no),
                        new_line: Some(new_line_no),
                    });
                    inline_word_highlights.push(None);
                    old_line_no = old_line_no.saturating_add(1);
                    new_line_no = new_line_no.saturating_add(1);
                }
            }
        }

        let budget = DiffSyntaxBudget::default();
        let old_text = old_lines.join("\n");
        let old_document =
            prepare_bench_diff_syntax_document(language, budget, old_text.as_str(), None);
        let new_text = new_lines.join("\n");
        let new_document =
            prepare_bench_diff_syntax_document(language, budget, new_text.as_str(), None);

        Self {
            inline_rows,
            inline_word_highlights,
            language,
            theme: AppTheme::zed_ayu_dark(),
            old_document,
            new_document,
        }
    }

    pub fn run_window_pending_step(&self, start: usize, window: usize) -> u64 {
        self.hash_window_step(start, window).0
    }

    pub fn run_window_step(&self, start: usize, window: usize) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let (hash, pending) = self.hash_window_step(start, window);
            if !pending {
                return hash;
            }
            if std::time::Instant::now() >= deadline {
                return hash;
            }

            let mut applied = 0usize;
            if let Some(document) = self.old_document {
                applied = applied.saturating_add(
                    drain_completed_prepared_diff_syntax_chunk_builds_for_document(document),
                );
            }
            if let Some(document) = self.new_document {
                applied = applied.saturating_add(
                    drain_completed_prepared_diff_syntax_chunk_builds_for_document(document),
                );
            }
            if applied == 0 && self.has_pending_chunks() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    pub fn prime_window(&self, window: usize) {
        let _ = self.run_window_step(0, window);
    }

    pub fn next_start_row(&self, start: usize, window: usize) -> usize {
        let step = (window.max(1) / 2).saturating_add(1);
        start.wrapping_add(step) % self.inline_rows.len().max(1)
    }

    #[cfg(test)]
    pub(super) fn visible_rows(&self) -> usize {
        self.inline_rows.len()
    }

    fn has_pending_chunks(&self) -> bool {
        self.old_document
            .is_some_and(has_pending_prepared_diff_syntax_chunk_builds_for_document)
            || self
                .new_document
                .is_some_and(has_pending_prepared_diff_syntax_chunk_builds_for_document)
    }

    fn projected_syntax_line(
        &self,
        line: &AnnotatedDiffLine,
    ) -> super::diff_text::PreparedDiffSyntaxLine {
        super::diff_text::prepared_diff_syntax_line_for_inline_diff_row(
            self.old_document,
            self.new_document,
            line,
        )
    }

    fn hash_window_step(&self, start: usize, window: usize) -> (u64, bool) {
        if self.inline_rows.is_empty() || window == 0 {
            return (0, false);
        }

        let start = start % self.inline_rows.len();
        let end = (start + window).min(self.inline_rows.len());
        let mut pending = false;
        let mut h = FxHasher::default();
        for row_ix in start..end {
            let Some(line) = self.inline_rows.get(row_ix) else {
                continue;
            };
            let word_ranges = self
                .inline_word_highlights
                .get(row_ix)
                .and_then(|ranges| ranges.as_deref())
                .unwrap_or(&[]);
            let projected = self.projected_syntax_line(line);
            let syntax_mode =
                super::diff_text::syntax_mode_for_prepared_document(projected.document);
            let word_color = match line.kind {
                DiffLineKind::Add => Some(self.theme.colors.success),
                DiffLineKind::Remove => Some(self.theme.colors.danger),
                _ => None,
            };
            let (styled, is_pending) =
                super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                    self.theme,
                    diff_content_text(line),
                    word_ranges,
                    "",
                    super::diff_text::DiffSyntaxConfig {
                        language: Some(self.language),
                        mode: syntax_mode,
                    },
                    word_color,
                    projected,
                )
                .into_parts();
            pending |= is_pending;
            row_ix.hash(&mut h);
            is_pending.hash(&mut h);
            styled.text_hash.hash(&mut h);
            styled.highlights_hash.hash(&mut h);
        }
        self.inline_rows.len().hash(&mut h);
        (h.finish(), pending)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LargeHtmlSyntaxSource {
    External,
    Synthetic,
}

pub struct LargeHtmlSyntaxFixture {
    source: LargeHtmlSyntaxSource,
    text: Arc<str>,
    line_starts: Arc<[usize]>,
    line_count: usize,
    theme: AppTheme,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
}

impl LargeHtmlSyntaxFixture {
    pub fn new(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
    ) -> Self {
        Self::new_internal(fixture_path, synthetic_lines, synthetic_line_bytes, false)
    }

    pub fn new_prewarmed(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
    ) -> Self {
        Self::new_internal(fixture_path, synthetic_lines, synthetic_line_bytes, true)
    }

    fn new_internal(
        fixture_path: Option<&str>,
        synthetic_lines: usize,
        synthetic_line_bytes: usize,
        prewarm_document: bool,
    ) -> Self {
        let (source, text) = load_large_html_bench_text(fixture_path).unwrap_or_else(|| {
            (
                LargeHtmlSyntaxSource::Synthetic,
                build_synthetic_large_html_text(synthetic_lines, synthetic_line_bytes),
            )
        });
        let text: Arc<str> = Arc::from(text);
        let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));
        let line_count = line_starts.len().max(1);
        let prepared_document = prewarm_document
            .then(|| Self::prepare_document(text.as_ref()))
            .flatten();

        Self {
            source,
            text,
            line_starts,
            line_count,
            theme: AppTheme::zed_ayu_dark(),
            prepared_document,
        }
    }

    pub fn source_label(&self) -> &'static str {
        match self.source {
            LargeHtmlSyntaxSource::External => "external_html_fixture",
            LargeHtmlSyntaxSource::Synthetic => "synthetic_html_fixture",
        }
    }

    pub fn run_background_prepare_step(&self) -> u64 {
        let prepared = prepare_diff_syntax_document_in_background_text(
            DiffSyntaxLanguage::Html,
            DiffSyntaxMode::Auto,
            self.text.as_ref().to_owned().into(),
            Arc::clone(&self.line_starts),
        );

        let mut h = FxHasher::default();
        self.text.len().hash(&mut h);
        self.line_count.hash(&mut h);
        self.source_label().hash(&mut h);
        prepared.is_some().hash(&mut h);
        h.finish()
    }

    pub fn run_visible_window_pending_step(&self, start_line: usize, window_lines: usize) -> u64 {
        let Some(document) = self.prepared_document_handle() else {
            return 0;
        };
        let Some(result) =
            self.request_visible_window_for_lines(document, start_line, window_lines)
        else {
            return 0;
        };
        self.hash_visible_window_result(start_line, window_lines, &result)
    }

    pub fn run_visible_window_step(&self, start_line: usize, window_lines: usize) -> u64 {
        let Some(document) = self.prepared_document_handle() else {
            return 0;
        };
        let Some(result) =
            self.request_visible_window_until_ready(document, start_line, window_lines)
        else {
            return 0;
        };
        self.hash_visible_window_result(start_line, window_lines, &result)
    }

    pub fn prime_visible_window(&self, window_lines: usize) {
        let _ = self.run_visible_window_step(0, window_lines);
    }

    pub fn next_start_line(&self, start_line: usize, window_lines: usize) -> usize {
        let step = (window_lines.max(1) / 2).saturating_add(1);
        start_line.wrapping_add(step) % self.line_count.max(1)
    }

    #[cfg(test)]
    pub(super) fn line_count(&self) -> usize {
        self.line_count
    }

    pub(super) fn prepared_document_handle(
        &self,
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        self.prepared_document
            .or_else(|| Self::prepare_document(self.text.as_ref()))
    }

    fn prepare_document(text: &str) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        prepare_bench_diff_syntax_document(
            DiffSyntaxLanguage::Html,
            DiffSyntaxBudget::default(),
            text,
            None,
        )
    }

    fn visible_window_byte_range(&self, start_line: usize, window_lines: usize) -> Range<usize> {
        if self.line_count == 0 || window_lines == 0 {
            return 0..0;
        }

        let start_line = start_line % self.line_count.max(1);
        let end_line = (start_line + window_lines.max(1)).min(self.line_count);
        let text_len = self.text.len();
        let start = self
            .line_starts
            .get(start_line)
            .copied()
            .unwrap_or(text_len)
            .min(text_len);
        let end = self
            .line_starts
            .get(end_line)
            .copied()
            .unwrap_or(text_len)
            .min(text_len)
            .max(start);
        start..end
    }

    pub(super) fn request_visible_window_for_lines(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        start_line: usize,
        window_lines: usize,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        let byte_range = self.visible_window_byte_range(start_line, window_lines);
        self.request_visible_window(document, byte_range)
    }

    fn request_visible_window_until_ready(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        start_line: usize,
        window_lines: usize,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        let byte_range = self.visible_window_byte_range(start_line, window_lines);
        let mut result = self.request_visible_window(document, byte_range.clone());
        for _ in 0..64 {
            if match result.as_ref() {
                None => true,
                Some(highlights) => !highlights.pending,
            } {
                break;
            }

            let applied = drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if applied == 0 && has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
            {
                std::thread::yield_now();
            }
            result = self.request_visible_window(document, byte_range.clone());
        }
        result
    }

    fn request_visible_window(
        &self,
        document: super::diff_text::PreparedDiffSyntaxDocument,
        byte_range: Range<usize>,
    ) -> Option<super::diff_text::PreparedDocumentByteRangeHighlights> {
        super::diff_text::request_syntax_highlights_for_prepared_document_byte_range(
            self.theme,
            self.text.as_ref(),
            self.line_starts.as_ref(),
            document,
            DiffSyntaxLanguage::Html,
            byte_range,
        )
    }

    fn hash_visible_window_result(
        &self,
        start_line: usize,
        window_lines: usize,
        result: &super::diff_text::PreparedDocumentByteRangeHighlights,
    ) -> u64 {
        let mut h = FxHasher::default();
        start_line.hash(&mut h);
        window_lines.hash(&mut h);
        result.pending.hash(&mut h);
        result.highlights.len().hash(&mut h);
        for (range, _style) in result.highlights.iter().take(256) {
            range.start.hash(&mut h);
            range.end.hash(&mut h);
        }
        h.finish()
    }
}

pub struct FileDiffSyntaxCacheDropFixture {
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
}

impl FileDiffSyntaxCacheDropFixture {
    pub fn new(lines: usize, tokens_per_line: usize, replacements: usize) -> Self {
        Self {
            lines: lines.max(1),
            tokens_per_line: tokens_per_line.max(1),
            replacements: replacements.max(1),
        }
    }

    pub fn run_deferred_drop_step(&self) -> u64 {
        benchmark_diff_syntax_cache_replacement_drop_step(
            self.lines,
            self.tokens_per_line,
            self.replacements,
            true,
        )
    }

    pub fn run_inline_drop_control_step(&self) -> u64 {
        benchmark_diff_syntax_cache_replacement_drop_step(
            self.lines,
            self.tokens_per_line,
            self.replacements,
            false,
        )
    }

    pub fn run_deferred_drop_timed_step(&self, seed: usize) -> Duration {
        let mut total = Duration::ZERO;
        for step in 0..self.replacements {
            total = total.saturating_add(benchmark_diff_syntax_cache_drop_payload_timed_step(
                self.lines,
                self.tokens_per_line,
                seed.wrapping_add(step),
                true,
            ));
        }
        total
    }

    pub fn run_inline_drop_control_timed_step(&self, seed: usize) -> Duration {
        let mut total = Duration::ZERO;
        for step in 0..self.replacements {
            total = total.saturating_add(benchmark_diff_syntax_cache_drop_payload_timed_step(
                self.lines,
                self.tokens_per_line,
                seed.wrapping_add(step),
                false,
            ));
        }
        total
    }

    pub fn flush_deferred_drop_queue(&self) -> bool {
        benchmark_flush_diff_syntax_deferred_drop_queue()
    }
}

pub struct WorktreePreviewRenderFixture {
    lines: Vec<String>,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    theme: AppTheme,
}

impl WorktreePreviewRenderFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let generated_lines = build_synthetic_source_lines(lines, line_bytes);
        let language = diff_syntax_language_for_path("src/lib.rs");
        let syntax_mode = DiffSyntaxMode::Auto;
        let generated_text = generated_lines.join("\n");
        let prepared_document = language.and_then(|language| {
            prepare_bench_diff_syntax_document(
                language,
                DiffSyntaxBudget::default(),
                &generated_text,
                None,
            )
        });

        Self {
            lines: generated_lines,
            language,
            syntax_mode,
            prepared_document,
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    pub fn run_cached_lookup_step(&self, start: usize, window: usize) -> u64 {
        self.hash_window(start, window, self.prepared_document)
    }

    pub fn run_render_time_prepare_step(&self, start: usize, window: usize) -> u64 {
        let text = self.lines.join("\n");
        let prepared_document = self.language.and_then(|language| {
            prepare_bench_diff_syntax_document(
                language,
                DiffSyntaxBudget::default(),
                text.as_str(),
                None,
            )
        });
        self.hash_window(start, window, prepared_document)
    }

    fn hash_window(
        &self,
        start: usize,
        window: usize,
        prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        if self.lines.is_empty() || window == 0 {
            return 0;
        }

        let start = start % self.lines.len();
        let end = (start + window).min(self.lines.len());
        let mut h = FxHasher::default();
        for line_ix in start..end {
            let line = self.lines.get(line_ix).map(String::as_str).unwrap_or("");
            let styled =
                super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                    self.theme,
                    line,
                    &[],
                    "",
                    super::diff_text::DiffSyntaxConfig {
                        language: self.language,
                        mode: self.syntax_mode,
                    },
                    None,
                    super::diff_text::PreparedDiffSyntaxLine {
                        document: prepared_document,
                        line_ix,
                    },
                )
                .into_inner();
            line_ix.hash(&mut h);
            styled.text_hash.hash(&mut h);
            styled.highlights_hash.hash(&mut h);
        }
        h.finish()
    }

    #[cfg(test)]
    pub(super) fn syntax_mode(&self) -> DiffSyntaxMode {
        self.syntax_mode
    }

    #[cfg(test)]
    pub(super) fn has_prepared_document(&self) -> bool {
        self.prepared_document.is_some()
    }
}

pub struct MarkdownPreviewFixture {
    single_source: String,
    old_source: String,
    new_source: String,
    single_document: MarkdownPreviewDocument,
    diff_preview: MarkdownPreviewDiff,
    theme: AppTheme,
}

impl MarkdownPreviewFixture {
    pub fn new(sections: usize, line_bytes: usize) -> Self {
        let sections = sections.max(1);
        let line_bytes = line_bytes.max(48);
        let single_source = build_synthetic_markdown_document(sections, line_bytes, "single");
        let old_source = build_synthetic_markdown_document(sections, line_bytes, "before");
        let new_source = build_synthetic_markdown_document(sections, line_bytes, "after");
        let single_document = markdown_preview::parse_markdown(&single_source)
            .expect("synthetic markdown benchmark fixture should stay within preview limits");
        let diff_preview = markdown_preview::build_markdown_diff_preview(&old_source, &new_source)
            .expect("synthetic markdown diff benchmark fixture should stay within preview limits");

        Self {
            single_source,
            old_source,
            new_source,
            single_document,
            diff_preview,
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    pub fn run_parse_single_step(&self) -> u64 {
        let Some(document) = markdown_preview::parse_markdown(&self.single_source) else {
            return 0;
        };
        hash_markdown_preview_document(&document)
    }

    pub fn run_parse_diff_step(&self) -> u64 {
        let Some(preview) =
            markdown_preview::build_markdown_diff_preview(&self.old_source, &self.new_source)
        else {
            return 0;
        };
        let mut h = FxHasher::default();
        hash_markdown_preview_document_into(&preview.old, &mut h);
        hash_markdown_preview_document_into(&preview.new, &mut h);
        h.finish()
    }

    pub fn run_render_single_step(&self, start: usize, window: usize) -> u64 {
        self.hash_render_window(&self.single_document, start, window)
    }

    pub fn run_render_diff_step(&self, start: usize, window: usize) -> u64 {
        if window == 0 {
            return 0;
        }

        let left = self.render_window(&self.diff_preview.old, start, window);
        let right = self.render_window(&self.diff_preview.new, start, window);

        let mut h = FxHasher::default();
        start.hash(&mut h);
        window.hash(&mut h);
        std::hint::black_box(left).len().hash(&mut h);
        std::hint::black_box(right).len().hash(&mut h);
        h.finish()
    }

    fn hash_render_window(
        &self,
        document: &MarkdownPreviewDocument,
        start: usize,
        window: usize,
    ) -> u64 {
        if window == 0 {
            return 0;
        }

        let rows = self.render_window(document, start, window);
        let mut h = FxHasher::default();
        start.hash(&mut h);
        window.hash(&mut h);
        std::hint::black_box(rows).len().hash(&mut h);
        h.finish()
    }

    fn render_window(
        &self,
        document: &MarkdownPreviewDocument,
        start: usize,
        window: usize,
    ) -> Vec<AnyElement> {
        if document.rows.is_empty() || window == 0 {
            return Vec::new();
        }

        let start = start % document.rows.len();
        let end = (start + window).min(document.rows.len());
        super::history::render_markdown_preview_document_rows(
            self.theme,
            document,
            start..end,
            None,
            px(0.0),
            "benchmark_markdown_preview",
            None,
            None,
            DiffTextRegion::Inline,
        )
    }
}

fn load_large_html_bench_text(
    fixture_path: Option<&str>,
) -> Option<(LargeHtmlSyntaxSource, String)> {
    let path = fixture_path?.trim();
    if path.is_empty() {
        return None;
    }

    let text = std::fs::read_to_string(path).ok()?;
    if text.is_empty() {
        return None;
    }

    Some((LargeHtmlSyntaxSource::External, text))
}

fn build_synthetic_large_html_text(line_count: usize, target_line_bytes: usize) -> String {
    let line_count = line_count.max(12);
    let target_line_bytes = target_line_bytes.max(96);
    let mut lines = Vec::with_capacity(line_count);

    lines.push("<!doctype html>".to_string());
    lines.push("<html lang=\"en\">".to_string());
    lines.push("<head>".to_string());
    lines.push("<meta charset=\"utf-8\">".to_string());
    lines.push("<title>GitComet Synthetic HTML Fixture</title>".to_string());
    lines.push("<style>".to_string());
    lines.push(
        ".fixture-root { color: #222; background: linear-gradient(90deg, #fff, #f5f5f5); }"
            .to_string(),
    );
    lines.push("</style>".to_string());
    lines.push("</head>".to_string());
    lines.push("<body class=\"fixture-root\">".to_string());

    let reserved_suffix_lines = 2usize;
    let body_lines = line_count.saturating_sub(lines.len().saturating_add(reserved_suffix_lines));
    for ix in 0..body_lines {
        let mut line = match ix % 8 {
            0 => format!(
                r#"<style>.row-{ix} {{ color: rgb({r}, {g}, {b}); padding: {pad}px; }}</style>"#,
                r = (ix * 13) % 255,
                g = (ix * 29) % 255,
                b = (ix * 47) % 255,
                pad = (ix % 9) + 2,
            ),
            1 => format!(
                r#"<script>const card{ix} = {ix}; function bump{ix}() {{ return card{ix} + 1; }}</script>"#
            ),
            2 => format!(
                r#"<div class="row row-{ix}" data-row="{ix}" style="color: rgb({r}, {g}, {b}); background: linear-gradient(90deg, #fff, #eee);" onclick="const next = {ix}; return next + 1;">card {ix}</div>"#,
                r = (ix * 7) % 255,
                g = (ix * 17) % 255,
                b = (ix * 23) % 255,
            ),
            3 => format!(
                r#"<section id="panel-{ix}"><h2>Panel {ix}</h2><p>row {ix} content for syntax benchmarking</p></section>"#
            ),
            4 => {
                format!(r#"<!-- html comment {ix} with repeated tokens for benchmark coverage -->"#)
            }
            5 => {
                format!(r#"<template><span class="slot-{ix}">{{{{value_{ix}}}}}</span></template>"#)
            }
            6 => format!(
                r#"<svg viewBox="0 0 10 10"><path d="M0 0 L10 {y}" stroke="currentColor" /></svg>"#,
                y = (ix % 9) + 1,
            ),
            _ => format!(
                r#"<article data-kind="bench-{ix}" aria-label="row {ix}"><a href="/items/{ix}">open {ix}</a></article>"#
            ),
        };

        if line.len() < target_line_bytes {
            line.push(' ');
            while line.len() < target_line_bytes {
                line.push_str("<!-- filler_token_html_bench -->");
            }
        }
        lines.push(line);
    }

    lines.push("</body>".to_string());
    lines.push("</html>".to_string());
    lines.truncate(line_count);
    lines.join("\n")
}

fn build_synthetic_markdown_document(
    sections: usize,
    target_line_bytes: usize,
    variant: &str,
) -> String {
    let sections = sections.max(1);
    let target_line_bytes = target_line_bytes.max(48);
    let mut source = String::new();

    for ix in 0..sections {
        if !source.is_empty() {
            source.push('\n');
        }

        push_padded_markdown_line(
            &mut source,
            format!("# Section {variant} {ix}"),
            target_line_bytes,
            ix,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!(
                "Paragraph {variant} {ix} explains markdown preview rendering and diff tinting."
            ),
            target_line_bytes,
            ix + 1,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!("- [x] completed item {variant} {ix}"),
            target_line_bytes,
            ix + 2,
        );
        source.push('\n');
        push_padded_markdown_line(
            &mut source,
            format!("- [ ] pending item {variant} {ix}"),
            target_line_bytes,
            ix + 3,
        );
        source.push_str("\n\n");
        push_padded_markdown_line(
            &mut source,
            format!("> quoted note {variant} {ix} for preview rows"),
            target_line_bytes,
            ix + 4,
        );
        source.push_str("\n\n```rust\n");
        push_padded_markdown_line(
            &mut source,
            format!("fn section_{ix}_before_after() {{ println!(\"{variant}_{ix}\"); }}"),
            target_line_bytes,
            ix + 5,
        );
        source.push('\n');
        push_padded_markdown_line(
            &mut source,
            format!("let preview_{ix} = \"{variant}_code_{ix}\";"),
            target_line_bytes,
            ix + 6,
        );
        source.push_str("\n```\n\n| key | value |\n| --- | ----- |\n");
        push_padded_markdown_line(
            &mut source,
            format!("| section_{ix} | table value {variant} {ix} |"),
            target_line_bytes,
            ix + 7,
        );
        source.push('\n');
    }

    source
}

fn push_padded_markdown_line(
    buffer: &mut String,
    mut line: String,
    target_line_bytes: usize,
    seed: usize,
) {
    if line.len() < target_line_bytes {
        line.push(' ');
        while line.len() < target_line_bytes {
            line.push_str(" markdown_token_");
            line.push_str(&(seed % 997).to_string());
        }
    }
    buffer.push_str(&line);
}

fn hash_markdown_preview_document(document: &MarkdownPreviewDocument) -> u64 {
    let mut h = FxHasher::default();
    hash_markdown_preview_document_into(document, &mut h);
    h.finish()
}

fn hash_markdown_preview_document_into(document: &MarkdownPreviewDocument, hasher: &mut FxHasher) {
    document.rows.len().hash(hasher);
    if document.rows.is_empty() {
        return;
    }

    let step = (document.rows.len() / 8).max(1);
    for (ix, row) in document.rows.iter().enumerate().step_by(step).take(8) {
        ix.hash(hasher);
        std::mem::discriminant(&row.kind).hash(hasher);
        row.source_line_range.start.hash(hasher);
        row.source_line_range.end.hash(hasher);
        row.indent_level.hash(hasher);
        row.blockquote_level.hash(hasher);
        row.footnote_label
            .as_ref()
            .map(AsRef::<str>::as_ref)
            .hash(hasher);
        row.alert_kind.hash(hasher);
        row.starts_alert.hash(hasher);
        std::mem::discriminant(&row.change_hint).hash(hasher);
        row.inline_spans.len().hash(hasher);

        let sample_len = row.text.len().min(32);
        row.text
            .as_ref()
            .get(..sample_len)
            .unwrap_or("")
            .hash(hasher);
    }
}

fn build_synthetic_nested_query_stress_lines(
    count: usize,
    target_line_bytes: usize,
    nesting_depth: usize,
) -> Vec<String> {
    let target_line_bytes = target_line_bytes.max(256);
    let nesting_depth = nesting_depth.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let mut line = String::with_capacity(target_line_bytes.saturating_add(nesting_depth * 2));
        line.push_str("let stress_");
        line.push_str(&ix.to_string());
        line.push_str(" = ");
        line.push_str(&"(".repeat(nesting_depth));
        line.push_str("value_");
        line.push_str(&(ix % 97).to_string());
        line.push_str(&")".repeat(nesting_depth));
        line.push_str("; // nested");
        while line.len() < target_line_bytes {
            line.push_str(" (deep_token_");
            line.push_str(&(ix % 101).to_string());
            line.push(')');
        }
        lines.push(line);
    }
    lines
}
