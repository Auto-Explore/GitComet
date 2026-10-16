//! Diff limits and options.

pub(crate) const DEFAULT_BYTE_LIMIT: usize = 1_000_000;
// Chosen experimentally: this is sufficiently many for all the sample
// files (the highest is slow_1.rs/slow_2.rs at 1.3M nodes), but
// small enough to terminate in ~5 seconds like the test file in #306.
pub(crate) const DEFAULT_GRAPH_LIMIT: usize = 3_000_000;
pub(crate) const DEFAULT_PARSE_ERROR_LIMIT: usize = 0;

#[derive(Clone, Copy)]
pub(crate) struct DiffOptions<'a> {
    pub(crate) graph_limit: usize,
    pub(crate) byte_limit: usize,
    pub(crate) parse_error_limit: usize,
    pub(crate) ignore_comments: bool,
    pub(crate) nesting_limit: usize,
    pub(crate) should_stop: &'a dyn Fn() -> bool,
}

pub(crate) fn never_stop() -> bool {
    false
}

impl Default for DiffOptions<'static> {
    fn default() -> Self {
        Self {
            graph_limit: DEFAULT_GRAPH_LIMIT,
            byte_limit: DEFAULT_BYTE_LIMIT,
            parse_error_limit: DEFAULT_PARSE_ERROR_LIMIT,
            ignore_comments: false,
            nesting_limit: usize::MAX,
            should_stop: &never_stop,
        }
    }
}
