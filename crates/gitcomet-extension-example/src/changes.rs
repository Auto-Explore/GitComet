//! The example's Changes view: the working tree against HEAD in a hosted
//! file list grouped by the files' roles, the picked file in one diff pane and
//! the previous pick in a second, read-only one. Retargeting either leaves the
//! other and History as they were. Clicking the current pane's gutter flags
//! a line; a file with flagged lines gets a flag in the list and its
//! "Flagged" chip shows only those files; clicking a flag removes it. A
//! selection can be given a note shown under it, and clicking the note
//! removes it. The current pane can pop out into a window of its own and
//! comes back when that window closes. Its action-bar context names the
//! comparison and marks the repository reviewed.

use gitcomet_core::domain::{CommitId, DiffTarget};
use gitcomet_extension_api::{
    ChangeSource, DiffAnnotation, DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange,
    DiffLineSide, DiffPane, DiffPaneOptions, DiffPanePolicy, DiffSelectionAction, FileList,
    FileListFilterChip, FileListGroups, FileListMarks, FileListMode, FileListVisible, HostedAction,
    PopOutWindow, RepositoryViewContext, RowGlyph, RowMark,
};
use gitcomet_ui_kit::components::Button;
use gitcomet_ui_kit::gpui::prelude::*;
use gitcomet_ui_kit::gpui::{
    AnyElement, App, Context, Entity, SharedString, Subscription, WeakEntity, Window, div, px,
    rgb_to_hsla,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Flags = BTreeSet<(DiffLineSide, u32)>;

/// The list's groups, by what a file is for; the rest show under "Other".
const ROLES: [&str; 4] = ["Code", "Tests", "Docs", "Config"];

fn role_of(path: &Path) -> Option<usize> {
    let extension = path.extension().and_then(|extension| extension.to_str());
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    if path.components().any(|part| part.as_os_str() == "tests")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
    {
        return Some(1);
    }
    match extension? {
        "md" | "txt" | "rst" => Some(2),
        "toml" | "json" | "yml" | "yaml" | "lock" => Some(3),
        "rs" | "js" | "ts" | "py" | "go" | "c" | "h" | "cpp" | "java" | "swift" => Some(0),
        _ => None,
    }
}

pub struct ChangesView {
    context: RepositoryViewContext,
    list: Result<FileList, SharedString>,
    current: Option<DiffPane>,
    previous: Option<DiffPane>,
    /// The file the current pane shows.
    current_path: Option<PathBuf>,
    /// Flagged lines by file; the current pane shows its file's.
    flags: BTreeMap<PathBuf, Flags>,
    /// Bumped with every flag change.
    flags_revision: u64,
    /// The current pane's notes by id; a new pick clears them.
    notes: Vec<(u64, DiffInset)>,
    next_note: u64,
    /// The window the current pane is shown in instead of here.
    popped: Option<PopOutWindow>,
}

impl ChangesView {
    pub fn new(context: RepositoryViewContext, cx: &mut Context<Self>) -> Self {
        let view = cx.weak_entity();
        let list = context
            .window
            .create_file_list(
                &context.repository,
                ChangeSource::comparison(CommitId("HEAD".into()), None, Default::default()),
                move |_, target, cx| {
                    let _ = view.update(cx, |this, cx| this.show(target, cx));
                },
                cx,
            )
            .map_err(|error| SharedString::from(error.to_string()));
        if let Ok(list) = &list {
            list.set_mode(FileListMode::Grouped, cx);
            let labels: Vec<SharedString> = ROLES.into_iter().map(SharedString::from).collect();
            list.set_groups(Some(FileListGroups::new(0, labels, role_of)), cx);
        }
        Self {
            context,
            list,
            current: None,
            previous: None,
            current_path: None,
            flags: BTreeMap::new(),
            flags_revision: 0,
            notes: Vec::new(),
            next_note: 0,
            popped: None,
        }
    }

    pub fn popped(&self) -> Option<&PopOutWindow> {
        self.popped.as_ref()
    }

    /// Shows the current pane in a window of its own until that closes.
    fn pop_out(&mut self, cx: &mut Context<Self>) {
        let Some(current) = &self.current else {
            return;
        };
        if self.popped.is_some() {
            return;
        }
        let pane = current.view();
        let view = cx.weak_entity();
        self.popped = self
            .context
            .window
            .open_window(
                "Diff",
                move |_, _| pane,
                move |cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.popped = None;
                        cx.notify();
                    });
                },
                cx,
            )
            .ok();
        cx.notify();
    }

    /// Options for the current pane: gutter flags (a flag's own click
    /// removes it) and selection notes.
    fn current_options(&self, cx: &mut Context<Self>) -> DiffPaneOptions {
        let view = cx.weak_entity();
        let noted = view.clone();
        let toggle: gitcomet_extension_api::DiffGutterAction = Rc::new(move |side, line, cx| {
            let _ = view.update(cx, |this, cx| this.toggle_flag(side, line, cx));
        });
        DiffPaneOptions {
            on_gutter_click: Some(Rc::clone(&toggle)),
            on_annotation_click: Some(toggle),
            selection_actions: vec![DiffSelectionAction::new("Add note", move |range, cx| {
                add_note(&noted, range, cx);
            })],
            ..DiffPaneOptions::default()
        }
    }

    /// The current file's flagged lines.
    pub fn flags(&self) -> &Flags {
        static NONE: Flags = BTreeSet::new();
        self.current_path
            .as_ref()
            .and_then(|path| self.flags.get(path))
            .unwrap_or(&NONE)
    }

    /// Files with flagged lines.
    pub fn flagged_paths(&self) -> BTreeSet<PathBuf> {
        self.flags.keys().cloned().collect()
    }

    fn toggle_flag(&mut self, side: DiffLineSide, line: u32, cx: &mut Context<Self>) {
        let Some(path) = self.current_path.clone() else {
            return;
        };
        let flags = self.flags.entry(path.clone()).or_default();
        if !flags.remove(&(side, line)) {
            flags.insert((side, line));
        }
        if flags.is_empty() {
            self.flags.remove(&path);
        }
        self.flags_revision += 1;
        self.show_flags(cx);
        self.mark_flagged_files(cx);
    }

    /// The current file's flags as annotations in the current pane.
    fn show_flags(&self, cx: &mut Context<Self>) {
        let Some(current) = &self.current else {
            return;
        };
        let color = rgb_to_hsla(self.context.window.theme(cx).colors.accent.solid);
        let annotations =
            self.flags()
                .iter()
                .fold(DiffAnnotations::new(), |annotations, (side, line)| {
                    annotations.with(
                        *side,
                        *line,
                        DiffAnnotation::new(color).with_label("flagged"),
                    )
                });
        let legend = if self.flags().is_empty() {
            Vec::new()
        } else {
            vec![DiffLegendItem::new("Flagged", color)]
        };
        current.set_annotations(annotations, cx);
        current.set_legend(legend, cx);
    }

    /// Flags the flagged files in the list and offers a chip showing only them.
    fn mark_flagged_files(&self, cx: &mut Context<Self>) {
        let Ok(list) = &self.list else {
            return;
        };
        let color = rgb_to_hsla(self.context.window.theme(cx).colors.accent.solid);
        let mark =
            RowMark::new(color).with_glyph(RowGlyph::Icon(crate::review::FLAG_ICON_PATH.into()));
        let rows: BTreeMap<PathBuf, RowMark> = self
            .flags
            .keys()
            .map(|path| (path.clone(), mark.clone()))
            .collect();
        list.set_marks(FileListMarks::new(self.flags_revision, rows), cx);
        let visible = FileListVisible::new(self.flags_revision, self.flagged_paths());
        list.set_filter_chips(vec![FileListFilterChip::visible("Flagged", visible)], cx);
    }

    /// The current pane's notes.
    pub fn notes(&self) -> Vec<DiffInset> {
        self.notes.iter().map(|(_, note)| note.clone()).collect()
    }

    fn show_notes(&self, cx: &mut Context<Self>) {
        if let Some(current) = &self.current {
            current.set_insets(self.notes(), cx);
        }
    }

    pub fn current(&self) -> Option<&DiffPane> {
        self.current.as_ref()
    }

    pub fn previous(&self) -> Option<&DiffPane> {
        self.previous.as_ref()
    }

    fn pane(
        &self,
        target: DiffTarget,
        options: DiffPaneOptions,
        cx: &mut Context<Self>,
    ) -> Option<DiffPane> {
        self.context
            .window
            .create_diff_pane(&self.context.repository, target, options, cx)
            .ok()
    }

    /// Shows `target` in the current pane and moves what it showed to the
    /// previous one.
    fn show(&mut self, target: DiffTarget, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            self.current_path = target.file_path().map(Path::to_path_buf);
            let options = self.current_options(cx);
            self.current = self.pane(target, options, cx);
            cx.notify();
            return;
        };
        let shown = current.target(cx);
        if shown.as_ref() == Some(&target) {
            return;
        }
        if let Some(shown) = shown {
            match &self.previous {
                Some(previous) => previous.set_target(shown, cx),
                None => {
                    let options = DiffPaneOptions {
                        policy: DiffPanePolicy::read_only(),
                        ..DiffPaneOptions::default()
                    };
                    self.previous = self.pane(shown, options, cx);
                }
            }
        }
        self.current_path = target.file_path().map(Path::to_path_buf);
        self.notes.clear();
        self.show_flags(cx);
        current.set_insets(Vec::new(), cx);
        current.set_target(target, cx);
        cx.notify();
    }
}

/// Notes the selection under its last line.
fn add_note(view: &WeakEntity<ChangesView>, range: DiffLineRange, cx: &mut App) {
    let _ = view.update(cx, |this, cx| {
        let text = if range.start == range.end {
            format!("Note on line {}", range.start)
        } else {
            format!("Note on lines {}–{}", range.start, range.end)
        };
        let id = this.next_note;
        this.next_note += 1;
        let owner = cx.weak_entity();
        let remove = HostedAction::new("Remove note", move |cx| {
            let _ = owner.update(cx, |this, cx| {
                this.notes.retain(|(note, _)| *note != id);
                this.show_notes(cx);
            });
        });
        let note =
            DiffInset::new(range.side, range.end, [SharedString::from(text)]).with_action(remove);
        this.notes.push((id, note));
        this.show_notes(cx);
    });
}

fn slot(pane: Option<&DiffPane>, empty: &'static str) -> AnyElement {
    let body = match pane {
        Some(pane) => div().size_full().child(pane.view()),
        None => div().p_3().child(empty),
    };
    div().flex_1().min_h_0().child(body).into_any_element()
}

fn popped_slot() -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .p_3()
        .child("Shown in its own window")
        .into_any_element()
}

impl Render for ChangesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.window.theme(cx);
        let list = match &self.list {
            Ok(list) => div().size_full().child(list.view()),
            Err(error) => div().p_3().child(error.clone()),
        };
        div()
            .id("example_changes_view")
            .debug_selector(|| "example_changes_view".to_string())
            .size_full()
            .flex()
            .text_color(theme.colors.foreground.primary)
            .child(
                div()
                    .w(px(240.0))
                    .h_full()
                    .border_r_1()
                    .border_color(theme.colors.stroke.subtle)
                    .child(list),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .when(self.current.is_some() && self.popped.is_none(), |column| {
                        column.child(div().flex().flex_none().px_2().py_1().child(
                            Button::new("example_changes_pop_out", "Pop out").on_click(
                                theme,
                                cx,
                                |this, _, _, cx| this.pop_out(cx),
                            ),
                        ))
                    })
                    .child(match self.popped {
                        Some(_) => popped_slot(),
                        None => slot(self.current.as_ref(), "Pick a file"),
                    })
                    .child(slot(self.previous.as_ref(), "The previous pick shows here")),
            )
    }
}

/// The Changes view's action-bar context: what it compares, a button marking
/// the repository reviewed, and how often it was.
pub struct ChangesActions {
    context: RepositoryViewContext,
    reviews: Entity<crate::review::Reviews>,
    _observe: Subscription,
}

impl ChangesActions {
    pub fn new(context: RepositoryViewContext, cx: &mut Context<Self>) -> Self {
        let reviews = crate::review::reviews(cx);
        let observe = cx.observe(&reviews, |_, _, cx| cx.notify());
        Self {
            context,
            reviews,
            _observe: observe,
        }
    }
}

impl Render for ChangesActions {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.window.theme(cx);
        let count = self
            .reviews
            .read(cx)
            .count(self.context.window.id(), self.context.repository.workdir());
        div()
            .id("example_changes_actions")
            .debug_selector(|| "example_changes_actions".to_string())
            .flex()
            .items_center()
            .gap_2()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child("Working tree against HEAD")
            .child(
                Button::new("example_changes_mark_reviewed", "Mark reviewed").on_click(
                    theme,
                    cx,
                    |this, _, _, cx| {
                        let workdir = this.context.repository.workdir().to_path_buf();
                        crate::review::mark_reviewed(&this.context.window, &workdir, cx);
                    },
                ),
            )
            .child(format!("Reviewed {count} times"))
    }
}
