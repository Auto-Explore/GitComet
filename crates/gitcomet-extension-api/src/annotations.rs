//! Descriptor annotations are painted inside existing rows, without child views.
use crate::{HostedMenuItem, RepositoryViewContext, SlotSignal, ViewBuilder};
use gitcomet_core::domain::Commit;
use gitcomet_ui_kit::gpui::{AnyView, App, Hsla, SharedString, Window};
use std::ops::Range;
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
    /// What the mark stands for, to its annotator: a History mark's menu is
    /// told the keys of the marks it covers, and a chip shows its menu open
    /// by them while the marks move.
    pub key: Option<SharedString>,
}

impl RowMark {
    pub fn new(color: Hsla) -> Self {
        Self {
            label: None,
            color,
            glyph: None,
            key: None,
        }
    }

    pub fn with_key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
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

/// What an annotator draws on a History row: its opacity, a range bar, and
/// marks at the start and the end of the commit's message. An edge's marks
/// are drawn in order, the leading ones from the message's start and the
/// trailing ones ending at its end. Where they do not all fit, the first
/// stays and those that do not fit give way to a "+2" counting them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistoryRowAnnotation {
    pub opacity: f32,
    pub leading: Vec<RowMark>,
    pub trailing: Vec<RowMark>,
    pub range: Option<HistoryRangeMark>,
}

impl Default for HistoryRowAnnotation {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            leading: Vec::new(),
            trailing: Vec::new(),
            range: None,
        }
    }
}

impl HistoryRowAnnotation {
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Adds a mark after the leading marks so far.
    pub fn with_leading(mut self, mark: RowMark) -> Self {
        self.leading.push(mark);
        self
    }

    /// Adds a mark after the trailing marks so far.
    pub fn with_trailing(mut self, mark: RowMark) -> Self {
        self.trailing.push(mark);
        self
    }

    pub fn with_range(mut self, range: HistoryRangeMark) -> Self {
        self.range = Some(range);
        self
    }
}

pub type AnnotateHistory =
    Rc<dyn Fn(&RepositoryViewContext, &Commit, &App) -> HistoryRowAnnotation>;

/// Which end of a History row's message marks sit at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkEdge {
    Leading,
    Trailing,
}

/// The marks a right-click landed on: one mark, or the marks a "+2" stands
/// for, by their places in the edge's list as drawn and by the keys of
/// those that have one. The keys still name them once the list has moved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MarkTarget {
    pub edge: MarkEdge,
    pub marks: Range<usize>,
    pub keys: Vec<SharedString>,
}

impl MarkTarget {
    pub fn new(edge: MarkEdge, marks: Range<usize>) -> Self {
        Self {
            edge,
            marks,
            keys: Vec::new(),
        }
    }

    pub fn with_keys(mut self, keys: impl IntoIterator<Item = SharedString>) -> Self {
        self.keys = keys.into_iter().collect();
        self
    }
}

/// The menu a right-click on a History row's marks opens; empty opens the
/// commit's own menu instead.
pub type HistoryMarkMenu =
    Rc<dyn Fn(&RepositoryViewContext, &Commit, &MarkTarget, &App) -> Vec<HostedMenuItem>>;

#[derive(Clone)]
#[non_exhaustive]
pub struct HistoryAnnotator {
    pub signal: SlotSignal,
    pub annotate: AnnotateHistory,
    pub scope_header: Option<ViewBuilder<RepositoryViewContext>>,
    pub mark_menu: Option<HistoryMarkMenu>,
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
            mark_menu: None,
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

    /// A menu for a right-click on a mark this annotator drew, or on the
    /// "+2" standing for those that did not fit, asked for when the click
    /// lands, not when the row is drawn. Its marks are drawn as chips, and
    /// the chip whose menu is open shows it. Marks another annotator drew
    /// over this one's open that annotator's menu, if it has one; the rest of
    /// the row, a mark without a menu, or a menu with no action opens the
    /// commit's menu as before.
    pub fn with_mark_menu(
        mut self,
        menu: impl Fn(&RepositoryViewContext, &Commit, &MarkTarget, &App) -> Vec<HostedMenuItem>
        + 'static,
    ) -> Self {
        self.mark_menu = Some(Rc::new(menu));
        self
    }
}
