use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
    // --- resolved_output_recompute_incremental structural budgets ---
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "requested_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "conflict_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 300.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "unresolved_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "both_choice_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 75.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "outline_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "marker_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 675.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "manual_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "recomputed_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "requested_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "conflict_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 300.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "unresolved_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "both_choice_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 75.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "outline_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "marker_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 675.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "manual_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "dirty_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "recomputed_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "fallback_full_recompute",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // large_html_syntax synthetic sidecar budgets. These pin the default 20k-line
    // synthetic fixture shape and the current prepared-window cache-hit profile.
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "line_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_000_000.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // These sidecars now live under the current Criterion bench ids without the
    // trailing `/160`; `window_lines` remains pinned below as a structural metric.
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "loaded_chunks",
        comparator: StructuralBudgetComparator::AtLeast,
        // The pending path does not wait for background chunk building, so
        // loaded_chunks is inherently low and timing-dependent. Require at
        // least 1 chunk (document was prepared) but do not pin the full count.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 81.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_misses",
        // Sweep window starts at line 81 which crosses chunk boundaries not
        // covered by the initial primed window; 49 misses is the stable
        // measured value for the current 160-line sweep position.
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 60.0,
    },
    // --- worktree_preview_render structural budgets ---
    // Pin the deterministic fixture shape for both cached-lookup and render-time-prepare paths.
    // Defaults: 4000 lines, 200-line window, 128 bytes/line, Rust syntax (Auto mode, prepared doc present).
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_000.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_000.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- diff_scroll structural budgets ---
    // Pin the default diff-scroll fixture shape for the normal and long-line variants.
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "visible_text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 19_200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "min_line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "language_detected",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "visible_text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 819_200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "min_line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_096.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "language_detected",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_input_prepaint_windowed structural budgets ---
    // Pin the deterministic fixture shape for windowed and full-document paths.
    // Defaults: 20,000 lines, 80-row viewport, guard_rows=2, max_shape_bytes=4096.
    // Windowed variant (cold run): shapes 80 + 2*2 = 84 rows, all cache misses.
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "viewport_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "guard_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_entries_after",
        // Cold run: 80 + 2*2 = 84 rows shaped, all new cache entries.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 84.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_hits",
        // Cold run from empty cache — no hits expected.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_misses",
        // Cold run: all 84 rows are misses.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 84.0,
    },
    // Full-document control: shapes all 20,000 lines (+ guard rows), all misses.
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "viewport_rows",
        // Full doc: viewport_rows == total_lines == 20,000.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "guard_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_entries_after",
        // Full doc cold run: 20,000 unique lines (guard rows wrap to existing indices).
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_hits",
        // Cold run: guard rows (4) wrap to lines 0-3 which were already cached.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_misses",
        // Full doc cold run: 20,000 unique lines are misses; 4 guard rows are hits.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    // --- text_input_runs_streamed_highlight structural budgets ---
    // Defaults: 20,000 lines, 80 visible rows, scroll step = 40.
    // Dense fixture highlights every visible line; sparse highlights every 8th
    // line plus every 24th line for the overlay spans.
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "total_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3334.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 14.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "total_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3334.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 14.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
];
