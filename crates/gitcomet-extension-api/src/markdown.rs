//! Markdown rendered by the host, for text an extension shows in its own
//! views: a pull request's description, say.

use gitcomet_ui_kit::gpui::{AnyView, App, ScrollHandle, SharedString};
use std::rc::Rc;

/// How a [`MarkdownView`] sets its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownTextStyle {
    /// A document: the Markdown preview font size, inset from the view's
    /// left edge.
    #[default]
    Document,
    /// Words that read as part of the interface around them, such as a
    /// description in a details pane: the UI font size, starting at the
    /// view's left edge.
    Interface,
}

/// What the host implements behind a [`MarkdownView`].
#[doc(hidden)]
pub trait MarkdownViewImpl {
    fn view(&self) -> AnyView;
    fn set_source(&self, source: SharedString, cx: &mut App);
    fn set_scroll_handle(&self, handle: Option<ScrollHandle>, cx: &mut App);
    /// A host that draws one style only keeps it.
    fn set_text_style(&self, _style: MarkdownTextStyle, _cx: &mut App) {}
}

/// An owning handle to a read-only markdown view, from
/// [`WindowHost::create_markdown_view`](crate::WindowHost::create_markdown_view).
/// It lays out at its content's height, so it belongs inside a container
/// that scrolls.
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

    pub fn set_scroll_handle(&self, handle: Option<ScrollHandle>, cx: &mut App) {
        self.0.set_scroll_handle(handle, cx)
    }

    /// Sets the text as `style` says; a new view is a
    /// [`MarkdownTextStyle::Document`].
    pub fn set_text_style(&self, style: MarkdownTextStyle, cx: &mut App) {
        self.0.set_text_style(style, cx)
    }
}
