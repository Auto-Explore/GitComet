//! The example's Changes view: the working tree against HEAD in a hosted
//! file list, the picked file in one diff pane and the previous pick in a
//! second, read-only one. Retargeting either leaves the other and History
//! as they were.

use gitcomet_core::domain::{CommitId, DiffTarget};
use gitcomet_extension_api::{
    ChangeSource, DiffPane, DiffPaneOptions, DiffPanePolicy, FileList, RepositoryViewContext,
};
use gitcomet_ui_kit::gpui::prelude::*;
use gitcomet_ui_kit::gpui::{AnyElement, Context, SharedString, Window, div, px};

pub struct ChangesView {
    context: RepositoryViewContext,
    list: Result<FileList, SharedString>,
    current: Option<DiffPane>,
    previous: Option<DiffPane>,
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
        policy: DiffPanePolicy,
        cx: &mut Context<Self>,
    ) -> Option<DiffPane> {
        let options = DiffPaneOptions {
            policy,
            ..DiffPaneOptions::default()
        };
        self.context
            .window
            .create_diff_pane(&self.context.repository, target, options, cx)
            .ok()
    }

    /// Shows `target` in the current pane and moves what it showed to the
    /// previous one.
    fn show(&mut self, target: DiffTarget, cx: &mut Context<Self>) {
        let Some(current) = self.current.clone() else {
            self.current = self.pane(target, DiffPanePolicy::default(), cx);
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
                None => self.previous = self.pane(shown, DiffPanePolicy::read_only(), cx),
            }
        }
        current.set_target(target, cx);
        cx.notify();
    }
}

fn slot(pane: Option<&DiffPane>, empty: &'static str) -> AnyElement {
    let body = match pane {
        Some(pane) => div().size_full().child(pane.view()),
        None => div().p_3().child(empty),
    };
    div().flex_1().min_h_0().child(body).into_any_element()
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
                    .child(slot(self.current.as_ref(), "Pick a file"))
                    .child(slot(self.previous.as_ref(), "The previous pick shows here")),
            )
    }
}
