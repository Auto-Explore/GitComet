use super::*;

pub(super) fn preferred_scroll_master_index<const N: usize>(max_scrolls: [Pixels; N]) -> usize {
    let mut preferred_ix = 0usize;
    for ix in 1..N {
        if max_scrolls[ix] > max_scrolls[preferred_ix] {
            preferred_ix = ix;
        }
    }
    preferred_ix
}

pub(super) fn clamp_raw_scroll_y(raw_y: Pixels, max_scroll: Pixels) -> Pixels {
    let max_scroll = max_scroll.max(px(0.0));
    raw_y.clamp(-max_scroll, px(0.0))
}

#[cfg(test)]
pub(super) fn compute_synced_scroll_offsets<const N: usize>(
    offsets: [Pixels; N],
    max_scrolls: [Pixels; N],
    last_synced: [Pixels; N],
    preferred_ix: usize,
) -> [Pixels; N] {
    compute_synced_scroll_offsets_with_master(offsets, max_scrolls, last_synced, preferred_ix, None)
}

pub(super) fn compute_synced_scroll_offsets_with_master<const N: usize>(
    offsets: [Pixels; N],
    max_scrolls: [Pixels; N],
    last_synced: [Pixels; N],
    preferred_ix: usize,
    explicit_master_ix: Option<usize>,
) -> [Pixels; N] {
    if N == 0 {
        return offsets;
    }
    if offsets.iter().all(|offset| *offset == offsets[0]) {
        return offsets;
    }

    let preferred_ix = preferred_ix.min(N.saturating_sub(1));
    let mut changed_count = 0usize;
    let mut sole_changed_ix = preferred_ix;
    let mut preferred_changed = false;
    let mut largest_changed_ix = preferred_ix;

    for ix in 0..N {
        // GPUI clamps explicit offsets during paint, after this synchronizer
        // runs. If a pane's maximum changed between frames, treat the painted
        // clamp of our previous target as unchanged rather than fresh user
        // input from that follower.
        let last_at_current_max = clamp_raw_scroll_y(last_synced[ix], max_scrolls[ix]);
        if offsets[ix] == last_at_current_max {
            continue;
        }

        if changed_count == 0 || max_scrolls[ix] > max_scrolls[largest_changed_ix] {
            largest_changed_ix = ix;
        }
        if ix == preferred_ix {
            preferred_changed = true;
        }
        sole_changed_ix = ix;
        changed_count += 1;
    }

    let explicit_master_ix = explicit_master_ix.filter(|&ix| ix < N);
    if changed_count == 0 && explicit_master_ix.is_none() {
        // Nothing moved since the last sync — leave the offsets exactly as they
        // are. Re-driving everyone onto `preferred_ix` here would let a
        // transient flip in which handle is "widest" (the resolved output sizes
        // itself from a monospace width estimate, the columns from measured
        // rows) yank a clamped follower to a different offset with no user
        // input, which reads as a horizontal snap-back. Realignment happens on
        // the next real scroll instead.
        return offsets;
    }

    let master_ix = if let Some(explicit_master_ix) = explicit_master_ix {
        explicit_master_ix
    } else if changed_count == 1 {
        sole_changed_ix
    } else if preferred_changed {
        preferred_ix
    } else {
        largest_changed_ix
    };
    let master_y = offsets[master_ix];

    std::array::from_fn(|ix| clamp_raw_scroll_y(master_y, max_scrolls[ix]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SyncedScrollAxis {
    Horizontal,
    Vertical,
}

impl SyncedScrollAxis {
    const fn includes(self, mode: DiffScrollSync) -> bool {
        match self {
            Self::Horizontal => mode.includes_horizontal(),
            Self::Vertical => mode.includes_vertical(),
        }
    }

    const fn offset_component(self, offset: Point<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => offset.x,
            Self::Vertical => offset.y,
        }
    }

    const fn max_scroll_component(self, max_offset: Size<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => max_offset.width,
            Self::Vertical => max_offset.height,
        }
    }

    fn with_offset_component(self, offset: Point<Pixels>, value: Pixels) -> Point<Pixels> {
        match self {
            Self::Horizontal => point(value, offset.y),
            Self::Vertical => point(offset.x, value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConflictPreviewSyncGroup {
    /// Three-way, unfolded: base/ours/theirs columns and the resolved output
    /// all render full line spaces.
    ColumnsAndOutput,
    /// Three-way with hide-resolved or collapsed context: the columns share a
    /// projected row space the output does not.
    ColumnsOnly,
    /// Two-way: left (base handle) and right (theirs handle) sync as a pair;
    /// the ours handle is unused and the output owns its own scroll space.
    /// Used for block-local giant-file rows, and for the aligned view when the
    /// output-scroll-sync setting is off or the columns are folded.
    TwoWayPair,
    /// Two-way aligned, unfolded, with output scroll sync on: the left/right
    /// columns and the resolved output share the whole-file aligned row space.
    TwoWayPairAndOutput,
}

/// Sync one axis of the conflict-preview handle set for the given group.
///
/// Handles outside the group keep their own offsets; their baseline entries
/// are refreshed each frame so switching groups never sees phantom changes.
pub(super) fn sync_conflict_preview_axis(
    handles: &[ScrollHandle; 4],
    last_synced: &mut [Pixels; 4],
    axis: SyncedScrollAxis,
    mode: DiffScrollSync,
    group: ConflictPreviewSyncGroup,
    explicit_master_ix: Option<usize>,
) {
    // An editable output lays out at content width and therefore has a real
    // horizontal range. Keep it in the raw-pixel sync group so either side can
    // drive the other. A streamed/short output can still have a zero range; in
    // that case exclude it so its clamp at zero cannot pull overflowing source
    // columns back to the start.
    let output_has_horizontal_range = handles[3].max_offset().x > px(0.0);
    let group = match (axis, group, output_has_horizontal_range) {
        (SyncedScrollAxis::Horizontal, ConflictPreviewSyncGroup::ColumnsAndOutput, false) => {
            ConflictPreviewSyncGroup::ColumnsOnly
        }
        (SyncedScrollAxis::Horizontal, ConflictPreviewSyncGroup::TwoWayPairAndOutput, false) => {
            ConflictPreviewSyncGroup::TwoWayPair
        }
        // Vertically the resolved output stands on its own, as KDiff3's merge
        // result window does: it owns a scrollbar the diff windows are not
        // connected to. The columns share one aligned row space, so keeping
        // them together is exact; the output is a different document whose
        // lines correspond to aligned rows only through the merge structure,
        // and on a file that changes every few rows there is no continuous
        // correspondence to follow. Tying them together made the output creep
        // relative to the diffs instead of tracking them. The two are brought
        // together on navigation instead, where the block being visited gives
        // an exact position in both.
        //
        // Horizontally they stay coupled, which KDiff3 also does — one shared
        // horizontal scrollbar drives all three inputs and the merge result.
        (SyncedScrollAxis::Vertical, ConflictPreviewSyncGroup::ColumnsAndOutput, _) => {
            ConflictPreviewSyncGroup::ColumnsOnly
        }
        (SyncedScrollAxis::Vertical, ConflictPreviewSyncGroup::TwoWayPairAndOutput, _) => {
            ConflictPreviewSyncGroup::TwoWayPair
        }
        (_, group, _) => group,
    };
    match group {
        ConflictPreviewSyncGroup::ColumnsAndOutput => {
            maybe_sync_synced_scroll_offsets_with_master(
                handles,
                last_synced,
                axis,
                mode,
                explicit_master_ix,
            );
        }
        ConflictPreviewSyncGroup::ColumnsOnly => {
            let columns = [handles[0].clone(), handles[1].clone(), handles[2].clone()];
            let mut columns_last = [last_synced[0], last_synced[1], last_synced[2]];
            maybe_sync_synced_scroll_offsets_with_master(
                &columns,
                &mut columns_last,
                axis,
                mode,
                explicit_master_ix.filter(|&ix| ix < 3),
            );
            last_synced[..3].copy_from_slice(&columns_last);
            last_synced[3] = axis.offset_component(handles[3].offset());
        }
        ConflictPreviewSyncGroup::TwoWayPair => {
            let pair = [handles[0].clone(), handles[2].clone()];
            let mut pair_last = [last_synced[0], last_synced[2]];
            let pair_master = match explicit_master_ix {
                Some(0) => Some(0),
                Some(2) => Some(1),
                _ => None,
            };
            maybe_sync_synced_scroll_offsets_with_master(
                &pair,
                &mut pair_last,
                axis,
                mode,
                pair_master,
            );
            last_synced[0] = pair_last[0];
            last_synced[2] = pair_last[1];
            last_synced[1] = axis.offset_component(handles[1].offset());
            last_synced[3] = axis.offset_component(handles[3].offset());
        }
        ConflictPreviewSyncGroup::TwoWayPairAndOutput => {
            let group = [handles[0].clone(), handles[2].clone(), handles[3].clone()];
            let mut group_last = [last_synced[0], last_synced[2], last_synced[3]];
            maybe_sync_synced_scroll_offsets_with_master(
                &group,
                &mut group_last,
                axis,
                mode,
                match explicit_master_ix {
                    Some(0) => Some(0),
                    Some(2) => Some(1),
                    Some(3) => Some(2),
                    _ => None,
                },
            );
            last_synced[0] = group_last[0];
            last_synced[2] = group_last[1];
            last_synced[3] = group_last[2];
            last_synced[1] = axis.offset_component(handles[1].offset());
        }
    }
}

pub(super) fn snapshot_synced_scroll_offsets<const N: usize>(
    handles: &[ScrollHandle; N],
    axis: SyncedScrollAxis,
) -> [Pixels; N] {
    std::array::from_fn(|ix| axis.offset_component(handles[ix].offset()))
}

pub(super) fn sync_synced_scroll_offsets_with_master<const N: usize>(
    handles: &[ScrollHandle; N],
    last_synced: &mut [Pixels; N],
    axis: SyncedScrollAxis,
    explicit_master_ix: Option<usize>,
) {
    let offsets: [Point<Pixels>; N] = std::array::from_fn(|ix| handles[ix].offset());
    let max_scrolls: [Pixels; N] = std::array::from_fn(|ix| {
        axis.max_scroll_component(handles[ix].max_offset().into())
            .max(px(0.0))
    });
    let offset_components: [Pixels; N] =
        std::array::from_fn(|ix| axis.offset_component(offsets[ix]));
    let targets = compute_synced_scroll_offsets_with_master(
        offset_components,
        max_scrolls,
        *last_synced,
        preferred_scroll_master_index(max_scrolls),
        explicit_master_ix,
    );

    if axis == SyncedScrollAxis::Horizontal && std::env::var_os("GC_SCROLL_DEBUG").is_some() {
        let f = |arr: &[Pixels; N]| {
            arr.iter()
                .map(|p| format!("{:.0}", f32::from(*p)))
                .collect::<Vec<_>>()
                .join(",")
        };
        eprintln!(
            "[hsync] off=[{}] max=[{}] last=[{}] -> tgt=[{}]",
            f(&offset_components),
            f(&max_scrolls),
            f(last_synced),
            f(&targets),
        );
    }

    for ix in 0..N {
        if axis.offset_component(offsets[ix]) != targets[ix] {
            handles[ix].set_offset(axis.with_offset_component(offsets[ix], targets[ix]));
        }
    }
    *last_synced = targets;
}

pub(super) fn maybe_sync_synced_scroll_offsets<const N: usize>(
    handles: &[ScrollHandle; N],
    last_synced: &mut [Pixels; N],
    axis: SyncedScrollAxis,
    mode: DiffScrollSync,
) {
    maybe_sync_synced_scroll_offsets_with_master(handles, last_synced, axis, mode, None);
}

pub(super) fn maybe_sync_synced_scroll_offsets_with_master<const N: usize>(
    handles: &[ScrollHandle; N],
    last_synced: &mut [Pixels; N],
    axis: SyncedScrollAxis,
    mode: DiffScrollSync,
    explicit_master_ix: Option<usize>,
) {
    if axis.includes(mode) {
        sync_synced_scroll_offsets_with_master(handles, last_synced, axis, explicit_master_ix);
    } else {
        *last_synced = snapshot_synced_scroll_offsets(handles, axis);
    }
}

impl MainPaneView {
    pub(super) fn diff_split_scroll_handles(&self) -> [ScrollHandle; 2] {
        [
            uniform_list_base_handle(&self.diff_scroll),
            uniform_list_base_handle(&self.diff_split_right_scroll),
        ]
    }

    pub(super) fn conflict_preview_scroll_handles(&self) -> [ScrollHandle; 4] {
        [
            uniform_list_base_handle(&self.conflict_resolver_diff_scroll),
            uniform_list_base_handle(&self.conflict_preview_ours_scroll),
            uniform_list_base_handle(&self.conflict_preview_theirs_scroll),
            self.conflict_resolved_output_editor_scroll.clone(),
        ]
    }

    /// Forward a horizontal wheel gesture over the resolved-output pane onto the
    /// diff columns. Native scrolling moves the output's content-width handle;
    /// forwarding the same delta lets the narrower columns respond immediately,
    /// and the normal bidirectional sync reconciles their clamped offsets.
    pub(in crate::view) fn forward_conflict_output_horizontal_wheel(
        &self,
        event: &gpui::ScrollWheelEvent,
        window: &gpui::Window,
    ) -> bool {
        // Only when output/column sync and horizontal diff sync are enabled.
        if !self.mergetool_output_scroll_sync || !self.diff_scroll_sync.includes_horizontal() {
            return false;
        }
        let delta_x = event.delta.pixel_delta(window.line_height()).x;
        if delta_x == px(0.0) {
            return false;
        }
        let handles = self.conflict_preview_scroll_handles();
        // Indices into `handles`: base/ours/theirs are the diff columns. Native
        // overflow handling owns the output handle at index 3.
        let columns: &[usize] = match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => &[0, 1, 2],
            ConflictResolverViewMode::TwoWayDiff => &[0, 2],
        };
        let mut changed = false;
        for &ix in columns {
            let handle = &handles[ix];
            let max_x = handle.max_offset().x.max(px(0.0));
            if max_x <= px(0.0) {
                continue;
            }
            let cur = handle.offset();
            // Mirrors gpui's own overflow-scroll wheel handling: add the raw
            // delta, then clamp into the scrollable range [-max_x, 0].
            let next_x = (cur.x + delta_x).clamp(-max_x, px(0.0));
            if next_x != cur.x {
                handle.set_offset(point(next_x, cur.y));
                changed = true;
            }
        }
        changed
    }

    pub(in crate::view) fn sync_diff_split_scroll(&mut self) {
        let handles = self.diff_split_scroll_handles();
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.diff_split_last_synced_y,
            SyncedScrollAxis::Vertical,
            self.diff_scroll_sync,
        );
        maybe_sync_synced_scroll_offsets(
            &handles,
            &mut self.diff_split_last_synced_x,
            SyncedScrollAxis::Horizontal,
            self.diff_scroll_sync,
        );
    }

    pub(in crate::view) fn record_conflict_vertical_wheel_master(&mut self, master_ix: usize) {
        self.conflict_preview_vertical_wheel_master = Some(master_ix);
        self.conflict_output_gutter_wheel_sync_pending = true;
    }

    pub(in crate::view) fn sync_conflict_preview_scroll(&mut self) {
        let vertical_wheel_master = self.conflict_preview_vertical_wheel_master.take();
        let handles = self.conflict_preview_scroll_handles();
        let group = self.conflict_preview_sync_group();
        for (axis, last_synced) in [
            (
                SyncedScrollAxis::Vertical,
                &mut self.conflict_preview_last_synced_y,
            ),
            (
                SyncedScrollAxis::Horizontal,
                &mut self.conflict_preview_last_synced_x,
            ),
        ] {
            sync_conflict_preview_axis(
                &handles,
                last_synced,
                axis,
                self.diff_scroll_sync,
                group,
                if axis == SyncedScrollAxis::Vertical {
                    vertical_wheel_master
                } else {
                    None
                },
            );
        }
    }

    /// Which conflict-preview lists share a row space and may be raw-offset
    /// synced in the current resolver mode.
    ///
    /// The resolved output renders full merged lines, so it only joins the
    /// group when the columns render an unfolded whole-file row space — the
    /// three-way unfolded columns or the section 30 aligned two-way full mode — and
    /// only when the merge-tool output-scroll-sync setting is on. Folded
    /// column spaces (hide-resolved / collapsed context) and block-local
    /// giant-file two-way rows keep the output independent, because raw
    /// offsets are meaningless across mismatched row spaces.
    pub(super) fn conflict_preview_sync_group(&self) -> ConflictPreviewSyncGroup {
        let folded =
            self.conflict_resolver.hide_resolved || self.conflict_resolver.collapse_context;
        let output_follows = self.mergetool_output_scroll_sync;
        match self.conflict_resolver.view_mode {
            ConflictResolverViewMode::ThreeWay => {
                if folded || !output_follows {
                    ConflictPreviewSyncGroup::ColumnsOnly
                } else {
                    ConflictPreviewSyncGroup::ColumnsAndOutput
                }
            }
            ConflictResolverViewMode::TwoWayDiff => {
                if !self.conflict_resolver.two_way_uses_aligned_rows() || folded || !output_follows
                {
                    ConflictPreviewSyncGroup::TwoWayPair
                } else {
                    ConflictPreviewSyncGroup::TwoWayPairAndOutput
                }
            }
        }
    }

    pub(in crate::view) fn sync_conflict_resolved_output_gutter_scroll(&mut self) {
        let handles = [
            uniform_list_base_handle(&self.conflict_resolved_preview_gutter_scroll),
            self.conflict_resolved_output_editor_scroll.clone(),
        ];
        let explicit_master_ix = self.conflict_output_gutter_wheel_sync_pending.then_some(1);
        self.conflict_output_gutter_wheel_sync_pending = false;
        sync_synced_scroll_offsets_with_master(
            &handles,
            &mut self.conflict_resolved_preview_gutter_last_synced_y,
            SyncedScrollAxis::Vertical,
            explicit_master_ix,
        );
    }

    pub(in crate::view) fn main_pane_content_width(&self, cx: &mut gpui::Context<Self>) -> Pixels {
        let _ = cx;

        super::pane_content_width_for_layout(
            self.last_window_size.width,
            self.layout_sidebar_render_width,
            self.layout_details_render_width,
            self.layout_sidebar_collapsed,
            self.layout_details_collapsed,
        )
    }
}
