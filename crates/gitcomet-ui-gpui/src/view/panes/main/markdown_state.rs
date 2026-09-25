use super::*;
use std::cell::RefCell;

/// The rendered preview of a whole file: its parsed document, and what the
/// renderer keeps between frames.
pub(in crate::view) struct WorktreeMarkdownPreview {
    pub(in crate::view) document: LoadableMarkdownDoc,
    /// The file and content revision `document` was parsed from.
    pub(in crate::view) path: Option<std::path::PathBuf>,
    pub(in crate::view) source_rev: u64,
    pub(in crate::view) seq: u64,
    pub(in crate::view) inflight: Option<u64>,
    /// Sizes read from the headers of the pictures the preview draws, so a
    /// picture that has not decoded yet can still hold its box open.
    pub(in crate::view) picture_sizes: rows::MarkdownPreviewPictureSizes,
    /// Where each sideways-scrolling block is scrolled to, so its scrollbar
    /// has something to read.
    pub(in crate::view) block_scrolls: rows::MarkdownDocumentBlockScrolls,
    /// Block grouping of the document last drawn, not re-derived every frame.
    pub(in crate::view) blocks: rows::MarkdownDocumentBlockCache,
    /// How tall each drawn block was, so frames build only what is near the
    /// viewport.
    pub(in crate::view) layout: rows::MarkdownDocumentLayoutCache,
    /// Pictures still decoding that already have someone waiting to repaint
    /// the pane when they finish.
    pub(in crate::view) image_waits: FxHashSet<gpui::Resource>,
}

impl Default for WorktreeMarkdownPreview {
    fn default() -> Self {
        Self {
            document: Loadable::NotLoaded,
            path: None,
            source_rev: 0,
            seq: 0,
            inflight: None,
            picture_sizes: Default::default(),
            block_scrolls: Default::default(),
            blocks: Default::default(),
            layout: Default::default(),
            image_waits: FxHashSet::default(),
        }
    }
}

impl WorktreeMarkdownPreview {
    /// Forget the parsed document: another file, or none, is on screen.
    pub(in crate::view) fn invalidate(&mut self) {
        self.path = None;
        self.source_rev = 0;
        self.document = Loadable::NotLoaded;
        self.inflight = None;
    }
}

/// The rendered preview of a file's diff: the parsed pair, what it was built
/// from, and what the renderer keeps between frames.
pub(in crate::view) struct DiffMarkdownPreview {
    pub(in crate::view) preview: LoadableMarkdownDiff,
    pub(in crate::view) seq: u64,
    pub(in crate::view) inflight: Option<u64>,
    /// What `preview` was built from.
    pub(in crate::view) cache_repo_id: Option<RepoId>,
    pub(in crate::view) cache_rev: u64,
    pub(in crate::view) cache_content_signature: Option<u64>,
    pub(in crate::view) cache_target: Option<DiffTarget>,
    /// As the file preview keeps them, for the old, new, and inline documents,
    /// whose row indices overlap.
    pub(in crate::view) block_scrolls: [rows::MarkdownDocumentBlockScrolls; 3],
    pub(in crate::view) layouts: [rows::MarkdownDocumentLayoutCache; 3],
    /// Where the diff drew its changes last frame, which preview and layout
    /// that frame drew, and the one a follow-up frame was already asked for to
    /// measure.
    pub(in crate::view) change_extents: rows::MarkdownChangeExtents,
    pub(in crate::view) change_extents_key: Option<(u64, DiffViewMode)>,
    pub(in crate::view) change_extents_requested: Option<(u64, DiffViewMode)>,
}

impl Default for DiffMarkdownPreview {
    fn default() -> Self {
        Self {
            preview: Loadable::NotLoaded,
            seq: 0,
            inflight: None,
            cache_repo_id: None,
            cache_rev: 0,
            cache_content_signature: None,
            cache_target: None,
            block_scrolls: Default::default(),
            layouts: Default::default(),
            change_extents: Default::default(),
            change_extents_key: None,
            change_extents_requested: None,
        }
    }
}

impl DiffMarkdownPreview {
    /// Drop the built preview and what it was built from.
    pub(in crate::view) fn clear_cache(&mut self) {
        self.cache_repo_id = None;
        self.cache_target = None;
        self.cache_rev = 0;
        self.cache_content_signature = None;
        self.preview = Loadable::NotLoaded;
        self.inflight = None;
    }
}

/// Search's reveal and the pointer's link, across the rendered previews.
#[derive(Default)]
pub(in crate::view) struct MarkdownPreviewInteraction {
    /// Row the quick-search cursor wants revealed, shared with the renderer
    /// that measures it. See [`rows::MarkdownPreviewRevealRequest`].
    pub(in crate::view) reveal: rows::MarkdownPreviewRevealRequest,
    /// The link under the pointer, underlined while hovered.
    pub(in crate::view) hovered_link: Option<rows::MarkdownPreviewHoveredLink>,
    /// A link under the pointer that a click would not follow, so moving
    /// across it does not check it again.
    pub(in crate::view) plain_link: Option<rows::MarkdownPreviewHoveredLink>,
}

/// Which remote pictures the rendered previews may fetch.
pub(in crate::view) struct RemoteMarkdownImages {
    pub(in crate::view) policy: RemoteMarkdownImagePolicy,
    pub(in crate::view) approved_urls: Arc<FxHashSet<SharedString>>,
    pub(super) approval_revision: u64,
    pub(super) summary_cache: RefCell<RemoteMarkdownImageSummaryCache>,
}

impl RemoteMarkdownImages {
    pub(in crate::view) fn new(policy: RemoteMarkdownImagePolicy) -> Self {
        Self {
            policy,
            approved_urls: Arc::default(),
            approval_revision: 0,
            summary_cache: RefCell::default(),
        }
    }
}
