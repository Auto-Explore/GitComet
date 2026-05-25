use super::*;

#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct CachedDiffTextSegment {
    pub(super) text: SharedString,
    pub(super) in_word: bool,
    pub(super) in_query: bool,
    pub(super) syntax: SyntaxTokenKind,
}

#[derive(Clone, Debug)]
pub(super) struct CachedDiffStyledText {
    pub(super) text: SharedString,
    pub(super) highlights: Arc<[(Range<usize>, gpui::HighlightStyle)]>,
    pub(super) highlights_hash: u64,
    pub(super) text_hash: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SyntaxTokenKind {
    None,
    Comment,
    CommentDoc,
    String,
    StringEscape,
    StringRegex,
    StringSpecial,
    Keyword,
    KeywordControl,
    Preproc,
    Number,
    Boolean,
    Function,
    FunctionMethod,
    FunctionSpecial,
    Constructor,
    Type,
    TypeBuiltin,
    TypeInterface,
    Namespace,
    Variable,
    VariableParameter,
    VariableSpecial,
    VariableBuiltin,
    Property,
    Label,
    Constant,
    ConstantBuiltin,
    Operator,
    Punctuation,
    PunctuationBracket,
    PunctuationDelimiter,
    PunctuationSpecial,
    PunctuationListMarker,
    Tag,
    Attribute,
    MarkupHeading,
    MarkupLink,
    TextLiteral,
    DiffPlus,
    DiffMinus,
    DiffDelta,
    Lifetime,
}
