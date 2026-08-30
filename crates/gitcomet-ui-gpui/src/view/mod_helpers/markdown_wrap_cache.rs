use super::*;

pub(crate) type LoadableMarkdownDoc =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDocument>>;

pub(crate) type LoadableMarkdownDiff =
    Loadable<Arc<crate::view::markdown_preview::MarkdownPreviewDiff>>;

/// The rendered markdown surface quick search is looking at.
///
/// Each shape has its own row space and its own way of being scrolled, which
/// is why search dispatches on this rather than on the preview kind alone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum MarkdownSearchSurface {
    /// Rendered file preview: one flowing document, no fixed row height.
    Worktree,
    /// Rendered markdown diff, inline: one virtualized list on `diff_scroll`.
    DiffInline,
    /// Rendered markdown diff, split: two lists sharing one visual row space.
    DiffSplit,
    /// Merge tool rendered preview: one unwrapped list per input column.
    Conflict,
}

/// Which markdown preview list a wrap plan belongs to. Split view wraps its
/// two columns to different widths, and the inline and worktree lists have
/// their own row sets, so each keeps its own plan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) enum MarkdownPreviewList {
    Worktree,
    Inline,
    Old,
    New,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct MarkdownPreviewWrapKey {
    pub(in crate::view) width_px: u32,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) theme_is_dark: bool,
    pub(in crate::view) editor_font_family_hash: u64,
    pub(in crate::view) document_rev: u64,
}

/// Cached visual-row mappings for the wrapped markdown preview lists.
///
/// The plans are rebuilt whenever the viewport width, UI scale, font, or the
/// underlying document changes; the key makes that a cheap equality check on
/// every frame instead of a re-wrap.
///
/// A slot holding a key with no plan means "measured at this key, not
/// wrapped" — the document was too large to wrap, and the list renders
/// unwrapped. Keeping the key is what stops that verdict from being
/// recomputed on every single frame.
#[derive(Debug)]
pub(crate) struct MarkdownPreviewWrapSlot {
    key: MarkdownPreviewWrapKey,
    /// `None` once the document proved too large to wrap.
    plan: Option<crate::view::markdown_preview::MarkdownPreviewWrapPlan>,
}

#[derive(Debug, Default)]
pub(in crate::view) struct MarkdownPreviewWrapCache {
    slots: [Option<MarkdownPreviewWrapSlot>; 4],
}

impl MarkdownPreviewWrapCache {
    pub(crate) fn slot(list: MarkdownPreviewList) -> usize {
        match list {
            MarkdownPreviewList::Worktree => 0,
            MarkdownPreviewList::Inline => 1,
            MarkdownPreviewList::Old => 2,
            MarkdownPreviewList::New => 3,
        }
    }

    pub(in crate::view) fn plan(
        &self,
        list: MarkdownPreviewList,
    ) -> Option<&crate::view::markdown_preview::MarkdownPreviewWrapPlan> {
        self.slots[Self::slot(list)].as_ref()?.plan.as_ref()
    }

    /// The plan for `list`, but only while it describes document `document_rev`.
    ///
    /// A plan indexes rows of the document it was built from, so readers that
    /// resolve a list position to a source row must not use one left over from
    /// an earlier document — the row it names may not exist any more.
    pub(in crate::view) fn plan_for_rev(
        &self,
        list: MarkdownPreviewList,
        document_rev: u64,
    ) -> Option<&crate::view::markdown_preview::MarkdownPreviewWrapPlan> {
        let slot = self.slots[Self::slot(list)].as_ref()?;
        if slot.key.document_rev != document_rev {
            return None;
        }
        slot.plan.as_ref()
    }

    pub(in crate::view) fn is_current(
        &self,
        list: MarkdownPreviewList,
        key: MarkdownPreviewWrapKey,
    ) -> bool {
        self.slots[Self::slot(list)]
            .as_ref()
            .is_some_and(|slot| slot.key == key)
    }

    pub(in crate::view) fn store(
        &mut self,
        list: MarkdownPreviewList,
        key: MarkdownPreviewWrapKey,
        plan: Option<crate::view::markdown_preview::MarkdownPreviewWrapPlan>,
    ) {
        self.slots[Self::slot(list)] = Some(MarkdownPreviewWrapSlot { key, plan });
    }

    /// Number of visual rows a list renders, or `None` when it is unwrapped.
    pub(in crate::view) fn plan_len(&self, list: MarkdownPreviewList) -> Option<usize> {
        self.plan(list).map(|plan| plan.len())
    }

    /// True once a list has been measured at some key, whether or not that
    /// produced a plan.
    #[cfg(test)]
    pub(in crate::view) fn has_key(&self, list: MarkdownPreviewList) -> bool {
        self.slots[Self::slot(list)].is_some()
    }

    pub(in crate::view) fn clear_list(&mut self, list: MarkdownPreviewList) {
        self.slots[Self::slot(list)] = None;
    }
}

#[cfg(test)]
mod markdown_preview_wrap_cache_tests {
    use super::*;

    pub(crate) fn key(width_px: u32) -> MarkdownPreviewWrapKey {
        MarkdownPreviewWrapKey {
            width_px,
            ui_scale_percent: 100,
            theme_is_dark: false,
            editor_font_family_hash: 7,
            document_rev: 1,
        }
    }

    #[test]
    pub(crate) fn storing_no_plan_still_records_the_key_so_the_verdict_is_not_recomputed() {
        // An oversized document renders unwrapped. Forgetting the key would
        // make every frame re-attempt the wrap it already knows will fail.
        let mut cache = MarkdownPreviewWrapCache::default();
        cache.store(MarkdownPreviewList::Inline, key(800), None);

        assert!(cache.plan(MarkdownPreviewList::Inline).is_none());
        assert!(cache.is_current(MarkdownPreviewList::Inline, key(800)));
        assert!(cache.has_key(MarkdownPreviewList::Inline));
        assert!(!cache.is_current(MarkdownPreviewList::Inline, key(808)));
    }

    #[test]
    pub(crate) fn clearing_a_list_drops_both_its_key_and_plan() {
        let mut cache = MarkdownPreviewWrapCache::default();
        cache.store(MarkdownPreviewList::Old, key(800), Some(Default::default()));
        cache.clear_list(MarkdownPreviewList::Old);

        assert!(!cache.has_key(MarkdownPreviewList::Old));
        assert!(cache.plan(MarkdownPreviewList::Old).is_none());
    }
}
