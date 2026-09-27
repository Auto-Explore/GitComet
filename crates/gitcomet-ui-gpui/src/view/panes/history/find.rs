//! The history list's find bar (Cmd-F): type to match commits by summary,
//! author, or SHA prefix, then step the selection between the matches. The
//! bar itself is the shared [`components::QuickSearchBar`].

use super::*;
use components::QuickSearchStatus;
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::text_search::TextSearchOptions;
use gitcomet_state::history_find::HistoryFindMsg;
use std::rc::Rc;

/// Quiet time after a keystroke before the query is searched and the first
/// match selected. Row dimming follows every keystroke immediately; this only
/// spares the whole-history scan and the selection's detail load.
pub(in crate::view) const HISTORY_FIND_SETTLE_MS: u64 = 120;

/// Gap between the column header and the floating bar.
const FIND_BAR_TOP_GAP_PX: f32 = 6.0;
const FIND_BAR_RIGHT_GAP_PX: f32 = 8.0;

pub(in crate::view) struct HistoryFind {
    pub(in crate::view) input: Entity<components::TextInput>,
    pub(in crate::view) open: bool,
    /// Match case, whole word and regex, for this session.
    options: TextSearchOptions,
    /// The query as typed, `None` when blank. It may be an invalid regex,
    /// which is shown as such but never searched for.
    query: Option<HistoryFindQuery>,
    /// Set by each query edit: select the first match as soon as one is known.
    /// An indexed search reports matches as its scan reaches them.
    jump_to_first: bool,
    /// The search last asked of the store, so a render does not re-send it
    /// while the reply is in flight.
    requested: Option<(HistoryFindQuery, usize)>,
    matches: Option<(FindMatchesKey, Rc<FindMatches>)>,
    /// The status last shown for an answered query. Held while a new query's
    /// scan has nothing to report yet, so the count does not blink on every
    /// keystroke.
    status: Option<QuickSearchStatus>,
    /// Running while the user is still typing; see [`HISTORY_FIND_SETTLE_MS`].
    settling: Option<gpui::Task<()>>,
    _input_subscription: gpui::Subscription,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FindMatchesKey {
    Indexed {
        /// The store's search; its match chunks only grow.
        generation: u64,
        /// The displayed graph: its projection decides which raw rows show.
        graph: usize,
    },
    Paged {
        query: HistoryFindQuery,
        page: usize,
        visible: usize,
    },
}

/// Matching rows, as ascending visible indices of the displayed list.
#[derive(Clone, Debug, Default)]
pub(in crate::view) struct FindMatches {
    pub(in crate::view) visible: Vec<usize>,
    /// Store match chunks already mapped into `visible`.
    chunks_seen: usize,
    /// `false` while an indexed scan is still running.
    pub(in crate::view) complete: bool,
    /// The store has not answered this query yet.
    pub(in crate::view) pending: bool,
    /// The scan stopped on an error; `visible` holds what it found first.
    pub(in crate::view) failed: bool,
}

/// Whether a commit row fades while the find bar has a searchable query (see
/// [`HistoryView::history_find_query`]). Decided from the row's own text,
/// which every drawn row has loaded, so a keystroke restyles the rows in the
/// same frame instead of waiting on the scan. The selected row never fades,
/// so the commit being looked at stays readable.
pub(in crate::view) fn history_find_row_dimmed(
    query: Option<&HistoryFindQuery>,
    commit: &Commit,
    selected: bool,
) -> bool {
    !selected && query.is_some_and(|query| !query.matches(commit))
}

impl HistoryView {
    pub(in crate::view) fn history_find_is_open(&self) -> bool {
        self.find.as_ref().is_some_and(|find| find.open)
    }

    /// The query rows are matched against while the bar is open: none for a
    /// blank query or an invalid regex.
    pub(in crate::view) fn history_find_query(&self) -> Option<&HistoryFindQuery> {
        self.find
            .as_ref()
            .filter(|find| find.open)
            .and_then(|find| find.query.as_ref())
            .filter(|query| query.regex_error().is_none())
    }

    /// Open the bar, or refocus it with the query selected when it is already
    /// open, the same as Cmd-F in a diff.
    pub(in crate::view) fn open_history_find(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let theme = self.theme;
        let find = self.find.get_or_insert_with(|| {
            let input = cx.new(|cx| {
                let mut input = components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "find commit".into(),
                        leading_icon: Some("icons/zoom.svg"),
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                input.set_theme(theme, cx);
                input
            });
            let subscription = cx.observe_in(&input, window, |this, input, window, cx| {
                this.handle_history_find_input(input, window, cx);
            });
            HistoryFind {
                input,
                open: false,
                options: TextSearchOptions::default(),
                query: None,
                jump_to_first: false,
                requested: None,
                matches: None,
                status: None,
                settling: None,
                _input_subscription: subscription,
            }
        });
        let reopened = !find.open;
        find.open = true;
        let input = find.input.clone();
        input.update(cx, |input, cx| {
            input.clear_transient_key_presses();
            input.select_all_text(window, cx);
        });
        let focus = input.read_with(cx, |input, _| input.focus_handle());
        window.focus(&focus, cx);
        if reopened {
            // The last query comes back with the bar and is searched again,
            // but reopening leaves the selection where it is.
            let text = input.read_with(cx, |input, _| input.text().to_owned());
            self.set_history_find_query(&text, false);
        }
        cx.notify();
    }

    pub(in crate::view) fn close_history_find(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(find) = self.find.as_mut().filter(|find| find.open) else {
            return;
        };
        find.open = false;
        find.jump_to_first = false;
        find.matches = None;
        find.settling = None;
        if find.requested.take().is_some()
            && let Some(repo_id) = self.active_repo_id()
        {
            self.store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
                repo_id,
                query: None,
                index: None,
            }));
        }
        window.focus(&self.history_panel_focus_handle, cx);
        cx.notify();
    }

    fn handle_history_find_input(
        &mut self,
        input: Entity<components::TextInput>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let (escape_pressed, enter_pressed) = input.update(cx, |input, _| {
            let keys = (input.take_escape_pressed(), input.take_enter_pressed());
            input.clear_transient_key_presses();
            keys
        });
        if !self.history_find_is_open() {
            return;
        }
        if escape_pressed {
            self.close_history_find(window, cx);
            return;
        }
        let text = input.read_with(cx, |input, _| input.text().to_owned());
        // TextInput also notifies for caret blinks and selection moves.
        let query_changed = self.find.as_ref().is_some_and(|find| match &find.query {
            Some(query) => query.text() != text,
            None => !text.trim().is_empty(),
        });
        if query_changed {
            self.set_history_find_query(&text, true);
            self.settle_history_find(cx);
            cx.notify();
        }
        if enter_pressed {
            if self
                .find
                .as_mut()
                .and_then(|find| find.settling.take())
                .is_some()
            {
                // Typed and pressed Enter at once: search now and let the
                // pending first-match jump land.
                cx.notify();
            } else {
                self.history_find_step(true, cx);
            }
        }
    }

    fn set_history_find_query(&mut self, text: &str, jump_to_first: bool) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        find.query = HistoryFindQuery::new(text, find.options);
        find.jump_to_first = jump_to_first
            && find
                .query
                .as_ref()
                .is_some_and(|query| query.regex_error().is_none());
        find.matches = None;
    }

    /// A toggle was clicked: search again with `options` at once, selecting
    /// the first match as a query edit would, and keep typing in the input.
    fn set_history_find_options(
        &mut self,
        options: TextSearchOptions,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(find) = self.find.as_mut().filter(|find| find.open) else {
            return;
        };
        if find.options != options {
            find.options = options;
            find.settling = None;
            let text = find.input.read(cx).text().to_owned();
            self.set_history_find_query(&text, true);
        }
        self.focus_history_find_input(window, cx);
        cx.notify();
    }

    fn settle_history_find(&mut self, cx: &mut gpui::Context<Self>) {
        let delay = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(HISTORY_FIND_SETTLE_MS));
        let settling = cx.spawn(async move |view, cx| {
            delay.await;
            let _ = view.update(cx, |this, cx| {
                if let Some(find) = this.find.as_mut() {
                    find.settling = None;
                }
                cx.notify();
            });
        });
        if let Some(find) = self.find.as_mut() {
            find.settling = Some(settling);
        }
    }

    /// Keep the store's scan in step with the query and the displayed index,
    /// then take the first-match jump a query edit asked for. Runs each render
    /// once typing has settled.
    pub(super) fn sync_history_find(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(find) = self
            .find
            .as_ref()
            .filter(|find| find.open && find.settling.is_none())
        else {
            return;
        };
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        let index = self
            .indexed
            .presentation
            .as_ref()
            .filter(|shown| shown.key.repo_id == repo_id)
            .map(|shown| shown.graph.projection.index.clone());
        // An invalid regex is never searched; like a blank query it only
        // stops a search already asked for.
        let query = find
            .query
            .as_ref()
            .filter(|query| query.regex_error().is_none());
        let wanted = query
            .cloned()
            .zip(index.clone())
            .map(|(query, index)| (query, Arc::as_ptr(&index) as usize));
        // The store drops an unfinished scan when repository loads are
        // cancelled (a tab switch, a finished action), so the request is
        // repeated whenever its answer is missing. The reducer ignores a
        // repeat of a search it already has.
        let unanswered = query.zip(index.as_ref()).is_some_and(|(query, index)| {
            self.active_repo()
                .is_some_and(|repo| !repo.history_state.find.is_for(query, index))
        });
        if wanted != find.requested || unanswered {
            self.store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
                repo_id,
                query: wanted.as_ref().map(|(query, _)| query.clone()),
                index,
            }));
            if let Some(find) = self.find.as_mut() {
                find.requested = wanted;
            }
        }

        if self.find.as_ref().is_some_and(|find| find.jump_to_first)
            && let Some(matches) = self.history_find_matches()
            && (!matches.visible.is_empty() || matches.complete)
        {
            if let Some(find) = self.find.as_mut() {
                find.jump_to_first = false;
            }
            if let Some(&first) = matches.visible.first() {
                self.select_history_find_match(first, cx);
            }
        }
    }

    /// Matches for the current query over the displayed list, cached until
    /// the query, the list, or the store's results change.
    pub(in crate::view) fn history_find_matches(&mut self) -> Option<Rc<FindMatches>> {
        let query = self.history_find_query()?.clone();
        let repo_id = self.active_repo_id()?;
        // Held separately so the cache below can be updated while reading it.
        let state = Arc::clone(&self.state);
        let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;

        if let Some(shown) = self
            .indexed
            .presentation
            .clone()
            .filter(|shown| shown.key.repo_id == repo_id)
        {
            let projection = &shown.graph.projection;
            let results = &repo.history_state.find;
            if !results.is_for(&query, &projection.index) {
                return Some(Rc::new(FindMatches {
                    pending: true,
                    ..FindMatches::default()
                }));
            }
            let key = FindMatchesKey::Indexed {
                generation: results.generation(),
                graph: Arc::as_ptr(&shown.graph) as usize,
            };
            let find = self.find.as_mut()?;
            if find
                .matches
                .as_ref()
                .is_none_or(|(cached, _)| *cached != key)
            {
                find.matches = Some((key, Rc::default()));
            }
            let (_, cached) = find.matches.as_mut()?;
            let complete = results.done || results.error.is_some();
            let failed = results.error.is_some();
            // A long scan reports many chunks; map only the ones not seen yet.
            if cached.chunks_seen < results.matches.len()
                || cached.complete != complete
                || cached.failed != failed
            {
                let matches = Rc::make_mut(cached);
                for chunk in &results.matches[matches.chunks_seen..] {
                    // Stash helper rows are hidden from the list; so are their matches.
                    matches.visible.extend(
                        chunk
                            .iter()
                            .filter_map(|&raw| projection.visible_position(raw as usize)),
                    );
                }
                matches.chunks_seen = results.matches.len();
                matches.complete = complete;
                matches.failed = failed;
            }
            return Some(Rc::clone(cached));
        }

        // Without an index the list is the loaded log page; match it directly.
        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id)?;
        let key = FindMatchesKey::Paged {
            query: query.clone(),
            page: Arc::as_ptr(&cache.page) as usize,
            visible: cache.base.visible_indices.len(),
        };
        if let Some((cached, matches)) = self.find.as_ref().and_then(|find| find.matches.as_ref())
            && *cached == key
        {
            return Some(Rc::clone(matches));
        }
        let matches = Rc::new(FindMatches {
            visible: cache
                .base
                .visible_indices
                .iter()
                .enumerate()
                .filter(|(_, commit_ix)| {
                    cache
                        .page
                        .commits
                        .get(*commit_ix)
                        .is_some_and(|commit| query.matches(commit))
                })
                .map(|(visible_ix, _)| visible_ix)
                .collect(),
            complete: true,
            ..FindMatches::default()
        });
        if let Some(find) = self.find.as_mut() {
            find.matches = Some((key, Rc::clone(&matches)));
        }
        Some(matches)
    }

    /// The selected commit's visible index, if a commit is selected and shown.
    fn history_find_selected_visible_ix(&self) -> Option<usize> {
        let repo = self.active_repo()?;
        let selected = repo.history_state.selected_commit.as_ref()?;
        if let Some(shown) = self
            .indexed
            .presentation
            .as_ref()
            .filter(|shown| shown.key.repo_id == repo.id)
        {
            return shown.graph.projection.position(selected.as_ref());
        }
        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo.id)?;
        cache.base.visible_indices.iter().position(|commit_ix| {
            cache
                .page
                .commits
                .get(commit_ix)
                .is_some_and(|commit| &commit.id == selected)
        })
    }

    /// Move the selection to the next (or previous) match after the selected
    /// commit, wrapping around at either end.
    pub(in crate::view) fn history_find_step(
        &mut self,
        forward: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(matches) = self.history_find_matches() else {
            return false;
        };
        if matches.visible.is_empty() {
            return false;
        }
        if let Some(find) = self.find.as_mut() {
            // Stepping takes over from the pending first-match jump.
            find.jump_to_first = false;
        }
        let selected = self.history_find_selected_visible_ix();
        let target = match (selected, forward) {
            (Some(selected), true) => {
                let next = matches.visible.partition_point(|&row| row <= selected);
                matches.visible.get(next).or(matches.visible.first())
            }
            (Some(selected), false) => {
                let prev = matches.visible.partition_point(|&row| row < selected);
                prev.checked_sub(1)
                    .and_then(|ix| matches.visible.get(ix))
                    .or(matches.visible.last())
            }
            (None, true) => matches.visible.first(),
            (None, false) => matches.visible.last(),
        };
        let Some(&target) = target else {
            return false;
        };
        self.select_history_find_match(target, cx);
        true
    }

    /// Select a match the way clicking its row would and bring it into view.
    fn select_history_find_match(&mut self, visible_ix: usize, cx: &mut gpui::Context<Self>) {
        let Some(repo_id) = self.active_repo_id() else {
            return;
        };
        if self.indexed.presentation.is_some() {
            if self.select_indexed_commit_row(repo_id, visible_ix, true, cx) {
                self.dismiss_history_refs_hover(cx);
            }
            return;
        }
        let plan = self.ensure_history_list_plan();
        if self.select_paged_commit_row(repo_id, &plan, visible_ix, cx) {
            cx.notify();
        }
    }

    /// What the bar's match label reports.
    pub(in crate::view) fn history_find_status(&mut self) -> QuickSearchStatus {
        let Some(query) = self
            .find
            .as_ref()
            .filter(|find| find.open)
            .and_then(|find| find.query.as_ref())
        else {
            return QuickSearchStatus::Empty;
        };
        if query.regex_error().is_some() {
            return QuickSearchStatus::InvalidRegex;
        }
        // No matches without a loaded list or an answer from the store. The
        // scan reports only once it has matches or is done, so an empty,
        // unfinished result has not answered yet either.
        let answered = self.history_find_matches().filter(|matches| {
            !matches.pending && (!matches.visible.is_empty() || matches.complete)
        });
        let Some(matches) = answered else {
            return self
                .find
                .as_ref()
                .and_then(|find| find.status)
                .unwrap_or(QuickSearchStatus::Searching);
        };
        let status = if matches.failed {
            QuickSearchStatus::Failed
        } else if matches.visible.is_empty() {
            QuickSearchStatus::NoMatches
        } else {
            QuickSearchStatus::Position {
                current: self
                    .history_find_selected_visible_ix()
                    .and_then(|selected| matches.visible.binary_search(&selected).ok()),
                total: matches.visible.len(),
                complete: matches.complete,
            }
        };
        if let Some(find) = self.find.as_mut() {
            find.status = Some(status);
        }
        status
    }

    pub(super) fn render_history_find(
        &mut self,
        header_height: Pixels,
        right_gutter: Pixels,
        cx: &mut gpui::Context<Self>,
    ) -> Option<AnyElement> {
        let find = self.find.as_ref().filter(|find| find.open)?;
        let (input, options) = (find.input.clone(), find.options);
        let theme = self.theme;
        let ui_scale =
            ui_scale::UiScale::from_percent(self.ui_scale_percent).with_appearance(theme.metrics);
        let status = self.history_find_status();
        let can_step = self
            .history_find_matches()
            .is_some_and(|matches| !matches.visible.is_empty());
        let step = |forward: bool| {
            move |this: &mut Self, window: &mut Window, cx: &mut gpui::Context<Self>| {
                this.history_find_step(forward, cx);
                this.focus_history_find_input(window, cx);
            }
        };

        let panel = components::QuickSearchBar::<Self>::new("history_find", status)
            .input(input)
            .options(options, |this, next, window, cx| {
                this.set_history_find_options(next, window, cx);
            })
            .navigation(can_step, step(false), step(true))
            .on_close(|this, window, cx| this.close_history_find(window, cx))
            .render(theme, ui_scale, cx)
            // The bar is shared and keyless; Shift-Enter in its input steps
            // back through history matches only here.
            .key_context("HistoryFind")
            .on_action(cx.listener(|this, _: &HistoryFindPrevious, _window, cx| {
                this.history_find_step(false, cx);
            }))
            .occlude()
            .with_animation(
                "history_find_mount",
                Animation::new(Duration::from_millis(120)).with_easing(gpui::quadratic),
                |panel, delta| {
                    let slide_y = (1.0 - delta) * -8.0;
                    panel.opacity(delta).relative().top(px(slide_y))
                },
            );

        Some(
            div()
                .id("history_find")
                .debug_selector(|| "history_find".to_string())
                .absolute()
                .top(header_height + ui_scale.px(FIND_BAR_TOP_GAP_PX))
                .right(right_gutter + ui_scale.px(FIND_BAR_RIGHT_GAP_PX))
                .child(panel)
                .into_any_element(),
        )
    }

    fn focus_history_find_input(&self, window: &mut Window, cx: &mut gpui::Context<Self>) {
        if let Some(find) = self.find.as_ref().filter(|find| find.open) {
            let focus = find.input.read(cx).focus_handle();
            window.focus(&focus, cx);
        }
    }
}
