use super::highlight::*;
use super::shaping::*;
use super::state::*;
use super::*;

/// The span of a highlight source that covers a live window.
///
/// A deletion since the source was installed makes it longer than the buffer
/// over the edited span, so reach that much further or the bottom of the window
/// comes back short.
fn interpolated_source_window(
    interpolation: &HighlightInterpolation,
    byte_range: &Range<usize>,
) -> Range<usize> {
    let mut source_range = interpolation.to_source_range(byte_range);
    source_range.end = source_range
        .end
        .saturating_add(interpolation.source_lookahead());
    source_range
}

impl TextInput {
    pub(super) fn invalidate_highlights(&mut self) {
        self.highlight.provider_cache = None;
        self.highlight.epoch = self.highlight.epoch.wrapping_add(1).max(1);
        // Highlight providers are rebound on keystrokes and caret movement.
        // Dropping the document height here clamps the outer scroll handle to
        // the unwrapped height before the next shaping pass can restore it.
        self.bump_shape_style_epoch_preserving_wrap_rows();
    }

    /// Background syntax chunks landed for the text the provider already
    /// describes. Its tokens improved; the text it was built over did not, so
    /// the interpolation anchor must survive — resetting it here would snap
    /// every highlight to coordinates the buffer left behind.
    pub(super) fn note_provider_highlights_changed(&mut self) {
        self.highlight.interpolated_cache = None;
        self.invalidate_highlights();
    }

    /// Record a text edit against the highlights currently on screen.
    ///
    /// The highlight source is not recomputed here — that is debounced by the
    /// owner. Instead the edit is folded into the interpolation so the stale
    /// highlights keep pointing at the tokens they describe.
    pub(super) fn note_text_edit_for_highlights(
        &mut self,
        replaced: &Range<usize>,
        inserted: &Range<usize>,
    ) {
        if self.highlight.provider.is_none()
            && self.highlight.highlights.is_empty()
            && self.highlight.superseded.is_none()
        {
            return;
        }

        self.highlight.interpolation.record_edit(replaced, inserted);
        // A source held in reserve has to keep tracking the buffer too, or it
        // would answer in coordinates that stopped being true the moment it
        // was set aside.
        if let Some(superseded) = self.highlight.superseded.as_mut() {
            superseded.interpolation.record_edit(replaced, inserted);
        }
        self.highlight.interpolated_cache = None;
        self.highlight.prepaint_runs_cache = None;
    }

    /// Drop the accumulated edits because the highlight source was replaced by
    /// one built over the buffer's current text.
    fn reset_highlight_interpolation(&mut self) {
        self.highlight.interpolation.reset();
        self.highlight.interpolated_cache = None;
    }

    /// Set the outgoing highlight source aside so it can cover for its
    /// replacement until that replacement has tokens to show. See
    /// `SupersededHighlights`.
    fn supersede_current_highlight_source(&mut self) {
        if !self.highlight.answered {
            // This source never settled either, so it is no better a fallback
            // than the incoming one. Keep whatever reserve is already held.
            return;
        }
        if self.highlight.provider.is_none() && self.highlight.highlights.is_empty() {
            self.highlight.superseded = None;
            return;
        }

        self.highlight.superseded = Some(SupersededHighlights {
            provider: self.highlight.provider.clone(),
            highlights: Arc::clone(&self.highlight.highlights),
            interpolation: std::mem::take(&mut self.highlight.interpolation),
        });
    }

    /// The highlights covering `byte_range`, in the buffer's live coordinates.
    ///
    /// Collapses the provider and static-vector paths so both interpolate: a
    /// text edit moves highlights published through `set_highlights` exactly as
    /// it moves a provider's.
    pub(super) fn effective_highlights_for_window(
        &mut self,
        byte_range: Range<usize>,
    ) -> ResolvedProviderHighlights {
        let resolved = self.resolve_current_highlight_source(&byte_range);
        if !resolved.pending {
            // The source has settled, so it is now the truth and is fit to
            // stand in for its own replacement later.
            self.highlight.answered = true;
            self.highlight.superseded = None;
            return resolved;
        }

        // Still waiting on tokens. Rather than paint the viewport in the base
        // color for a frame or two, answer from the source this one replaced,
        // carried across the edits since by its own interpolation.
        let Some(superseded) = self.highlight.superseded.take() else {
            return resolved;
        };
        let stale = self.resolve_superseded_highlight_source(&superseded, &byte_range);
        self.highlight.superseded = Some(superseded);
        ResolvedProviderHighlights {
            // Keep the caller polling: this is a stopgap, not the answer.
            pending: true,
            highlights: stale,
        }
    }

    fn resolve_current_highlight_source(
        &mut self,
        byte_range: &Range<usize>,
    ) -> ResolvedProviderHighlights {
        if self.highlight.interpolation.is_exact() {
            return if self.highlight.provider.is_some() {
                self.resolve_provider_highlights(byte_range.start, byte_range.end)
            } else {
                ResolvedProviderHighlights {
                    pending: false,
                    highlights: Arc::clone(&self.highlight.highlights),
                }
            };
        }

        if let Some(cache) = self.highlight.interpolated_cache.as_ref()
            && cache.highlight_epoch == self.highlight.epoch
            && cache.interpolation_generation == self.highlight.interpolation.generation()
            && cache.byte_start <= byte_range.start
            && cache.byte_end >= byte_range.end
        {
            return ResolvedProviderHighlights {
                pending: cache.pending,
                highlights: Arc::clone(&cache.highlights),
            };
        }

        let source_range = interpolated_source_window(&self.highlight.interpolation, byte_range);
        let source = if self.highlight.provider.is_some() {
            self.resolve_provider_highlights(source_range.start, source_range.end)
        } else {
            ResolvedProviderHighlights {
                pending: false,
                highlights: Arc::clone(&self.highlight.highlights),
            }
        };
        let highlights = Arc::new(
            self.highlight
                .interpolation
                .map_highlights(source.highlights.as_slice(), self.content.len()),
        );
        self.debug_assert_highlights_on_char_boundaries(&highlights);

        self.highlight.interpolated_cache = Some(InterpolatedHighlightCache {
            highlight_epoch: self.highlight.epoch,
            interpolation_generation: self.highlight.interpolation.generation(),
            byte_start: byte_range.start,
            byte_end: byte_range.end,
            pending: source.pending,
            highlights: Arc::clone(&highlights),
        });
        ResolvedProviderHighlights {
            pending: source.pending,
            highlights,
        }
    }

    /// Answer a window from the source that is being replaced.
    ///
    /// Deliberately uncached: `provider_cache` belongs to the source that
    /// replaced this one, and mixing two providers' answers under one key would
    /// outlive the handoff. This runs for the frame or two the replacement
    /// needs to build its tokens.
    fn resolve_superseded_highlight_source(
        &self,
        superseded: &SupersededHighlights,
        byte_range: &Range<usize>,
    ) -> Arc<Vec<(Range<usize>, gpui::HighlightStyle)>> {
        let source_range = interpolated_source_window(&superseded.interpolation, byte_range);
        let highlights = match superseded.provider.as_ref() {
            Some(provider) => {
                let mut resolved = provider.resolve(source_range).highlights;
                resolved.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
                superseded
                    .interpolation
                    .map_highlights(&resolved, self.content.len())
            }
            None => superseded
                .interpolation
                .map_highlights(superseded.highlights.as_slice(), self.content.len()),
        };
        let highlights = Arc::new(highlights);
        self.debug_assert_highlights_on_char_boundaries(&highlights);
        highlights
    }

    /// A bound landing mid-character would corrupt the byte-length runs
    /// `text_run_for_style` builds from these ranges.
    #[inline]
    fn debug_assert_highlights_on_char_boundaries(
        &self,
        highlights: &[(Range<usize>, gpui::HighlightStyle)],
    ) {
        #[cfg(debug_assertions)]
        {
            let text = self.content.as_ref();
            for (range, _) in highlights {
                debug_assert!(
                    text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                    "interpolated highlight {range:?} must land on character boundaries"
                );
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = highlights;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn debug_effective_highlights_for_range(
        &mut self,
        byte_range: Range<usize>,
    ) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
        self.effective_highlights_for_window(byte_range)
            .highlights
            .as_ref()
            .clone()
    }

    pub fn set_highlights(
        &mut self,
        mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
        cx: &mut Context<Self>,
    ) {
        highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        if self.highlight.provider.is_none()
            && self.highlight.highlights.as_slice() == highlights.as_slice()
        {
            // Republishing the same vector still describes the buffer as it
            // stands now. A caller that rewrote the text first (a SHA field
            // swapping one 40-char id for another, say) left an edit patch
            // behind, and mapping these highlights through it would drop the
            // rewritten span back to the base color — so drop the patch even
            // though the vector itself is unchanged.
            if !self.highlight.interpolation.is_exact() {
                self.reset_highlight_interpolation();
                self.invalidate_highlights();
                cx.notify();
            }
            return;
        }
        self.supersede_current_highlight_source();
        self.highlight.highlights = Arc::new(highlights);
        self.highlight.provider = None;
        self.highlight.provider_binding_key = None;
        self.highlight.provider_poll_task.take();
        // A materialized vector answers the moment it is published.
        self.highlight.answered = true;
        self.highlight.superseded = None;
        // A fresh highlight source describes the buffer as it stands now.
        self.reset_highlight_interpolation();
        self.invalidate_highlights();
        cx.notify();
    }

    /// `source_len` is the byte length of the text the provider answers in.
    /// Highlight interpolation anchors edits to that text, so a provider built
    /// over a snapshot the buffer has since moved past would be mapped with the
    /// wrong origin — callers must read the buffer immediately before building.
    pub(super) fn install_highlight_provider(
        &mut self,
        provider: HighlightProvider,
        binding_key: Option<u64>,
        source_len: usize,
        cx: &mut Context<Self>,
    ) {
        debug_assert_eq!(
            source_len,
            self.content.len(),
            "a highlight provider must describe the buffer's current text"
        );

        if !should_reset_highlight_provider_binding(
            self.highlight.provider.is_some(),
            self.highlight.provider_binding_key,
            binding_key,
        ) {
            return;
        }

        // A provider over a freshly prepared document cannot answer until its
        // token chunks are built, so hold the outgoing source to cover the gap.
        self.supersede_current_highlight_source();
        self.highlight.provider = Some(provider);
        self.highlight.provider_binding_key = binding_key;
        self.highlight.provider_poll_task.take();
        self.highlight.highlights = Arc::new(Vec::new());
        self.highlight.answered = false;
        // Only past the early return: an unchanged binding key means the same
        // closure over the same text, so its anchor must survive.
        self.reset_highlight_interpolation();
        self.invalidate_highlights();
        cx.notify();
    }

    /// Replace the full highlight vector with a lazy provider that generates
    /// highlights on demand for only the visible byte range. Use this for large
    /// documents where materializing all highlights is wasteful.
    ///
    /// `binding_key` identifies the source the provider speaks for. Reinstalling
    /// under the same key keeps the existing highlight cache; a new key resets
    /// it, along with the edit interpolation that was tracking the old source.
    ///
    /// `source_len` is the byte length of the text the provider was built over;
    /// see `install_highlight_provider`.
    pub fn set_highlight_provider_with_key(
        &mut self,
        binding_key: u64,
        provider: HighlightProvider,
        source_len: usize,
        cx: &mut Context<Self>,
    ) {
        self.install_highlight_provider(provider, Some(binding_key), source_len, cx);
    }

    pub(super) fn resolve_provider_highlights(
        &mut self,
        byte_start: usize,
        byte_end: usize,
    ) -> ResolvedProviderHighlights {
        let requested_range = byte_start..byte_end;
        if let Some(cache) = self.highlight.provider_cache.as_mut()
            && let Some(resolved) = cache.resolve(self.highlight.epoch, &requested_range)
        {
            return resolved;
        }
        let Some(ref provider) = self.highlight.provider else {
            return ResolvedProviderHighlights {
                pending: false,
                highlights: Arc::new(Vec::new()),
            };
        };
        let mut result = provider.resolve(requested_range.clone());
        result
            .highlights
            .sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        let pending = result.pending;
        let highlights = Arc::new(result.highlights);
        self.highlight
            .provider_cache
            .get_or_insert_with(|| ProviderHighlightCache::new(self.highlight.epoch))
            .insert(
                self.highlight.epoch,
                requested_range,
                pending,
                Arc::clone(&highlights),
            );
        ResolvedProviderHighlights {
            pending,
            highlights,
        }
    }

    pub(super) fn ensure_highlight_provider_poll(&mut self, cx: &mut Context<Self>) {
        if self.highlight.provider_poll_task.is_some() {
            return;
        }

        let task = cx.spawn(
            async move |input: gpui::WeakEntity<TextInput>, cx: &mut gpui::AsyncApp| loop {
                // Route the poll delay through gpui's executor rather than
                // `smol::Timer`: the smol timer drives on the global async-io
                // reactor thread, which breaks the deterministic test scheduler
                // (it asserts against cross-thread activity at teardown).
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

                let should_continue = input
                    .update(cx, |input, cx| {
                        let Some(provider) = input.highlight.provider.clone() else {
                            input.highlight.provider_poll_task = None;
                            return false;
                        };

                        let applied = provider.drain_pending();
                        if applied > 0 {
                            input.note_provider_highlights_changed();
                            cx.notify();
                        }

                        let pending = provider.has_pending();
                        if !pending {
                            input.highlight.provider_poll_task = None;
                        }
                        pending
                    })
                    .unwrap_or(false);

                if !should_continue {
                    break;
                }
            },
        );
        self.highlight.provider_poll_task = Some(task);
    }

    pub(super) fn streamed_highlight_runs_for_visible_window(
        &mut self,
        display_text: &LineTextSource<'_>,
        line_starts: &[usize],
        visible_line_range: Range<usize>,
        shape_style: &TextShapeStyle<'_>,
    ) -> Option<Arc<VisibleWindowTextRuns>> {
        let Some(highlights) = shape_style.highlights else {
            self.highlight.prepaint_runs_cache = None;
            return None;
        };
        let line_count = line_starts.len().max(1);
        if highlights.is_empty()
            || line_count <= TEXT_INPUT_STREAMED_HIGHLIGHT_LEGACY_LINE_THRESHOLD
            || visible_line_range.is_empty()
        {
            self.highlight.prepaint_runs_cache = None;
            return None;
        }

        if let Some(cache) = self.highlight.prepaint_runs_cache.as_ref()
            && cache.highlight_epoch == self.highlight.epoch
            && cache.interpolation_generation == self.highlight.interpolation.generation()
            && cache.shape_style_epoch == self.layout.shape_style_epoch
            && cache.visible_start == visible_line_range.start
            && cache.visible_end == visible_line_range.end
        {
            return Some(Arc::clone(&cache.line_runs));
        }

        let line_runs = Arc::new(build_streamed_highlight_runs_for_visible_window(
            shape_style.base_font,
            shape_style.text_color,
            display_text,
            line_starts,
            visible_line_range.clone(),
            highlights,
        ));
        self.highlight.prepaint_runs_cache = Some(PrepaintHighlightRunsCache {
            highlight_epoch: self.highlight.epoch,
            interpolation_generation: self.highlight.interpolation.generation(),
            shape_style_epoch: self.layout.shape_style_epoch,
            visible_start: visible_line_range.start,
            visible_end: visible_line_range.end,
            line_runs: Arc::clone(&line_runs),
        });
        Some(line_runs)
    }
}
