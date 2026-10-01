//! The history list's find bar (Cmd-F): type to match commits by summary,
//! author, or SHA prefix, then step the selection between the matches. The
//! bar itself is the shared [`components::QuickSearchBar`].

use super::indexed_graph::IndexedGraph;
use super::*;
use components::QuickSearchStatus;
use gitcomet_core::history_find::HistoryFindQuery;
use gitcomet_core::history_index::HistoryIndex;
use gitcomet_core::text_search::TextSearchOptions;
use gitcomet_state::history_find::HistoryFindMsg;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Weak;

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
    requested: Option<(FindRequest, u64)>,
    /// Steps that cannot yet be answered by a streaming scan.
    steps: VecDeque<bool>,
    matches: Option<(FindMatchesKey, Rc<FindMatches>)>,
    /// The answer from the list this one replaced (a fetch, a commit), moved
    /// onto its rows and shown until the same query is answered over it.
    carried: Option<CarriedMatches>,
    /// Running while the user is still typing; see [`HISTORY_FIND_SETTLE_MS`].
    settling: Option<gpui::Task<()>>,
    _input_subscription: gpui::Subscription,
}

/// A weak identity reserves the allocation's address without retaining its data.
/// Bare addresses can be reused after a page or graph has been replaced.
struct FindIdentity<T>(Weak<T>);

impl<T> From<&Arc<T>> for FindIdentity<T> {
    fn from(value: &Arc<T>) -> Self {
        Self(Arc::downgrade(value))
    }
}
impl<T> PartialEq for FindIdentity<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl<T> Eq for FindIdentity<T> {}

#[derive(Eq, PartialEq)]
struct FindRequest {
    repo_id: RepoId,
    query: HistoryFindQuery,
    index: FindIdentity<HistoryIndex>,
    stashes_rev: u64,
}

#[derive(Eq, PartialEq)]
enum FindMatchesKey {
    Indexed {
        generation: u64,
        stashes_rev: u64,
        graph: FindIdentity<IndexedGraph>,
    },
    Paged {
        repo_id: RepoId,
        query: HistoryFindQuery,
        page: FindIdentity<LogPage>,
        stashes_rev: u64,
        visible: usize,
        complete: bool,
    },
}

struct CarriedMatches {
    query: HistoryFindQuery,
    stashes_rev: u64,
    graph: FindIdentity<IndexedGraph>,
    matches: Rc<FindMatches>,
}

/// Most matches moved onto a replacement list, one lookup each on the UI
/// thread; past this the bar says "Searching…" until the store answers.
const HISTORY_FIND_CARRY_LIMIT: usize = 50_000;

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

/// Most highlighted matches per cell, as in the sidebar searches.
const HISTORY_FIND_MAX_HIGHLIGHTS: usize = 16;

/// What the find query matched in a row's shown text, so the row can show
/// why it is a match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::view) struct HistoryFindHighlights {
    pub(in crate::view) summary: Vec<std::ops::Range<usize>>,
    pub(in crate::view) author: Vec<std::ops::Range<usize>>,
    /// Bytes of the short SHA to light up: all of it when the query matched
    /// the id as a prefix, since it names the commit rather than some text.
    pub(in crate::view) sha: usize,
}

/// How a drawn commit row shows the find query (see
/// [`HistoryView::history_find_query`]): whether it fades, and what to
/// highlight. Decided from the text the row shows (a stash shows its
/// message), which every drawn row has, so a keystroke restyles rows in the
/// same frame instead of waiting on the scan. The selected row never fades,
/// so the commit being looked at stays readable. Runs per visible row per
/// frame, so a miss is matched once and never searched for highlights.
pub(in crate::view) fn history_find_row_marks(
    query: Option<&HistoryFindQuery>,
    commit: &Commit,
    selected: bool,
    summary: &str,
    author: &str,
    short_sha: &str,
) -> (bool, Option<HistoryFindHighlights>) {
    match query {
        Some(query) if query.matches_fields(commit.id.as_ref(), summary, author) => (
            false,
            history_find_highlights(Some(query), commit, summary, author, short_sha),
        ),
        Some(_) => (!selected, None),
        None => (false, None),
    }
}

/// Highlights for a row showing `summary`, `author` and `short_sha`, which
/// can differ from the commit's own text (a stash shows its message). `None`
/// without a query or when nothing shown matches.
pub(in crate::view) fn history_find_highlights(
    query: Option<&HistoryFindQuery>,
    commit: &Commit,
    summary: &str,
    author: &str,
    short_sha: &str,
) -> Option<HistoryFindHighlights> {
    let query = query?;
    let mut highlights = HistoryFindHighlights {
        sha: query
            .sha_prefix_len(commit.id.as_ref())
            .map_or(0, |_| short_sha.len()),
        ..HistoryFindHighlights::default()
    };
    query.text_ranges_into(
        summary,
        &mut highlights.summary,
        HISTORY_FIND_MAX_HIGHLIGHTS,
    );
    query.text_ranges_into(author, &mut highlights.author, HISTORY_FIND_MAX_HIGHLIGHTS);
    (!highlights.summary.is_empty() || !highlights.author.is_empty() || highlights.sha > 0)
        .then_some(highlights)
}

/// What the find query matched in the commit details pane: the same fields a
/// row is matched on, so the pane shows why the selected commit matched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::view) struct CommitDetailsFindHighlights {
    /// Ranges in the message's summary line, which starts the message.
    pub(in crate::view) summary: Vec<std::ops::Range<usize>>,
    /// Ranges in the author name as the pane shows it.
    pub(in crate::view) author: Vec<std::ops::Range<usize>>,
    /// The query matched the id as a prefix; the whole SHA lights up.
    pub(in crate::view) sha: bool,
    /// Length of the abbreviated SHA shown beside the full one when the query
    /// was an abbreviation: the list's short SHA, or longer if more was typed.
    pub(in crate::view) short_sha_len: Option<usize>,
}

/// Highlights for the details of commit `id`. `None` without a query or when
/// nothing shown matches.
pub(in crate::view) fn history_find_detail_highlights(
    query: Option<&HistoryFindQuery>,
    id: &str,
    message: &str,
    author: &str,
    row_summary: &str,
) -> Option<CommitDetailsFindHighlights> {
    let query = query?;
    let prefix = query.sha_prefix_len(id);
    let mut highlights = CommitDetailsFindHighlights {
        sha: prefix.is_some(),
        short_sha_len: prefix.filter(|&len| len < id.len()).map(|len| {
            len.max(crate::view::caches::HISTORY_SHORT_SHA_LEN)
                .min(id.len())
        }),
        ..CommitDetailsFindHighlights::default()
    };
    let summary = &message[..message.find('\n').unwrap_or(message.len())];
    // Ranges still address the raw details message. Only text that the row
    // actually searched may be highlighted; a hidden stash prefix cannot match.
    if let Some(prefix) = summary.strip_suffix(row_summary) {
        query.text_ranges_into(
            row_summary,
            &mut highlights.summary,
            HISTORY_FIND_MAX_HIGHLIGHTS,
        );
        for range in &mut highlights.summary {
            range.start += prefix.len();
            range.end += prefix.len();
        }
    }
    query.text_ranges_into(author, &mut highlights.author, HISTORY_FIND_MAX_HIGHLIGHTS);
    (!highlights.summary.is_empty() || !highlights.author.is_empty() || highlights.sha)
        .then_some(highlights)
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
                carried: None,
                steps: VecDeque::new(),
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
        find.carried = None;
        find.settling = None;
        find.steps.clear();
        // Unlike pausing the search, closing frees every repo's commit text.
        find.requested = None;
        self.store.dispatch(Msg::HistoryFind(HistoryFindMsg::Close));
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

    pub(super) fn set_history_find_query(&mut self, text: &str, jump_to_first: bool) {
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
        find.carried = None;
        find.steps.clear();
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
    pub(super) fn sync_history_find(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(find) = self
            .find
            .as_ref()
            .filter(|find| find.open && find.settling.is_none())
        else {
            return false;
        };
        let Some(repo) = self.active_repo() else {
            return false;
        };
        let repo_id = repo.id;
        let results = &repo.history_state.find;
        let generation = results.generation();
        let index = self
            .indexed
            .presentation
            .as_ref()
            .filter(|shown| shown.key.repo_id == repo_id)
            .map(|shown| shown.graph.projection.index.clone());
        let query = find
            .query
            .as_ref()
            .filter(|query| query.regex_error().is_none());
        let wanted = query.zip(index.as_ref()).map(|(query, index)| FindRequest {
            repo_id,
            query: query.clone(),
            index: index.into(),
            stashes_rev: repo.stashes_rev,
        });
        let answered = query
            .zip(index.as_ref())
            .is_some_and(|(query, index)| results.is_for(query, index, repo.stashes_rev));
        let changed = find.requested.as_ref().map(|(request, _)| request) != wanted.as_ref();
        // A generation change distinguishes a cancelled search from the same
        // snapshot waiting for its request to reach the store.
        let interrupted = !answered
            && find
                .requested
                .as_ref()
                .is_some_and(|(_, sent_generation)| *sent_generation != generation);
        if changed || interrupted {
            if let Some(wanted) = wanted {
                self.store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
                    repo_id,
                    query: Some(wanted.query.clone()),
                    index,
                }));
                self.find.as_mut().unwrap().requested = Some((wanted, generation));
            } else {
                self.stop_history_find_request();
            }
        } else if answered
            && let Some((_, acknowledged)) =
                self.find.as_mut().and_then(|find| find.requested.as_mut())
        {
            *acknowledged = generation;
        }

        let mut moved = false;
        if self.find.as_ref().is_some_and(|find| find.jump_to_first)
            && let Some(matches) = self.history_find_matches()
            && (!matches.visible.is_empty() || (matches.complete && !matches.failed))
        {
            if let Some(&first) = matches.visible.first() {
                moved = self.select_history_find_match(first, cx);
                if moved {
                    self.find.as_mut().unwrap().jump_to_first = false;
                }
            } else {
                self.find.as_mut().unwrap().jump_to_first = false;
            }
        }
        self.drive_history_find_steps(cx) || moved
    }

    fn stop_history_find_request(&mut self) {
        if let Some((request, _)) = self.find.as_mut().and_then(|find| find.requested.take()) {
            self.store.dispatch(Msg::HistoryFind(HistoryFindMsg::Find {
                repo_id: request.repo_id,
                query: None,
                index: None,
            }));
        }
    }

    pub(in crate::view) fn cancel_history_find_navigation(&mut self) {
        if let Some(find) = self.find.as_mut() {
            find.jump_to_first = false;
            find.steps.clear();
        }
    }

    pub(super) fn history_find_repo_changed(&mut self) {
        self.stop_history_find_request();
        self.cancel_history_find_navigation();
        if let Some(find) = self.find.as_mut() {
            find.matches = None;
            find.carried = None;
            find.settling = None;
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
            let find = self.find.as_mut()?;
            let answered = results.is_for(&query, &projection.index, repo.stashes_rev)
                && (results.done || results.error.is_some());
            if answered {
                find.carried = None;
            } else if let Some(carried) = find.carried.as_ref().filter(|carried| {
                carried.query == query
                    && carried.stashes_rev == repo.stashes_rev
                    && carried.graph == FindIdentity::from(&shown.graph)
            }) {
                return Some(Rc::clone(&carried.matches));
            }
            if !results.is_for(&query, &projection.index, repo.stashes_rev) {
                return Some(Rc::new(FindMatches {
                    pending: true,
                    ..FindMatches::default()
                }));
            }
            let key = FindMatchesKey::Indexed {
                generation: results.generation(),
                stashes_rev: repo.stashes_rev,
                graph: (&shown.graph).into(),
            };
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
        // The page answers for the whole history only when nothing is behind
        // it, or when no index is coming to search the rest.
        let complete = cache.page.next_cursor.is_none()
            || repo.history_state.log_snapshot.is_none()
            || repo.history_state.indexed.error.is_some();
        let key = FindMatchesKey::Paged {
            repo_id,
            query: query.clone(),
            page: (&cache.page).into(),
            stashes_rev: cache.base.request.stashes_rev,
            visible: cache.base.visible_indices.len(),
            complete,
        };
        if let Some((cached, matches)) = self.find.as_ref().and_then(|find| find.matches.as_ref())
            && *cached == key
        {
            return Some(Rc::clone(matches));
        }
        // Matched on what each row shows, as the indexed scan does.
        let matches = Rc::new(FindMatches {
            visible: cache
                .base
                .visible_indices
                .iter()
                .zip(cache.base.row_vms.iter())
                .enumerate()
                .filter(|(_, (commit_ix, row))| {
                    cache.page.commits.get(*commit_ix).is_some_and(|commit| {
                        query.matches_fields(
                            commit.id.as_ref(),
                            row.summary.as_ref(),
                            row.author.as_ref(),
                        )
                    })
                })
                .map(|(visible_ix, _)| visible_ix)
                .collect(),
            complete,
            ..FindMatches::default()
        });
        if let Some(find) = self.find.as_mut() {
            find.matches = Some((key, Rc::clone(&matches)));
        }
        Some(matches)
    }

    /// The indexed list is about to show `next` instead of `old`. Move the
    /// bar's answer onto `next`'s rows by commit id, so a refresh does not
    /// blank it while the same query is searched again over the new index.
    pub(super) fn carry_history_find_matches(
        &mut self,
        old: &Arc<IndexedGraph>,
        next: &Arc<IndexedGraph>,
    ) {
        let Some(query) = self.history_find_query().cloned() else {
            return;
        };
        let stashes_rev = self.active_repo().map_or(0, |repo| repo.stashes_rev);
        let Some(find) = self.find.as_mut() else {
            return;
        };
        if Arc::ptr_eq(&old.projection.index, &next.projection.index) {
            // The store's answer is for this index already.
            return;
        }
        let old_identity = FindIdentity::from(old);
        let answer = find
            .carried
            .as_ref()
            .filter(|carried| carried.query == query && carried.stashes_rev == stashes_rev && carried.graph == old_identity)
            .map(|carried| Rc::clone(&carried.matches))
            .or_else(|| {
                find.matches.as_ref().and_then(|(key, matches)| {
                    matches!(key, FindMatchesKey::Indexed { graph, stashes_rev: revision, .. } if *graph == old_identity && *revision == stashes_rev)
                        .then(|| Rc::clone(matches))
                })
            })
            .filter(|matches| {
                !matches.pending
                    && !matches.failed
                    && (matches.complete || !matches.visible.is_empty())
                    && matches.visible.len() <= HISTORY_FIND_CARRY_LIMIT
            });
        find.carried = answer.map(|answer| {
            let (from, to) = (&old.projection, &next.projection);
            let mut visible: Vec<usize> = answer
                .visible
                .iter()
                .filter_map(|&row| {
                    let id = from.index.id_bytes(from.raw_position(row)?)?;
                    to.visible_position(to.index.position_bytes(id)?)
                })
                .collect();
            visible.sort_unstable();
            CarriedMatches {
                query,
                stashes_rev,
                graph: next.into(),
                // New commits have not been searched yet.
                matches: Rc::new(FindMatches {
                    visible,
                    ..FindMatches::default()
                }),
            }
        });
    }

    /// The selected commit's visible index, if a commit is selected and shown.
    fn history_find_selected_visible_ix(&self) -> Option<usize> {
        let repo = self.active_repo()?;
        let selected = match self
            .pending_history_selections
            .back()
            .map(|pending| &pending.selection)
        {
            Some(HistoryPrimarySelection::Commit(id)) => id,
            Some(_) => return None,
            None => repo.history_state.selected_commit.as_ref()?,
        };
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
        cache.base.visible_ix_by_commit.get(selected).copied()
    }

    /// Move the selection to the next (or previous) match after the selected
    /// commit, wrapping around at either end.
    pub(in crate::view) fn history_find_step(
        &mut self,
        forward: bool,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        if self.history_find_query().is_none() {
            return false;
        }
        if self
            .history_find_matches()
            .is_some_and(|matches| matches.failed && matches.visible.is_empty())
        {
            // Enter explicitly retries a failed read; renders never retry it
            // on their own. Partial results remain navigable after a failure.
            let find = self.find.as_mut().unwrap();
            find.requested = None;
            find.jump_to_first = true;
            find.steps.clear();
            cx.notify();
            return true;
        }
        let find = self.find.as_mut().unwrap();
        find.jump_to_first = false;
        find.steps.push_back(forward);
        self.drive_history_find_steps(cx);
        true
    }

    fn drive_history_find_steps(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let mut moved = false;
        while let Some(&forward) = self.find.as_ref().and_then(|find| find.steps.front()) {
            let Some(matches) = self.history_find_matches() else {
                break;
            };
            let selected = self.history_find_selected_visible_ix();
            let wrap = if !matches.complete {
                None
            } else if forward {
                matches.visible.first()
            } else {
                matches.visible.last()
            };
            let target = match (selected, forward) {
                (Some(selected), true) => {
                    let next = matches.visible.partition_point(|&row| row <= selected);
                    matches.visible.get(next).or(wrap)
                }
                (Some(selected), false) => {
                    let prev = matches.visible.partition_point(|&row| row < selected);
                    prev.checked_sub(1)
                        .and_then(|ix| matches.visible.get(ix))
                        .or(wrap)
                }
                (None, true) => matches.visible.first(),
                (None, false) => wrap,
            }
            .copied();
            let Some(target) = target else {
                if matches.complete {
                    self.find.as_mut().unwrap().steps.clear();
                }
                break;
            };
            if !self.select_history_find_match(target, cx) {
                break;
            }
            self.find.as_mut().unwrap().steps.pop_front();
            moved = true;
        }
        moved
    }

    /// Select a match the way clicking its row would and bring it into view.
    fn select_history_find_match(
        &mut self,
        visible_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo_id) = self.active_repo_id() else {
            return false;
        };
        if self
            .indexed
            .presentation
            .as_ref()
            .is_some_and(|shown| shown.key.repo_id == repo_id)
        {
            if self.select_indexed_commit_row(repo_id, visible_ix, true, cx) {
                self.dismiss_history_refs_hover(cx);
                return true;
            }
            return false;
        }
        let plan = self.ensure_history_list_plan();
        if self.select_paged_commit_row(repo_id, &plan, visible_ix, cx) {
            cx.notify();
            return true;
        }
        false
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
            return QuickSearchStatus::Searching;
        };
        if matches.failed {
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
        }
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
