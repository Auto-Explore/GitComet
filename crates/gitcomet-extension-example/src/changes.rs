//! The example's Changes view: the working tree against HEAD in a hosted
//! file list, the picked file in one diff pane and the previous pick in a
//! second, read-only one. Retargeting either leaves the other and History
//! as they were. Clicking the current pane's gutter flags a line, and a
//! selection can be given a note shown under it. The current pane can pop
//! out into a window of its own and comes back when that window closes.

use gitcomet_core::domain::{CommitId, DiffTarget};
use gitcomet_extension_api::{
    ChangeSource, DiffAnnotation, DiffAnnotations, DiffInset, DiffLegendItem, DiffLineRange,
    DiffLineSide, DiffPane, DiffPaneOptions, DiffPanePolicy, DiffSelectionAction, FileList,
    PopOutWindow, RepositoryViewContext,
};
use gitcomet_ui_kit::components::Button;
use gitcomet_ui_kit::gpui::prelude::*;
use gitcomet_ui_kit::gpui::{
    AnyElement, App, Context, SharedString, WeakEntity, Window, div, px, rgb_to_hsla,
};
use std::collections::BTreeSet;
use std::rc::Rc;

pub struct ChangesView {
    context: RepositoryViewContext,
    list: Result<FileList, SharedString>,
    current: Option<DiffPane>,
    previous: Option<DiffPane>,
    /// The current pane's flagged lines and notes; a new pick clears them.
    flags: BTreeSet<(DiffLineSide, u32)>,
    notes: Vec<DiffInset>,
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
                ChangeSource::Comparison {
                    from: CommitId("HEAD".into()),
                    to: None,
                    options: Default::default(),
                },
                move |_, target, cx| {
                    let _ = view.update(cx, |this, cx| this.show(target, cx));
                },
                cx,
            )
            .map_err(|error| SharedString::from(error.to_string()));
        Self {
            context,
            list,
            current: None,
            previous: None,
            flags: BTreeSet::new(),
            notes: Vec::new(),
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

    /// Options for the current pane: gutter flags and selection notes.
    fn current_options(&self, cx: &mut Context<Self>) -> DiffPaneOptions {
        let view = cx.weak_entity();
        let noted = view.clone();
        DiffPaneOptions {
            on_gutter_click: Some(Rc::new(move |side, line, cx| {
                let _ = view.update(cx, |this, cx| this.toggle_flag(side, line, cx));
            })),
            selection_actions: vec![DiffSelectionAction::new("Add note", move |range, cx| {
                add_note(&noted, range, cx);
            })],
            ..DiffPaneOptions::default()
        }
    }

    pub fn flags(&self) -> &BTreeSet<(DiffLineSide, u32)> {
        &self.flags
    }

    fn toggle_flag(&mut self, side: DiffLineSide, line: u32, cx: &mut Context<Self>) {
        if !self.flags.remove(&(side, line)) {
            self.flags.insert((side, line));
        }
        let color = rgb_to_hsla(self.context.window.theme(cx).colors.accent.solid);
        let annotations =
            self.flags
                .iter()
                .fold(DiffAnnotations::new(), |annotations, (side, line)| {
                    annotations.with(
                        *side,
                        *line,
                        DiffAnnotation::new(color).with_label("flagged"),
                    )
                });
        if let Some(current) = &self.current {
            current.set_annotations(annotations, cx);
            current.set_legend(vec![DiffLegendItem::new("Flagged", color)], cx);
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
        self.flags.clear();
        self.notes.clear();
        current.set_annotations(DiffAnnotations::new(), cx);
        current.set_legend(Vec::new(), cx);
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
        this.notes.push(DiffInset::new(
            range.side,
            range.end,
            [SharedString::from(text)],
        ));
        if let Some(current) = &this.current {
            current.set_insets(this.notes.clone(), cx);
        }
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
