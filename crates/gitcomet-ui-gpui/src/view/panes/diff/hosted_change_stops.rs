//! What a hosted pane tells its owner about change navigation: how many
//! stops the shown file has and which one the cursor is on.
use super::diff_cache::FileDiffCacheError;
use super::helpers::state::{DiffVisibleLayoutKey, DiffWrapVisibleCacheKey};
use super::input::DiffPaneInput;
use super::{DiffBody, DiffRenderer, DiffViewMode};
use gitcomet_core::domain::DiffTarget;
use gitcomet_extension_api::{DiffChangeStops, DiffPaneEvent};
use gitcomet_state::model::Loadable;

/// Everything [`DiffRenderer::diff_nav_entries`] and the cursor's place in
/// them are read from. The stops are counted again only when it changes.
#[derive(Clone, Debug, PartialEq)]
struct StopsInputs {
    layout: DiffVisibleLayoutKey,
    wrap: Option<DiffWrapVisibleCacheKey>,
    revs: (u64, u64),
    markdown_layout: Option<DiffViewMode>,
    anchor: Option<usize>,
}

/// The inputs the last stops were counted from, and the stops last sent.
#[derive(Default)]
pub(in crate::view) struct ReportedStops {
    inputs: Option<StopsInputs>,
    sent: Option<DiffChangeStops>,
}

impl DiffRenderer {
    /// Sends the owner the shown target's stops when they differ from the
    /// last ones sent. Runs after the body is laid out, so its rows are the
    /// ones the stops are counted over.
    pub(in crate::view) fn report_hosted_change_stops(&mut self, cx: &mut gpui::Context<Self>) {
        if matches!(&self.input, DiffPaneInput::Snapshot { .. }) {
            return;
        }
        let Some(decor) = self.hosted_decor.as_ref() else {
            return;
        };
        let Some(handler) = decor.options.on_event.as_ref() else {
            return;
        };
        let Some(target) = self.rendered_diff_target() else {
            return;
        };
        if !self.change_stops_ready() {
            return;
        }
        let inputs = StopsInputs {
            layout: self.diff_visible_layout_key(),
            wrap: self.diff_wrap_visible_cache_key,
            revs: (
                self.rendered_patch_diff_rev(),
                self.rendered_file_diff_rev(),
            ),
            markdown_layout: self.is_markdown_preview_active().then_some(self.diff_view),
            anchor: self.diff_selection_anchor,
        };
        let previous = decor.stops.sent.as_ref().filter(|sent| {
            &sent.target == target && sent.target.old_file_path() == target.old_file_path()
        });
        if previous.is_some() && decor.stops.inputs.as_ref() == Some(&inputs) {
            return;
        }
        let entries = self.diff_nav_entries();
        let current = inputs
            .anchor
            .and_then(|anchor| entries.iter().position(|&entry| entry == anchor));
        let report = previous
            .is_none_or(|sent| sent.count != entries.len() || sent.current != current)
            .then(|| {
                (
                    handler.clone(),
                    DiffChangeStops::new(target.clone(), entries.len(), current),
                )
            });
        let decor = self.hosted_decor.as_mut().expect("checked above");
        decor.stops.inputs = Some(inputs);
        if let Some((handler, stops)) = report {
            decor.stops.sent = Some(stops.clone());
            cx.defer(move |cx| handler(DiffPaneEvent::ChangeStops(stops), cx));
        }
    }

    /// Whether the body on screen has what its stops are counted from.
    fn change_stops_ready(&self) -> bool {
        if self.review_diff_is_loading() {
            return false;
        }
        if self.is_markdown_preview_active() {
            return self.markdown_preview_row_count().is_some();
        }
        match self.diff_surface().body {
            DiffBody::FilePreview => {
                matches!(self.worktree_preview, Loadable::Ready(_))
                    || (!matches!(self.worktree_preview, Loadable::Error(_))
                        && self
                            .rendered_diff_target()
                            .and_then(DiffTarget::file_path)
                            .is_some_and(|path| self.preview_is_picture(path)))
            }
            DiffBody::FileDiff => match self.rendered_file_diff_loadable() {
                Some(Loadable::Ready(Some(_))) => {
                    self.rendered_file_diff_cache_is_current()
                        && self
                            .file_diff_cache_error
                            .as_ref()
                            .is_none_or(|error| matches!(error, FileDiffCacheError::NotText { .. }))
                }
                Some(Loadable::Ready(None)) => true,
                _ => false,
            },
            DiffBody::Patch => {
                self.rendered_patch_diff_cache_is_current()
                    && matches!(
                        self.rendered_patch_diff_loadable(),
                        Some(Loadable::Ready(_))
                    )
            }
            DiffBody::SubmoduleSummary
            | DiffBody::DirectoryNotice
            | DiffBody::FileEditor
            | DiffBody::Conflict => true,
        }
    }
}
