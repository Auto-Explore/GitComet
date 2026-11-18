use super::{ElasticScroll, ElasticScrollDriver};
use gpui::{ElementId, IntoElement, Styled};

impl ElasticScroll {
    /// Wraps a styled vertical scroller with matching native and precise-wheel
    /// axis handling. The child retains its sizing, overflow and scroll model.
    ///
    /// Keep the handle in the owning view, and track that same handle on the
    /// child. Wrap only the scrolling body: headers, sticky overlays and
    /// scrollbars belong outside this element.
    ///
    /// ```no_run
    /// use gitcomet_ui_kit::elastic_scroll::ElasticScroll;
    /// use gpui::{ScrollHandle, div, prelude::*, px};
    ///
    /// let scroll = ScrollHandle::new(); // Retain this in the owning view.
    /// let body = ElasticScroll::vertical(
    ///     "body_elastic",
    ///     scroll.clone(),
    ///     div()
    ///         .id("body")
    ///         .h(px(240.0))
    ///         .overflow_y_scroll()
    ///         .track_scroll(&scroll)
    ///         .child(div().h(px(700.0))),
    /// );
    /// ```
    ///
    /// For a `UniformList`, pass its retained `UniformListScrollHandle` as the
    /// driver and call `track_scroll` on the list before wrapping it. For a
    /// variable-height list, pass the same `ListState` to this constructor and
    /// to `gpui::list`. Both keep their existing virtualization and native
    /// scrolling when elastic routing is unavailable.
    ///
    /// A custom [`ElasticScrollDriver`] can likewise wrap its styled viewport;
    /// the viewport's native handler must use the same logical model. Use
    /// [`Self::new`] followed by [`Self::vertical_only`] when a child has already
    /// been type-erased or needs a different native axis policy.
    pub fn vertical(
        id: impl Into<ElementId>,
        driver: impl ElasticScrollDriver,
        child: impl IntoElement + Styled,
    ) -> Self {
        Self::new(id, driver, crate::restrict_scroll_to_vertical_axis(child)).vertical_only()
    }
}
