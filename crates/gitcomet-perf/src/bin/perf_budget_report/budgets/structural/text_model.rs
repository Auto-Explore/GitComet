use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
    // --- text_input_long_line_cap structural budgets ---
    // Defaults: 256 KiB line, 4096-byte cap, 64 iterations.
    // Capped variant truncates the line; uncapped variant processes the full line.
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "capped_len",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "iterations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "cap_active",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "capped_len",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "iterations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "cap_active",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- text_input_wrap_incremental_tabs structural budgets ---
    // Defaults: 20,000 tabbed lines, requested 128-byte minimum => 131-byte
    // generated lines, 720 px wrap width => 92 wrap columns. The first edit
    // mutates line 0 and invalidates the edited line plus one neighbor.
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 131.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "edit_line_ix",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 131.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "edit_line_ix",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_input_wrap_incremental_burst_edits structural budgets ---
    // Defaults: 20,000 tabbed lines, 128-byte minimum => 131-byte generated
    // lines, 720 px wrap => 92 columns, 12 edits per burst. Each burst scatters
    // 12 edits across well-spaced lines (stride 17); the burst fixture now
    // mirrors live TextInput dirty invalidation by rescanning only the edited
    // line for each single-line mutation. Full recompute still recomputes all
    // 20,000 lines per edit.
    // These sidecars follow the Criterion bench ids with the default `/12`
    // burst-size segment, so structural lookups must match that emitted path.
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "edits_per_burst",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "edits_per_burst",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_model_snapshot_clone_cost structural budgets ---
    // Defaults: 2 MiB minimum text-model document expands to 2,097,154 bytes
    // across 37,183 stored line-start markers. Both variants clone 8,192
    // times and sample a 96-byte prefix from each clone.
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "document_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_097_154.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "line_starts",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 37_183.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "clone_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 8_192.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "sampled_prefix_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "snapshot_path",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "document_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_097_154.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "line_starts",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 37_183.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "clone_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 8_192.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "sampled_prefix_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "snapshot_path",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- text_model_bulk_load_large structural budgets ---
    // Defaults: 20,000 lines × ~130 bytes/line + newlines ≈ 2.5+ MiB source.
    // Piece-table variants produce document_bytes_after == source_bytes and
    // line_starts_after >= 20,001.  String-push control has no line tracking.
    // append_large uses 2 chunks, from_large_text uses 1, string_push uses ≈80.
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 20_001.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 20_001.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    // --- text_model_fragmented_edits structural budgets ---
    // Defaults: 512 KiB minimum source expands to 524,295 bytes. The
    // deterministic 500-edit sequence deletes 3,681 bytes, inserts 3,990
    // bytes, and leaves a 524,604-byte document with 9,806 line starts.
    // readback_operations encodes post-edit validation work:
    // 0 = edit-only/control, 1 = single as_str(), 64 = shared-string loop.
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
];
