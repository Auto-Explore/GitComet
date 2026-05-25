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
use crate::view::markdown_preview::{
    self, MarkdownChangeHint, MarkdownInlineSpan, MarkdownInlineStyle, MarkdownPreviewDiff,
    MarkdownPreviewDocument, MarkdownPreviewRow, MarkdownPreviewRowKind,
    MarkdownPreviewRowStyledTextCache, MarkdownPreviewRowWidthCache,
};
use crate::view::panes::main::diff_cache::render_svg_image_diff_preview;
pub struct FileDiffSyntaxPrepareSource {
    text: SharedString,
    line_starts: Arc<[usize]>,
}

impl FileDiffSyntaxPrepareSource {
    fn line_count(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.line_starts.len().max(1)
        }
    }
}

pub struct FileDiffSyntaxPrepareFixture {
    lines: Vec<String>,
    warm_text: SharedString,
    warm_line_starts: Arc<[usize]>,
    language: DiffSyntaxLanguage,
    theme: AppTheme,
    budget: DiffSyntaxBudget,
}

impl FileDiffSyntaxPrepareFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        Self::from_lines(language, build_synthetic_source_lines(lines, line_bytes))
    }

    pub fn new_json_proxy(lines: usize, line_bytes: usize) -> Self {
        Self::from_lines(
            DiffSyntaxLanguage::Json,
            build_synthetic_json_proxy_lines(lines, line_bytes),
        )
    }

    pub fn new_query_stress(lines: usize, line_bytes: usize, nesting_depth: usize) -> Self {
        let language =
            diff_syntax_language_for_path("src/lib.rs").unwrap_or(DiffSyntaxLanguage::Rust);
        let lines = build_synthetic_nested_query_stress_lines(lines, line_bytes, nesting_depth);
        Self::from_lines(language, lines)
    }

    pub fn new_json_query_stress(lines: usize, line_bytes: usize, nesting_depth: usize) -> Self {
        let lines =
            build_synthetic_nested_json_query_stress_lines(lines, line_bytes, nesting_depth);
        Self::from_lines(DiffSyntaxLanguage::Json, lines)
    }

    fn from_lines(language: DiffSyntaxLanguage, lines: Vec<String>) -> Self {
        let (warm_text, warm_line_starts) = shared_source_text_and_line_starts(&lines);
        Self {
            lines,
            warm_text,
            warm_line_starts,
            language,
            theme: AppTheme::gitcomet_dark(),
            budget: DiffSyntaxBudget::default(),
        }
    }

    pub fn prewarm(&self) {
        let _ = self.prepare_warm_document();
    }

    pub fn run_prepare_cold(&self, nonce: u64) -> u64 {
        let source = self.prepare_cold_source(nonce);
        self.run_prepare_cold_from_source(&source)
    }

    pub fn prepare_cold_source(&self, nonce: u64) -> FileDiffSyntaxPrepareSource {
        let mut text =
            String::with_capacity(self.warm_text.len().saturating_add(self.lines.len() * 32));
        let mut line_starts = Vec::with_capacity(self.lines.len());

        for (ix, line) in self.lines.iter().enumerate() {
            line_starts.push(text.len());
            push_cold_variant_line(&mut text, line, self.language, nonce, ix);
            if ix + 1 < self.lines.len() {
                text.push('\n');
            }
        }

        FileDiffSyntaxPrepareSource {
            text: text.into(),
            line_starts: Arc::from(line_starts),
        }
    }

    pub fn run_prepare_cold_from_source(&self, source: &FileDiffSyntaxPrepareSource) -> u64 {
        let document =
            self.prepare_document_from_shared(source.text.clone(), Arc::clone(&source.line_starts));
        self.hash_prepared_source(source, document)
    }

    pub fn run_prepare_warm(&self) -> u64 {
        let document = self.prepare_warm_document();
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
        let (text, line_starts) = shared_source_text_and_line_starts(lines);
        self.prepare_document_from_shared(text, line_starts)
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> &[String] {
        &self.lines
    }

    fn prepare_warm_document(&self) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        self.prepare_document_from_shared(
            self.warm_text.clone(),
            Arc::clone(&self.warm_line_starts),
        )
    }

    fn prepare_document_from_shared(
        &self,
        text: SharedString,
        line_starts: Arc<[usize]>,
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        prepare_bench_diff_syntax_document_from_shared(
            self.language,
            self.budget,
            text,
            line_starts,
            None,
        )
    }

    fn hash_prepared(
        &self,
        lines: &[String],
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        self.hash_prepared_line(lines, document, 0)
    }

    fn hash_prepared_source(
        &self,
        source: &FileDiffSyntaxPrepareSource,
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    ) -> u64 {
        self.hash_prepared_shared(
            source.text.as_ref(),
            source.line_starts.as_ref(),
            source.line_count(),
            document,
            0,
        )
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

    fn hash_prepared_shared(
        &self,
        text: &str,
        line_starts: &[usize],
        line_count: usize,
        document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
        line_ix: usize,
    ) -> u64 {
        let line_ix = line_ix.min(line_count.saturating_sub(1));
        let text = super::diff_text::resolved_output_line_text(text, line_starts, line_ix);
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
        line_count.hash(&mut h);
        line_ix.hash(&mut h);
        styled.text_hash.hash(&mut h);
        styled.highlights_hash.hash(&mut h);
        h.finish()
    }
}

fn push_cold_variant_line(
    text: &mut String,
    line: &str,
    language: DiffSyntaxLanguage,
    nonce: u64,
    ix: usize,
) {
    use std::fmt::Write;

    if language == DiffSyntaxLanguage::Json {
        for marker in ["\"payload\":\"", "\"value\":\""] {
            if let Some(start) = line.find(marker) {
                let insert_at = start + marker.len();
                text.push_str(&line[..insert_at]);
                let _ = write!(text, "cold_{nonce}_{ix}_");
                text.push_str(&line[insert_at..]);
                return;
            }
        }
    }

    text.push_str(line);
    let _ = write!(text, " // cold_{nonce}_{ix}");
}

fn shared_source_text_and_line_starts(lines: &[String]) -> (SharedString, Arc<[usize]>) {
    let source_len = if lines.is_empty() {
        0
    } else {
        lines.iter().map(String::len).sum::<usize>() + lines.len().saturating_sub(1)
    };
    crate::view::panes::main::preview_source_text_and_line_starts_from_lines(lines, source_len)
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
            theme: AppTheme::gitcomet_dark(),
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
                    inline_rows.push(AnnotatedDiffLine {
                        kind: DiffLineKind::Add,
                        text: format!("+{new_line}").into(),
                        old_line: None,
                        new_line: Some(new_line_no),
                    });
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
            language,
            theme: AppTheme::gitcomet_dark(),
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

    fn hash_window_step(&self, start: usize, window: usize) -> (u64, bool) {
        if self.inline_rows.is_empty() || window == 0 {
            return (0, false);
        }

        let start = start % self.inline_rows.len();
        let end = (start + window).min(self.inline_rows.len());
        let visible_rows = self.inline_rows[start..end]
            .iter()
            .map(|line| super::diff_text::InlineDiffSyntaxOnlyRow {
                text: diff_content_text(line),
                line,
            })
            .collect::<Vec<_>>();
        let styled_rows =
            super::diff_text::build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
                self.theme,
                Some(self.language),
                super::diff_text::PreparedDiffSyntaxTextSource {
                    document: self.old_document,
                },
                super::diff_text::PreparedDiffSyntaxTextSource {
                    document: self.new_document,
                },
                visible_rows.as_slice(),
                super::diff_text::DiffSyntaxMode::HeuristicOnly,
            );
        let mut pending = false;
        let mut h = FxHasher::default();
        for (offset, prepared) in styled_rows.into_iter().enumerate() {
            let row_ix = start + offset;
            let (styled, is_pending) = prepared.into_parts();
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LargeHtmlSyntaxMetrics {
    pub text_bytes: u64,
    pub line_count: u64,
    pub window_lines: u64,
    pub start_line: u64,
    pub visible_byte_len: u64,
    pub prepared_document_available: u64,
    pub cache_document_present: u64,
    pub pending: u64,
    pub highlight_spans: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_evictions: u64,
    pub chunk_build_ms: u64,
    pub loaded_chunks: u64,
}

pub struct LargeHtmlSyntaxFixture {
    source: LargeHtmlSyntaxSource,
    text: SharedString,
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
        let text = SharedString::from(text);
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
            theme: AppTheme::gitcomet_dark(),
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
        self.run_background_prepare_with_metrics().0
    }

    pub fn run_background_prepare_with_metrics(&self) -> (u64, LargeHtmlSyntaxMetrics) {
        let prepared = prepare_diff_syntax_document_in_background_text(
            DiffSyntaxLanguage::Html,
            DiffSyntaxMode::Auto,
            self.text.clone(),
            Arc::clone(&self.line_starts),
        );

        let mut h = FxHasher::default();
        self.text.len().hash(&mut h);
        self.line_count.hash(&mut h);
        self.source_label().hash(&mut h);
        prepared.is_some().hash(&mut h);
        (
            h.finish(),
            LargeHtmlSyntaxMetrics {
                text_bytes: bench_counter_u64(self.text.len()),
                line_count: bench_counter_u64(self.line_count),
                prepared_document_available: u64::from(prepared.is_some()),
                ..Default::default()
            },
        )
    }

    pub fn run_visible_window_pending_step(&self, start_line: usize, window_lines: usize) -> u64 {
        self.run_visible_window_pending_with_metrics(start_line, window_lines)
            .0
    }

    pub fn run_visible_window_pending_with_metrics(
        &self,
        start_line: usize,
        window_lines: usize,
    ) -> (u64, LargeHtmlSyntaxMetrics) {
        self.run_visible_window_with_metrics_impl(start_line, window_lines, false)
    }

    pub fn run_visible_window_step(&self, start_line: usize, window_lines: usize) -> u64 {
        self.run_visible_window_with_metrics(start_line, window_lines)
            .0
    }

    pub fn run_visible_window_with_metrics(
        &self,
        start_line: usize,
        window_lines: usize,
    ) -> (u64, LargeHtmlSyntaxMetrics) {
        self.run_visible_window_with_metrics_impl(start_line, window_lines, true)
    }

    pub fn prime_visible_window(&self, window_lines: usize) {
        let _ = self.run_visible_window_step(0, window_lines);
    }

    pub fn prime_visible_window_until_ready(&self, window_lines: usize) {
        for i in 0..32 {
            let (_, metrics) = self.run_visible_window_with_metrics(0, window_lines);
            if metrics.pending == 0 {
                break;
            }
            if i < 8 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
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

    fn run_visible_window_with_metrics_impl(
        &self,
        start_line: usize,
        window_lines: usize,
        wait_until_ready: bool,
    ) -> (u64, LargeHtmlSyntaxMetrics) {
        let byte_range = self.visible_window_byte_range(start_line, window_lines);
        let base_metrics = LargeHtmlSyntaxMetrics {
            text_bytes: bench_counter_u64(self.text.len()),
            line_count: bench_counter_u64(self.line_count),
            window_lines: bench_counter_u64(window_lines),
            start_line: bench_counter_u64(start_line),
            visible_byte_len: bench_counter_u64(byte_range.len()),
            ..Default::default()
        };
        let Some(document) = self.prepared_document_handle() else {
            return (0, base_metrics);
        };

        benchmark_reset_diff_syntax_prepared_cache_metrics();
        let result = if wait_until_ready {
            self.request_visible_window_until_ready(document, start_line, window_lines)
        } else {
            self.request_visible_window_for_lines(document, start_line, window_lines)
        };
        let cache_metrics = benchmark_diff_syntax_prepared_cache_metrics();
        let loaded_chunks =
            benchmark_diff_syntax_prepared_loaded_chunk_count(document).unwrap_or_default();
        let cache_document_present =
            benchmark_diff_syntax_prepared_cache_contains_document(document);
        let hash = result
            .as_ref()
            .map(|result| self.hash_visible_window_result(start_line, window_lines, result))
            .unwrap_or_default();
        let highlight_spans = result
            .as_ref()
            .map(|result| bench_counter_u64(result.highlights.len()))
            .unwrap_or_default();
        let pending = u64::from(result.as_ref().is_some_and(|result| result.pending));

        (
            hash,
            LargeHtmlSyntaxMetrics {
                prepared_document_available: 1,
                cache_document_present: u64::from(cache_document_present),
                pending,
                highlight_spans,
                cache_hits: cache_metrics.hit,
                cache_misses: cache_metrics.miss,
                cache_evictions: cache_metrics.evict,
                chunk_build_ms: cache_metrics.chunk_build_ms,
                loaded_chunks: bench_counter_u64(loaded_chunks),
                ..base_metrics
            },
        )
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
        for i in 0..128 {
            if match result.as_ref() {
                None => true,
                Some(highlights) => !highlights.pending,
            } {
                break;
            }

            let applied = drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if applied == 0 && has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
            {
                if i < 32 {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
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
    source_text: SharedString,
    line_starts: Arc<[usize]>,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    prepared_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
    theme: AppTheme,
}

impl WorktreePreviewRenderFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let generated_lines = build_synthetic_source_lines(lines, line_bytes);
        let (source_text, line_starts) = shared_source_text_and_line_starts(&generated_lines);
        let language = diff_syntax_language_for_path("src/lib.rs");
        let syntax_mode = DiffSyntaxMode::Auto;
        let prepared_document = language.and_then(|language| {
            prepare_bench_diff_syntax_document_from_shared(
                language,
                DiffSyntaxBudget::default(),
                source_text.clone(),
                Arc::clone(&line_starts),
                None,
            )
        });

        Self {
            lines: generated_lines,
            source_text,
            line_starts,
            language,
            syntax_mode,
            prepared_document,
            theme: AppTheme::gitcomet_dark(),
        }
    }

    pub fn run_cached_lookup_step(&self, start: usize, window: usize) -> u64 {
        self.hash_window(start, window, self.prepared_document)
    }

    pub fn run_render_time_prepare_step(&self, start: usize, window: usize) -> u64 {
        let prepared_document = self.prepare_document_from_shared_source();
        self.hash_window(start, window, prepared_document)
    }

    fn prepare_document_from_shared_source(
        &self,
    ) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
        self.language.and_then(|language| {
            prepare_bench_diff_syntax_document_from_shared(
                language,
                DiffSyntaxBudget::default(),
                self.source_text.clone(),
                Arc::clone(&self.line_starts),
                None,
            )
        })
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
        let highlight_palette = super::diff_text::syntax_highlight_palette(self.theme);
        let mut h = FxHasher::default();
        for line_ix in start..end {
            let line = super::diff_text::resolved_output_line_text(
                self.source_text.as_ref(),
                &self.line_starts,
                line_ix,
            );
            let styled = super::diff_text::build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
                self.theme,
                &highlight_palette,
                super::diff_text::PreparedDiffTextBuildRequest {
                    build: super::diff_text::DiffTextBuildRequest {
                        text: line,
                        word_ranges: &[],
                        query: "",
                        syntax: super::diff_text::DiffSyntaxConfig {
                            language: self.language,
                            mode: self.syntax_mode,
                        },
                        word_color: None,
                    },
                    prepared_line: super::diff_text::PreparedDiffSyntaxLine {
                        document: prepared_document,
                        line_ix,
                    },
                },
            )
            .into_inner();
            line_ix.hash(&mut h);
            styled.text_hash.hash(&mut h);
            styled.highlights_hash.hash(&mut h);
        }
        h.finish()
    }

    pub fn run_cached_lookup_with_metrics(
        &self,
        start: usize,
        window: usize,
    ) -> (u64, WorktreePreviewRenderMetrics) {
        let hash = self.hash_window(start, window, self.prepared_document);
        let actual_start = if self.lines.is_empty() {
            0
        } else {
            start % self.lines.len()
        };
        let actual_end = if self.lines.is_empty() {
            0
        } else {
            (actual_start + window).min(self.lines.len())
        };
        let metrics = WorktreePreviewRenderMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            window_size: bench_counter_u64(actual_end.saturating_sub(actual_start)),
            line_bytes: bench_counter_u64(self.lines.first().map(|l| l.len()).unwrap_or(0)),
            prepared_document_available: u64::from(self.prepared_document.is_some()),
            syntax_mode_auto: u64::from(self.syntax_mode == DiffSyntaxMode::Auto),
        };
        (hash, metrics)
    }

    pub fn run_render_time_prepare_with_metrics(
        &self,
        start: usize,
        window: usize,
    ) -> (u64, WorktreePreviewRenderMetrics) {
        let prepared_document = self.prepare_document_from_shared_source();
        let hash = self.hash_window(start, window, prepared_document);
        let actual_start = if self.lines.is_empty() {
            0
        } else {
            start % self.lines.len()
        };
        let actual_end = if self.lines.is_empty() {
            0
        } else {
            (actual_start + window).min(self.lines.len())
        };
        let metrics = WorktreePreviewRenderMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            window_size: bench_counter_u64(actual_end.saturating_sub(actual_start)),
            line_bytes: bench_counter_u64(self.lines.first().map(|l| l.len()).unwrap_or(0)),
            prepared_document_available: u64::from(prepared_document.is_some()),
            syntax_mode_auto: u64::from(self.syntax_mode == DiffSyntaxMode::Auto),
        };
        (hash, metrics)
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorktreePreviewRenderMetrics {
    pub total_lines: u64,
    pub window_size: u64,
    pub line_bytes: u64,
    pub prepared_document_available: u64,
    pub syntax_mode_auto: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarkdownPreviewFirstWindowMetrics {
    pub old_total_rows: u64,
    pub new_total_rows: u64,
    pub old_rows_rendered: u64,
    pub new_rows_rendered: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MarkdownPreviewScrollMetrics {
    pub total_rows: u64,
    pub start_row: u64,
    pub window_size: u64,
    pub rows_rendered: u64,
    pub scroll_step_rows: u64,
    pub long_rows: u64,
    pub long_row_bytes: u64,
    pub heading_rows: u64,
    pub list_rows: u64,
    pub table_rows: u64,
    pub code_rows: u64,
    pub blockquote_rows: u64,
    pub details_rows: u64,
}

const RICH_MARKDOWN_SCROLL_TOTAL_ROWS: usize = 5_000;
const RICH_MARKDOWN_SCROLL_LONG_ROWS: usize = 500;
const RICH_MARKDOWN_SCROLL_LONG_ROW_BYTES: usize = 2_000;
const RICH_MARKDOWN_SCROLL_PATTERN_ROWS: usize = 20;
const RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct MarkdownPreviewScrollProfile {
    total_rows: u64,
    long_rows: u64,
    long_row_bytes: u64,
    heading_rows: u64,
    list_rows: u64,
    table_rows: u64,
    code_rows: u64,
    blockquote_rows: u64,
    details_rows: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImagePreviewFirstPaintMetrics {
    pub old_bytes: u64,
    pub new_bytes: u64,
    pub total_bytes: u64,
    pub images_rendered: u64,
    pub placeholder_cells: u64,
    pub divider_count: u64,
}

/// Benchmark fixture for the ready-image path in `render_selected_file_diff`.
///
/// The production diff renderer consumes already-built image preview cache
/// entries. Keep the synthetic byte payloads around for metrics, but prebuild
/// the ready `gpui::Image` handles once so the hot loop only fingerprints the
/// two cached cells plus their divider.
pub struct ImagePreviewFirstPaintFixture {
    old_bytes: usize,
    new_bytes: usize,
    cells: [Option<Arc<gpui::Image>>; 2],
}

impl ImagePreviewFirstPaintFixture {
    pub fn new(old_bytes: usize, new_bytes: usize) -> Self {
        let old_png =
            build_synthetic_png_like_payload(old_bytes.max(64 * 1024), 0x4f4c_445f_504e_4701);
        let new_png =
            build_synthetic_png_like_payload(new_bytes.max(64 * 1024), 0x4e45_575f_504e_4702);
        Self {
            old_bytes: old_png.len(),
            new_bytes: new_png.len(),
            cells: [
                Some(Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    old_png,
                ))),
                Some(Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    new_png,
                ))),
            ],
        }
    }

    pub fn measure_first_paint(&self) -> ImagePreviewFirstPaintMetrics {
        let old_bytes = bench_counter_u64(self.old_bytes);
        let new_bytes = bench_counter_u64(self.new_bytes);
        ImagePreviewFirstPaintMetrics {
            old_bytes,
            new_bytes,
            total_bytes: old_bytes.saturating_add(new_bytes),
            images_rendered: 2,
            placeholder_cells: 0,
            divider_count: 1,
        }
    }

    pub fn run_first_paint_step(&self) -> u64 {
        let mut h = FxHasher::default();
        for (cell_ix, image) in self.cells.iter().enumerate() {
            cell_ix.hash(&mut h);
            match image {
                Some(image) => {
                    1u8.hash(&mut h); // actual image cell
                    image.id().hash(&mut h);
                    image.bytes.len().hash(&mut h);
                    1u8.hash(&mut h); // ObjectFit::Contain
                }
                None => {
                    0u8.hash(&mut h); // placeholder
                    0usize.hash(&mut h);
                    0u8.hash(&mut h);
                }
            }
        }
        1u8.hash(&mut h); // divider between before/after columns
        h.finish()
    }
}

/// Benchmark fixture for steady-state markdown Preview-mode scrolling.
///
/// The rich fixture intentionally constructs preview rows directly so it can
/// model a rendered 5k-row document with 500 2k-character rows without being
/// constrained by the production single-document 1 MiB source-size guard.
pub struct MarkdownPreviewScrollFixture {
    document: MarkdownPreviewDocument,
    theme: AppTheme,
    profile: MarkdownPreviewScrollProfile,
}

impl MarkdownPreviewScrollFixture {
    pub fn new_sectioned(sections: usize, line_bytes: usize) -> Self {
        let sections = sections.max(1);
        let line_bytes = line_bytes.max(48);
        let source = build_synthetic_markdown_document(sections, line_bytes, "scroll");
        let document = markdown_preview::parse_markdown(&source).expect(
            "synthetic markdown scroll benchmark fixture should stay within preview limits",
        );
        let profile = profile_markdown_preview_scroll_document(&document, 0);
        Self {
            document,
            theme: AppTheme::gitcomet_dark(),
            profile,
        }
    }

    pub fn new_rich_5000_rows() -> Self {
        let document = build_synthetic_rich_markdown_scroll_document();
        let profile = profile_markdown_preview_scroll_document(
            &document,
            RICH_MARKDOWN_SCROLL_LONG_ROW_BYTES,
        );
        debug_assert_eq!(profile.total_rows, RICH_MARKDOWN_SCROLL_TOTAL_ROWS as u64);
        debug_assert_eq!(profile.long_rows, RICH_MARKDOWN_SCROLL_LONG_ROWS as u64);
        debug_assert_eq!(
            profile.long_row_bytes,
            RICH_MARKDOWN_SCROLL_LONG_ROW_BYTES as u64
        );
        Self {
            document,
            theme: AppTheme::gitcomet_dark(),
            profile,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        hash_markdown_preview_window(self.theme, &self.document, start, window)
    }

    pub fn run_scroll_step_with_metrics(
        &self,
        start: usize,
        window: usize,
        scroll_step_rows: usize,
    ) -> (u64, MarkdownPreviewScrollMetrics) {
        let hash = self.run_scroll_step(start, window);
        let (actual_start, actual_end) =
            markdown_preview_document_window_bounds(&self.document, start, window);
        let rows_rendered = actual_end.saturating_sub(actual_start);
        let metrics = MarkdownPreviewScrollMetrics {
            total_rows: self.profile.total_rows,
            start_row: bench_counter_u64(actual_start),
            window_size: bench_counter_u64(rows_rendered),
            rows_rendered: bench_counter_u64(rows_rendered),
            scroll_step_rows: bench_counter_u64(scroll_step_rows),
            long_rows: self.profile.long_rows,
            long_row_bytes: self.profile.long_row_bytes,
            heading_rows: self.profile.heading_rows,
            list_rows: self.profile.list_rows,
            table_rows: self.profile.table_rows,
            code_rows: self.profile.code_rows,
            blockquote_rows: self.profile.blockquote_rows,
            details_rows: self.profile.details_rows,
        };
        (hash, metrics)
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
            theme: AppTheme::gitcomet_dark(),
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
        hash_markdown_preview_window(self.theme, &self.single_document, start, window)
    }

    /// Measure first-window diff rendering metrics (used for sidecar emission).
    pub fn measure_first_window_diff(&self, window: usize) -> MarkdownPreviewFirstWindowMetrics {
        let old_total = self.diff_preview.old.rows.len();
        let new_total = self.diff_preview.new.rows.len();
        let old_end = window.min(old_total);
        let new_end = window.min(new_total);

        let old_rendered =
            render_markdown_preview_window(self.theme, &self.diff_preview.old, 0, old_end);
        let new_rendered =
            render_markdown_preview_window(self.theme, &self.diff_preview.new, 0, new_end);

        MarkdownPreviewFirstWindowMetrics {
            old_total_rows: old_total as u64,
            new_total_rows: new_total as u64,
            old_rows_rendered: old_rendered.len() as u64,
            new_rows_rendered: new_rendered.len() as u64,
        }
    }

    /// Run the first-window diff rendering step (for Criterion iteration).
    pub fn run_first_window_diff_step(&self, window: usize) -> u64 {
        self.run_render_diff_step(0, window)
    }

    pub fn run_render_diff_step(&self, start: usize, window: usize) -> u64 {
        if window == 0 {
            return 0;
        }

        let left =
            render_markdown_preview_window(self.theme, &self.diff_preview.old, start, window);
        let right =
            render_markdown_preview_window(self.theme, &self.diff_preview.new, start, window);

        let mut h = FxHasher::default();
        start.hash(&mut h);
        window.hash(&mut h);
        std::hint::black_box(left).len().hash(&mut h);
        std::hint::black_box(right).len().hash(&mut h);
        h.finish()
    }
}

fn hash_markdown_preview_window(
    theme: AppTheme,
    document: &MarkdownPreviewDocument,
    start: usize,
    window: usize,
) -> u64 {
    if window == 0 {
        return 0;
    }

    let rows = render_markdown_preview_window(theme, document, start, window);
    let mut h = FxHasher::default();
    start.hash(&mut h);
    window.hash(&mut h);
    std::hint::black_box(rows).len().hash(&mut h);
    h.finish()
}

fn render_markdown_preview_window(
    theme: AppTheme,
    document: &MarkdownPreviewDocument,
    start: usize,
    window: usize,
) -> Vec<AnyElement> {
    let (start, end) = markdown_preview_document_window_bounds(document, start, window);
    if start == end {
        return Vec::new();
    }

    super::history::render_markdown_preview_document_rows(
        document,
        start..end,
        &super::history::MarkdownPreviewRenderContext {
            theme,
            bar_color: None,
            min_width: px(0.0),
            editor_font_family: crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY.into(),
            ui_scale_percent: crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
            view: None,
            text_region: DiffTextRegion::Inline,
        },
    )
}

fn markdown_preview_document_window_bounds(
    document: &MarkdownPreviewDocument,
    start: usize,
    window: usize,
) -> (usize, usize) {
    if document.rows.is_empty() || window == 0 {
        return (0, 0);
    }

    let start = start % document.rows.len();
    let end = (start + window).min(document.rows.len());
    (start, end)
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

fn profile_markdown_preview_scroll_document(
    document: &MarkdownPreviewDocument,
    long_row_bytes: usize,
) -> MarkdownPreviewScrollProfile {
    let mut profile = MarkdownPreviewScrollProfile {
        total_rows: bench_counter_u64(document.rows.len()),
        long_row_bytes: bench_counter_u64(long_row_bytes),
        ..MarkdownPreviewScrollProfile::default()
    };

    if long_row_bytes > 0 {
        profile.long_rows = bench_counter_u64(
            document
                .rows
                .iter()
                .filter(|row| row.text.len() >= long_row_bytes)
                .count(),
        );
    }

    for row in &document.rows {
        match row.kind {
            MarkdownPreviewRowKind::Heading { .. } => profile.heading_rows += 1,
            MarkdownPreviewRowKind::ListItem { .. } => profile.list_rows += 1,
            MarkdownPreviewRowKind::TableRow { .. } => profile.table_rows += 1,
            MarkdownPreviewRowKind::CodeLine { .. } => profile.code_rows += 1,
            MarkdownPreviewRowKind::BlockquoteLine => profile.blockquote_rows += 1,
            MarkdownPreviewRowKind::DetailsSummary => profile.details_rows += 1,
            _ => {}
        }
    }

    profile
}

fn build_synthetic_rich_markdown_scroll_document() -> MarkdownPreviewDocument {
    let block_count = RICH_MARKDOWN_SCROLL_TOTAL_ROWS / RICH_MARKDOWN_SCROLL_PATTERN_ROWS;
    debug_assert_eq!(
        block_count.saturating_mul(RICH_MARKDOWN_SCROLL_PATTERN_ROWS),
        RICH_MARKDOWN_SCROLL_TOTAL_ROWS
    );
    debug_assert_eq!(
        block_count.saturating_mul(2),
        RICH_MARKDOWN_SCROLL_LONG_ROWS
    );

    let mut rows = Vec::with_capacity(RICH_MARKDOWN_SCROLL_TOTAL_ROWS);

    for block_ix in 0..block_count {
        let row_base = block_ix * RICH_MARKDOWN_SCROLL_PATTERN_ROWS;
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::Heading { level: 1 },
            padded_markdown_preview_text(
                format!("Preview heading cluster {block_ix} sets the document theme"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base,
            ),
            row_base,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::Paragraph,
            padded_markdown_preview_text(
                format!(
                    "Paragraph row {block_ix} exercises rich preview shaping across emphasis, links, tables, and list content for sustained scrolling"
                ),
                RICH_MARKDOWN_SCROLL_LONG_ROW_BYTES,
                row_base + 1,
            ),
            row_base + 1,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::Paragraph,
            padded_markdown_preview_text(
                format!("Short paragraph row {block_ix} keeps mixed markdown content dense"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 2,
            ),
            row_base + 2,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::DetailsSummary,
            padded_markdown_preview_text(
                format!("Details summary {block_ix} expands benchmark notes"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 3,
            ),
            row_base + 3,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::ListItem { number: None },
            padded_markdown_preview_text(
                format!("unordered preview item {block_ix} keeps list layout active"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 4,
            ),
            row_base + 4,
            None,
            false,
            1,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::ListItem {
                number: Some(u64::try_from(block_ix + 1).unwrap_or(u64::MAX)),
            },
            padded_markdown_preview_text(
                format!("ordered preview item {block_ix} validates numbered list rendering"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 5,
            ),
            row_base + 5,
            None,
            false,
            1,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::BlockquoteLine,
            padded_markdown_preview_text(
                format!("blockquote note {block_ix} keeps quoted styling in rotation"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 6,
            ),
            row_base + 6,
            None,
            false,
            0,
            1,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: false,
            },
            padded_markdown_preview_text(
                format!("fn render_window_{block_ix}() -> usize {{ {block_ix} + 1 }}"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 7,
            ),
            row_base + 7,
            Some(DiffSyntaxLanguage::Rust),
            false,
            0,
            0,
            false,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true,
            },
            padded_markdown_preview_text(
                format!(
                    "let scrolling_preview_row_{block_ix} = cache.refresh_preview_window({block_ix}, 200, true);"
                ),
                RICH_MARKDOWN_SCROLL_LONG_ROW_BYTES,
                row_base + 8,
            ),
            row_base + 8,
            Some(DiffSyntaxLanguage::Rust),
            true,
            0,
            0,
            false,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::TableRow { is_header: true },
            padded_markdown_preview_text(
                format!("column_{block_ix} | value | notes | preview"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 9,
            ),
            row_base + 9,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::TableRow { is_header: false },
            padded_markdown_preview_text(
                format!("feature_{block_ix} | stable | row_count | 200"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 10,
            ),
            row_base + 10,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::Heading { level: 2 },
            padded_markdown_preview_text(
                format!("Secondary heading {block_ix} keeps heading hierarchy varied"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 11,
            ),
            row_base + 11,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::Paragraph,
            padded_markdown_preview_text(
                format!("Compact body row {block_ix} keeps paragraph batches realistic"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 12,
            ),
            row_base + 12,
            None,
            false,
            0,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::ListItem { number: None },
            padded_markdown_preview_text(
                format!("nested bullet row {block_ix} keeps indentation work active"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 13,
            ),
            row_base + 13,
            None,
            false,
            2,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::ListItem {
                number: Some(u64::try_from(block_ix.saturating_mul(2) + 1).unwrap_or(u64::MAX)),
            },
            padded_markdown_preview_text(
                format!("nested ordered row {block_ix} keeps numbering logic warm"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 14,
            ),
            row_base + 14,
            None,
            false,
            2,
            0,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::BlockquoteLine,
            padded_markdown_preview_text(
                format!("quoted follow-up row {block_ix} preserves blockquote density"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 15,
            ),
            row_base + 15,
            None,
            false,
            0,
            1,
            true,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::CodeLine {
                is_first: true,
                is_last: false,
            },
            padded_markdown_preview_text(
                format!("let section_{block_ix}_visible = viewport.start + {block_ix};"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 16,
            ),
            row_base + 16,
            Some(DiffSyntaxLanguage::Rust),
            false,
            0,
            0,
            false,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::CodeLine {
                is_first: false,
                is_last: true,
            },
            padded_markdown_preview_text(
                format!("viewport.finish(section_{block_ix}_visible, 200);"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 17,
            ),
            row_base + 17,
            Some(DiffSyntaxLanguage::Rust),
            false,
            0,
            0,
            false,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::ThematicBreak,
            String::new(),
            row_base + 18,
            None,
            false,
            0,
            0,
            false,
        ));
        rows.push(build_markdown_preview_row(
            MarkdownPreviewRowKind::TableRow { is_header: false },
            padded_markdown_preview_text(
                format!("metric_{block_ix} | scroll_step | 24 | markdown"),
                RICH_MARKDOWN_SCROLL_SHORT_ROW_BYTES,
                row_base + 19,
            ),
            row_base + 19,
            None,
            false,
            0,
            0,
            true,
        ));
    }

    debug_assert_eq!(rows.len(), RICH_MARKDOWN_SCROLL_TOTAL_ROWS);
    MarkdownPreviewDocument { rows }
}

fn build_markdown_preview_row(
    kind: MarkdownPreviewRowKind,
    text: String,
    source_line: usize,
    code_language: Option<DiffSyntaxLanguage>,
    code_block_horizontal_scroll_hint: bool,
    indent_level: u8,
    blockquote_level: u8,
    styled_inline: bool,
) -> MarkdownPreviewRow {
    let inline_spans = if styled_inline {
        build_markdown_preview_inline_spans(&text)
    } else {
        Arc::default()
    };

    MarkdownPreviewRow {
        kind,
        text: text.into(),
        inline_spans,
        code_language,
        code_block_horizontal_scroll_hint,
        source_line_range: source_line..source_line.saturating_add(1),
        change_hint: MarkdownChangeHint::None,
        indent_level,
        blockquote_level,
        footnote_label: None,
        alert_kind: None,
        starts_alert: false,
        styled_text_cache: MarkdownPreviewRowStyledTextCache::default(),
        measured_width_px: MarkdownPreviewRowWidthCache::default(),
    }
}

fn build_markdown_preview_inline_spans(text: &str) -> Arc<Vec<MarkdownInlineSpan>> {
    let len = text.len();
    let mut spans = Vec::new();
    for (start, width, style) in [
        (0usize, 8usize, MarkdownInlineStyle::Bold),
        (12usize, 6usize, MarkdownInlineStyle::Italic),
        (24usize, 8usize, MarkdownInlineStyle::Link),
        (36usize, 6usize, MarkdownInlineStyle::Code),
    ] {
        if start >= len {
            continue;
        }
        let end = start.saturating_add(width).min(len);
        if end > start {
            spans.push(MarkdownInlineSpan {
                byte_range: start..end,
                style,
            });
        }
    }
    Arc::new(spans)
}

fn padded_markdown_preview_text(mut base: String, target_bytes: usize, seed: usize) -> String {
    if base.len() < target_bytes {
        base.push(' ');
        while base.len() < target_bytes {
            base.push_str("preview_scroll_token_");
            base.push_str(&(seed % 997).to_string());
            base.push(' ');
        }
        base.truncate(target_bytes);
    }
    base
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

// ---------------------------------------------------------------------------
// SVG dual-path first-window fixture
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SvgDualPathFirstWindowMetrics {
    pub old_svg_bytes: u64,
    pub new_svg_bytes: u64,
    pub rasterize_success: u64,
    pub fallback_triggered: u64,
    pub rasterized_png_bytes: u64,
    pub images_rendered: u64,
    pub divider_count: u64,
}

/// Benchmark fixture for the SVG dual-path in `ensure_file_image_diff_cache`.
///
/// Production SVG image diffs take one of two paths:
///
/// 1. **Rasterize path** — valid SVG is parsed by `resvg::usvg` and rendered to
///    the `RenderImage` preview that file-diff rendering consumes.
/// 2. **Fallback path** — invalid or oversized SVG fails rasterization and is
///    written to a temp file for external viewing.
///
/// This fixture holds one valid SVG (exercises path 1) and one intentionally
/// invalid SVG payload (exercises path 2 as a hash-based proxy to avoid
/// filesystem I/O in the hot loop).
pub struct SvgDualPathFirstWindowFixture {
    /// Valid SVG document — triggers the rasterize-to-PNG path.
    old_svg: Vec<u8>,
    /// Invalid SVG payload — triggers the fallback path.
    new_svg: Vec<u8>,
    /// Pre-rasterized PNG for the valid SVG, used for metrics only.
    old_rasterized_png_len: usize,
}

impl SvgDualPathFirstWindowFixture {
    pub fn new(shapes: usize, fallback_bytes: usize) -> Self {
        let old_svg = build_synthetic_svg_document(shapes.max(4), 0x5560_01D0_5EED_0001);
        let new_svg =
            build_synthetic_invalid_svg_payload(fallback_bytes.max(1024), 0x5560_4E50_FA11_0002);

        // Pre-rasterize once to know the PNG size for metrics.
        let old_rasterized_png_len = crate::view::diff_utils::rasterize_svg_preview_png(&old_svg)
            .map(|png| png.len())
            .unwrap_or(0);

        Self {
            old_svg,
            new_svg,
            old_rasterized_png_len,
        }
    }

    fn exercise_first_window(&self) -> (u64, SvgDualPathFirstWindowMetrics) {
        let mut h = FxHasher::default();
        let mut rasterize_success = 0u64;

        // Path 1: render the valid SVG through the live file-diff preview path.
        if let Some(render) = render_svg_image_diff_preview(&self.old_svg) {
            let size = render.size(0);
            1u8.hash(&mut h); // rasterize-success marker
            size.width.0.hash(&mut h);
            size.height.0.hash(&mut h);
            rasterize_success = 1;
        } else {
            0u8.hash(&mut h); // should not happen for valid SVG
        }

        // Path 2: fallback for invalid SVG — hash bytes as proxy for tempfile write.
        {
            2u8.hash(&mut h); // fallback marker
            self.new_svg.len().hash(&mut h);
            // Hash a sample of the payload to model the I/O + hashing cost of
            // `cached_image_diff_path` without actual filesystem writes.
            let sample = &self.new_svg[..self.new_svg.len().min(4096)];
            sample.hash(&mut h);
        }

        // Divider between before/after columns.
        1u8.hash(&mut h);

        let metrics = SvgDualPathFirstWindowMetrics {
            old_svg_bytes: bench_counter_u64(self.old_svg.len()),
            new_svg_bytes: bench_counter_u64(self.new_svg.len()),
            rasterize_success,
            fallback_triggered: 1,
            rasterized_png_bytes: bench_counter_u64(self.old_rasterized_png_len),
            images_rendered: rasterize_success,
            divider_count: 1,
        };

        (h.finish(), metrics)
    }

    pub fn measure_first_window(&self) -> SvgDualPathFirstWindowMetrics {
        let (_hash, metrics) = self.exercise_first_window();
        metrics
    }

    /// Hot-path step exercising both SVG paths.
    ///
    /// Returns a deterministic hash to prevent dead-code elimination.
    pub fn run_first_window_step(&self, _window: usize) -> u64 {
        let (hash, _metrics) = self.exercise_first_window();
        hash
    }
}

fn build_synthetic_svg_document(shape_count: usize, seed: u64) -> Vec<u8> {
    let mut svg = String::with_capacity(shape_count * 80 + 200);
    svg.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">"#);
    svg.push('\n');

    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    for _ in 0..shape_count {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let v = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let cx = (v >> 48) as u16 % 512;
        let cy = (v >> 32) as u16 % 512;
        let r = ((v >> 24) as u16 % 20) + 1;
        let color = v & 0xFF_FFFF;
        use std::fmt::Write;
        let _ = write!(
            svg,
            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"#{color:06x}\"/>",
        );
        svg.push('\n');
    }
    svg.push_str("</svg>\n");
    svg.into_bytes()
}

fn build_synthetic_invalid_svg_payload(target_bytes: usize, seed: u64) -> Vec<u8> {
    let target_bytes = target_bytes.max(64);
    let mut payload = Vec::with_capacity(target_bytes);
    // Starts like XML but is not valid SVG — triggers fallback in resvg.
    payload.extend_from_slice(b"<svg invalid-not-parseable ");

    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    while payload.len() < target_bytes {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let block = state.wrapping_mul(0x2545_f491_4f6c_dd1d).to_le_bytes();
        let remaining = target_bytes - payload.len();
        payload.extend_from_slice(&block[..remaining.min(block.len())]);
    }
    payload
}

fn build_synthetic_png_like_payload(target_bytes: usize, seed: u64) -> Vec<u8> {
    let target_bytes = target_bytes.max(8);
    let mut payload = Vec::with_capacity(target_bytes);
    payload.extend_from_slice(b"\x89PNG\r\n\x1a\n");

    let mut state = seed ^ 0x9e37_79b9_7f4a_7c15;
    while payload.len() < target_bytes {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let block = state.wrapping_mul(0x2545_f491_4f6c_dd1d).to_le_bytes();
        let remaining = target_bytes - payload.len();
        payload.extend_from_slice(&block[..remaining.min(block.len())]);
    }

    payload
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

fn build_synthetic_json_proxy_lines(count: usize, target_line_bytes: usize) -> Vec<String> {
    use std::fmt::Write;

    let count = count.max(1);
    let target_line_bytes = target_line_bytes.max(96);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let mut line = String::with_capacity(target_line_bytes.saturating_add(128));
        if ix == 0 {
            line.push('{');
        }
        let _ = write!(
            line,
            "\"row_{ix:06}\":{{\"id\":{ix},\"active\":{},\"kind\":\"record\",\"payload\":\"",
            if ix % 2 == 0 { "true" } else { "false" },
        );
        while line.len().saturating_add(96) < target_line_bytes {
            line.push_str("json_token_");
            let _ = write!(line, "{:03}", ix % 997);
            line.push('_');
        }
        let _ = write!(
            line,
            "\",\"metrics\":{{\"count\":{},\"score\":{},\"tags\":[\"alpha\",\"beta\",\"gamma\"]}}}}",
            ix.saturating_mul(3).saturating_add(1),
            ix.saturating_mul(17).saturating_add(5),
        );
        if ix + 1 < count {
            line.push(',');
        } else {
            line.push('}');
        }
        lines.push(line);
    }
    lines
}

fn build_synthetic_nested_json_query_stress_lines(
    count: usize,
    target_line_bytes: usize,
    nesting_depth: usize,
) -> Vec<String> {
    use std::fmt::Write;

    let count = count.max(1);
    let target_line_bytes = target_line_bytes.max(256);
    let nesting_depth = nesting_depth.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let mut line =
            String::with_capacity(target_line_bytes.saturating_add(nesting_depth * 24 + 192));
        if ix == 0 {
            line.push('{');
        }
        let _ = write!(line, "\"row_{ix:06}\":");
        for depth in 0..nesting_depth {
            let _ = write!(line, "{{\"level_{depth:03}\":");
        }
        line.push_str("{\"value\":\"");
        while line.len().saturating_add(nesting_depth * 14 + 160) < target_line_bytes {
            line.push_str("nested_json_token_");
            let _ = write!(line, "{:03}", (ix + nesting_depth) % 997);
            line.push('_');
        }
        let _ = write!(
            line,
            "\",\"index\":{ix},\"flags\":[true,false,null],\"items\":[{{\"leaf\":{},\"kind\":\"deep\"}}]}}",
            if ix % 2 == 0 { "true" } else { "false" },
        );
        for _ in 0..nesting_depth {
            line.push('}');
        }
        if ix + 1 < count {
            line.push(',');
        } else {
            line.push('}');
        }
        lines.push(line);
    }
    lines
}
