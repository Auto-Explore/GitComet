use crate::rope::{Point, Rope};
use crate::text_model::TextModelSnapshot;
use std::sync::Arc;

/// Where each row of a text begins, as the sorted array of row starts would
/// answer it.
pub(super) trait LineStartLookup {
    /// Number of rows: `starts.len()`.
    fn line_count(&self) -> usize;

    /// Start of row `line_ix`: `starts.get(line_ix)`.
    fn line_start(&self, line_ix: usize) -> Option<usize>;

    /// Rows starting at or before `offset`:
    /// `starts.partition_point(|&start| start <= offset)`.
    fn lines_starting_through(&self, offset: usize) -> usize;
}

impl LineStartLookup for [usize] {
    fn line_count(&self) -> usize {
        self.len()
    }

    fn line_start(&self, line_ix: usize) -> Option<usize> {
        self.get(line_ix).copied()
    }

    fn lines_starting_through(&self, offset: usize) -> usize {
        self.partition_point(|&start| start <= offset)
    }
}

/// Answered by descent, O(log n) a lookup: no array to rebuild after an edit.
impl LineStartLookup for Rope {
    fn line_count(&self) -> usize {
        Rope::line_count(self) as usize
    }

    fn line_start(&self, line_ix: usize) -> Option<usize> {
        let row = u32::try_from(line_ix).ok()?;
        (row < Rope::line_count(self)).then(|| self.point_to_offset(Point::new(row, 0)))
    }

    fn lines_starting_through(&self, offset: usize) -> usize {
        // A row starts at or before `offset` when its line break is before it.
        self.offset_to_point(offset).row as usize + 1
    }
}

/// The line starts of the text a frame displays, kept with its layout for
/// hit-testing.
#[derive(Clone, Debug)]
pub(super) enum LineStarts {
    /// A placeholder or masked rendering: a synthesized string, indexed whole.
    Whole(Arc<[usize]>),
    /// The document as of the frame. Cloning the rope shares its tree.
    Document(Rope),
}

impl LineStarts {
    /// For `substitute` when the frame displays one, else for the document.
    pub(super) fn for_display(substitute: Option<&str>, content: &TextModelSnapshot) -> Self {
        match substitute {
            Some(text) => Self::Whole(super::shaping::compute_line_starts(text).into()),
            None => Self::Document(content.rope()),
        }
    }

    /// The whole array, for a pass that visits every row anyway. `content` is
    /// the snapshot a `Document` came from, which caches its array.
    pub(super) fn to_array(&self, content: &TextModelSnapshot) -> Arc<[usize]> {
        match self {
            Self::Whole(starts) => Arc::clone(starts),
            Self::Document(_) => content.shared_line_starts(),
        }
    }
}

impl LineStartLookup for LineStarts {
    fn line_count(&self) -> usize {
        match self {
            Self::Whole(starts) => starts.line_count(),
            Self::Document(rope) => LineStartLookup::line_count(rope),
        }
    }

    fn line_start(&self, line_ix: usize) -> Option<usize> {
        match self {
            Self::Whole(starts) => starts.line_start(line_ix),
            Self::Document(rope) => rope.line_start(line_ix),
        }
    }

    fn lines_starting_through(&self, offset: usize) -> usize {
        match self {
            Self::Whole(starts) => starts.lines_starting_through(offset),
            Self::Document(rope) => rope.lines_starting_through(offset),
        }
    }
}
