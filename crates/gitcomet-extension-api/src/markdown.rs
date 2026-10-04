//! Markdown rendered by the host, for text an extension shows in its own
//! views: a pull request's description, say.

use gitcomet_ui_kit::gpui::{AnyView, App, SharedString};
use std::rc::Rc;

/// What the host implements behind a [`MarkdownView`].
#[doc(hidden)]
pub trait MarkdownViewImpl {
    fn view(&self) -> AnyView;
    fn set_source(&self, source: SharedString, cx: &mut App);
}

/// An owning handle to a read-only markdown view, from
/// [`WindowHost::create_markdown_view`](crate::WindowHost::create_markdown_view).
/// It lays out at its content's height, so it belongs inside a container
/// that scrolls. It uses History's markdown renderer, read-only: nothing can
/// be selected, links are shown but do not open, and a line holding inline
/// code is set in the editor font throughout.
#[derive(Clone)]
pub struct MarkdownView(Rc<dyn MarkdownViewImpl>);

impl MarkdownView {
    #[doc(hidden)]
    pub fn new(view: Rc<dyn MarkdownViewImpl>) -> Self {
        Self(view)
    }

    pub fn view(&self) -> AnyView {
        self.0.view()
    }

    /// Shows `source`, replacing what was there.
    pub fn set_source(&self, source: impl Into<SharedString>, cx: &mut App) {
        self.0.set_source(source.into(), cx)
    }
}
