use tree_sitter::StreamingIterator;

use super::{TS_CURSOR, invalidate_ts_parser_language_fast_path};

/// A query loop's proof that it runs inside [`catch_treesitter_query_panic`],
/// the only place that recovers from what [`QueryCatch::advance`] resumes.
pub(crate) struct QueryCatch(());

impl QueryCatch {
    /// Tree-sitter evaluates query predicates while advancing, and panics when
    /// a recovered node's byte range runs past the text: contain that, then
    /// resume the unwind, which skips the crash hook.
    pub(crate) fn advance(&self, query: &mut impl StreamingIterator) {
        if let Err(panic) = gitcomet_core::panic::contain(|| query.advance()) {
            std::panic::resume_unwind(Box::new(panic.message));
        }
    }
}

/// A query loop that panicked is a syntax miss, not a UI crash. Upstream
/// predicate panics arrive from [`QueryCatch::advance`] without a crash report;
/// a panic in the loop's own code still writes one.
pub(crate) fn catch_treesitter_query_panic<R>(f: impl FnOnce(&QueryCatch) -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&QueryCatch(())))) {
        Ok(result) => Some(result),
        Err(_) => {
            TS_CURSOR.with(|cursor| {
                *cursor.borrow_mut() = tree_sitter::QueryCursor::new();
            });
            invalidate_ts_parser_language_fast_path();
            None
        }
    }
}
