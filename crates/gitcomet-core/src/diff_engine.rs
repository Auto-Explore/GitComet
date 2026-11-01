use std::collections::BTreeMap;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum DiffEngine {
    #[default]
    LineByLine,
    Difftastic,
}

impl DiffEngine {
    pub const ALL: [Self; 2] = [Self::LineByLine, Self::Difftastic];

    pub const fn key(self) -> &'static str {
        match self {
            Self::LineByLine => "line_by_line",
            Self::Difftastic => "difftastic",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value {
            "line_by_line" => Some(Self::LineByLine),
            "difftastic" => Some(Self::Difftastic),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::LineByLine => "Line by line",
            Self::Difftastic => "Difftastic",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffEngineFallback {
    MissingSources,
    UnsupportedLanguage,
    ParseErrors,
    ByteLimit,
    GraphLimit,
    NestingLimit,
    Timeout,
    InvalidOutput,
    Failed,
}

impl DiffEngineFallback {
    pub const fn label(self) -> &'static str {
        match self {
            Self::MissingSources => "Complete source texts are unavailable",
            Self::UnsupportedLanguage => "Language is not supported by Difftastic",
            Self::ParseErrors => "Difftastic could not parse this file",
            Self::ByteLimit => "File exceeds Difftastic's size limit",
            Self::GraphLimit => "Comparison exceeds Difftastic's complexity limit",
            Self::NestingLimit => "File is nested too deeply for Difftastic",
            Self::Timeout => "Difftastic exceeded its time limit",
            Self::InvalidOutput => "Difftastic returned an invalid comparison",
            Self::Failed => "Difftastic could not complete this comparison",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuralDiff {
    pub language: String,
    pub aligned_lines: Vec<AlignedLine>,
    pub old_changes: BTreeMap<usize, Vec<Range<usize>>>,
    pub new_changes: BTreeMap<usize, Vec<Range<usize>>>,
    pub blocks: Vec<Range<usize>>,
}

pub type AlignedLine = (Option<usize>, Option<usize>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructuralDiffOutcome {
    Compared(StructuralDiff),
    Unchanged,
    Fallback(DiffEngineFallback),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DiffEngineStatus {
    #[default]
    LineByLine,
    Difftastic {
        syntactic_changes: bool,
        text_differs: bool,
    },
    Fallback(DiffEngineFallback),
}
