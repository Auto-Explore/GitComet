//! The Reveal Commit dialog: type or paste a commit reference, see what it
//! names, and jump the history log to its row.
//!
//! Visually a sibling of the command palette — same scrim, surface, and search
//! row — but it resolves one reference rather than filtering a fixed list, so
//! the "list" below the input is at most a single row.

use crate::theme::AppTheme;
use crate::ui_scale;
use gitcomet_core::domain::{Commit, CommitId};
use gitcomet_state::model::{CommitLookup, Loadable, RepoId};
use gitcomet_state::msg::Msg;
use gitcomet_state::store::AppStore;
use gpui::prelude::*;
use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, FontWeight, MouseButton, MouseDownEvent,
    SharedString, WeakEntity, Window, div, px,
};
use std::sync::Arc;

use super::{GitCometView, components};

/// Shortest query worth asking git about.
///
/// Deliberately below git's four-character minimum abbreviation, because the
/// input takes any revision and not only shas: a two-character tag like `v1`
/// has to stay reachable. One character is the only length nobody means.
const MIN_LOOKUP_QUERY_LEN: usize = 2;

/// Shown while the query is too short to look up. Made-up values on purpose:
/// real ones from the repository would read as results.
const EXAMPLES: &[(&str, &str)] = &[
    ("a1b2c3d", "Commit SHA — 4 characters or more, or all 40"),
    ("main", "Branch"),
    ("origin/main", "Remote-tracking branch"),
    ("v1.2.0", "Tag"),
    ("HEAD~3", "Three commits before HEAD"),
];

const EXACT_MATCH_HEADLINE: &str = "Matches are exact.";
const EXACT_MATCH_DETAIL: &str = "This does not search commit messages — a short SHA must be unique, \
and branch and tag names must be typed in full.";

/// The failure line for `query`, in the user's terms.
///
/// An ambiguous prefix is not "no match" — it matches several commits — so it
/// gets its own wording. gix reports it as "Short id … is ambiguous".
fn no_match_message(query: &str, error: &str) -> (String, &'static str) {
    if error.contains("is ambiguous") {
        (
            format!("“{query}” matches more than one commit."),
            "Type more characters of the SHA to pick one.",
        )
    } else {
        (
            format!("No commit matches “{query}”."),
            "Matches are exact: a short SHA needs 4 characters or more, and branch and tag \
names must be typed in full.",
        )
    }
}

/// What the row under the input is showing.
#[derive(Debug, Eq, PartialEq)]
enum RevealRow<'a> {
    /// Too little typed to ask git anything.
    Hint,
    /// A lookup is in flight, or the answer on hand is for an older query.
    Resolving,
    Match(&'a Commit),
    NoMatch(&'a str),
}

/// The row for `query`, given whatever the store last resolved.
///
/// A lookup is only believed when it answers the query as it stands now:
/// replies land a keystroke or two behind, and showing the previous reference's
/// commit under the current text would invite revealing the wrong commit.
fn reveal_row<'a>(query: &str, lookup: Option<&'a CommitLookup>) -> RevealRow<'a> {
    if query.len() < MIN_LOOKUP_QUERY_LEN {
        return RevealRow::Hint;
    }
    let Some(lookup) = lookup else {
        return RevealRow::Resolving;
    };
    if lookup.reference.as_ref().map(|r| r.as_ref()) != Some(query) {
        return RevealRow::Resolving;
    }
    match &lookup.result {
        Loadable::Ready(commit) => RevealRow::Match(commit),
        Loadable::Error(message) => RevealRow::NoMatch(message.as_str()),
        Loadable::NotLoaded | Loadable::Loading => RevealRow::Resolving,
    }
}

pub(crate) struct RevealCommitView {
    pub(crate) query_input: Entity<components::TextInput>,
    restore_focus: Option<FocusHandle>,
    fallback_focus: Option<FocusHandle>,
    root_view: WeakEntity<GitCometView>,
    /// Held directly rather than reached through `root_view`: lookups are also
    /// issued from `sync_from_state`, which already runs inside the root's own
    /// update, and updating the root from there would be re-entrant.
    store: Arc<AppStore>,
    theme: AppTheme,
    open: bool,
    repo_id: Option<RepoId>,
    query: SharedString,
    lookup: Option<CommitLookup>,
    _input_subscription: gpui::Subscription,
}

impl RevealCommitView {
    pub(crate) fn new(
        theme: AppTheme,
        repo_id: Option<RepoId>,
        root_view: WeakEntity<GitCometView>,
        store: Arc<AppStore>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let query_input = cx.new(|cx| {
            let mut input = components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Paste a commit SHA, or type a branch, tag, or revision…".into(),
                    chromeless: true,
                    ..Default::default()
                },
                window,
                cx,
            );
            input.set_theme(theme, cx);
            input
        });
        let input_subscription = cx.observe_in(&query_input, window, |this, input, window, cx| {
            this.handle_input_notification(input, window, cx);
        });

        Self {
            query_input,
            restore_focus: None,
            fallback_focus: None,
            root_view,
            store,
            theme,
            open: false,
            repo_id,
            query: SharedString::default(),
            lookup: None,
            _input_subscription: input_subscription,
        }
    }

    pub(crate) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        self.query_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    /// Push the active repository and its latest lookup in from the store.
    ///
    /// Called on every state snapshot, so both halves have to be cheap and
    /// silent when nothing moved.
    pub(crate) fn sync_from_state(
        &mut self,
        repo_id: Option<RepoId>,
        lookup: Option<&CommitLookup>,
        cx: &mut gpui::Context<Self>,
    ) {
        let repo_changed = self.repo_id != repo_id;
        let lookup_changed = self.lookup.as_ref() != lookup;
        if !repo_changed && !lookup_changed {
            return;
        }
        self.repo_id = repo_id;
        if lookup_changed {
            self.lookup = lookup.cloned();
        }
        // A repository switch leaves the typed query unanswered: the new
        // repository has its own lookup slot, which nobody has asked about this
        // reference. Without re-asking, the row sits on "Resolving…" until the
        // user edits the query.
        if repo_changed && self.open {
            self.request_lookup();
        }
        if self.open {
            cx.notify();
        }
    }

    /// Only the tests need this: the root's own `reveal_commit_open` flag is
    /// what production reads, and the two are asserted to agree.
    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(
        &mut self,
        restore_focus: Option<FocusHandle>,
        fallback_focus: FocusHandle,
        repo_id: Option<RepoId>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.open = true;
        self.restore_focus = restore_focus;
        self.fallback_focus = Some(fallback_focus);
        self.repo_id = repo_id;
        self.query = SharedString::default();
        self.query_input.update(cx, |input, cx| {
            // Reset the input completely, keys included — the same shape
            // `reset_picker_search_input` uses when a picker opens.
            input.clear_transient_key_presses();
            input.set_text("", cx);
        });
        let focus = self
            .query_input
            .read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        let dialog_focus = self
            .query_input
            .read_with(cx, |input, _| input.focus_handle());
        let mut restore_focus = self.restore_focus.take();
        // The dialog's own input is never a place to restore focus to.
        if restore_focus.as_ref() == Some(&dialog_focus) {
            restore_focus = None;
        }
        let focus = restore_focus.or_else(|| self.fallback_focus.take());
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn handle_input_notification(
        &mut self,
        input: Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (escape_pressed, enter_pressed) = input.update(cx, |input, _| {
            let keys = (input.take_escape_pressed(), input.take_enter_pressed());
            // Nothing here navigates a list, but leaving arrow/tab/page flags
            // latched would let them fire into whatever reads the input next.
            input.clear_transient_key_presses();
            keys
        });

        if !self.open {
            return;
        }
        if escape_pressed {
            self.close_and_notify_root(None, window, cx);
            return;
        }

        let query_changed =
            input.read_with(cx, |input, _| input.text().trim() != self.query.as_ref());

        // TextInput also notifies for cursor blinking, selection movement, and
        // focus bookkeeping. Resolving a reference on those would put a git
        // lookup behind every blink tick.
        if !query_changed && !enter_pressed {
            return;
        }

        if query_changed {
            self.query = input.read_with(cx, |input, _| {
                SharedString::from(input.text().trim().to_owned())
            });
            self.request_lookup();
        }

        if enter_pressed {
            self.activate(window, cx);
            return;
        }

        cx.notify();
    }

    /// Ask the store what the current query names.
    ///
    /// Fires on every edit: the reducer's request counter drops replies a later
    /// keystroke has overtaken, so there is nothing here for a debounce to
    /// protect.
    fn request_lookup(&self) {
        let Some(repo_id) = self.repo_id else {
            return;
        };
        if self.query.len() < MIN_LOOKUP_QUERY_LEN {
            return;
        }
        let reference = CommitId(self.query.as_ref().into());
        self.store.dispatch(Msg::ResolveCommitLookup {
            repo_id,
            reference,
            purpose: gitcomet_state::model::CommitLookupPurpose::RevealDialog,
        });
    }

    /// Reveal whatever the row is offering.
    ///
    /// With a resolved commit the *full* id is handed over, so the history walk
    /// matches loaded rows outright instead of going through prefix comparison.
    /// While the lookup is still in flight the raw query goes instead —
    /// `Msg::RevealCommit` resolves it itself and reports a bad reference — so
    /// hitting Enter ahead of the answer is never swallowed.
    fn activate(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let target = match reveal_row(self.query.as_ref(), self.lookup.as_ref()) {
            RevealRow::Match(commit) => Some(commit.id.clone()),
            RevealRow::Resolving => Some(CommitId(self.query.as_ref().into())),
            RevealRow::Hint | RevealRow::NoMatch(_) => None,
        };
        let Some(target) = target else {
            return;
        };
        self.close_and_notify_root(Some(target), window, cx);
    }

    fn close_and_notify_root(
        &mut self,
        target: Option<CommitId>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        self.close(window, cx);
        let root_view = self.root_view.clone();
        let _ = root_view.update(cx, |root, root_cx| {
            root.reveal_commit_did_close(target, window, root_cx);
        });
    }

    fn render_row(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);

        // A headline plus a quieter explanation under it.
        let message = |headline: String, detail: Option<&'static str>| {
            div()
                .w_full()
                .flex()
                .flex_col()
                .gap(scaled_px(2.0))
                .px(scaled_px(14.0))
                .child(
                    div()
                        .text_size(theme.ui_text(13.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(headline),
                )
                .when_some(detail, |this, detail| {
                    this.child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(detail),
                    )
                })
                .into_any_element()
        };

        match reveal_row(self.query.as_ref(), self.lookup.as_ref()) {
            RevealRow::Hint => self.render_examples(cx),
            RevealRow::Resolving => message("Resolving…".to_string(), None),
            RevealRow::NoMatch(error) => {
                let (headline, detail) = no_match_message(self.query.as_ref(), error);
                message(headline, Some(detail))
            }
            RevealRow::Match(commit) => self.render_match(commit, cx),
        }
    }

    /// The empty state: what can be typed, and that it has to be exact.
    fn render_examples(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);

        let rows = EXAMPLES.iter().map(|(reference, meaning)| {
            div()
                .flex()
                .items_center()
                .h(scaled_px(22.0))
                .child(
                    div()
                        .w(scaled_px(120.0))
                        .flex_none()
                        .font_family(super::UI_MONOSPACE_FONT_FAMILY)
                        .text_size(theme.ui_text(13.0))
                        .text_color(theme.colors.foreground.primary)
                        .child(*reference),
                )
                .child(
                    div()
                        .text_size(theme.ui_text(13.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(*meaning),
                )
        });

        div()
            .debug_selector(|| "reveal_commit_examples".to_string())
            .w_full()
            .flex()
            .flex_col()
            .px(scaled_px(14.0))
            .child(
                div()
                    .pb(scaled_px(4.0))
                    .text_size(theme.ui_text(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.colors.foreground.secondary)
                    .child("Examples"),
            )
            .children(rows)
            .child(
                div()
                    .mt(scaled_px(10.0))
                    .pt(scaled_px(10.0))
                    .border_t_1()
                    .border_color(theme.colors.stroke.subtle)
                    .flex()
                    .flex_col()
                    .gap(scaled_px(2.0))
                    .child(
                        div()
                            .text_size(theme.ui_text(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.foreground.primary)
                            .child(EXACT_MATCH_HEADLINE),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(EXACT_MATCH_DETAIL),
                    ),
            )
            .into_any_element()
    }

    fn render_match(&self, commit: &Commit, cx: &mut gpui::Context<Self>) -> AnyElement {
        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);
        // The row already carries the selected wash, and `.hover()` *replaces*
        // the background rather than stacking on it — so the hover fill has to
        // be the two washes added up, or hovering would make the row lighter.
        let resting = theme.active_overlay();
        let hovered = crate::theme::with_alpha(
            resting,
            (resting.alpha + theme.hover_overlay().alpha).min(1.0),
        );

        let sha = commit.id.as_ref();
        let short: SharedString = sha.get(0..8).unwrap_or(sha).to_owned().into();
        let summary = SharedString::from(commit.summary.to_string());
        let author = SharedString::from(commit.author.to_string());
        // Signed seconds since the epoch; `duration_since` reports a pre-epoch
        // timestamp as an error carrying the distance backwards.
        let unix_secs = match commit.time.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_secs() as i64,
            Err(error) => -(error.duration().as_secs() as i64),
        };
        let date = super::date_time::format_relative_time(unix_secs, std::time::SystemTime::now());
        let target = commit.id.clone();

        let secondary = div()
            .flex()
            .items_center()
            .gap(scaled_px(8.0))
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(
                div()
                    .font_family(super::UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.accent.foreground)
                    .child(short),
            )
            .when(!author.is_empty(), |row| {
                row.child(div().child("•")).child(div().child(author))
            })
            .child(div().child("•"))
            .child(div().child(date));

        div()
            .relative()
            .w_full()
            .px(scaled_px(6.0))
            .child(
                div()
                    // Without an id gpui never repaints on mouse-move, so the
                    // hover fill below would be computed and dropped every frame.
                    .id("reveal_commit_match")
                    .debug_selector(|| "reveal_commit_match".to_string())
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(scaled_px(2.0))
                    .px(scaled_px(10.0))
                    .py(scaled_px(8.0))
                    .rounded(px(theme.radii.row))
                    .bg(resting)
                    .hover(move |style| style.bg(hovered))
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .overflow_hidden()
                            .text_size(theme.ui_text(14.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.colors.foreground.primary)
                            .child(summary),
                    )
                    .child(secondary)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                            this.close_and_notify_root(Some(target.clone()), window, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .bottom_0()
                    .w(scaled_px(3.0))
                    .rounded_tr(px(theme.radii.row))
                    .rounded_br(px(theme.radii.row))
                    .bg(theme.colors.accent.foreground),
            )
            .into_any_element()
    }
}

impl Render for RevealCommitView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let theme = self.theme;
        let ui_scale = ui_scale::UiScale::current(cx);
        let scaled_px = crate::ui_scale::scaler(ui_scale);
        let dialog_width = scaled_px(620.0);
        let top_offset = scaled_px(56.0);
        // Shorter than the palette's 48 px: the title above already gives the
        // input its breathing room.
        let input_height = scaled_px(40.0);

        let body = components::modal_surface(theme)
            .child(
                div()
                    .debug_selector(|| "reveal_commit_title".to_string())
                    .w_full()
                    .px(scaled_px(14.0))
                    .pt(scaled_px(12.0))
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child("Go to"),
            )
            .child(
                div()
                    .w_full()
                    .h(input_height)
                    .flex()
                    .items_center()
                    .px(scaled_px(14.0))
                    .border_b_1()
                    .border_color(theme.colors.stroke.subtle)
                    .child(self.query_input.clone()),
            )
            .child(div().w_full().py(scaled_px(8.0)).child(self.render_row(cx)));

        let scrim = components::modal_scrim(theme).on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.close_and_notify_root(None, window, cx);
            }),
        );

        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim)
            .child(
                div()
                    .absolute()
                    .top(top_offset)
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(div().w(dialog_width).max_w(dialog_width).child(body)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn commit(id: &str) -> Commit {
        Commit {
            id: CommitId(id.into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "the reland".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    fn lookup(reference: &str, result: Loadable<Commit>) -> CommitLookup {
        CommitLookup {
            request: 1,
            reference: Some(CommitId(reference.into())),
            result,
        }
    }

    #[test]
    fn a_query_too_short_to_be_a_reference_asks_git_nothing() {
        assert_eq!(reveal_row("", None), RevealRow::Hint);
        assert_eq!(reveal_row("d", None), RevealRow::Hint);
        // Two characters is already a plausible tag, so the lookup starts here.
        assert_eq!(reveal_row("v1", None), RevealRow::Resolving);
    }

    #[test]
    fn an_answer_for_an_earlier_query_never_stands_in_for_this_one() {
        let stale = lookup("deadbee", Loadable::Ready(commit("deadbeef")));
        assert_eq!(reveal_row("deadbeef", Some(&stale)), RevealRow::Resolving);
        assert_eq!(
            reveal_row("deadbee", Some(&stale)),
            RevealRow::Match(&commit("deadbeef"))
        );
    }

    /// An ambiguous prefix matches *several* commits; calling it "no match" would
    /// send the user hunting for a typo that is not there. The error text is
    /// gix's own (`revision/spec/parse/error.rs`) as it reaches the dialog.
    #[test]
    fn an_ambiguous_prefix_is_not_reported_as_no_match() {
        let gix_error = "gix rev-parse a1b2: Short id a1b2 is ambiguous. Candidates are:\n\
                         a1b2c3d commit 2026-01-01 - first\n\
                         a1b2f00 commit 2026-01-02 - second";
        let (headline, detail) = no_match_message("a1b2", gix_error);
        assert_eq!(headline, "“a1b2” matches more than one commit.");
        assert!(detail.contains("more characters"), "got {detail:?}");
    }

    #[test]
    fn an_unknown_reference_explains_that_matches_are_exact() {
        let (headline, detail) = no_match_message("mian", "gix rev-parse mian: not found");
        assert_eq!(headline, "No commit matches “mian”.");
        assert!(detail.starts_with("Matches are exact"), "got {detail:?}");
        assert!(detail.contains("4 characters"), "got {detail:?}");
    }

    #[test]
    fn a_resolved_reference_reports_the_commit_and_a_failure_reports_the_reason() {
        let found = lookup("deadbee", Loadable::Ready(commit("deadbeef")));
        assert!(matches!(
            reveal_row("deadbee", Some(&found)),
            RevealRow::Match(c) if c.id.as_ref() == "deadbeef"
        ));

        let missing = lookup("nosuchref", Loadable::Error("gix rev-parse".into()));
        assert_eq!(
            reveal_row("nosuchref", Some(&missing)),
            RevealRow::NoMatch("gix rev-parse")
        );

        let pending = lookup("deadbee", Loadable::Loading);
        assert_eq!(reveal_row("deadbee", Some(&pending)), RevealRow::Resolving);
    }
}
