use crate::text_utils::{LineEndingDetectionMode, detect_line_ending_from_texts};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ConflictOutputSource {
    Base,
    Ours,
    Theirs,
}

/// A unique ordered set of conflict-output sources.
///
/// The associated constants preserve the original single-choice API while
/// allowing arbitrary A/B/C toggle order without allocation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConflictOutputChoice {
    sources: [Option<ConflictOutputSource>; 3],
    len: u8,
}

#[allow(non_upper_case_globals)]
impl ConflictOutputChoice {
    pub const Base: Self = Self::from_source_const(ConflictOutputSource::Base);
    pub const Ours: Self = Self::from_source_const(ConflictOutputSource::Ours);
    pub const Theirs: Self = Self::from_source_const(ConflictOutputSource::Theirs);
    pub const Both: Self = Self {
        sources: [
            Some(ConflictOutputSource::Ours),
            Some(ConflictOutputSource::Theirs),
            None,
        ],
        len: 2,
    };

    const fn from_source_const(source: ConflictOutputSource) -> Self {
        Self {
            sources: [Some(source), None, None],
            len: 1,
        }
    }

    pub const fn empty() -> Self {
        Self {
            sources: [None, None, None],
            len: 0,
        }
    }

    pub fn from_sources(sources: impl IntoIterator<Item = ConflictOutputSource>) -> Self {
        let mut choice = Self::empty();
        for source in sources {
            choice.append(source);
        }
        choice
    }

    pub fn iter(self) -> impl Iterator<Item = ConflictOutputSource> {
        self.sources
            .into_iter()
            .take(usize::from(self.len))
            .flatten()
    }

    pub fn contains(self, source: ConflictOutputSource) -> bool {
        self.iter().any(|candidate| candidate == source)
    }

    pub fn first(self) -> Option<ConflictOutputSource> {
        self.iter().next()
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    pub fn len(self) -> usize {
        usize::from(self.len)
    }

    pub fn append(&mut self, source: ConflictOutputSource) {
        if self.contains(source) || self.len() == self.sources.len() {
            return;
        }
        self.sources[self.len()] = Some(source);
        self.len += 1;
    }

    pub fn toggle(&mut self, source: ConflictOutputSource) {
        let Some(index) = self.iter().position(|candidate| candidate == source) else {
            self.append(source);
            return;
        };
        let len = self.len();
        self.sources.copy_within(index + 1..len, index);
        self.sources[len - 1] = None;
        self.len -= 1;
    }
}

impl Default for ConflictOutputChoice {
    fn default() -> Self {
        Self::Ours
    }
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
    for source in block.choice.iter() {
        match source {
            ConflictOutputSource::Base => {
                if let Some(base) = block.base {
                    output.push_str(base);
                }
            }
            ConflictOutputSource::Ours => output.push_str(block.ours),
            ConflictOutputSource::Theirs => output.push_str(block.theirs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict_session::{ParsedConflictSegment, parse_conflict_marker_segments};

    fn labels() -> ConflictMarkerLabels<'static> {
        ConflictMarkerLabels {
            local: "LOCAL",
            remote: "REMOTE",
            base: "BASE",
        }
    }

    fn preserve_markers_options() -> GenerateResolvedTextOptions<'static> {
        GenerateResolvedTextOptions {
            unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
            labels: Some(labels()),
        }
    }

    fn build_output_segments<'a>(
        parsed_segments: &'a [ParsedConflictSegment],
    ) -> Vec<ConflictOutputSegmentRef<'a>> {
        parsed_segments
            .iter()
            .map(|segment| match segment {
                ParsedConflictSegment::Text(text) => ConflictOutputSegmentRef::Text(text),
                ParsedConflictSegment::Conflict(block) => {
                    ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                        base: block.base.as_deref(),
                        ours: &block.ours,
                        theirs: &block.theirs,
                        choice: ConflictOutputChoice::Ours,
                        resolved: false,
                    })
                }
            })
            .collect()
    }

    #[test]
    fn generate_resolved_text_clean_context_only() {
        let segments = vec![
            ConflictOutputSegmentRef::Text("hello\n"),
            ConflictOutputSegmentRef::Text("world\n"),
        ];

        let output = generate_resolved_text(&segments, GenerateResolvedTextOptions::default());
        assert_eq!(output, "hello\nworld\n");
    }

    #[test]
    fn generate_resolved_text_resolved_choices() {
        let ours = generate_resolved_text(
            &[ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: Some("BASE\n"),
                ours: "OURS\n",
                theirs: "THEIRS\n",
                choice: ConflictOutputChoice::Ours,
                resolved: true,
            })],
            preserve_markers_options(),
        );
        assert_eq!(ours, "OURS\n");

        let theirs = generate_resolved_text(
            &[ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: Some("BASE\n"),
                ours: "OURS\n",
                theirs: "THEIRS\n",
                choice: ConflictOutputChoice::Theirs,
                resolved: true,
            })],
            preserve_markers_options(),
        );
        assert_eq!(theirs, "THEIRS\n");

        let both = generate_resolved_text(
            &[ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: Some("BASE\n"),
                ours: "OURS\n",
                theirs: "THEIRS\n",
                choice: ConflictOutputChoice::Both,
                resolved: true,
            })],
            preserve_markers_options(),
        );
        assert_eq!(both, "OURS\nTHEIRS\n");

        let base = generate_resolved_text(
            &[ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: Some("BASE\n"),
                ours: "OURS\n",
                theirs: "THEIRS\n",
                choice: ConflictOutputChoice::Base,
                resolved: true,
            })],
            preserve_markers_options(),
        );
        assert_eq!(base, "BASE\n");
    }

    #[test]
    fn generate_resolved_text_unresolved_two_way_preserves_markers() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: None,
            ours: "ours\n",
            theirs: "theirs\n",
            choice: ConflictOutputChoice::Ours,
            resolved: false,
        })];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "<<<<<<< LOCAL\nours\n=======\ntheirs\n>>>>>>> REMOTE\n"
        );
    }

    #[test]
    fn generate_resolved_text_unresolved_diff3_preserves_markers() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: Some("base\n"),
            ours: "ours\n",
            theirs: "theirs\n",
            choice: ConflictOutputChoice::Ours,
            resolved: false,
        })];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "<<<<<<< LOCAL\nours\n||||||| BASE\nbase\n=======\ntheirs\n>>>>>>> REMOTE\n"
        );
    }

    #[test]
    fn generate_resolved_text_unresolved_preserves_crlf() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: Some("base\r\n"),
            ours: "ours\r\n",
            theirs: "theirs\r\n",
            choice: ConflictOutputChoice::Ours,
            resolved: false,
        })];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "<<<<<<< LOCAL\r\nours\r\n||||||| BASE\r\nbase\r\n=======\r\ntheirs\r\n>>>>>>> REMOTE\r\n"
        );
    }

    #[test]
    fn generate_resolved_text_unresolved_preserves_cr() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: Some("base\r"),
            ours: "ours\r",
            theirs: "theirs\r",
            choice: ConflictOutputChoice::Ours,
            resolved: false,
        })];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "<<<<<<< LOCAL\rours\r||||||| BASE\rbase\r=======\rtheirs\r>>>>>>> REMOTE\r"
        );
    }

    #[test]
    fn generate_resolved_text_unresolved_without_trailing_newline_still_well_formed() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: Some("base no newline"),
            ours: "ours no newline",
            theirs: "theirs no newline",
            choice: ConflictOutputChoice::Ours,
            resolved: false,
        })];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "<<<<<<< LOCAL\nours no newline\n||||||| BASE\nbase no newline\n=======\ntheirs no newline\n>>>>>>> REMOTE\n"
        );
    }

    #[test]
    fn generate_resolved_text_mixed_resolved_and_unresolved_blocks() {
        let segments = vec![
            ConflictOutputSegmentRef::Text("header\n"),
            ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: None,
                ours: "A\n",
                theirs: "B\n",
                choice: ConflictOutputChoice::Ours,
                resolved: true,
            }),
            ConflictOutputSegmentRef::Text("middle\n"),
            ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
                base: None,
                ours: "C\n",
                theirs: "D\n",
                choice: ConflictOutputChoice::Theirs,
                resolved: false,
            }),
            ConflictOutputSegmentRef::Text("footer\n"),
        ];

        let output = generate_resolved_text(&segments, preserve_markers_options());
        assert_eq!(
            output,
            "header\nA\nmiddle\n<<<<<<< LOCAL\nC\n=======\nD\n>>>>>>> REMOTE\nfooter\n"
        );
    }

    #[test]
    fn generate_resolved_text_unresolved_without_labels_collapses_to_choice() {
        let segments = [ConflictOutputSegmentRef::Block(ConflictOutputBlockRef {
            base: Some("BASE\n"),
            ours: "OURS\n",
            theirs: "THEIRS\n",
            choice: ConflictOutputChoice::Theirs,
            resolved: false,
        })];

        let output = generate_resolved_text(
            &segments,
            GenerateResolvedTextOptions {
                unresolved_mode: UnresolvedConflictMode::PreserveMarkers,
                labels: None,
            },
        );
        assert_eq!(output, "THEIRS\n");
    }

    #[test]
    fn parse_generate_parse_round_trip_preserves_structure() {
        let input = "\
before
<<<<<<< ours
left one
=======
right one
>>>>>>> theirs
middle
<<<<<<< ours
left two
||||||| base
base two
=======
right two
>>>>>>> theirs
after
";
        let parsed = parse_conflict_marker_segments(input);
        let output_segments = build_output_segments(&parsed);
        let rendered = generate_resolved_text(&output_segments, preserve_markers_options());
        let reparsed = parse_conflict_marker_segments(&rendered);

        assert_eq!(parsed, reparsed);
    }
}
