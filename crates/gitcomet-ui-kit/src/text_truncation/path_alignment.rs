use super::width_cache_key;
use gpui::Pixels;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct PathAlignmentLayoutKey {
    pub width_key: Option<u32>,
    pub style_key: u64,
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
pub struct PathTruncationAlignmentGroup(Rc<RefCell<PathAlignmentState>>);

/// Anchors are tracked per visible-row signature and layout key. One list
/// lays its rows out in more than one pass a frame (gpui's uniform list
/// measures its first item on its own, unconstrained) and at more than one
/// width (rows with and without a stat column). With a single slot each pass
/// reset the others, the anchor never resolved, and every frame reported an
/// ellipsis and notified the owner view again.
#[derive(Debug, Default)]
struct PathAlignmentState {
    visible_signature: Option<u64>,
    render_epoch: u64,
    /// The slot the last layout call used.
    current: Option<usize>,
    slots: Vec<PathAlignmentSlot>,
}

#[derive(Debug)]
struct PathAlignmentSlot {
    visible_signature: u64,
    layout_key: PathAlignmentLayoutKey,
    layout_epoch: u64,
    resolved_anchor: Option<Pixels>,
    pending_anchor: Option<Pixels>,
    notified_for_pending: bool,
}

/// A pane's lists each keep their own group; this bounds the passes and widths
/// one group tracks.
const PATH_ALIGNMENT_SLOTS: usize = 8;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PathAlignmentSnapshot {
    pub visible_signature: Option<u64>,
    pub layout_key: Option<PathAlignmentLayoutKey>,
    pub resolved_anchor: Option<Pixels>,
    pub pending_anchor: Option<Pixels>,
    pub render_epoch: u64,
    pub layout_epoch: u64,
    pub notified_for_pending: bool,
}

impl PathAlignmentState {
    fn prepare_layout(&mut self, layout_key: PathAlignmentLayoutKey) -> &mut PathAlignmentSlot {
        let signature = self.visible_signature.unwrap_or_default();
        let render_epoch = self.render_epoch;
        let ix =
            match self.slots.iter().position(|slot| {
                slot.visible_signature == signature && slot.layout_key == layout_key
            }) {
                Some(ix) => ix,
                None => {
                    if self.slots.len() >= PATH_ALIGNMENT_SLOTS {
                        // Drop the least recently laid out.
                        if let Some(oldest) = self
                            .slots
                            .iter()
                            .enumerate()
                            .min_by_key(|(_, slot)| slot.layout_epoch)
                            .map(|(ix, _)| ix)
                        {
                            self.slots.swap_remove(oldest);
                        }
                    }
                    self.slots.push(PathAlignmentSlot {
                        visible_signature: signature,
                        layout_key,
                        layout_epoch: render_epoch,
                        resolved_anchor: None,
                        pending_anchor: None,
                        notified_for_pending: false,
                    });
                    self.slots.len() - 1
                }
            };
        self.current = Some(ix);
        let slot = &mut self.slots[ix];
        if slot.layout_epoch != render_epoch {
            slot.layout_epoch = render_epoch;
            if let Some(pending_anchor) = slot.pending_anchor {
                slot.resolved_anchor = Some(
                    slot.resolved_anchor
                        .map_or(pending_anchor, |current| current.min(pending_anchor)),
                );
            }
            slot.pending_anchor = None;
            slot.notified_for_pending = false;
        }
        slot
    }
}

impl PathTruncationAlignmentGroup {
    pub fn visible_rows(&self, visible_signature: u64) -> Self {
        self.begin_visible_rows(visible_signature);
        self.clone()
    }

    pub(super) fn begin_visible_rows(&self, visible_signature: u64) {
        let mut state = self.0.borrow_mut();
        state.visible_signature = Some(visible_signature);
        state.render_epoch = state.render_epoch.wrapping_add(1);
        state.current = None;
    }

    pub fn path_anchor_for_layout(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
    ) -> Option<Pixels> {
        let mut state = self.0.borrow_mut();
        state
            .prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key))
            .resolved_anchor
    }

    pub fn report_natural_ellipsis(
        &self,
        max_width: Option<Pixels>,
        style_key: u64,
        ellipsis_x: Pixels,
    ) -> bool {
        let mut state = self.0.borrow_mut();
        let slot = state.prepare_layout(PathAlignmentLayoutKey::new(max_width, style_key));
        let tightened = slot
            .pending_anchor
            .is_none_or(|current| ellipsis_x < current);
        if !tightened {
            return false;
        }

        slot.pending_anchor = Some(ellipsis_x);
        if slot.notified_for_pending {
            return false;
        }

        slot.notified_for_pending = true;
        true
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_for_test(&self) -> PathAlignmentSnapshot {
        let state = self.0.borrow();
        let slot = state.current.and_then(|ix| state.slots.get(ix));
        PathAlignmentSnapshot {
            visible_signature: state.visible_signature,
            layout_key: slot.map(|slot| slot.layout_key),
            resolved_anchor: slot.and_then(|slot| slot.resolved_anchor),
            pending_anchor: slot.and_then(|slot| slot.pending_anchor),
            render_epoch: state.render_epoch,
            layout_epoch: slot.map_or(0, |slot| slot.layout_epoch),
            notified_for_pending: slot.is_some_and(|slot| slot.notified_for_pending),
        }
    }
}
