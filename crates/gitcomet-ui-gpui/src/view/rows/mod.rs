use super::*;
use rustc_hash::FxHashMap as HashMap;
use std::cell::RefCell;

pub(in crate::view) const MAX_LINES_FOR_SYNTAX_HIGHLIGHTING: usize = 4_000;
const TEXT_LAYOUT_CACHE_PARTIAL_EVICT_DIVISOR: usize = 8;
const MAX_CACHED_LINE_NUMBER: usize = 16_384;

thread_local! {
    static LINE_NUMBER_STRINGS: RefCell<Vec<SharedString>> =
        RefCell::new(vec![SharedString::default()]);
}

fn line_number_string(n: Option<u32>) -> SharedString {
    let Some(n) = n else {
        return SharedString::default();
    };
    let ix = n as usize;
    if ix > MAX_CACHED_LINE_NUMBER {
        return n.to_string().into();
    }
    LINE_NUMBER_STRINGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() <= ix {
            let start = cache.len();
            cache.reserve(ix + 1 - start);
            for v in start..=ix {
                cache.push(v.to_string().into());
            }
        }
        cache[ix].clone()
    })
}

fn insert_with_partial_cache_eviction<V>(
    cache: &mut HashMap<u64, V>,
    key: u64,
    value: V,
    max_entries: usize,
) {
    if max_entries == 0 {
        return;
    }

    if !cache.contains_key(&key) && cache.len() >= max_entries {
        let evict_count = (max_entries / TEXT_LAYOUT_CACHE_PARTIAL_EVICT_DIVISOR).max(1);
        let target_len = max_entries.saturating_sub(evict_count);
        let remove_count = cache.len().saturating_sub(target_len);
        let keys_to_remove: Vec<u64> = cache.keys().take(remove_count).copied().collect();
        for old_key in keys_to_remove {
            cache.remove(&old_key);
        }
    }

    cache.insert(key, value);
}

mod canvas;
#[cfg(test)]
mod canvas_tests;
mod conflict_canvas;
mod conflict_resolver;
mod diff;
mod diff_canvas;
mod diff_text;
mod history;
mod history_canvas;
mod history_graph_paint;
mod sidebar;
mod status;

pub(crate) mod benchmarks;

pub(in crate::view) use diff_text::{
    BackgroundPreparedDiffSyntaxDocument, DiffSyntaxBudget, DiffSyntaxEdit, DiffSyntaxLanguage,
    DiffSyntaxMode, PrepareDiffSyntaxDocumentResult, PreparedDiffSyntaxDocument,
    PreparedDiffSyntaxLine, PreparedDiffSyntaxReparseSeed,
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds,
    has_pending_prepared_diff_syntax_chunk_builds_for_document,
    inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_line_for_inline_diff_row, prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_reparse_seed, request_syntax_highlights_for_prepared_document_byte_range,
    resolved_output_line_text, syntax_highlights_for_line,
};

#[cfg(test)]
pub(in crate::view) use diff_text::{
    PreparedDiffSyntaxParseMode, prepare_diff_syntax_document_in_background_text,
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_line_number_string_cache() {
        LINE_NUMBER_STRINGS.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.clear();
            cache.push(SharedString::default());
        });
    }

    fn line_number_string_cache_len() -> usize {
        LINE_NUMBER_STRINGS.with(|cache| cache.borrow().len())
    }

    #[test]
    fn line_number_cache_does_not_grow_for_uncached_large_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(8)), SharedString::from("8"));
        assert_eq!(line_number_string_cache_len(), 9);

        let uncached_line = (MAX_CACHED_LINE_NUMBER as u32).saturating_add(1);
        assert_eq!(
            line_number_string(Some(uncached_line)),
            uncached_line.to_string()
        );
        assert_eq!(line_number_string_cache_len(), 9);
    }

    #[test]
    fn line_number_cache_still_caches_small_numbers() {
        reset_line_number_string_cache();
        assert_eq!(line_number_string_cache_len(), 1);

        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string(Some(3)), SharedString::from("3"));
        assert_eq!(line_number_string(Some(1)), SharedString::from("1"));
        assert_eq!(line_number_string_cache_len(), 4);
    }

    #[test]
    fn partial_eviction_keeps_cache_bounded_without_full_clear() {
        let max_entries = 8usize;
        let mut cache: HashMap<u64, u64> = HashMap::default();
        for key in 0..max_entries as u64 {
            insert_with_partial_cache_eviction(&mut cache, key, key, max_entries);
        }

        assert_eq!(cache.len(), max_entries);
        insert_with_partial_cache_eviction(&mut cache, 999, 999, max_entries);

        assert!(cache.len() > 1);
        assert!(cache.len() <= max_entries);
        assert!(cache.contains_key(&999));
    }

    #[test]
    fn partial_eviction_updates_existing_entry_without_changing_len() {
        let max_entries = 8usize;
        let mut cache: HashMap<u64, u64> = HashMap::default();
        for key in 0..max_entries as u64 {
            insert_with_partial_cache_eviction(&mut cache, key, key, max_entries);
        }

        let len_before = cache.len();
        insert_with_partial_cache_eviction(&mut cache, 3, 300, max_entries);

        assert_eq!(cache.len(), len_before);
        assert_eq!(cache.get(&3), Some(&300));
    }

    #[test]
    fn partial_eviction_stays_within_limit_across_many_inserts() {
        let max_entries = 8usize;
        let mut cache: HashMap<u64, u64> = HashMap::default();
        for key in 0..128u64 {
            insert_with_partial_cache_eviction(&mut cache, key, key, max_entries);
        }

        assert!(cache.len() <= max_entries);
        assert!(cache.contains_key(&127));
    }
}
