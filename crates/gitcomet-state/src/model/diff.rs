//! Diff and conflict state for the selected file.

use super::{Loadable, RepoState, Shared};
use crate::msg::RepoPath;
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictSession, ConflictStageParts, canonicalize_stage_parts,
};
use gitcomet_core::domain::*;
use gitcomet_core::services::BlameLine;
use gitcomet_core::text_format::{TextAttributes, TextEncoding, TextOverride};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DiffState {
    pub blame_path: Option<PathBuf>,
    pub blame_source: Option<BlameSource>,
    pub blame: Loadable<Shared<Vec<BlameLine>>>,
    /// Annotations to keep painting while blame reloads for the same target, so
    /// the annotation column does not blank out on every refresh.
    pub retained_blame_while_loading: Option<Shared<Vec<BlameLine>>>,
    pub diff_target: Option<DiffTarget>,
    /// When true, the selected `diff_target` is rendered as a full-content file
    /// preview (the same renderer used for added/removed files — syntax
    /// highlighted, no green/red) rather than a diff. Set by `OpenFileContent`.
    pub content_preview: bool,
    /// When true, the file-content view is the editable buffer rather than the
    /// read-only preview. Only ever set together with `content_preview`, and
    /// only for a `WorkingTree` target — editing is always of the file on disk.
    /// Set by `OpenFileEditor`, cleared by `ExitDiffEditMode`.
    pub edit_mode: bool,
    /// The view that opened the editor. Editing always retargets the working
    /// tree, so both the original target and whether it was a diff or a
    /// full-content preview have to be retained explicitly for Save/Discard to
    /// return to the right place.
    pub edit_return_view: Option<FileEditReturnView>,
    pub diff_target_rev: u64,
    pub diff_state_rev: u64,
    /// A reload of the *same* target is in flight and the content still on
    /// screen is the generation from before it. Set when a reload keeps that
    /// content rather than blanking it, and cleared when the reload lands.
    ///
    /// Anything that builds a patch out of the rendered rows — staging a line or
    /// a hunk out of the diff — has to sit out this window: those rows describe
    /// the index as it was before the last command, so a patch cut from them no
    /// longer applies.
    pub diff_reload_in_flight: bool,
    pub diff_rev: u64,
    pub diff: Loadable<Shared<Diff>>,
    pub diff_file_rev: u64,
    pub diff_file: Loadable<Option<Shared<FileDiffText>>>,
    pub diff_preview_text_file_rev: u64,
    pub diff_preview_text_file: Loadable<Option<Shared<DiffPreviewTextFile>>>,
    pub submodule_summary_rev: u64,
    pub submodule_summary: Loadable<Shared<SubmoduleDiffSummary>>,
    pub inline_submodule_diff_rev: u64,
    pub inline_submodule_diff: Option<InlineSubmoduleDiffState>,
    pub diff_file_image: Loadable<Option<Shared<FileDiffImage>>>,
    pub text_attributes_rev: u64,
    /// `.gitattributes` and config for the selected file.
    pub text_attributes: Loadable<Arc<TextAttributes>>,
    pub text_override_rev: u64,
    /// The user's encoding / line-ending / tab-size choice for the open file.
    /// Dropped when another path is selected.
    pub text_override: Option<OpenFileTextOverride>,
}

/// A [`TextOverride`] and the file it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenFileTextOverride {
    pub path: PathBuf,
    pub value: TextOverride,
}

impl DiffState {
    /// The override for `path` when it is the open file.
    pub fn text_override_for(&self, path: &std::path::Path) -> Option<TextOverride> {
        self.text_override
            .as_ref()
            .filter(|open| open.path == path)
            .map(|open| open.value)
    }

    /// The user's encoding choice for the selected file.
    pub fn selected_encoding_override(&self) -> Option<TextEncoding> {
        let path = self.diff_target.as_ref()?.file_path()?;
        self.text_override_for(path)?.encoding
    }
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            blame_path: None,
            blame_source: None,
            blame: Loadable::NotLoaded,
            retained_blame_while_loading: None,
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            edit_return_view: None,
            diff_target_rev: 0,
            diff_state_rev: 0,
            diff_reload_in_flight: false,
            diff_rev: 0,
            diff: Loadable::NotLoaded,
            diff_file_rev: 0,
            diff_file: Loadable::NotLoaded,
            diff_preview_text_file_rev: 0,
            diff_preview_text_file: Loadable::NotLoaded,
            submodule_summary_rev: 0,
            submodule_summary: Loadable::NotLoaded,
            inline_submodule_diff_rev: 0,
            inline_submodule_diff: None,
            diff_file_image: Loadable::NotLoaded,
            text_attributes_rev: 0,
            text_attributes: Loadable::NotLoaded,
            text_override_rev: 0,
            text_override: None,
        }
    }
}

/// Main-pane destination restored when an editable working-tree buffer closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEditReturnView {
    pub target: DiffTarget,
    pub content_preview: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineSubmoduleDiffSection {
    Range(SubmoduleDiffRangeKind),
    LiveStaged,
    LiveUnstaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineSubmoduleDiffEntry {
    pub path: PathBuf,
    pub kind: FileStatusKind,
    pub target: DiffTarget,
    pub section: InlineSubmoduleDiffSection,
}

/// Which half of a submodule summary a changed file sits in, and therefore
/// which target the inline diff opens it with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleChangeSection {
    /// One of the summary's pointer ranges, by slot.
    Range(usize),
    LiveStaged,
    LiveUnstaged,
}

/// The changed file at `section`/`index` and the target it opens under, or
/// `None` when there is nothing to open.
///
/// The one place a summary's coordinates become a target, so a row and the entry
/// a click resolves cannot describe different files.
fn submodule_change_at(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<(
    &SubmoduleInnerChange,
    DiffTarget,
    InlineSubmoduleDiffSection,
)> {
    match section {
        SubmoduleChangeSection::Range(slot) => {
            let range = summary.ranges.get(slot)?;
            let change = range.changes.get(index)?;
            let (from_commit_id, to_commit_id) = (range.from.as_ref()?, range.to.as_ref()?);
            Some((
                change,
                DiffTarget::commit_range(
                    from_commit_id.clone(),
                    Some(to_commit_id.clone()),
                    Some(change.path.clone()),
                )
                .with_old_path(change.old_path.clone()),
                InlineSubmoduleDiffSection::Range(range.kind),
            ))
        }
        // Only a worktree summary has live halves, and only it gets live rows.
        SubmoduleChangeSection::LiveStaged | SubmoduleChangeSection::LiveUnstaged
            if summary.mode != SubmoduleDiffSummaryMode::Worktree =>
        {
            None
        }
        SubmoduleChangeSection::LiveStaged => {
            let change = summary.live_staged.get(index)?;
            Some((
                change,
                DiffTarget::working_tree(change.path.clone(), DiffArea::Staged),
                InlineSubmoduleDiffSection::LiveStaged,
            ))
        }
        SubmoduleChangeSection::LiveUnstaged => {
            let change = summary.live_unstaged.get(index)?;
            Some((
                change,
                DiffTarget::working_tree(change.path.clone(), DiffArea::Unstaged),
                InlineSubmoduleDiffSection::LiveUnstaged,
            ))
        }
    }
}

/// The target alone, without the entry around it: what a row needs per frame.
pub fn submodule_inline_diff_target(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<DiffTarget> {
    submodule_change_at(summary, section, index).map(|(_, target, _)| target)
}

/// The inline-diff entry for one changed file; `submodule_inline_diff_entries`
/// is this in a loop.
pub fn submodule_inline_diff_entry(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<InlineSubmoduleDiffEntry> {
    submodule_change_at(summary, section, index).map(|(change, target, section)| {
        InlineSubmoduleDiffEntry {
            path: change.path.clone(),
            kind: change.kind,
            target,
            section,
        }
    })
}

pub fn submodule_inline_diff_entries(
    summary: &SubmoduleDiffSummary,
) -> Vec<InlineSubmoduleDiffEntry> {
    let capacity = summary
        .ranges
        .iter()
        .map(|range| range.changes.len())
        .sum::<usize>()
        + summary.live_staged.len()
        + summary.live_unstaged.len();
    let mut entries = Vec::with_capacity(capacity);
    let sections = (0..summary.ranges.len())
        .map(SubmoduleChangeSection::Range)
        .chain([
            SubmoduleChangeSection::LiveStaged,
            SubmoduleChangeSection::LiveUnstaged,
        ]);
    for section in sections {
        let count = match section {
            SubmoduleChangeSection::Range(slot) => summary.ranges[slot].changes.len(),
            SubmoduleChangeSection::LiveStaged => summary.live_staged.len(),
            SubmoduleChangeSection::LiveUnstaged => summary.live_unstaged.len(),
        };
        entries.extend(
            (0..count).filter_map(|index| submodule_inline_diff_entry(summary, section, index)),
        );
    }
    entries
}

/// The inline-diff entries for a linked worktree's changed files, in the order
/// the rows are rendered: staged first, then unstaged, the same order the
/// working-tree pane uses.
///
/// One builder rather than two, because the indices have to agree. The rows are
/// rebuilt from every scan while the open diff carries the list it was opened
/// with, so the reducer re-resolves that list against each new scan
/// (`refresh_worktree_inline_diff_entries`) -- and a second, separately written
/// ordering in the view would silently desynchronize the two.
pub fn worktree_inline_diff_entries(
    summary: &WorktreeDirtySummary,
) -> Vec<InlineSubmoduleDiffEntry> {
    let staged = summary.staged.iter().map(|f| (f, DiffArea::Staged));
    let unstaged = summary.unstaged.iter().map(|f| (f, DiffArea::Unstaged));
    staged
        .chain(unstaged)
        .map(|(file, area)| InlineSubmoduleDiffEntry {
            path: file.path.clone(),
            kind: file.kind,
            target: DiffTarget::working_tree(file.path.clone(), area),
            section: match area {
                DiffArea::Staged => InlineSubmoduleDiffSection::LiveStaged,
                _ => InlineSubmoduleDiffSection::LiveUnstaged,
            },
        })
        .collect()
}

/// Which foreign repository the inline diff is showing, and therefore how the
/// UI labels it. The machinery is the same either way: a throwaway handle opened
/// at `submodule_repo_path`, with its files and diffs parked on the active repo.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForeignDiffOrigin {
    Submodule,
    Worktree {
        branch: Option<String>,
        detached: bool,
    },
}

#[derive(Clone, Debug)]
pub struct InlineSubmoduleDiffState {
    pub origin: ForeignDiffOrigin,
    pub submodule_repo_path: PathBuf,
    pub parent_submodule_path: PathBuf,
    pub entries: Arc<[InlineSubmoduleDiffEntry]>,
    pub selected_ix: usize,
    pub target: DiffTarget,
    pub rev: u64,
    pub diff_rev: u64,
    pub diff: Loadable<Shared<Diff>>,
    pub diff_file_rev: u64,
    pub diff_file: Loadable<Option<Shared<FileDiffText>>>,
    pub diff_file_image: Loadable<Option<Shared<FileDiffImage>>>,
}

#[derive(Clone, Debug)]
pub struct ConflictState {
    pub conflict_file_path: Option<PathBuf>,
    pub conflict_file_load_mode: ConflictFileLoadMode,
    pub conflict_file: Loadable<Option<ConflictFile>>,
    pub conflict_session: Option<ConflictSession>,
    /// Session stashed across a same-path conflict reload so
    /// `conflict_file_loaded` can restore resolutions (and skip the on-open
    /// autosolve). Cleared on path switch and consumed on load completion.
    pub session_pending_restore: Option<ConflictSession>,
    pub conflict_hide_resolved: bool,
    pub conflict_rev: u64,
}

impl Default for ConflictState {
    fn default() -> Self {
        Self {
            conflict_file_path: None,
            conflict_file_load_mode: ConflictFileLoadMode::CurrentOnly,
            conflict_file: Loadable::NotLoaded,
            conflict_session: None,
            session_pending_restore: None,
            conflict_hide_resolved: false,
            conflict_rev: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictFile {
    pub path: RepoPath,
    pub base_bytes: Option<Arc<[u8]>>,
    pub ours_bytes: Option<Arc<[u8]>>,
    pub theirs_bytes: Option<Arc<[u8]>>,
    pub current_bytes: Option<Arc<[u8]>>,
    pub base: Option<Arc<str>>,
    pub ours: Option<Arc<str>>,
    pub theirs: Option<Arc<str>>,
    pub current: Option<Arc<str>>,
}

impl ConflictFile {
    /// Build a conflict file from stage/current parts, canonicalizing UTF-8
    /// payloads down to text-only storage.
    pub fn from_loaded_stage_parts(
        path: impl Into<RepoPath>,
        base: ConflictStageParts,
        ours: ConflictStageParts,
        theirs: ConflictStageParts,
        current: ConflictStageParts,
    ) -> Self {
        let (base_bytes, base) = canonicalize_stage_parts(base.0, base.1);
        let (ours_bytes, ours) = canonicalize_stage_parts(ours.0, ours.1);
        let (theirs_bytes, theirs) = canonicalize_stage_parts(theirs.0, theirs.1);
        let (current_bytes, current) = canonicalize_stage_parts(current.0, current.1);

        Self {
            path: path.into(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes,
            base,
            ours,
            theirs,
            current,
        }
    }

    /// Build a conflict file directly from an existing session without
    /// round-tripping through staged parts first.
    pub fn from_shared_conflict_session(
        path: impl Into<RepoPath>,
        session: &ConflictSession,
    ) -> Self {
        let (base_bytes, base) = conflict_file_side_from_payload(&session.base);
        let (ours_bytes, ours) = conflict_file_side_from_payload(&session.ours);
        let (theirs_bytes, theirs) = conflict_file_side_from_payload(&session.theirs);
        let (current_bytes, current) = session
            .current
            .as_ref()
            .map(conflict_file_side_from_payload)
            .unwrap_or((None, None));

        Self {
            path: path.into(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes,
            base,
            ours,
            theirs,
            current,
        }
    }
}

fn conflict_file_side_from_payload(
    payload: &ConflictPayload,
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    payload.clone().into_stage_parts()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConflictFileLoadMode {
    #[default]
    CurrentOnly,
    Full,
}

impl RepoState {
    pub(crate) fn set_conflict_file_path(&mut self, v: Option<PathBuf>) {
        self.conflict_state.conflict_file_path = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_file_load_mode(&mut self, v: ConflictFileLoadMode) {
        if self.conflict_state.conflict_file_load_mode == v {
            return;
        }
        self.conflict_state.conflict_file_load_mode = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_file(&mut self, v: Loadable<Option<ConflictFile>>) {
        self.conflict_state.conflict_file = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_session(&mut self, v: Option<ConflictSession>) {
        self.conflict_state.conflict_session = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_hide_resolved(&mut self, v: bool) {
        if self.conflict_state.conflict_hide_resolved == v {
            return;
        }
        self.conflict_state.conflict_hide_resolved = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn bump_conflict_rev(&mut self) {
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_diff_target(&mut self, target: Option<DiffTarget>) {
        if self.diff_state.diff_target != target {
            self.diff_state.diff_target_rev = self.diff_state.diff_target_rev.wrapping_add(1);
        }
        // The override lasts while its file stays open, across views of it.
        let keeps_override = self.diff_state.text_override.as_ref().is_none_or(|open| {
            target.as_ref().and_then(DiffTarget::file_path) == Some(open.path.as_path())
        });
        if !keeps_override {
            self.diff_state.text_override = None;
            self.bump_text_override_rev();
        }
        // Attributes belong to one path; another file must not be read under
        // them while its own are loading.
        let old_path = self
            .diff_state
            .diff_target
            .as_ref()
            .and_then(DiffTarget::file_path);
        if old_path != target.as_ref().and_then(DiffTarget::file_path)
            && !matches!(self.diff_state.text_attributes, Loadable::NotLoaded)
        {
            self.diff_state.text_attributes = Loadable::NotLoaded;
            self.diff_state.text_attributes_rev =
                self.diff_state.text_attributes_rev.wrapping_add(1);
        }
        self.diff_state.diff_target = target;
    }

    pub(crate) fn bump_text_override_rev(&mut self) {
        self.diff_state.text_override_rev = self.diff_state.text_override_rev.wrapping_add(1);
    }

    pub(crate) fn bump_diff_state_rev(&mut self) {
        self.diff_state.diff_state_rev = self.diff_state.diff_state_rev.wrapping_add(1);
    }
}
