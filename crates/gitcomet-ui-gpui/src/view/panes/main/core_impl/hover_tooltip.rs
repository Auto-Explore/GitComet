use super::*;

impl MainPaneView {
    /// Record the hovered blame annotation sub-area and drive the shared tooltip
    /// host. `next` is the (row, area) now hovered, or `None` when leaving; the
    /// blame canvas repaints on `notify` and renders the accent highlight from
    /// this state. Callers gate this so it only runs when the hover changes.
    pub(in crate::view) fn update_blame_annot_hover(
        &mut self,
        next: Option<(usize, rows::AnnotArea)>,
        tooltip: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.blame_annot_hover == next {
            return;
        }
        self.blame_annot_hover = next;
        // Only a pointer on the button itself owns a stage tooltip; merely
        // hovering the row shows the button without one.
        let stage_hover_owns_tooltip = self
            .diff_stage_gutter_hover
            .is_some_and(|hover| hover.on_button);
        self.apply_diff_hover_tooltip(tooltip, stage_hover_owns_tooltip, cx);
        cx.notify();
    }

    /// Drop a stage-gutter hover whose button was not painted in the frame just
    /// gone. Called while `diff_stage_gutter_cells` still holds that frame's
    /// buttons, so an entry missing from it means the row no longer offers one
    /// and can no longer clear the hover itself. Without this the button and its
    /// tooltip stay pinned under a pointer that is over something else.
    pub(in crate::view) fn clear_diff_stage_gutter_hover_if_unpainted(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let unpainted = self.diff_stage_gutter_hover.is_some_and(|hover| {
            !self
                .diff_stage_gutter_cells
                .contains_key(&(hover.visible_ix, hover.slot))
        });
        if unpainted {
            self.update_diff_stage_gutter_hover(None, None, cx);
        }
    }

    /// Record the hovered stage/unstage gutter button and drive the shared
    /// tooltip host, mirroring [`Self::update_blame_annot_hover`]. The row canvas
    /// paints the button from this state (never from the live cursor), so it
    /// stays in step with the value folded into the canvas revision key.
    pub(in crate::view) fn update_diff_stage_gutter_hover(
        &mut self,
        next: Option<rows::DiffStageHover>,
        tooltip: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.diff_stage_gutter_hover == next {
            return;
        }
        self.diff_stage_gutter_hover = next;
        let blame_hover_owns_tooltip = self.blame_annot_hover.is_some();
        self.apply_diff_hover_tooltip(tooltip, blame_hover_owns_tooltip, cx);
        cx.notify();
    }

    /// Shared tooltip plumbing for the two diff-row hover systems (blame column
    /// and stage gutter). Both write to the same host, so a hover that is leaving
    /// must not clear a tooltip the other one just set: `other_hover_active` says
    /// whether the other system currently owns the tooltip.
    pub(super) fn apply_diff_hover_tooltip(
        &mut self,
        tooltip: Option<SharedString>,
        other_hover_active: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(host) = self.tooltip_host.upgrade() else {
            return;
        };
        host.update(cx, |host, cx| match tooltip {
            Some(text) => {
                host.set_tooltip_text_if_changed(Some(text), cx);
            }
            None => {
                if !other_hover_active {
                    host.clear_tooltip(cx);
                }
            }
        });
    }

    /// Drop the cached wrapped-row projection so it is recomputed against the
    /// current text width (which depends on whether the annotation column is
    /// shown).
    pub(in crate::view) fn invalidate_diff_wrap_visible_cache(&mut self) {
        self.diff_wrap_visible_rows.clear();
        self.diff_wrap_visible_cache_key = None;
    }
}
