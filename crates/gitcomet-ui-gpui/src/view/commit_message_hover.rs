//! Hover card showing a commit's full message.
//!
//! Modelled on [`super::history_refs_hover`]: a host entity holding the open
//! card, its own open/close delays with generation counters for cancellation,
//! and a pure layout function that flips above/below and clamps to the window.
//!
//! gpui's built-in tooltip is not usable here on two counts: its show delay is a
//! hardcoded 500ms with no per-element override, and the history rows are canvas
//! painted, so there is no stateful element to hang `.tooltip()` on.

use super::*;
use crate::view::commit_message_text::commit_message_highlights;

/// Longer than the other hover affordances in the app (the refs hover opens at
/// 160ms, the SHA menu at 300ms): this card covers the row under the pointer, so
/// it has to be clearly deliberate rather than something you trip over while
/// reading down the list.
const COMMIT_MESSAGE_HOVER_OPEN_DELAY_MS: u64 = 700;
const COMMIT_MESSAGE_HOVER_CLOSE_GRACE_MS: u64 = 120;
/// How far the pointer may drift while the open delay runs before the delay is
/// re-armed. Small enough that deliberate movement always restarts it, large
/// enough that a hand resting on the mouse does not.
const COMMIT_MESSAGE_HOVER_STEADY_RADIUS_PX: f32 = 2.0;
const COMMIT_MESSAGE_HOVER_WIDTH_PX: f32 = 420.0;
const COMMIT_MESSAGE_HOVER_MAX_HEIGHT_PX: f32 = 320.0;
const COMMIT_MESSAGE_HOVER_POINTER_INSET_PX: f32 = 16.0;
/// Cap on the message actually shaped. A pathological commit body would
/// otherwise shape tens of thousands of glyphs on a hover.
const COMMIT_MESSAGE_HOVER_MAX_CHARS: usize = 4_000;

#[derive(Clone, Debug)]
pub(in crate::view) struct CommitMessageHoverState {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) commit_id: CommitId,
    /// Subject line, known from the log page without any git read, so the card
    /// can open immediately and fill its body in when the message arrives.
    pub(in crate::view) summary: SharedString,
    pub(in crate::view) source_bounds: Bounds<Pixels>,
    pub(in crate::view) source_pointer_x: Pixels,
}

fn same_hover_target(lhs: &CommitMessageHoverState, rhs: &CommitMessageHoverState) -> bool {
    lhs.repo_id == rhs.repo_id && lhs.commit_id == rhs.commit_id
}

/// Whether the pointer has stayed close enough to where the open delay was armed
/// to count as still. Compared on each axis rather than by true distance: the
/// cost of a hypotenuse per mouse-move event buys nothing at this radius.
fn within_steady_radius(armed_at: Point<Pixels>, pointer: Point<Pixels>) -> bool {
    let radius = px(COMMIT_MESSAGE_HOVER_STEADY_RADIUS_PX);
    (pointer.x - armed_at.x).abs() <= radius && (pointer.y - armed_at.y).abs() <= radius
}

#[derive(Clone, Copy, Debug)]
struct CommitMessageHoverLayout {
    anchor: Point<Pixels>,
    anchor_corner: Anchor,
    panel_w: Pixels,
    max_panel_h: Pixels,
}

pub(in crate::view) struct CommitMessageHoverHost {
    theme: AppTheme,
    /// Held directly rather than reached through the root view: the root opens
    /// this host inside its own update, so calling back into it from here would
    /// be a re-entrant lease.
    store: Arc<AppStore>,
    state: Option<CommitMessageHoverState>,
    pending_show: Option<CommitMessageHoverState>,
    /// Pointer position the running open-delay was armed from.
    armed_at: Option<Point<Pixels>>,
    show_seq: u64,
    close_seq: u64,
    /// Held rather than detached so a new hover cancels the pending one; see
    /// the same note on `TooltipHost::pending_delay`.
    open_delay: Option<gpui::Task<()>>,
    close_delay: Option<gpui::Task<()>>,
}

impl CommitMessageHoverHost {
    pub(in crate::view) fn new(theme: AppTheme, store: Arc<AppStore>) -> Self {
        Self {
            theme,
            store,
            state: None,
            pending_show: None,
            armed_at: None,
            show_seq: 0,
            close_seq: 0,
            open_delay: None,
            close_delay: None,
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in crate::view) fn show(
        &mut self,
        next: CommitMessageHoverState,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Already open on this commit: only the pointer moved within the row.
        if self
            .state
            .as_ref()
            .is_some_and(|state| same_hover_target(state, &next))
        {
            self.close_delay = None;
            return;
        }

        // The delay measures how long the pointer has been *still*, not how long
        // it has been somewhere in the row: any real movement re-arms it, so
        // reading down the list never trips the card. `armed_at` is the position
        // the running timer was started from, so slow drift accumulates against
        // it and eventually re-arms rather than sliding under the threshold one
        // pixel at a time.
        if self
            .pending_show
            .as_ref()
            .is_some_and(|state| same_hover_target(state, &next))
            && self
                .armed_at
                .is_some_and(|armed_at| within_steady_radius(armed_at, pointer))
        {
            return;
        }

        self.pending_show = Some(next);
        self.armed_at = Some(pointer);
        self.show_seq = self.show_seq.wrapping_add(1);
        let seq = self.show_seq;

        if !crate::ui_runtime::current().uses_tooltip_delay() {
            self.open_pending(seq, cx);
            return;
        }

        self.open_delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(COMMIT_MESSAGE_HOVER_OPEN_DELAY_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.open_pending(seq, cx));
        }));
    }

    fn open_pending(&mut self, seq: u64, cx: &mut gpui::Context<Self>) {
        if self.show_seq != seq {
            return;
        }
        let Some(next) = self.pending_show.take() else {
            return;
        };
        self.armed_at = None;

        // Ask for the body now rather than at hover time, so a pointer merely
        // sweeping across rows issues no git reads at all.
        self.store.dispatch(Msg::LoadHoverCommitMessage {
            repo_id: next.repo_id,
            commit_id: next.commit_id.clone(),
        });

        self.state = Some(next);
        self.close_delay = None;
        cx.notify();
    }

    /// Driven once per pointer move from the window root, rather than from each
    /// history row: a row-level handler would have to call into the root on
    /// every move for every visible row, which is both wasteful and re-entrant
    /// when the root is already mid-update.
    pub(in crate::view) fn on_mouse_moved(
        &mut self,
        position: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(pending) = &self.pending_show
            && !pending.source_bounds.contains(&position)
        {
            self.pending_show = None;
            self.armed_at = None;
            self.show_seq = self.show_seq.wrapping_add(1);
        }
        if self
            .state
            .as_ref()
            .is_some_and(|state| !state.source_bounds.contains(&position))
        {
            self.schedule_close(cx);
        }
    }

    /// Called when the pointer leaves the row that opened the card.
    pub(in crate::view) fn schedule_close(&mut self, cx: &mut gpui::Context<Self>) {
        self.pending_show = None;
        self.armed_at = None;
        self.show_seq = self.show_seq.wrapping_add(1);
        if self.state.is_none() {
            return;
        }

        self.close_seq = self.close_seq.wrapping_add(1);
        let seq = self.close_seq;

        if !crate::ui_runtime::current().uses_tooltip_delay() {
            self.close_now(seq, cx);
            return;
        }

        self.close_delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(COMMIT_MESSAGE_HOVER_CLOSE_GRACE_MS))
                .await;
            let _ = this.update(cx, |this, cx| this.close_now(seq, cx));
        }));
    }

    fn close_now(&mut self, seq: u64, cx: &mut gpui::Context<Self>) {
        if self.close_seq != seq {
            return;
        }
        if self.state.take().is_some() {
            cx.notify();
        }
    }

    /// Hides the card outright, e.g. on a click or when a menu opens over it.
    pub(in crate::view) fn dismiss(&mut self, cx: &mut gpui::Context<Self>) {
        self.pending_show = None;
        self.armed_at = None;
        self.open_delay = None;
        self.close_delay = None;
        self.show_seq = self.show_seq.wrapping_add(1);
        self.close_seq = self.close_seq.wrapping_add(1);
        if self.state.take().is_some() {
            cx.notify();
        }
    }
}

/// Places the card against the hovered row, preferring below and flipping above
/// when there is more room there, and keeps it inside the window.
#[allow(clippy::too_many_arguments)]
fn commit_message_hover_layout(
    source: Bounds<Pixels>,
    source_pointer_x: Pixels,
    window_size: Size<Pixels>,
    preferred_panel_w: Pixels,
    preferred_max_panel_h: Pixels,
    pointer_inset: Pixels,
    gap: Pixels,
    margin: Pixels,
) -> CommitMessageHoverLayout {
    let horizontal_margin = margin.min(window_size.width * 0.5);
    let min_x = horizontal_margin;
    let max_right = (window_size.width - horizontal_margin).max(min_x);
    let available_w = (max_right - min_x).max(px(0.0));
    let panel_w = preferred_panel_w.min(available_w);
    let max_x = (max_right - panel_w).max(min_x);
    let pointer_inset = pointer_inset.min(panel_w * 0.5).max(px(0.0));

    let mut preferred_x = source.left();
    if source_pointer_x < preferred_x + pointer_inset {
        preferred_x = source_pointer_x - pointer_inset;
    } else if source_pointer_x > preferred_x + panel_w - pointer_inset {
        preferred_x = source_pointer_x - panel_w + pointer_inset;
    }
    let anchor_x = preferred_x.max(min_x).min(max_x);

    let vertical_margin = margin.min(window_size.height * 0.5);
    let min_y = vertical_margin;
    let max_y = (window_size.height - vertical_margin).max(min_y);
    let below_anchor_y = (source.bottom() + gap).max(min_y).min(max_y);
    let above_anchor_y = (source.top() - gap).max(min_y).min(max_y);
    let below_h = preferred_max_panel_h.min((max_y - below_anchor_y).max(px(0.0)));
    let above_h = preferred_max_panel_h.min((above_anchor_y - min_y).max(px(0.0)));

    if above_h > below_h {
        CommitMessageHoverLayout {
            anchor: point(anchor_x, above_anchor_y),
            anchor_corner: Anchor::BottomLeft,
            panel_w,
            max_panel_h: above_h,
        }
    } else {
        CommitMessageHoverLayout {
            anchor: point(anchor_x, below_anchor_y),
            anchor_corner: Anchor::TopLeft,
            panel_w,
            max_panel_h: below_h,
        }
    }
}

/// Message the card shows: the loaded body when it has arrived, otherwise the
/// subject already known from the log page.
fn hover_card_message(state: &CommitMessageHoverState, loaded: Option<&Arc<str>>) -> SharedString {
    let text = loaded.map_or_else(
        || state.summary.to_string(),
        |message| message.trim_end().to_string(),
    );
    if text.chars().count() <= COMMIT_MESSAGE_HOVER_MAX_CHARS {
        return text.into();
    }
    let cut = text
        .char_indices()
        .nth(COMMIT_MESSAGE_HOVER_MAX_CHARS)
        .map_or(text.len(), |(ix, _)| ix);
    format!("{}…", &text[..cut]).into()
}

impl Render for CommitMessageHoverHost {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.clone() else {
            return div().into_any_element();
        };

        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let loaded = self
            .store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == state.repo_id)
            .and_then(|repo| repo.hover_commit_message.clone())
            .and_then(|(id, message)| match message {
                Loadable::Ready(message) if id == state.commit_id => Some(message),
                _ => None,
            });

        let message = hover_card_message(&state, loaded.as_ref());
        let highlights = commit_message_highlights(message.as_ref(), theme);

        let layout = commit_message_hover_layout(
            state.source_bounds,
            state.source_pointer_x,
            window.viewport_size(),
            ui_scale.px(COMMIT_MESSAGE_HOVER_WIDTH_PX),
            ui_scale.px(COMMIT_MESSAGE_HOVER_MAX_HEIGHT_PX),
            ui_scale.px(COMMIT_MESSAGE_HOVER_POINTER_INSET_PX),
            ui_scale.px(4.0),
            ui_scale.px(8.0),
        );

        gpui::anchored()
            .position(layout.anchor)
            .anchor(layout.anchor_corner)
            .child(
                div()
                    .debug_selector(|| "commit_message_hover".to_string())
                    .w(layout.panel_w)
                    .max_h(layout.max_panel_h)
                    .overflow_hidden()
                    .px_2()
                    .py_1p5()
                    .bg(theme.colors.tooltip_bg)
                    .border_1()
                    .border_color(theme.colors.border)
                    .rounded(px(theme.radii.popover))
                    .shadow(crate::theme::shadow_popover(theme))
                    .text_xs()
                    .text_color(theme.colors.tooltip_text)
                    .child(
                        gpui::StyledText::new(message)
                            .with_default_highlights(&window.text_style(), highlights),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(summary: &str) -> CommitMessageHoverState {
        CommitMessageHoverState {
            repo_id: RepoId(1),
            commit_id: CommitId("abc".into()),
            summary: summary.to_string().into(),
            source_bounds: Bounds::new(point(px(0.0), px(100.0)), size(px(400.0), px(28.0))),
            source_pointer_x: px(120.0),
        }
    }

    #[test]
    fn shows_the_summary_until_the_body_arrives() {
        let state = state("Fix the thing");
        assert_eq!(hover_card_message(&state, None).as_ref(), "Fix the thing");

        let body: Arc<str> = Arc::from("Fix the thing\n\nWhy it broke.\n");
        assert_eq!(
            hover_card_message(&state, Some(&body)).as_ref(),
            "Fix the thing\n\nWhy it broke."
        );
    }

    #[test]
    fn caps_a_pathological_body_on_a_character_boundary() {
        let body: Arc<str> = Arc::from("é".repeat(COMMIT_MESSAGE_HOVER_MAX_CHARS + 500).as_str());
        let shown = hover_card_message(&state("s"), Some(&body));

        assert!(shown.ends_with('…'));
        assert_eq!(shown.chars().count(), COMMIT_MESSAGE_HOVER_MAX_CHARS + 1);
    }

    #[test]
    fn a_still_pointer_keeps_the_running_delay() {
        let armed = point(px(100.0), px(50.0));
        // Sub-pixel jitter from the pointer device must not restart the wait.
        assert!(within_steady_radius(armed, armed));
        assert!(within_steady_radius(armed, point(px(101.0), px(51.0))));
        assert!(within_steady_radius(
            armed,
            point(
                px(100.0 + COMMIT_MESSAGE_HOVER_STEADY_RADIUS_PX),
                px(50.0 - COMMIT_MESSAGE_HOVER_STEADY_RADIUS_PX)
            )
        ));
    }

    #[test]
    fn deliberate_movement_re_arms_the_delay() {
        let armed = point(px(100.0), px(50.0));
        // Reading down the list moves mostly on y; scanning a message, on x.
        assert!(!within_steady_radius(armed, point(px(100.0), px(60.0))));
        assert!(!within_steady_radius(armed, point(px(140.0), px(50.0))));
        // Drift accumulates against the arm position rather than the previous
        // event, so a slow crawl still re-arms instead of sliding under the
        // threshold one pixel at a time.
        assert!(!within_steady_radius(
            armed,
            point(
                px(100.0 + COMMIT_MESSAGE_HOVER_STEADY_RADIUS_PX + 0.5),
                px(50.0)
            )
        ));
    }

    #[test]
    fn card_flips_above_the_row_when_there_is_more_room_there() {
        let window = size(px(1000.0), px(600.0));
        // Row near the bottom: below has no room, so the card goes above.
        let low = Bounds::new(point(px(0.0), px(560.0)), size(px(400.0), px(28.0)));
        let layout = commit_message_hover_layout(
            low,
            px(120.0),
            window,
            px(420.0),
            px(320.0),
            px(16.0),
            px(4.0),
            px(8.0),
        );
        assert_eq!(layout.anchor_corner, Anchor::BottomLeft);

        let high = Bounds::new(point(px(0.0), px(40.0)), size(px(400.0), px(28.0)));
        let layout = commit_message_hover_layout(
            high,
            px(120.0),
            window,
            px(420.0),
            px(320.0),
            px(16.0),
            px(4.0),
            px(8.0),
        );
        assert_eq!(layout.anchor_corner, Anchor::TopLeft);
    }

    #[test]
    fn card_stays_inside_the_window_next_to_a_row_at_the_right_edge() {
        let window = size(px(500.0), px(600.0));
        let row = Bounds::new(point(px(420.0), px(100.0)), size(px(400.0), px(28.0)));
        let layout = commit_message_hover_layout(
            row,
            px(480.0),
            window,
            px(420.0),
            px(320.0),
            px(16.0),
            px(4.0),
            px(8.0),
        );

        assert!(layout.anchor.x >= px(8.0));
        assert!(layout.anchor.x + layout.panel_w <= px(492.0));
    }
}
