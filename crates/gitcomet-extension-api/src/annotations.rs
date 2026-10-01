//! Descriptor annotations are painted inside existing rows, without child views.
use crate::{RepositoryViewContext, SlotSignal, ViewBuilder};
use gitcomet_core::domain::Commit;
use gitcomet_ui_kit::gpui::{App, Hsla, SharedString};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct RowMark {
    pub label: SharedString,
    pub color: Hsla,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HistoryRangeMark {
    pub color: Hsla,
    pub starts: bool,
    pub ends: bool,
}

#[derive(Clone, Debug, PartialEq)]
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

pub type AnnotateHistory =
    Rc<dyn Fn(&RepositoryViewContext, &Commit, &App) -> HistoryRowAnnotation>;

#[derive(Clone)]
pub struct HistoryAnnotator {
    pub signal: SlotSignal,
    pub annotate: AnnotateHistory,
    pub scope_header: Option<ViewBuilder<RepositoryViewContext>>,
}
