//! Descriptor annotations are painted inside existing rows, without child views.
use crate::{RepositoryViewContext, SlotSignal, ViewBuilder};
use gitcomet_core::domain::Commit;
use gitcomet_ui_kit::gpui::{AnyView, App, Hsla, SharedString, Window};
use std::rc::Rc;

/// A small mark in a row, in one colour: a glyph, a label, or both. File
/// lists draw the glyph in a column before the file icon and the label at
/// the end; History and sidebar rows draw the glyph before the label.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowMark {
    pub label: Option<SharedString>,
    pub color: Hsla,
    pub glyph: Option<RowGlyph>,
}

impl RowMark {
    pub fn new(color: Hsla) -> Self {
        Self {
            label: None,
            color,
            glyph: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_glyph(mut self, glyph: RowGlyph) -> Self {
        self.glyph = Some(glyph);
        self
    }
}

/// A mark's glyph, drawn in the mark's colour.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowGlyph {
    /// An svg asset path, such as `extensions/<id>/icons/flag.svg`.
    Icon(SharedString),
    /// A character or two, such as a status letter.
    Text(SharedString),
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistoryRangeMark {
    pub color: Hsla,
    pub starts: bool,
    pub ends: bool,
}

impl HistoryRangeMark {
    /// A range bar through the row; `starts`/`ends` cap it at this row.
    pub fn new(color: Hsla, starts: bool, ends: bool) -> Self {
        Self {
            color,
            starts,
            ends,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistoryRowAnnotation {
    pub opacity: f32,
    pub leading: Option<RowMark>,
    pub trailing: Option<RowMark>,
    pub range: Option<HistoryRangeMark>,
}

impl Default for HistoryRowAnnotation {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            leading: None,
            trailing: None,
            range: None,
        }
    }
}

impl HistoryRowAnnotation {
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn with_leading(mut self, mark: RowMark) -> Self {
        self.leading = Some(mark);
        self
    }

    pub fn with_trailing(mut self, mark: RowMark) -> Self {
        self.trailing = Some(mark);
        self
    }

    pub fn with_range(mut self, range: HistoryRangeMark) -> Self {
        self.range = Some(range);
        self
    }
}

pub type AnnotateHistory =
    Rc<dyn Fn(&RepositoryViewContext, &Commit, &App) -> HistoryRowAnnotation>;

#[derive(Clone)]
#[non_exhaustive]
pub struct HistoryAnnotator {
    pub signal: SlotSignal,
    pub annotate: AnnotateHistory,
    pub scope_header: Option<ViewBuilder<RepositoryViewContext>>,
}

impl HistoryAnnotator {
    pub fn new(
        signal: SlotSignal,
        annotate: impl Fn(&RepositoryViewContext, &Commit, &App) -> HistoryRowAnnotation + 'static,
    ) -> Self {
        Self {
            signal,
            annotate: Rc::new(annotate),
            scope_header: None,
        }
    }

    /// A header above History, built once per repository.
    pub fn with_scope_header(
        mut self,
        build: impl Fn(RepositoryViewContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        self.scope_header = Some(Rc::new(build));
        self
    }
}
