use super::*;

pub(in crate::view) const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER: &str = "<Merge Conflict>";

/// Same, for a block whose sides differ only in whitespace.
pub(in crate::view) const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER: &str =
    "<Merge Conflict (Whitespace only)>";

pub(in crate::view) const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_LF: &str = "<Merge Conflict>\n";
pub(in crate::view) const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CRLF: &str = "<Merge Conflict>\r\n";
pub(in crate::view) const UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CR: &str = "<Merge Conflict>\r";

pub(in crate::view) const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_LF: &str =
    "<Merge Conflict (Whitespace only)>\n";
pub(in crate::view) const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CRLF: &str =
    "<Merge Conflict (Whitespace only)>\r\n";
pub(in crate::view) const UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CR: &str =
    "<Merge Conflict (Whitespace only)>\r";

/// Whether one output line is an unresolved-conflict placeholder row.
///
/// The resolved output is a text document, so a placeholder row is identified
/// by its own content — the same fact the reader sees. This keeps the gutter
/// marker in step with the text however the marker array was built, mirroring
/// how kdiff3 derives both from a single `srcSelect`.
pub(in crate::view) fn line_is_unresolved_conflict_placeholder(line: &str) -> bool {
    let line = line.trim_end_matches(['\r', '\n']);
    line == UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER
        || line == UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER
}

/// Whether this block renders as a placeholder row rather than as text.
///
/// An unresolved block with no side picked has nothing to show, so the output
/// carries one `<Merge Conflict>` row in its place.
pub(in crate::view) fn uses_unresolved_merge_conflict_placeholder(block: &ConflictBlock) -> bool {
    !block.resolved && block.choice.is_empty()
}

/// The single `<Merge Conflict>` row an unresolved block occupies, kdiff3's
/// `MergeBlockList::buildFromDiff3` (one `MergeEditLine` per block).
///
/// A block that spans several aligned source rows still collapses to this one
/// row, so the output's line numbers count the output's own lines rather than
/// the source columns'.
pub(in crate::view) fn unresolved_merge_conflict_placeholder_text(
    block: &ConflictBlock,
) -> &'static str {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, detect_conflict_block_line_ending,
    };

    let line_ending = detect_conflict_block_line_ending(ConflictOutputBlockRef {
        base: block.base.as_deref(),
        ours: &block.ours,
        theirs: &block.theirs,
        choice: block.choice,
        resolved: block.resolved,
    });
    // kdiff3 mergeresultwindow.cpp: a block whose sides differ only in
    // whitespace names itself, so the trivial ones can be told apart from real
    // clashes without opening them.
    if block.whitespace_only {
        return match line_ending {
            "\r\n" => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CRLF,
            "\r" => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_CR,
            _ => UNRESOLVED_WHITESPACE_CONFLICT_PLACEHOLDER_LF,
        };
    }
    match line_ending {
        "\r\n" => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CRLF,
        "\r" => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_CR,
        _ => UNRESOLVED_MERGE_CONFLICT_PLACEHOLDER_LF,
    }
}

pub(in crate::view) fn editable_conflict_block_len(block: &ConflictBlock) -> usize {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    if uses_unresolved_merge_conflict_placeholder(block) {
        return unresolved_merge_conflict_placeholder_text(block).len();
    }
    block.choice.iter().fold(0usize, |len, source| {
        len.saturating_add(match source {
            ConflictOutputSource::Base => block.base.as_ref().map_or(0, ConflictText::len),
            ConflictOutputSource::Ours => block.ours.len(),
            ConflictOutputSource::Theirs => block.theirs.len(),
        })
    })
}

/// Generate the editable merge-output projection.
///
/// A truly unresolved block has no selected sources and occupies one named
/// KDiff3-style placeholder row, however many aligned rows it spans in the
/// source columns. Resolved blocks retain their ordered source selection, while
/// marker-preserving save/export use [`generate_resolved_text_with_options`]
/// directly.
pub fn generate_resolved_text(segments: &[ConflictSegment]) -> String {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    let mut output = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(text) => output.push_str(text),
            ConflictSegment::Block(block) if uses_unresolved_merge_conflict_placeholder(block) => {
                output.push_str(unresolved_merge_conflict_placeholder_text(block));
            }
            ConflictSegment::Block(block) => {
                for source in block.choice.iter() {
                    match source {
                        ConflictOutputSource::Base => {
                            if let Some(base) = block.base.as_deref() {
                                output.push_str(base);
                            }
                        }
                        ConflictOutputSource::Ours => output.push_str(&block.ours),
                        ConflictOutputSource::Theirs => output.push_str(&block.theirs),
                    }
                }
            }
        }
    }
    output
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedOutputText {
    Shared(Arc<str>),
    Owned(String),
}

impl ResolvedOutputText {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shared(text) => text.as_ref(),
            Self::Owned(text) => text.as_str(),
        }
    }

    pub fn line_count(&self) -> usize {
        text_line_count_usize(self.as_str())
    }

    pub fn into_shared_string(self) -> gpui::SharedString {
        match self {
            Self::Shared(text) => text.into(),
            Self::Owned(text) => text.into(),
        }
    }
}

pub fn bootstrap_resolved_output_text(
    segments: &[ConflictSegment],
    current_text: Option<&Arc<str>>,
    ours_text: Option<&Arc<str>>,
    theirs_text: Option<&Arc<str>>,
) -> ResolvedOutputText {
    if segments.is_empty() {
        return current_text
            .or(ours_text)
            .or(theirs_text)
            .cloned()
            .map(ResolvedOutputText::Shared)
            .unwrap_or_else(|| ResolvedOutputText::Owned(String::new()));
    }

    ResolvedOutputText::Owned(generate_resolved_text(segments))
}

pub fn generate_resolved_text_with_options(
    segments: &[ConflictSegment],
    options: gitcomet_core::conflict_output::GenerateResolvedTextOptions<'_>,
) -> String {
    use gitcomet_core::conflict_output::{
        ConflictOutputBlockRef, ConflictOutputSegmentRef,
        generate_resolved_text as generate_core_resolved_text,
    };

    let core_segments: Vec<ConflictOutputSegmentRef<'_>> = segments
        .iter()
        .map(|segment| match segment {
            ConflictSegment::Text(text) => ConflictOutputSegmentRef::Text(text),
            ConflictSegment::Block(block) => {
                ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                    base: block.base.as_deref(),
                    ours: &block.ours,
                    theirs: &block.theirs,
                    choice: block.choice,
                    resolved: block.resolved,
                })
            }
        })
        .collect();

    generate_core_resolved_text(&core_segments, options)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum ResolvedOutputFragmentSource {
    TextSegment { segment_ix: usize },
    BlockBase { segment_ix: usize },
    BlockOurs { segment_ix: usize },
    BlockTheirs { segment_ix: usize },
    UnresolvedPlaceholder { text: &'static str },
}

pub(in crate::view) fn resolved_output_block_source_fragment(
    segment_ix: usize,
    block: &ConflictBlock,
    source: gitcomet_core::conflict_output::ConflictOutputSource,
) -> Option<(ResolvedOutputFragmentSource, &str)> {
    use gitcomet_core::conflict_output::ConflictOutputSource;

    match source {
        ConflictOutputSource::Base => block
            .base
            .as_deref()
            .map(|base| (ResolvedOutputFragmentSource::BlockBase { segment_ix }, base)),
        ConflictOutputSource::Ours => Some((
            ResolvedOutputFragmentSource::BlockOurs { segment_ix },
            block.ours.as_str(),
        )),
        ConflictOutputSource::Theirs => Some((
            ResolvedOutputFragmentSource::BlockTheirs { segment_ix },
            block.theirs.as_str(),
        )),
    }
}
