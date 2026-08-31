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
/// Footer type size. A step under the card's `text_xs` body, so attribution
/// reads as secondary to the message it describes.
const COMMIT_MESSAGE_HOVER_FOOTER_FONT_PX: f32 = 10.5;

#[derive(Clone, Debug)]
pub(in crate::view) struct CommitMessageHoverState {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) commit_id: CommitId,
    /// Subject line, known from the log page without any git read, so the card
    /// can open immediately and fill its body in when the message arrives.
    pub(in crate::view) summary: SharedString,
    /// Author and date, shown in the card's footer. The date column can be
    /// hidden, and the author column narrow, so this is often the only place
    /// they are legible.
    pub(in crate::view) author: SharedString,
    pub(in crate::view) when: SharedString,
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

/// The shaped form of the card's body, kept so a re-render that changes nothing
/// does not copy and re-scan the message again.
///
/// The theme is deliberately not part of the key: it feeds the highlight colours,
/// but `set_theme` already knows when it changes and drops this outright, which
/// is cheaper than carrying a >2KB [`AppTheme`] here and field-comparing it on
/// every frame.
struct CommitMessageHoverBody {
    commit_id: CommitId,
    /// Whether the full `%B` body had arrived when this was built. The card
    /// opens on the subject line alone and swaps to the body once loaded, and
    /// that is the only content transition it has.
    loaded: bool,
    message: SharedString,
    highlights: crate::view::commit_message_text::TextHighlights,
}

pub(in crate::view) struct CommitMessageHoverHost {
    theme: AppTheme,
    /// Held directly rather than reached through the root view: the root opens
    /// this host inside its own update, so calling back into it from here would
    /// be a re-entrant lease. Used only to dispatch; state is read from
    /// `ui_model` below.
    store: Arc<AppStore>,
    /// The poller-synced snapshot every other view renders from. Reading the
    /// store directly here would take its lock on the UI thread once per frame
    /// -- see the note in `poller.rs` -- and could show a snapshot newer than
    /// the one the rest of the frame was laid out from.
    ui_model: Entity<AppUiModel>,
    state: Option<CommitMessageHoverState>,
    body: Option<CommitMessageHoverBody>,
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
    pub(in crate::view) fn new(
        theme: AppTheme,
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
    ) -> Self {
        Self {
            theme,
            store,
            ui_model,
            state: None,
            body: None,
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
        // The cached body carries theme-coloured highlights, so it is rebuilt
        // from here rather than by comparing themes on every render.
        self.body = None;
        cx.notify();
    }

    pub(in crate::view) fn show(
        &mut self,
        next: CommitMessageHoverState,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        // Already open on this commit: only the pointer moved within the row.
        // Nothing to do -- a close armed by an earlier excursion is cancelled by
        // `on_mouse_moved`, which is the one place that arms it.
        if self
            .state
            .as_ref()
            .is_some_and(|state| same_hover_target(state, &next))
        {
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
        if self
            .pending_show
            .as_ref()
            .is_some_and(|pending| !pending.source_bounds.contains(&position))
        {
            self.cancel_pending_show();
        }
        let Some(state) = self.state.as_ref() else {
            return;
        };
        if state.source_bounds.contains(&position) {
            // Back on the row the card belongs to, so a close armed by an
            // earlier excursion is off again. This lives here rather than in
            // `show`: the timer is armed from this handler, so it has to be
            // disarmed from it too, or a pointer that dips out of the cell and
            // returns inside the grace period loses the card it is pointing at.
            self.close_delay = None;
            return;
        }
        self.schedule_close(cx);
    }

    /// Abandons a card that was waiting out its open delay. Bumping the
    /// generation is what makes the timer still running for it a no-op.
    fn cancel_pending_show(&mut self) {
        self.pending_show = None;
        self.armed_at = None;
        self.show_seq = self.show_seq.wrapping_add(1);
    }

    /// Takes the open card down. Returns whether there was one, so callers that
    /// only notify on a real change can say so.
    fn clear_card(&mut self) -> bool {
        self.body = None;
        self.state.take().is_some()
    }

    /// Called when the pointer leaves the row that opened the card.
    pub(in crate::view) fn schedule_close(&mut self, cx: &mut gpui::Context<Self>) {
        self.cancel_pending_show();
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
        if self.clear_card() {
            cx.notify();
        }
    }

    /// Hides the card outright, e.g. on a click or when a menu opens over it.
    pub(in crate::view) fn dismiss(&mut self, cx: &mut gpui::Context<Self>) {
        self.cancel_pending_show();
        self.open_delay = None;
        self.close_delay = None;
        self.close_seq = self.close_seq.wrapping_add(1);
        if self.clear_card() {
            cx.notify();
        }
    }

    #[cfg(test)]
    fn is_open(&self) -> bool {
        self.state.is_some()
    }

    #[cfg(test)]
    fn close_is_armed(&self) -> bool {
        self.close_delay.is_some()
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
            .ui_model
            .read(cx)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == state.repo_id)
            .and_then(|repo| repo.hover_commit_message.as_ref())
            .and_then(|(id, message)| match message {
                Loadable::Ready(message) if *id == state.commit_id => Some(Arc::clone(message)),
                _ => None,
            });

        // Copying the body and scanning it for links is proportional to the
        // message length and the card re-renders on every root render, so it is
        // done once per (commit, loaded) rather than once per frame.
        if self.body.as_ref().is_some_and(|body| {
            body.commit_id != state.commit_id || body.loaded != loaded.is_some()
        }) {
            self.body = None;
        }
        // `get_or_insert_with` rather than a store-then-unwrap: the body is
        // borrowed straight out of the cache, so there is no `Option` to
        // unwrap and no panic path through `render`.
        let body = self.body.get_or_insert_with(|| {
            let message = hover_card_message(&state, loaded.as_ref());
            let highlights = commit_message_highlights(message.as_ref(), theme);
            CommitMessageHoverBody {
                commit_id: state.commit_id.clone(),
                loaded: loaded.is_some(),
                message,
                highlights,
            }
        });
        // Handed to `compute_runs` as an iterator, so the cached highlights are
        // read in place instead of being copied into a fresh `Vec` per frame.
        let message_text = gpui::StyledText::new(body.message.clone())
            .with_default_highlights(&window.text_style(), body.highlights.iter().cloned());

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

        let mut footer = div()
            .flex()
            .items_center()
            .gap_1p5()
            .pt_1()
            .text_color(theme.colors.foreground.secondary);
        if !state.author.is_empty() {
            footer = footer
                .child(components::author_avatar(
                    theme,
                    ui_scale,
                    state.author.as_ref(),
                ))
                .child(
                    div()
                        .min_w(px(0.0))
                        .line_clamp(1)
                        .child(state.author.clone()),
                );
        }
        if !state.when.is_empty() {
            footer = footer.child(
                div()
                    .flex_none()
                    .ml_auto()
                    .whitespace_nowrap()
                    .child(state.when.clone()),
            );
        }
        let has_footer = !state.author.is_empty() || !state.when.is_empty();

        gpui::anchored()
            .position(layout.anchor)
            .anchor(layout.anchor_corner)
            .child(
                div()
                    .debug_selector(|| "commit_message_hover".to_string())
                    .w(layout.panel_w)
                    .max_h(layout.max_panel_h)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .px_2()
                    .py_1p5()
                    .bg(theme.colors.surface.raised)
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .rounded(px(theme.radii.popover))
                    .shadow(crate::theme::shadow_popover(theme))
                    .text_xs()
                    .text_color(theme.colors.foreground.primary)
                    .child(message_text)
                    .when(has_footer, |card| {
                        card.child(
                            // Separated and a size down, so it reads as
                            // attribution rather than part of the message.
                            div()
                                .mt_1()
                                .pt_1()
                                .border_t_1()
                                .border_color(theme.colors.stroke.subtle)
                                .text_size(ui_scale.px(COMMIT_MESSAGE_HOVER_FOOTER_FONT_PX))
                                .child(footer),
                        )
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::test_support::NoopBackend;

    fn state(summary: &str) -> CommitMessageHoverState {
        state_for("abc", summary)
    }

    fn state_for(commit: &str, summary: &str) -> CommitMessageHoverState {
        CommitMessageHoverState {
            repo_id: RepoId(1),
            commit_id: CommitId(commit.into()),
            summary: summary.to_string().into(),
            author: "Ada Lovelace".into(),
            when: "2 days ago".into(),
            source_bounds: Bounds::new(point(px(0.0), px(100.0)), size(px(400.0), px(28.0))),
            source_pointer_x: px(120.0),
        }
    }

    /// Inside `state()`'s `source_bounds`, and outside it.
    fn on_row() -> Point<Pixels> {
        point(px(120.0), px(110.0))
    }
    fn off_row() -> Point<Pixels> {
        point(px(120.0), px(400.0))
    }

    fn host(cx: &mut gpui::TestAppContext) -> Entity<CommitMessageHoverHost> {
        let (store, _events) = AppStore::new(Arc::new(NoopBackend));
        let store = Arc::new(store);
        cx.update(|cx| {
            let ui_model = cx.new(|_cx| AppUiModel::new(store.snapshot()));
            cx.new(|_cx| CommitMessageHoverHost::new(AppTheme::gitcomet_dark(), store, ui_model))
        })
    }

    /// The open/close delays only exist under the live runtime; the
    /// deterministic one runs both immediately, which is what makes the
    /// timer-free assertions below deterministic.
    fn live<T>(f: impl FnOnce() -> T) -> T {
        crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), f)
    }

    #[gpui::test]
    fn a_pointer_returning_to_the_row_cancels_the_pending_close(cx: &mut gpui::TestAppContext) {
        let host = host(cx);

        host.update(cx, |host, cx| {
            live(|| {
                host.show(state("Fix the thing"), on_row(), cx);
                // The live runtime waits out the open delay, so drive the card
                // open directly rather than sleeping for it.
                host.open_pending(host.show_seq, cx);
                assert!(host.is_open(), "the card should be open to begin with");

                // A pointer that leaves the message cell arms the grace timer...
                host.on_mouse_moved(off_row(), cx);
                assert!(host.close_is_armed(), "leaving the row arms the close");
                assert!(host.is_open(), "but the card stays up during the grace");

                // ...and coming back inside it disarms the timer again. Without
                // this the card closes 120ms later with the pointer sitting on
                // the very message it describes.
                host.on_mouse_moved(on_row(), cx);
                assert!(
                    !host.close_is_armed(),
                    "returning to the row must cancel the pending close"
                );
                assert!(host.is_open());
            })
        });
    }

    #[gpui::test]
    fn leaving_the_row_closes_the_card_once_the_grace_elapses(cx: &mut gpui::TestAppContext) {
        let host = host(cx);

        // Deterministic runtime: no timers, so the close lands inline.
        host.update(cx, |host, cx| {
            host.show(state("Fix the thing"), on_row(), cx);
            assert!(host.is_open());
            host.on_mouse_moved(off_row(), cx);
            assert!(!host.is_open(), "the card closes once the pointer is away");
        });
    }

    #[gpui::test]
    fn hovering_a_second_commit_replaces_the_card(cx: &mut gpui::TestAppContext) {
        let host = host(cx);

        host.update(cx, |host, cx| {
            host.show(state_for("aaa", "first"), on_row(), cx);
            host.show(state_for("bbb", "second"), on_row(), cx);
            assert_eq!(
                host.state.as_ref().map(|state| state.commit_id.clone()),
                Some(CommitId("bbb".into()))
            );
        });
    }

    #[gpui::test]
    fn dismiss_drops_the_open_card_and_anything_pending(cx: &mut gpui::TestAppContext) {
        let host = host(cx);

        host.update(cx, |host, cx| {
            live(|| {
                // One card open, another waiting out its delay behind it.
                host.show(state_for("aaa", "first"), on_row(), cx);
                host.open_pending(host.show_seq, cx);
                host.show(state_for("bbb", "second"), on_row(), cx);
                assert!(host.is_open());
                assert!(host.pending_show.is_some());

                host.dismiss(cx);
                assert!(!host.is_open(), "dismiss takes the card down");
                assert!(host.pending_show.is_none(), "and abandons the pending one");
                assert!(
                    host.body.is_none(),
                    "the cached body must not outlive the card it belongs to"
                );

                // The abandoned open-delay must not resurrect the card when it
                // fires: the generation it captured is stale.
                let stale_seq = host.show_seq.wrapping_sub(1);
                host.open_pending(stale_seq, cx);
                assert!(!host.is_open(), "a stale open timer must not reopen it");
            })
        });
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
