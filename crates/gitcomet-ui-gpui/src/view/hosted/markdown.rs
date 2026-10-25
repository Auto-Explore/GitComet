use super::*;
use crate::kit::drag_autoscroll::{DRAG_AUTOSCROLL_TICK, drag_autoscroll_step};
use crate::view::markdown_interaction::MarkdownInteraction;
use crate::view::markdown_preview::{MarkdownPreviewDocument, parse_markdown};
use crate::view::panes::diff::services::{DiffCommand, DiffCommands, DiffPopoverAnchor};
use crate::view::text_geometry::{TextGeometry, normalized_selection, source_selection_range};
use std::cell::Cell;
use std::rc::Rc;

pub(crate) struct MarkdownView {
    theme: Rc<Cell<AppTheme>>,
    source: SharedString,
    document: Option<Arc<MarkdownPreviewDocument>>,
    blocks: crate::view::rows::MarkdownDocumentBlockCache,
    block_scrolls: crate::view::rows::MarkdownDocumentBlockScrolls,
    layout: crate::view::rows::MarkdownDocumentLayoutCache,
    pub(in crate::view) focus: FocusHandle,
    pub(in crate::view) hitboxes: FxHashMap<(usize, DiffTextRegion), DiffTextHitbox>,
    pub(in crate::view) motion_targets: Vec<DiffTextMotionTarget>,
    anchor: Option<DiffTextPos>,
    head: Option<DiffTextPos>,
    pub(in crate::view) selecting: bool,
    last_mouse: Point<Pixels>,
    scroll: Option<ScrollHandle>,
    autoscroll: Option<gpui::Task<()>>,
    owner: crate::kit::text_selection_owner::SelectionOwnerToken,
    _owner_observer: gpui::Subscription,
    hovered_link: Option<rows::MarkdownPreviewHoveredLink>,
    reveal: rows::MarkdownPreviewRevealRequest,
    commands: DiffCommands,
    disclosures: crate::view::markdown_disclosures::MarkdownDisclosures,
    preferences: crate::view::hosted::environment::DiffPreferences,
    approved_images: Arc<FxHashSet<SharedString>>,
}

impl MarkdownView {
    pub(in crate::view) fn new(
        theme: Rc<Cell<AppTheme>>,
        commands: DiffCommands,
        preferences: crate::view::hosted::environment::DiffPreferences,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let owner_observer =
            crate::kit::text_selection_owner::observe(cx, |this: &mut Self, cx| {
                if this.owner.is_stale(cx) && (this.anchor.is_some() || this.selecting) {
                    this.clear_selection();
                    cx.notify();
                }
            });
        Self {
            theme,
            source: SharedString::default(),
            document: None,
            blocks: Default::default(),
            block_scrolls: Default::default(),
            layout: Default::default(),
            focus: cx.focus_handle(),
            hitboxes: Default::default(),
            motion_targets: Vec::new(),
            anchor: None,
            head: None,
            selecting: false,
            last_mouse: point(px(0.0), px(0.0)),
            scroll: None,
            autoscroll: None,
            owner: Default::default(),
            _owner_observer: owner_observer,
            hovered_link: None,
            reveal: Default::default(),
            commands,
            disclosures: Default::default(),
            preferences,
            approved_images: Arc::default(),
        }
    }

    /// What each parsed row is, for tests.
    #[cfg(test)]
    pub(in crate::view) fn row_kinds(
        &self,
    ) -> Vec<crate::view::markdown_preview::MarkdownPreviewRowKind> {
        self.document
            .iter()
            .flat_map(|document| document.rows.iter().map(|row| row.kind))
            .collect()
    }

    fn set_source(&mut self, source: &str, cx: &mut gpui::Context<Self>) {
        if self.source.as_ref() == source {
            return;
        }
        self.source = source.to_string().into();
        self.document = parse_markdown(source).map(Arc::new);
        self.disclosures = Default::default();
        self.approved_images = Arc::default();
        self.blocks = Default::default();
        self.block_scrolls = Default::default();
        self.layout = Default::default();
        self.clear_selection();
        self.hitboxes.clear();
        self.motion_targets.clear();
        self.hovered_link = None;
        self.reveal = Default::default();
        cx.notify();
    }

    fn clear_selection(&mut self) {
        self.anchor = None;
        self.head = None;
        self.selecting = false;
        self.autoscroll = None;
    }

    pub(in crate::view) fn toggle_disclosure(&mut self, id: usize, cx: &mut gpui::Context<Self>) {
        let hides_selection = if let (Some(document), Some((start, end))) =
            (&self.document, normalized_selection(self.anchor, self.head))
        {
            document.rows.iter().enumerate().any(|(ix, row)| {
                (start.source_visible_ix..=end.source_visible_ix).contains(&ix)
                    && self.disclosures.would_hide(row, id)
            })
        } else {
            false
        };
        if !self.disclosures.toggle(id) {
            return;
        }
        if hides_selection {
            self.clear_selection();
        }
        self.invalidate_visibility();
        cx.notify();
    }

    fn invalidate_visibility(&mut self) {
        self.layout = Default::default();
        self.blocks = Default::default();
        self.hitboxes.clear();
        self.motion_targets.clear();
        self.hovered_link = None;
    }

    fn geometry(&self) -> TextGeometry<'_> {
        TextGeometry {
            hitboxes: &self.hitboxes,
            motion_targets: &self.motion_targets,
            layout_cache: None,
            anchor: self.anchor,
            selecting: self.selecting,
        }
    }

    pub(in crate::view) fn selection_color(&self) -> gpui::Rgba {
        self.theme.get().colors.editor.selection_background
    }

    pub(in crate::view) fn selection_range(
        &self,
        row: usize,
        visual: Range<usize>,
    ) -> Option<Range<usize>> {
        let selected = source_selection_range(
            self.anchor,
            self.head,
            false,
            row,
            DiffTextRegion::Inline,
            visual.end,
        )?;
        let start = selected.start.max(visual.start);
        let end = selected.end.min(visual.end);
        (start < end)
            .then_some(start.saturating_sub(visual.start)..end.saturating_sub(visual.start))
    }

    pub(in crate::view) fn has_selection(&self) -> bool {
        normalized_selection(self.anchor, self.head).is_some_and(|(start, end)| start != end)
    }

    pub(in crate::view) fn row_mouse_down(
        &mut self,
        row: usize,
        position: Point<Pixels>,
        click_count: usize,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let geometry = self.geometry();
        let pos = if click_count == 1 {
            geometry.diff_text_pos_from_nearest_hitbox(row, DiffTextRegion::Inline, position)
        } else {
            geometry.diff_text_pos_from_hitbox(row, DiffTextRegion::Inline, position)
        };
        let Some(pos) = pos else { return };
        if click_count == 1 {
            self.begin_selection(pos, position, window, cx);
            return;
        }
        let Some(text) = self
            .document
            .as_ref()
            .and_then(|document| document.rows.get(row))
            .map(|row| &row.text)
        else {
            return;
        };
        let range = if click_count == 2 {
            crate::text_selection::token_range_for_offset(text, pos.offset)
        } else {
            0..text.len()
        };
        self.owner.adopt(window, cx);
        self.selecting = false;
        self.anchor = Some(DiffTextPos {
            offset: range.start,
            ..pos
        });
        self.head = Some(DiffTextPos {
            offset: range.end,
            ..pos
        });
        cx.notify();
    }

    pub(in crate::view) fn begin_selection(
        &mut self,
        pos: DiffTextPos,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.owner.adopt(window, cx);
        self.anchor = Some(pos);
        self.head = Some(pos);
        self.selecting = true;
        self.last_mouse = position;
        self.hovered_link = None;
        self.autoscroll = Some(cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor().timer(DRAG_AUTOSCROLL_TICK).await;
                let keep_running = view
                    .update(cx, |view, cx| {
                        if !view.selecting {
                            return false;
                        }
                        if view.tick_autoscroll() {
                            cx.notify();
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        }));
        cx.notify();
    }

    pub(in crate::view) fn selection_move(&mut self, position: Point<Pixels>) -> bool {
        if !self.selecting {
            return false;
        }
        self.last_mouse = position;
        let Some(pos) = self.geometry().diff_text_pos_for_mouse(position) else {
            return false;
        };
        let changed = self.head != Some(pos);
        self.head = Some(pos);
        changed
    }

    fn tick_autoscroll(&mut self) -> bool {
        let mut scrolled = false;
        if let Some(scroll) = &self.scroll {
            let bounds = scroll.bounds();
            if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) {
                let old = scroll.offset();
                let max = scroll.max_offset();
                let next = point(
                    (old.x
                        + drag_autoscroll_step(self.last_mouse.x, bounds.left(), bounds.right()))
                    .clamp(-max.x, px(0.0)),
                    (old.y
                        + drag_autoscroll_step(self.last_mouse.y, bounds.top(), bounds.bottom()))
                    .clamp(-max.y, px(0.0)),
                );
                scrolled = next != old;
                if scrolled {
                    scroll.set_offset(next);
                }
            }
        }
        self.selection_move(self.last_mouse) || scrolled
    }

    fn select_all(&mut self, window: &Window, cx: &mut gpui::Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };
        let Some((last, row)) = document.rows.iter().enumerate().rfind(|(_, row)| {
            row.kind != crate::view::markdown_preview::MarkdownPreviewRowKind::Spacer
                && self.disclosures.is_visible(row)
        }) else {
            return;
        };
        self.anchor = Some(DiffTextPos {
            source_visible_ix: 0,
            region: DiffTextRegion::Inline,
            offset: 0,
        });
        self.head = Some(DiffTextPos {
            source_visible_ix: last,
            region: DiffTextRegion::Inline,
            offset: row.text.len(),
        });
        self.selecting = false;
        self.owner.adopt(window, cx);
        cx.notify();
    }

    fn copy(&self, cx: &mut gpui::Context<Self>) {
        let Some(document) = &self.document else {
            return;
        };
        if !self.has_selection() {
            return;
        }
        let lines: Vec<_> = document
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| self.disclosures.is_visible(row))
            .filter_map(|(row_ix, row)| {
                let range = source_selection_range(
                    self.anchor,
                    self.head,
                    false,
                    row_ix,
                    DiffTextRegion::Inline,
                    row.text.len(),
                )?;
                row.text.get(range).map(str::to_owned)
            })
            .collect();
        crate::clipboard::write_text(
            cx,
            lines.join("\n"),
            crate::clipboard::CopySource::Extension,
        );
    }

    pub(in crate::view) fn link_at(
        &self,
        row: usize,
        position: Point<Pixels>,
    ) -> Option<(SharedString, Range<usize>)> {
        let hitbox = self.hitboxes.get(&(row, DiffTextRegion::Inline))?;
        if !hitbox.bounds.contains(&position) {
            return None;
        }
        let hit =
            self.geometry()
                .diff_text_hit_in_hitbox(hitbox, DiffTextRegion::Inline, position)?;
        if hit.past_painted_text {
            return None;
        }
        let row = self.document.as_ref()?.rows.get(row)?;
        let span_ix = row.inline_spans.iter().position(|span| {
            span.byte_range.contains(&hit.pos.offset) && span.link_url.is_some()
        })?;
        Some((
            row.inline_spans[span_ix].link_url.clone()?,
            crate::view::markdown_interaction::linked_span_range(row, span_ix),
        ))
    }

    fn anchor_row(&self, destination: &str) -> Option<usize> {
        use crate::view::markdown_preview::{
            MarkdownLinkTarget, classify_markdown_link_destination, markdown_preview_anchor_row,
        };
        let MarkdownLinkTarget::Anchor(fragment) = classify_markdown_link_destination(destination)?
        else {
            return None;
        };
        markdown_preview_anchor_row(
            self.document.as_ref()?,
            &rows::percent_decode_link_path(&fragment),
        )
    }

    pub(in crate::view) fn hover(
        &mut self,
        row: Option<usize>,
        position: Point<Pixels>,
        button_held: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::view::markdown_preview::{
            MarkdownLinkTarget, classify_markdown_link_destination,
        };
        let hovered = row.filter(|_| !button_held).and_then(|row_ix| {
            let (destination, byte_range) = self.link_at(row_ix, position)?;
            let followable = matches!(
                classify_markdown_link_destination(&destination),
                Some(MarkdownLinkTarget::Web(_))
            ) || self.anchor_row(&destination).is_some();
            followable.then_some(rows::MarkdownPreviewHoveredLink {
                region: DiffTextRegion::Inline,
                row_ix,
                byte_range,
            })
        });
        if self.hovered_link != hovered {
            self.hovered_link = hovered;
            cx.notify();
        }
    }

    pub(in crate::view) fn link_click(
        &mut self,
        row: usize,
        position: Point<Pixels>,
        click_count: usize,
        follow: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if click_count != 1 || self.has_selection() {
            return;
        }
        let Some((destination, range)) = self.link_at(row, position) else {
            return;
        };
        if self.link_at(row, window.mouse_position()) != Some((destination.clone(), range.clone()))
        {
            return;
        }
        let anchor = self
            .hitboxes
            .get(&(row, DiffTextRegion::Inline))
            .and_then(|hitbox| {
                self.geometry()
                    .diff_text_bounds_in_hitbox(hitbox, range, position)
            });
        self.open_link(destination, anchor, position, follow, window, cx);
    }

    pub(in crate::view) fn open_link(
        &mut self,
        destination: SharedString,
        anchor: Option<Bounds<Pixels>>,
        position: Point<Pixels>,
        follow: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open_image_link(destination, None, anchor, position, follow, window, cx);
    }

    pub(in crate::view) fn approve_image(
        &mut self,
        url: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.preferences.read(cx).security.remote_markdown_images
            != RemoteMarkdownImagePolicy::AskBeforeLoading
        {
            return;
        }
        if Arc::make_mut(&mut self.approved_images).insert(url) {
            self.layout = Default::default();
            cx.notify();
        }
    }

    pub(in crate::view) fn open_image_link(
        &mut self,
        destination: SharedString,
        load_remote_image_url: Option<SharedString>,
        anchor: Option<Bounds<Pixels>>,
        position: Point<Pixels>,
        follow: bool,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        use crate::view::markdown_preview::{
            MarkdownLinkTarget, classify_markdown_link_destination,
        };
        if let Some(row) = self.anchor_row(&destination) {
            if let Some(url) = load_remote_image_url {
                self.approve_image(url, cx);
                return;
            }
            if let Some(target) = self
                .document
                .as_ref()
                .and_then(|document| document.rows.get(row))
                && self.disclosures.reveal(target)
            {
                self.invalidate_visibility();
            }
            self.reveal.request_top(row);
            cx.notify();
            return;
        }
        let Some(MarkdownLinkTarget::Web(url)) = classify_markdown_link_destination(&destination)
        else {
            if let Some(url) = load_remote_image_url {
                self.approve_image(url, cx);
            }
            return;
        };
        if follow {
            crate::view::panes::diff::diff_text::open_markdown_web_link(
                url,
                self.commands.clone(),
                cx,
            );
            return;
        }
        let kind = PopoverKind::WebLinkMenu {
            url,
            load_remote_image_url,
        };
        let mut request = PopoverRequest::from(kind).returning_focus_to(self.focus.clone());
        request.markdown_image_approval = Some(MarkdownInteraction::Hosted(cx.entity()));
        self.commands.defer(
            DiffCommand::OpenPopover {
                request: Box::new(request),
                anchor: anchor
                    .map(DiffPopoverAnchor::Bounds)
                    .unwrap_or(DiffPopoverAnchor::Point(position)),
                pane: None,
                window: window.window_handle(),
            },
            cx,
        );
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(document) = self.document.clone() else {
            return div()
                .debug_selector(|| "hosted_markdown_empty".to_string())
                .into_any_element();
        };
        let context = crate::view::rows::MarkdownDocumentContext {
            theme: self.theme.get(),
            ui_scale_percent: crate::ui_scale::current(cx).percent,
            editor_font_family: crate::font_preferences::current_editor_font_family(cx).into(),
            image_root: None,
            remote_image_access: rows::MarkdownRemoteImageAccess {
                policy: self.preferences.read(cx).security.remote_markdown_images,
                approved_urls: self.approved_images.clone(),
                approval_view: Some(MarkdownInteraction::Hosted(cx.entity())),
            },
            picture_sizes: Default::default(),
            drawn_pictures: None,
            row_boxes: Default::default(),
            block_scrolls: self.block_scrolls.clone(),
            blocks: self.blocks.clone(),
            layout: self.layout.clone(),
            view: Some(MarkdownInteraction::Hosted(cx.entity())),
            text_region: DiffTextRegion::Inline,
            change_bar_color: None,
            query: None,
            reveal: self.reveal.clone(),
            scroll: self.scroll.clone(),
            diagram_heights: crate::view::diagram_preview::block::DiagramHeights::new(cx),
            hovered_link: self.hovered_link.clone(),
            change_extents: None,
            tasks_editable: false,
            disclosures: Some(self.disclosures.clone()),
        };
        self.hitboxes.clear();
        self.motion_targets.clear();
        div()
            .id("hosted_markdown")
            .key_context("MarkdownPreview")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::kit::SelectAll, window, cx| {
                this.select_all(window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::kit::Copy, _, cx| {
                this.copy(cx);
                cx.stop_propagation();
            }))
            .debug_selector(|| "hosted_markdown".to_string())
            .w_full()
            .child(crate::view::rows::render_markdown_document(
                usize::from(crate::view::tab_width::DEFAULT_TAB_WIDTH),
                &document,
                &context,
            ))
            .child(crate::view::diff_text_selection::DiffTextSelectionTracker {
                view: MarkdownInteraction::Hosted(cx.entity()),
            })
            .into_any_element()
    }
}

/// The extension-facing handle; it owns the view.
pub(crate) struct HostedMarkdownView {
    pub(crate) entity: Entity<MarkdownView>,
}

impl gitcomet_extension_api::MarkdownViewImpl for HostedMarkdownView {
    fn view(&self) -> gpui::AnyView {
        self.entity.clone().into()
    }

    fn set_source(&self, source: SharedString, cx: &mut App) {
        self.entity
            .update(cx, |view, cx| view.set_source(source.as_ref(), cx));
    }

    fn set_scroll_handle(&self, handle: Option<ScrollHandle>, cx: &mut App) {
        self.entity.update(cx, |view, cx| {
            view.scroll = handle;
            cx.notify();
        });
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
