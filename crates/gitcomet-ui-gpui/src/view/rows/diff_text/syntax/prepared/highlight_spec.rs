use super::*;

impl TreesitterHighlightSpec {
    /// Whether match `pattern_ix` belongs to a shared layer.
    ///
    /// Reads the hoisted `has_combined_injections` gate first, so a grammar with no
    /// combined pattern costs one branch. Shared by both collectors, which used to
    /// inline this and drifted apart.
    pub(crate) fn is_combined_injection_pattern(&self, pattern_ix: usize) -> bool {
        self.has_combined_injections
            && self
                .injection_combined_patterns
                .get(pattern_ix)
                .copied()
                .unwrap_or(false)
    }
}

/// Merges the per-pattern range map both collectors build into a deterministically
/// ordered layer list.
///
/// The order is load-bearing: overlapping layers are applied in sequence and the
/// later one wins, so `FxHashMap` order would tie an overlap's colour to hash seeding.
pub(crate) fn combined_injection_groups_in_apply_order(
    combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>>,
) -> Vec<(DiffSyntaxLanguage, usize, Vec<Range<usize>>)> {
    let mut groups: Vec<(DiffSyntaxLanguage, usize, Vec<Range<usize>>)> = combined_ranges
        .into_iter()
        .filter_map(|((language, pattern_ix), ranges)| {
            let ranges = merge_sorted_injection_ranges(ranges);
            (!ranges.is_empty()).then_some((language, pattern_ix, ranges))
        })
        .collect();
    groups.sort_unstable_by_key(|(language, pattern_ix, _)| (*language, *pattern_ix));
    groups
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TreesitterQueryPass {
    pub(crate) byte_range: Range<usize>,
    pub(crate) containing_byte_range: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TreesitterInjectionMatch {
    /// Identity of the root text this injection was found in. Nested injections
    /// retain the root document's identity and use root-relative byte ranges, so
    /// pair lookup can consider every layer without admitting a sibling document.
    ///
    /// Without it the other three fields can describe two *different* documents'
    /// injections at once, and [`injected_syntax_pair_at`] iterates a cache
    /// shared by every document. Changing a Markdown fence from ```` ```html ````
    /// to ```` ```bash ```` keeps the fenced bytes and their offsets identical
    /// and moves only `language`, so both keys go live -- and in a diff both
    /// sides are tokenized on this same thread. The lookup would then answer a
    /// click with whichever grammar the hash map happened to yield first.
    pub(crate) document_hash: u64,
    pub(crate) language: DiffSyntaxLanguage,
    pub(crate) byte_start: usize,
    pub(crate) byte_end: usize,
    /// Hash of the injection content bytes. This ensures the cache is not
    /// confused when different parent documents happen to produce injection
    /// regions at the same byte offsets *with different content*. It cannot
    /// separate same-content revisions on its own; that is `document_hash`'s job.
    pub(crate) content_hash: u64,
}

#[derive(Clone)]
pub(crate) struct CachedInjection {
    /// Full tokenized lines in injection-local coordinates (all lines of the injection).
    pub(crate) all_line_tokens: Vec<Vec<SyntaxToken>>,
    /// Line starts for the injection text, used for coordinate remapping.
    pub(crate) injection_line_starts: Vec<usize>,
    /// First line in the parent document that this injection starts on.
    pub(crate) injection_start_line_ix: usize,
    /// The injected grammar's own tree, kept so a click can be answered by the
    /// grammar that actually owns those bytes.
    ///
    /// Tokens alone were enough while this cache only ever painted. Bracket
    /// matching reads the tree, and `prepared_document_syntax_pair_at_display_offset`
    /// had only the *host* tree -- so in an injected region there was no
    /// structure to pair against at all: clicking the `<` of `<html>` in a PHP
    /// file did nothing, because to PHP that whole span is one `text` node.
    /// Parsed during tokenization so the normal click path does not pay for a
    /// parse. Pair lookup recreates it only when this entry was evicted while
    /// the prepared document itself remained cached.
    ///
    /// Its offsets are injection-local: the injected text is parsed standalone,
    /// not with `included_ranges`, so document offsets need shifting by
    /// `byte_start` in both directions. The live engine differs here -- see
    /// `LiveSyntaxSnapshot::syntax_pair_at`, whose layers are already in document
    /// coordinates.
    pub(crate) tree: tree_sitter::Tree,
    /// Monotonic access counter for LRU eviction.
    pub(crate) last_access: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct TreesitterQueryAsset {
    pub(crate) highlights: &'static str,
    pub(crate) injections: Option<&'static str>,
    /// Extra patterns appended to `highlights` before it is compiled.
    ///
    /// Several grammars are used with the query their own crate ships, which
    /// cannot be edited here and in places captures nothing for constructs that
    /// matter -- brackets, or in Objective-C's case comments and strings. This
    /// is how those are filled in without vendoring a whole query and taking on
    /// the job of tracking upstream's. Appended, not prepended: overlapping
    /// captures resolve last-wins, so a supplement can also correct a capture
    /// upstream got wrong.
    pub(crate) supplement: Option<&'static str>,
}

impl TreesitterQueryAsset {
    pub(crate) const fn highlights(source: &'static str) -> Self {
        Self {
            highlights: source,
            injections: None,
            supplement: None,
        }
    }

    pub(crate) const fn with_injections(
        highlights: &'static str,
        injections: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: Some(injections),
            supplement: None,
        }
    }

    /// Appends in-tree patterns to a query this repo does not own.
    pub(crate) const fn with_supplement(
        highlights: &'static str,
        supplement: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: None,
            supplement: Some(supplement),
        }
    }

    pub(crate) const fn with_injections_and_supplement(
        highlights: &'static str,
        injections: &'static str,
        supplement: &'static str,
    ) -> Self {
        Self {
            highlights,
            injections: Some(injections),
            supplement: Some(supplement),
        }
    }
}
