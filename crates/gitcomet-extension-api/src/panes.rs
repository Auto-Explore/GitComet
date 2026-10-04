//! Hosted panes: diff panes and file lists an extension creates and mounts in
//! its own views. Each is independently owned: changing, cancelling, or
//! dropping one leaves every other pane and History as they were.
//!
//! A pane lives while its handle or its mounted view does; dropping both
//! closes its session and cancels its work.
//!
//! File-line features (selection, reveal, annotations, insets, blame, and
//! gutter actions) require a single-file target or a snapshot. Whole-commit
//! and whole-comparison patches display and search all files; their repeated
//! line numbers cannot be addressed by the side/line API.

use gitcomet_core::domain::{CommitFileChange, DiffTarget};
use gitcomet_core::text_format::TextEncoding;
use gitcomet_state::diff_session::ChangeSource;
use gitcomet_ui_kit::gpui::{AnyView, App, Hsla, SharedString};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

/// Which side of a file a line belongs to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum DiffLineSide {
    Old,
    New,
}

/// Lines `start..=end` (1-based) on one side of the file. Selections and
/// reveal anchors are file lines, never display rows, so they survive layout
/// and wrapping changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct DiffLineRange {
    pub side: DiffLineSide,
    pub start: u32,
    pub end: u32,
}

impl DiffLineRange {
    pub fn contains(&self, side: DiffLineSide, line: u32) -> bool {
        self.side == side && (self.start..=self.end).contains(&line)
    }
}

/// What a pane lets the user do. The pane checks it in its handlers, not
/// only when drawing controls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiffPanePolicy {
    pub close_button: bool,
    pub escape_clears_target: bool,
    pub allow_edit: bool,
    pub allow_stage: bool,
    pub allow_annotate: bool,
    pub file_navigation: bool,
    /// Clicking a line's gutter or annotation runs the pane's action.
    pub line_action: bool,
    pub select_lines: bool,
    pub search: bool,
    pub line_numbers: bool,
    /// Loads blame for the target's newer side and shows it in the gutter.
    pub blame: bool,
}

impl Default for DiffPanePolicy {
    fn default() -> Self {
        Self {
            close_button: true,
            escape_clears_target: true,
            allow_edit: true,
            allow_stage: true,
            allow_annotate: true,
            file_navigation: true,
            line_action: true,
            select_lines: true,
            search: true,
            line_numbers: true,
            blame: false,
        }
    }
}

impl DiffPanePolicy {
    /// A pane that only shows the diff.
    pub fn read_only() -> Self {
        Self {
            close_button: false,
            escape_clears_target: false,
            allow_edit: false,
            allow_stage: false,
            allow_annotate: false,
            file_navigation: false,
            line_action: false,
            select_lines: false,
            search: false,
            ..Self::default()
        }
    }

    pub fn with_blame(mut self, blame: bool) -> Self {
        self.blame = blame;
        self
    }
}

/// Row backgrounds; `None` keeps the theme's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DiffRowStyle {
    pub added_background: Option<Hsla>,
    pub removed_background: Option<Hsla>,
    pub context_background: Option<Hsla>,
}

impl DiffRowStyle {
    pub fn with_added_background(mut self, color: Hsla) -> Self {
        self.added_background = Some(color);
        self
    }

    pub fn with_removed_background(mut self, color: Hsla) -> Self {
        self.removed_background = Some(color);
        self
    }
}

/// A decoration on one line: a gutter mark and/or a tint over the row.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct DiffRowDecor {
    pub gutter: Option<SharedString>,
    pub tint: Option<Hsla>,
}

impl DiffRowDecor {
    pub fn gutter(mark: impl Into<SharedString>) -> Self {
        Self {
            gutter: Some(mark.into()),
            tint: None,
        }
    }

    pub fn with_tint(mut self, tint: Hsla) -> Self {
        self.tint = Some(tint);
        self
    }
}

/// Decorations by file line. Asked only for the rows being drawn.
pub type DiffRowDecorProvider = Rc<dyn Fn(DiffLineSide, u32) -> Option<DiffRowDecor>>;

/// A mark on one file line, drawn beside the row and on the scrollbar.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DiffAnnotation {
    pub color: Hsla,
    pub label: Option<SharedString>,
}

impl DiffAnnotation {
    pub fn new(color: Hsla) -> Self {
        Self { color, label: None }
    }

    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
}

/// Annotations indexed by file line and replaced whole. A pane looks up
/// only the rows it draws and places scrollbar markers once per change,
/// never per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiffAnnotations {
    lines: BTreeMap<(DiffLineSide, u32), DiffAnnotation>,
}

impl DiffAnnotations {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, side: DiffLineSide, line: u32, annotation: DiffAnnotation) {
        self.lines.insert((side, line), annotation);
    }

    pub fn with(mut self, side: DiffLineSide, line: u32, annotation: DiffAnnotation) -> Self {
        self.insert(side, line, annotation);
        self
    }

    pub fn get(&self, side: DiffLineSide, line: u32) -> Option<&DiffAnnotation> {
        self.lines.get(&(side, line))
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (DiffLineSide, u32, &DiffAnnotation)> {
        self.lines
            .iter()
            .map(|((side, line), annotation)| (*side, *line, annotation))
    }
}

/// One entry of a pane's legend: what a colour means.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffLegendItem {
    pub label: SharedString,
    pub color: Hsla,
}

impl DiffLegendItem {
    pub fn new(label: impl Into<SharedString>, color: Hsla) -> Self {
        Self {
            label: label.into(),
            color,
        }
    }
}

/// Runs when the user clicks a line's gutter.
pub type DiffGutterAction = Rc<dyn Fn(DiffLineSide, u32, &mut App)>;

/// Runs a selection action on the selected lines.
pub type DiffSelectionRun = Rc<dyn Fn(DiffLineRange, &mut App)>;

/// A button shown while lines are selected; `run` gets the selection.
#[derive(Clone)]
pub struct DiffSelectionAction {
    pub label: SharedString,
    pub run: DiffSelectionRun,
}

impl DiffSelectionAction {
    pub fn new(
        label: impl Into<SharedString>,
        run: impl Fn(DiffLineRange, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            run: Rc::new(run),
        }
    }
}

/// Rows shown after a file line that are not part of the file, such as a
/// review comment. Search, selection, copy, and reveal skip them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DiffInset {
    pub side: DiffLineSide,
    pub line: u32,
    pub lines: Vec<SharedString>,
    /// The text colour; the theme's secondary text by default.
    pub color: Option<Hsla>,
    /// Runs when the user clicks one of the inset's rows.
    pub on_click: Option<crate::HostedAction>,
}

impl DiffInset {
    pub fn new(
        side: DiffLineSide,
        line: u32,
        lines: impl IntoIterator<Item = SharedString>,
    ) -> Self {
        Self {
            side,
            line,
            lines: lines.into_iter().collect(),
            color: None,
            on_click: None,
        }
    }

    pub fn with_color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn with_action(mut self, action: crate::HostedAction) -> Self {
        self.on_click = Some(action);
        self
    }
}

/// The presentation of the two sides; changing it preserves file-line anchors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffLayout {
    #[default]
    Inline,
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffScrollAnchor {
    pub side: DiffLineSide,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiffPaneEvent {
    SelectionChanged(Option<DiffLineRange>),
    TargetChanged(Option<DiffTarget>),
    Loaded,
    Error(SharedString),
}

pub type DiffPaneEventHandler = Rc<dyn Fn(DiffPaneEvent, &mut App)>;

/// Build with `..DiffPaneOptions::default()`; fields are added over time.
#[derive(Clone, Default)]
pub struct DiffPaneOptions {
    pub file_navigation: DiffFileNavigation,
    pub layout: DiffLayout,
    /// Optional explicit width in logical pixels; otherwise follows its bounds.
    pub content_width: Option<f32>,
    pub on_event: Option<DiffPaneEventHandler>,
    pub policy: DiffPanePolicy,
    pub style: DiffRowStyle,
    pub decor: Option<DiffRowDecorProvider>,
    pub on_gutter_click: Option<DiffGutterAction>,
    /// Runs instead of `on_gutter_click` when the user clicks a line's
    /// annotation. Both need [`DiffPanePolicy::line_action`].
    pub on_annotation_click: Option<DiffGutterAction>,
    pub selection_actions: Vec<DiffSelectionAction>,
}

/// File navigation belongs to the pane's owner and its ordered file list.
#[derive(Clone, Default)]
pub struct DiffFileNavigation {
    pub previous: Option<crate::HostedAction>,
    pub next: Option<crate::HostedAction>,
}

/// Two texts compared without a repository, such as the files a difftool
/// was handed. `path` only picks the syntax.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct DiffSnapshot {
    pub path: PathBuf,
    pub old: Arc<str>,
    pub new: Arc<str>,
    pub patch: Option<Arc<gitcomet_core::domain::Diff>>,
}

impl DiffSnapshot {
    pub fn from_patch(path: impl Into<PathBuf>, patch: gitcomet_core::domain::Diff) -> Self {
        Self {
            path: path.into(),
            old: Arc::from(""),
            new: Arc::from(""),
            patch: Some(Arc::new(patch)),
        }
    }

    pub fn new(
        path: impl Into<PathBuf>,
        old: impl Into<Arc<str>>,
        new: impl Into<Arc<str>>,
    ) -> Self {
        Self {
            path: path.into(),
            old: old.into(),
            new: new.into(),
            patch: None,
        }
    }
}

/// What the host implements behind a [`DiffPane`].
///
/// A pane keeps the kind of source it was created with: a repository pane
/// ignores snapshots and a snapshot pane ignores targets and encodings.
#[doc(hidden)]
pub trait DiffPaneImpl {
    fn view(&self) -> AnyView;
    /// The repository target shown; `None` for a snapshot pane.
    fn target(&self, cx: &App) -> Option<DiffTarget>;
    fn set_target(&self, target: DiffTarget, cx: &mut App);
    fn set_snapshot(&self, snapshot: DiffSnapshot, cx: &mut App);
    fn set_encoding(&self, encoding: Option<TextEncoding>, cx: &mut App);
    /// Whether the current target is still loading.
    fn is_loading(&self, cx: &App) -> bool;
    fn selection(&self, cx: &App) -> Option<DiffLineRange>;
    /// Scrolls `line` of `side` into view.
    fn reveal(&self, side: DiffLineSide, line: u32, cx: &mut App);
    /// Highlights rows containing `query`; empty clears it. Ignored when the
    /// policy disallows search.
    fn set_search(&self, query: SharedString, cx: &mut App);
    /// The number of rows matching the search.
    fn search_matches(&self, cx: &App) -> usize;
    fn set_annotations(&self, annotations: Arc<DiffAnnotations>, cx: &mut App);
    fn set_legend(&self, legend: Vec<DiffLegendItem>, cx: &mut App);
    /// Replaces the pane's insets; each shows after its line on its side.
    fn set_insets(&self, insets: Vec<DiffInset>, cx: &mut App);
    /// The selected file lines' text, one line each; insets never included.
    fn selected_text(&self, cx: &App) -> Option<String>;
    /// Original file bytes for the selected whole lines, including line endings.
    fn selected_bytes(&self, cx: &App) -> Option<Arc<[u8]>>;
    fn set_policy(&self, policy: DiffPanePolicy, cx: &mut App);
    fn set_layout(&self, layout: DiffLayout, cx: &mut App);
    fn set_content_width(&self, width: Option<f32>, cx: &mut App);
    fn scroll_anchor(&self, cx: &App) -> Option<DiffScrollAnchor>;
    fn restore_scroll_anchor(&self, anchor: DiffScrollAnchor, cx: &mut App);
    fn set_file_navigation(&self, navigation: DiffFileNavigation, cx: &mut App);
}

/// An owning handle to a hosted diff pane. Mount [`DiffPane::view`] in a view
/// of your own.
#[derive(Clone)]
pub struct DiffPane(Rc<dyn DiffPaneImpl>);

impl DiffPane {
    #[doc(hidden)]
    pub fn new(pane: Rc<dyn DiffPaneImpl>) -> Self {
        Self(pane)
    }

    pub fn view(&self) -> AnyView {
        self.0.view()
    }

    pub fn target(&self, cx: &App) -> Option<DiffTarget> {
        self.0.target(cx)
    }

    pub fn set_target(&self, target: DiffTarget, cx: &mut App) {
        self.0.set_target(target, cx)
    }

    pub fn set_snapshot(&self, snapshot: DiffSnapshot, cx: &mut App) {
        self.0.set_snapshot(snapshot, cx)
    }

    pub fn set_encoding(&self, encoding: Option<TextEncoding>, cx: &mut App) {
        self.0.set_encoding(encoding, cx)
    }

    pub fn is_loading(&self, cx: &App) -> bool {
        self.0.is_loading(cx)
    }

    pub fn selection(&self, cx: &App) -> Option<DiffLineRange> {
        self.0.selection(cx)
    }

    pub fn reveal(&self, side: DiffLineSide, line: u32, cx: &mut App) {
        self.0.reveal(side, line, cx)
    }

    pub fn set_search(&self, query: impl Into<SharedString>, cx: &mut App) {
        self.0.set_search(query.into(), cx)
    }

    pub fn search_matches(&self, cx: &App) -> usize {
        self.0.search_matches(cx)
    }

    pub fn set_annotations(&self, annotations: impl Into<Arc<DiffAnnotations>>, cx: &mut App) {
        self.0.set_annotations(annotations.into(), cx)
    }

    pub fn set_legend(&self, legend: Vec<DiffLegendItem>, cx: &mut App) {
        self.0.set_legend(legend, cx)
    }

    pub fn set_insets(&self, insets: Vec<DiffInset>, cx: &mut App) {
        self.0.set_insets(insets, cx)
    }

    pub fn selected_text(&self, cx: &App) -> Option<String> {
        self.0.selected_text(cx)
    }
    pub fn selected_bytes(&self, cx: &App) -> Option<Arc<[u8]>> {
        self.0.selected_bytes(cx)
    }
    pub fn set_policy(&self, policy: DiffPanePolicy, cx: &mut App) {
        self.0.set_policy(policy, cx);
    }
    pub fn set_layout(&self, layout: DiffLayout, cx: &mut App) {
        self.0.set_layout(layout, cx);
    }
    pub fn set_content_width(&self, width: Option<f32>, cx: &mut App) {
        self.0.set_content_width(width, cx);
    }
    pub fn scroll_anchor(&self, cx: &App) -> Option<DiffScrollAnchor> {
        self.0.scroll_anchor(cx)
    }
    pub fn restore_scroll_anchor(&self, anchor: DiffScrollAnchor, cx: &mut App) {
        self.0.restore_scroll_anchor(anchor, cx);
    }
    pub fn set_file_navigation(&self, navigation: DiffFileNavigation, cx: &mut App) {
        self.0.set_file_navigation(navigation, cx);
    }
}

/// How a file list arranges its files. A new list starts in the layout the
/// user set as the default for changed-file lists, not in this type's
/// `Default`; [`FileList::set_mode`] overrides it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum FileListMode {
    /// Under their directories.
    #[default]
    Tree,
    /// One row per path.
    Flat,
    /// By change kind, or by the groups given to [`FileList::set_groups`],
    /// each group under a header that stays in view while its files scroll.
    Grouped,
}

/// Called when the user picks a file, with its diff target.
pub type FileSelected = Rc<dyn Fn(&CommitFileChange, DiffTarget, &mut App)>;

/// What the host implements behind a [`FileList`].
#[doc(hidden)]
pub trait FileListImpl {
    fn view(&self) -> AnyView;
    fn set_source(&self, source: ChangeSource, cx: &mut App);
    fn set_mode(&self, mode: FileListMode, cx: &mut App);
    fn set_sort(&self, sort: crate::FileListSort, cx: &mut App);
    fn set_kind_filter(&self, filter: crate::FileListFilter, cx: &mut App);
    fn set_marks(&self, marks: crate::FileListMarks, cx: &mut App);
    fn set_filter_chips(&self, chips: Vec<crate::FileListFilterChip>, cx: &mut App);
    fn set_groups(&self, groups: Option<crate::FileListGroups>, cx: &mut App);
    fn set_visible(&self, visible: Option<crate::FileListVisible>, cx: &mut App);
    /// Shows only files whose path contains `query` (case-insensitive).
    fn set_filter(&self, query: SharedString, cx: &mut App);
    /// The files as shown: sorted, filtered.
    fn files(&self, cx: &App) -> Vec<CommitFileChange>;
    fn selected(&self, cx: &App) -> Option<CommitFileChange>;
    /// Selects `path` as if clicked; `false` when it is not shown.
    fn select_path(&self, path: &Path, cx: &mut App) -> bool;
    fn is_loading(&self, cx: &App) -> bool;
}

/// An owning handle to a hosted file list with its own filter, sort,
/// collapse, selection, and scroll.
///
/// Its top row carries the filter chips and the two controls every
/// changed-file list has: a layout icon in the current layout's shape (a
/// click steps to the next layout, a right click offers them all) and a sort
/// menu. The list starts from the user's default layout and sort;
/// [`Self::set_mode`] and [`Self::set_sort`] override those, and what the
/// user picks in the controls holds for the life of the list.
#[derive(Clone)]
pub struct FileList(Rc<dyn FileListImpl>);

impl FileList {
    /// Orders the files. A worktree source's line counts and edits come from
    /// the status lane it lists, so the edit-size and
    /// [`Edits`](crate::FileListSort::Edits) sorts work there too.
    pub fn set_sort(&self, sort: crate::FileListSort, cx: &mut App) {
        self.0.set_sort(sort, cx)
    }
    pub fn set_kind_filter(&self, filter: crate::FileListFilter, cx: &mut App) {
        self.0.set_kind_filter(filter, cx)
    }
    pub fn set_marks(&self, marks: crate::FileListMarks, cx: &mut App) {
        self.0.set_marks(marks, cx)
    }
    pub fn set_filter_chips(&self, chips: Vec<crate::FileListFilterChip>, cx: &mut App) {
        self.0.set_filter_chips(chips, cx)
    }
    /// Groups the grouped mode shows instead of change kinds; `None` goes
    /// back to change kinds. A new set of labels expands every group.
    pub fn set_groups(&self, groups: Option<crate::FileListGroups>, cx: &mut App) {
        self.0.set_groups(groups, cx)
    }
    /// Shows only these paths (with the kind filter and query still
    /// applying); `None` shows every path.
    pub fn set_visible(&self, visible: Option<crate::FileListVisible>, cx: &mut App) {
        self.0.set_visible(visible, cx)
    }
    /// Paths in navigation order, including files inside collapsed directories.
    pub fn ordered_paths(&self, cx: &App) -> Vec<PathBuf> {
        self.0.files(cx).into_iter().map(|file| file.path).collect()
    }
    #[doc(hidden)]
    pub fn new(list: Rc<dyn FileListImpl>) -> Self {
        Self(list)
    }

    pub fn view(&self) -> AnyView {
        self.0.view()
    }

    pub fn set_source(&self, source: ChangeSource, cx: &mut App) {
        self.0.set_source(source, cx)
    }

    /// Arranges the files, in place of the user's default layout.
    pub fn set_mode(&self, mode: FileListMode, cx: &mut App) {
        self.0.set_mode(mode, cx)
    }

    pub fn set_filter(&self, query: impl Into<SharedString>, cx: &mut App) {
        self.0.set_filter(query.into(), cx)
    }

    pub fn files(&self, cx: &App) -> Vec<CommitFileChange> {
        self.0.files(cx)
    }

    pub fn selected(&self, cx: &App) -> Option<CommitFileChange> {
        self.0.selected(cx)
    }

    pub fn select_path(&self, path: &Path, cx: &mut App) -> bool {
        self.0.select_path(path, cx)
    }

    pub fn is_loading(&self, cx: &App) -> bool {
        self.0.is_loading(cx)
    }
}
