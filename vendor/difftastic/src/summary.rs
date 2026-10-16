//! Data types summarising the result of diffing content.

use crate::parse::guess_language;
use crate::parse::syntax::MatchedPos;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextFallbackReason {
    ByteLimit,
    ParseErrors,
    GraphLimit,
    NestingLimit,
}

#[derive(Debug, Clone)]
pub(crate) enum FileFormat {
    SupportedLanguage(guess_language::Language),
    PlainText,
    TextFallback { reason: TextFallbackReason },
}

#[derive(Debug)]
pub(crate) struct DiffResult {
    pub(crate) file_format: FileFormat,
    pub(crate) lhs_src: String,
    pub(crate) rhs_src: String,

    pub(crate) lhs_positions: Vec<MatchedPos>,
    pub(crate) rhs_positions: Vec<MatchedPos>,
}
