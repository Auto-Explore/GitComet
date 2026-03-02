use crate::text_utils::{LineEndingDetectionMode, detect_line_ending_from_texts};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictOutputChoice {
    Base,
    Ours,
    Theirs,
    Both,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictOutputBlockRef<'a> {
    pub base: Option<&'a str>,
    pub ours: &'a str,
    pub theirs: &'a str,
    pub choice: ConflictOutputChoice,
    pub resolved: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictOutputSegmentRef<'a> {
    Text(&'a str),
    Block(ConflictOutputBlockRef<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictMarkerLabels<'a> {
    pub local: &'a str,
    pub remote: &'a str,
    pub base: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnresolvedConflictMode {
    CollapseToChoice,
    PreserveMarkers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerateResolvedTextOptions<'a> {
    pub unresolved_mode: UnresolvedConflictMode,
    pub labels: Option<ConflictMarkerLabels<'a>>,
}

impl Default for GenerateResolvedTextOptions<'_> {
    fn default() -> Self {
        Self {
            unresolved_mode: UnresolvedConflictMode::CollapseToChoice,
            labels: None,
        }
    }
}

pub fn detect_conflict_block_line_ending(block: ConflictOutputBlockRef<'_>) -> &'static str {
    detect_line_ending_from_texts(
        [block.ours, block.theirs, block.base.unwrap_or_default()],
        LineEndingDetectionMode::Presence,
    )
}

pub fn render_unresolved_marker_block(
    block: ConflictOutputBlockRef<'_>,
    labels: ConflictMarkerLabels<'_>,
) -> String {
    let newline = detect_conflict_block_line_ending(block);
    let mut out = String::new();
    out.push_str("<<<<<<< ");
    out.push_str(labels.local);
    out.push_str(newline);
    out.push_str(block.ours);

    // Ensure each marker starts on its own line even when content lacks a
    // trailing line ending.
    if !block.ours.is_empty() && !block.ours.ends_with(newline) {
        out.push_str(newline);
    }
    if let Some(base) = block.base {
        out.push_str("||||||| ");
        out.push_str(labels.base);
        out.push_str(newline);
        out.push_str(base);
        if !base.is_empty() && !base.ends_with(newline) {
            out.push_str(newline);
        }
    }
    out.push_str("=======");
    out.push_str(newline);
    out.push_str(block.theirs);
    if !block.theirs.is_empty() && !block.theirs.ends_with(newline) {
        out.push_str(newline);
    }
    out.push_str(">>>>>>> ");
    out.push_str(labels.remote);
    out.push_str(newline);
    out
}

pub fn generate_resolved_text(
    segments: &[ConflictOutputSegmentRef<'_>],
    options: GenerateResolvedTextOptions<'_>,
) -> String {
    let mut output = String::new();
    for segment in segments {
        match *segment {
            ConflictOutputSegmentRef::Text(text) => output.push_str(text),
            ConflictOutputSegmentRef::Block(block) => {
                if block.resolved
                    || options.unresolved_mode == UnresolvedConflictMode::CollapseToChoice
                    || options.labels.is_none()
                {
                    append_chosen_block_text(&mut output, block);
                } else if let Some(labels) = options.labels {
                    output.push_str(&render_unresolved_marker_block(block, labels));
                }
            }
        }
    }
    output
}

fn append_chosen_block_text(output: &mut String, block: ConflictOutputBlockRef<'_>) {
    match block.choice {
        ConflictOutputChoice::Base => {
            if let Some(base) = block.base {
                output.push_str(base);
            }
        }
        ConflictOutputChoice::Ours => output.push_str(block.ours),
        ConflictOutputChoice::Theirs => output.push_str(block.theirs),
        ConflictOutputChoice::Both => {
            output.push_str(block.ours);
            output.push_str(block.theirs);
        }
    }
}
