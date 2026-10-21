use super::*;
use crate::view::hosted::markdown::MarkdownView;

pub(in crate::view) fn linked_span_range(
    row: &crate::view::markdown_preview::MarkdownPreviewRow,
    span_ix: usize,
) -> Range<usize> {
    let spans = &row.inline_spans;
    let url = &spans[span_ix].link_url;
    let same_link = |a: usize, b: usize| {
        spans[a].link_url.as_ref() == url.as_ref()
            && spans[b].link_url.as_ref() == url.as_ref()
            && spans[a].byte_range.end == spans[b].byte_range.start
    };
    let mut first = span_ix;
    while first > 0 && same_link(first - 1, first) {
        first -= 1;
    }
    let mut last = span_ix;
    while last + 1 < spans.len() && same_link(last, last + 1) {
        last += 1;
    }
    spans[first].byte_range.start..spans[last].byte_range.end
}

#[derive(Clone)]
pub(in crate::view) enum MarkdownInteraction {
    Diff(Entity<DiffRenderer>),
    Hosted(Entity<MarkdownView>),
}

impl From<Entity<DiffRenderer>> for MarkdownInteraction {
    fn from(view: Entity<DiffRenderer>) -> Self {
        Self::Diff(view)
    }
}

impl MarkdownInteraction {
    pub(in crate::view) fn approve_image(&self, url: SharedString, cx: &mut App) {
        match self {
            Self::Diff(view) => {
                view.update(cx, |this, cx| this.approve_remote_markdown_image(url, cx))
            }
            Self::Hosted(view) => view.update(cx, |this, cx| this.approve_image(url, cx)),
        }
    }
    pub(in crate::view) fn toggle_disclosure(&self, id: usize, cx: &mut App) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                this.toggle_markdown_disclosure(id, cx);
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.toggle_disclosure(id, cx);
            }),
        }
    }

    pub(in crate::view) fn activate_disclosure(
        &self,
        id: usize,
        row: usize,
        region: DiffTextRegion,
        event: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let gpui::ClickEvent::Mouse(mouse) = event {
            let selected = match self {
                Self::Diff(view) => view.read(cx).diff_text_has_selection(),
                Self::Hosted(view) => view.read(cx).has_selection(),
            };
            if mouse.down.click_count != 1 || selected {
                return;
            }
            let link_at = |position| match self {
                Self::Diff(view) => view
                    .read(cx)
                    .markdown_preview_link_span_at(row, region, position),
                Self::Hosted(view) => view.read(cx).link_at(row, position),
            };
            if link_at(mouse.down.position).is_some() || link_at(mouse.up.position).is_some() {
                self.link_click(
                    row,
                    region,
                    mouse.down.position,
                    mouse.down.click_count,
                    mouse.down.modifiers.secondary(),
                    window,
                    cx,
                );
                return;
            }
        }
        self.toggle_disclosure(id, cx);
    }

    pub(in crate::view) fn notify(&self, cx: &mut App) {
        match self {
            Self::Diff(view) => view.update(cx, |_, cx| cx.notify()),
            Self::Hosted(view) => view.update(cx, |_, cx| cx.notify()),
        }
    }

    pub(in crate::view) fn focus(&self, window: &mut Window, cx: &mut App) {
        let focus = match self {
            Self::Diff(view) => view.read(cx).diff_panel_focus_handle.clone(),
            Self::Hosted(view) => view.read(cx).focus.clone(),
        };
        window.focus(&focus, cx);
    }

    pub(in crate::view) fn selection_range(
        &self,
        region: DiffTextRegion,
        row: usize,
        range: Range<usize>,
        cx: &App,
    ) -> Option<Range<usize>> {
        match self {
            Self::Diff(view) => view
                .read(cx)
                .diff_text_local_selection_range_in(region, (row, range)),
            Self::Hosted(view) => view.read(cx).selection_range(row, range),
        }
    }

    pub(in crate::view) fn selection_color(&self, cx: &App) -> gpui::Rgba {
        match self {
            Self::Diff(view) => view.read(cx).diff_text_selection_color(),
            Self::Hosted(view) => view.read(cx).selection_color(),
        }
    }

    pub(in crate::view) fn set_hitbox(
        &self,
        row: usize,
        region: DiffTextRegion,
        hitbox: DiffTextHitbox,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => {
                view.update(cx, |this, _| this.set_diff_text_hitbox(row, region, hitbox))
            }
            Self::Hosted(view) => {
                view.update(cx, |this, _| {
                    this.hitboxes.insert((row, region), hitbox);
                });
            }
        }
    }

    pub(in crate::view) fn add_cell_hitbox(
        &self,
        row: usize,
        region: DiffTextRegion,
        row_len: usize,
        hitbox: DiffTextHitbox,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, _| {
                this.add_diff_text_cell_hitbox(row, region, row_len, hitbox)
            }),
            Self::Hosted(view) => view.update(cx, |this, _| {
                crate::view::text_geometry::add_cell_hitbox(
                    &mut this.hitboxes,
                    row,
                    region,
                    row_len,
                    hitbox,
                );
            }),
        }
    }

    pub(in crate::view) fn set_motion_target(
        &self,
        bounds: Bounds<Pixels>,
        start: DiffTextPos,
        end: DiffTextPos,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, _| {
                this.set_diff_text_motion_target(bounds, start, end)
            }),
            Self::Hosted(view) => view.update(cx, |this, _| {
                this.motion_targets
                    .push(DiffTextMotionTarget { bounds, start, end });
            }),
        }
    }

    pub(in crate::view) fn row_mouse_down(
        &self,
        row: usize,
        region: DiffTextRegion,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                this.handle_markdown_preview_row_mouse_down(
                    row,
                    region,
                    event.position,
                    event.click_count,
                    window,
                    cx,
                );
                cx.notify();
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.row_mouse_down(row, event.position, event.click_count, window, cx);
            }),
        }
    }

    pub(in crate::view) fn gap_mouse_down(
        &self,
        row: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                this.handle_diff_text_document_gap_mouse_down(row, region, position, window, cx);
                cx.notify();
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.begin_selection(
                    DiffTextPos {
                        source_visible_ix: row,
                        region,
                        offset: 0,
                    },
                    position,
                    window,
                    cx,
                );
            }),
        }
    }

    pub(in crate::view) fn hover(
        &self,
        row: Option<(usize, DiffTextRegion)>,
        position: Point<Pixels>,
        button_held: bool,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| match row {
                Some((row, region)) => {
                    this.update_markdown_preview_link_hover(row, region, position, button_held, cx);
                }
                None => this.clear_markdown_preview_link_hover(cx),
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.hover(row.map(|(row, _)| row), position, button_held, cx);
            }),
        }
    }

    pub(in crate::view) fn link_click(
        &self,
        row: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        click_count: usize,
        follow: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                this.handle_markdown_preview_link_click(
                    row,
                    region,
                    position,
                    click_count,
                    follow,
                    window,
                    cx,
                );
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.link_click(row, position, click_count, follow, window, cx);
            }),
        }
    }

    pub(in crate::view) fn image_link_click(
        &self,
        row: (usize, DiffTextRegion),
        destination: SharedString,
        load_remote_image_url: Option<SharedString>,
        anchor: Option<Bounds<Pixels>>,
        position: Point<Pixels>,
        follow: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (row, region) = row;
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                this.open_markdown_preview_link_menu(
                    region,
                    row,
                    destination,
                    load_remote_image_url,
                    anchor,
                    position,
                    follow,
                    window,
                    cx,
                );
                cx.notify();
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                this.open_image_link(
                    destination,
                    load_remote_image_url,
                    anchor,
                    position,
                    follow,
                    window,
                    cx,
                );
            }),
        }
    }

    pub(in crate::view) fn context_menu(
        &self,
        row: usize,
        region: DiffTextRegion,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Self::Diff(view) = self {
            view.update(cx, |this, cx| {
                this.open_diff_editor_context_menu(row, region, position, window, cx);
                cx.notify();
            });
        }
    }

    pub(in crate::view) fn toggle_task(
        &self,
        region: DiffTextRegion,
        task: crate::view::markdown_preview::MarkdownTaskMarker,
        cx: &mut App,
    ) {
        if let Self::Diff(view) = self {
            view.update(cx, |this, cx| {
                this.toggle_markdown_preview_task(region, task, cx)
            });
        }
    }

    pub(in crate::view) fn selection_move(&self, position: Point<Pixels>, cx: &mut App) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                if !this.diff_text_selecting {
                    return;
                }
                let before = this.diff_text_head;
                this.update_diff_text_selection_from_mouse(position);
                if before != this.diff_text_head {
                    cx.notify();
                }
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                if this.selection_move(position) {
                    cx.notify();
                }
            }),
        }
    }

    pub(in crate::view) fn selection_end(&self, cx: &mut App) {
        match self {
            Self::Diff(view) => view.update(cx, |this, cx| {
                if this.diff_text_selecting {
                    this.end_diff_text_selection();
                    cx.notify();
                }
            }),
            Self::Hosted(view) => view.update(cx, |this, cx| {
                if this.selecting {
                    this.selecting = false;
                    cx.notify();
                }
            }),
        }
    }
}
