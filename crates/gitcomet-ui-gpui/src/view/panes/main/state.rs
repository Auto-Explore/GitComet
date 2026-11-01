use super::*;

/// View-local editing state for one repo's interactive rebase setup.
#[derive(Default)]
pub(in crate::view) struct IRebaseViewState {
    pub(in crate::view) mode: ICommitEditorMode,
    pub(in crate::view) entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) original_entries: Vec<gitcomet_core::services::InteractiveRebaseEntry>,
    pub(in crate::view) source_colors: FxHashMap<String, u8>,
    /// Active auto-squash strategy, or None when auto-squash is off.
    pub(in crate::view) autosquash_mode: Option<AutosquashMode>,
    /// Commits folded away by auto-squash, keyed by the surviving commit id.
    /// Each survivor's `entries` row displays these ids; they are re-expanded
    /// into `fixup` todo entries when the rebase starts.
    pub(in crate::view) folded:
        FxHashMap<String, Vec<gitcomet_core::services::InteractiveRebaseEntry>>,
    pub(in crate::view) drag_state: Option<IRebaseDragState>,
    /// Variable-height virtualized list state, lazily created on first render
    /// (`ListState` has no `Default`). Kept in sync with `entries`/`folded` via
    /// `list_sig` (remeasure on same-count content change, reset on count change).
    pub(in crate::view) scroll: Option<gpui::ListState>,
    /// (content-hash, item-count) the `scroll` ListState was last synced to.
    pub(in crate::view) list_sig: (u64, usize),
    /// (ix_a, ix_b, version) — the two data-indices swapped by ▲/▼; drives fade-in animation.
    pub(in crate::view) reorder_anim: Option<(usize, usize, u32)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) enum ICommitEditorMode {
    #[default]
    Rebase,
    CherryPick,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::view) struct IRebaseDragState {
    pub(in crate::view) from_ix: usize,
    pub(in crate::view) to_ix: usize,
    /// Drop-target position in display order (0..=entry_count).
    pub(in crate::view) display_pos: usize,
}
