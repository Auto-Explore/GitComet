use super::width_cache_key;
use gpui::Pixels;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct PathAlignmentLayoutKey {
    pub(crate) width_key: Option<u32>,
    pub(crate) style_key: u64,
}

impl PathAlignmentLayoutKey {
    fn new(max_width: Option<Pixels>, style_key: u64) -> Self {
        Self {
            width_key: max_width.map(width_cache_key),
            style_key,
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct PathTruncationAlignmentGroup(Rc<RefCell<PathAlignmentState>>);

#[derive(Debug, Default)]
struct PathAlignmentState {
    visible_signature: Option<u64>,
    render_epoch: u64,
    layout_key: Option<PathAlignmentLayoutKey>,
    layout_epoch: u64,
    resolved_anchor: Option<Pixels>,
    pending_anchor: Option<Pixels>,
    notified_for_pending: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PathAlignmentSnapshot {
    pub(crate) visible_signature: Option<u64>,
    pub(crate) layout_key: Option<PathAlignmentLayoutKey>,
    pub(crate) resolved_anchor: Option<Pixels>,
    pub(crate) pending_anchor: Option<Pixels>,
    pub(crate) render_epoch: u64,
    pub(crate) layout_epoch: u64,
    pub(crate) notified_for_pending: bool,
}

impl PathAlignmentState {
    fn reset_layout_state(&mut self) {
        self.layout_key = None;
        self.layout_epoch = 0;
        self.resolved_anchor = None;
        self.pending_anchor = None;
        self.notified_for_pending = false;
    }

    fn prepare_layout(&mut self, layout_key: PathAlignmentLayoutKey) {
        if self.layout_key != Some(layout_key) {
            self.layout_key = Some(layout_key);
            self.layout_epoch = self.render_epoch;
            self.resolved_anchor = None;
            self.pending_anchor = None;
            self.notified_for_pending = false;
            return;
        }

        if self.layout_epoch != self.render_epoch {
            self.layout_epoch = self.render_epoch;
            if let Some(pending_anchor) = self.pending_anchor {
                self.resolved_anchor = Some(
                    self.resolved_anchor
                        .map_or(pending_anchor, |current| current.min(pending_anchor)),
                );
            }
            self.pending_anchor = None;
            self.notified_for_pending = false;
        }
    }
}

impl PathTruncationAlignmentGroup {
    pub(crate) fn visible_rows(&self, visible_signature: u64) -> Self {
        self.begin_visible_rows(visible_signature);
        self.clone()
    }

    pub(super) fn begin_visible_rows(&self, visible_signature: u64) {
        let mut state = self.0.borrow_mut();
        if state.visible_signature != Some(visible_signature) {
            state.visible_signature = Some(visible_signature);
            state.render_epoch = state.render_epoch.wrapping_add(1);
            state.reset_layout_state();
            return;
        }

        state.render_epoch = state.render_epoch.wrapping_add(1);
    }

    pub(crate) fn path_anchor_for_layout(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
    ) -> Option<Pixels> {
        let mut state = self.0.borrow_mut();
        state.prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key));
        state.resolved_anchor
    }

    pub(crate) fn report_natural_ellipsis(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
        ellipsis_x: Pixels,
    ) -> bool {
        let mut state = self.0.borrow_mut();
        state.prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key));
        let tightened = state
            .pending_anchor
            .is_none_or(|current| ellipsis_x < current);
        if !tightened {
            return false;
        }

        state.pending_anchor = Some(ellipsis_x);
        if state.notified_for_pending {
            return false;
        }

        state.notified_for_pending = true;
        true
    }

    #[cfg(test)]
    pub(crate) fn snapshot_for_test(&self) -> PathAlignmentSnapshot {
        let state = self.0.borrow();
        PathAlignmentSnapshot {
            visible_signature: state.visible_signature,
            layout_key: state.layout_key,
            resolved_anchor: state.resolved_anchor,
            pending_anchor: state.pending_anchor,
            render_epoch: state.render_epoch,
            layout_epoch: state.layout_epoch,
            notified_for_pending: state.notified_for_pending,
        }
    }
}
