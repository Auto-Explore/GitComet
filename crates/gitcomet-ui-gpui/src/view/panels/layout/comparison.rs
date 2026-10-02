//! Multi-selection and range comparison views: ordered commit cards and the
//! changed files between two points.

use super::*;
use crate::view::panes::{ComparisonCardCache, ComparisonOrderCache};
use crate::view::rows::CommitCard;
use rustc_hash::FxHashSet;

const MULTI_COMMIT_ROW_HEIGHT_PX: f32 = 44.0;

/// How much of the comparison body the compared-commit cards may fill before
/// they start scrolling instead of growing. A range comparison has two
/// endpoints, but a multi-selection comparison has one card per selected
/// commit, and an unbounded column of those would push the changed-file list —
/// the part the user actually came for — off the bottom of the pane. At half,
/// the two lists split the body evenly once the selection is large enough to
/// need it, whatever height the pane happens to have.
const COMPARISON_CARDS_MAX_BODY_FRACTION: f32 = 0.5;

/// Floor for the comparison's changed-file section — a label row, the filter
/// tabs, and a row or two of list. Keeps the capped card block above it from
/// claiming the whole pane when the pane is shorter than the card cap allows.
pub(super) const RANGE_FILES_SECTION_MIN_HEIGHT_PX: f32 = 70.0;

impl DetailsPaneView {
    /// Selected IDs in displayed history order, independent of the bounded
    /// metadata cache. Missing metadata must not shrink the selection's cards.
    pub(super) fn multi_selected_commit_ids_in_log_order(repo: &RepoState) -> Vec<CommitId> {
        let selection = &repo.history_state.multi_selection;
        let indexed = &repo.history_state.indexed;
        if let Some(index) = indexed.displayed_index.as_ref().or(indexed.index.as_ref()) {
            let mut selected = selection.commits.as_ref().clone();
            selected.sort_by_cached_key(|id| index.position(id.as_ref()).unwrap_or(usize::MAX));
            return selected;
        }
        let Loadable::Ready(page) = &repo.log else {
            return Vec::new();
        };
        // Hash the selection first: this runs per frame (twice, and once more
        // per visible row batch) over the whole loaded page, and
        // `CommitMultiSelection::contains` is a linear scan — so a large
        // selection against a large page would be quadratic on every repaint.
        let selected: FxHashSet<&CommitId> = selection.commits.iter().collect();
        page.commits
            .iter()
            .filter(|commit| selected.contains(&commit.id))
            .map(|commit| commit.id.clone())
            .collect()
    }

    /// Commits to preview as cards while a two-point comparison is active.
    /// Prefers a multi-selection *only* when it is genuinely multi, because that
    /// is the one case where the selection is what is being compared (its merged
    /// diff). A single leftover selection — every plain history click leaves one
    /// — describes an unrelated commit, so the mark + compare, branch/tag and
    /// working-tree flows derive their endpoints from the range itself, looking
    /// each SHA up in indexed ranges or the bootstrap page for its metadata.
    /// Ordered newest first (tip before base) to match the log. The working tree
    /// has no commit of its own, so a compare-against-working-tree range yields
    /// a single card.
    pub(super) fn range_comparison_commit_ids(repo: &RepoState) -> Vec<CommitId> {
        if repo.history_state.multi_selection.is_multi() {
            return Self::multi_selected_commit_ids_in_log_order(repo);
        }
        let Some(range) = repo.history_state.range_selection.as_ref() else {
            return Vec::new();
        };
        range
            .to
            .iter()
            .chain(std::iter::once(&range.from))
            .filter(|id| !gitcomet_core::domain::is_empty_tree_id(id.as_ref()))
            .cloned()
            .collect()
    }

    pub(super) fn comparison_commits<'a>(
        repo: &'a RepoState,
        ids: &[CommitId],
    ) -> Vec<Option<&'a Commit>> {
        let indexed = &repo.history_state.indexed;
        let page = match &repo.log {
            Loadable::Ready(page) => Some(page),
            _ => repo.history_state.retained_log_while_loading.as_ref(),
        };
        // Hash the fallback page once, including for legacy histories where it
        // may contain thousands of commits. Never scan it per selected ID.
        let bootstrap: FxHashMap<_, _> = page
            .into_iter()
            .flat_map(|page| &page.commits)
            .map(|commit| (&commit.id, commit))
            .collect();
        // Resolve by immutable ID against the cache's own index: during handoff
        // its row numbers can differ from those of the displayed presentation.
        ids.iter()
            .map(|id| {
                indexed
                    .range_index
                    .as_ref()
                    .and_then(|index| indexed.commit(&index.snapshot, index.position(id.as_ref())?))
                    .or_else(|| bootstrap.get(id).copied())
            })
            .collect()
    }

    fn comparison_order_key(repo: &RepoState) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut key = rustc_hash::FxHasher::default();
        repo.id.hash(&mut key);
        repo.log_rev.hash(&mut key);
        if !repo.history_state.multi_selection.is_multi()
            && let Some(range) = &repo.history_state.range_selection
        {
            range.from.hash(&mut key);
            range.to.hash(&mut key);
        }
        (Arc::as_ptr(&repo.history_state.multi_selection.commits) as usize).hash(&mut key);
        repo.history_state
            .indexed
            .displayed_index
            .as_ref()
            .or(repo.history_state.indexed.index.as_ref())
            .map(|index| Arc::as_ptr(index) as usize)
            .hash(&mut key);
        key.finish()
    }

    pub(super) fn ensure_comparison_order(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let key = Self::comparison_order_key(repo);
        if self
            .comparison_order
            .as_ref()
            .is_some_and(|cache| cache.key == key)
            || self.comparison_order_pending == Some(key)
        {
            return;
        }
        // Small endpoint comparisons are ready in their first frame. Large
        // selections are sorted on the executor and published by generation.
        if Self::comparison_count(repo) <= 256 {
            let ordered = Arc::new(Self::range_comparison_commit_ids(repo));
            self.comparison_order = Some(Self::comparison_order_cache(repo, key, ordered));
            self.comparison_order_pending = None;
            return;
        }
        let source = Self::comparison_order_cache(repo, key, Arc::new(Vec::new()));
        let repo = repo.clone();
        self.comparison_order_pending = Some(key);
        cx.spawn(async move |view, cx| {
            let ordered = cx
                .background_executor()
                .spawn(async move { Arc::new(Self::range_comparison_commit_ids(&repo)) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.comparison_order_pending != Some(key) {
                    return;
                }
                this.comparison_order_pending = None;
                if this
                    .active_repo()
                    .is_some_and(|repo| Self::comparison_order_key(repo) == key)
                {
                    this.comparison_order = Some(ComparisonOrderCache {
                        ids: ordered,
                        ..source
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn comparison_order_cache(
        repo: &RepoState,
        key: u64,
        ids: Arc<Vec<CommitId>>,
    ) -> ComparisonOrderCache {
        ComparisonOrderCache {
            key,
            ids,
            _selection: repo.history_state.multi_selection.commits.clone(),
            _index: repo
                .history_state
                .indexed
                .displayed_index
                .as_ref()
                .or(repo.history_state.indexed.index.as_ref())
                .cloned(),
        }
    }

    fn comparison_count(repo: &RepoState) -> usize {
        if repo.history_state.multi_selection.is_multi() {
            repo.history_state.multi_selection.commits.len()
        } else {
            Self::range_comparison_commit_ids(repo).len()
        }
    }

    #[cfg(test)]
    pub(super) fn range_comparison_commits_shared(
        &self,
        repo: &RepoState,
    ) -> std::rc::Rc<[CommitCard]> {
        let ids = Self::range_comparison_commit_ids(repo);
        self.comparison_cards(repo, &ids, 0)
    }

    /// Only the visible IDs and the blocks supplying their metadata invalidate cards.
    pub(super) fn comparison_cards(
        &self,
        repo: &RepoState,
        ids: &[CommitId],
        start: usize,
    ) -> std::rc::Rc<[CommitCard]> {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        repo.id.hash(&mut hasher);
        repo.log_rev.hash(&mut hasher);
        start.hash(&mut hasher);
        ids.hash(&mut hasher);
        let indexed = &repo.history_state.indexed;
        let mut blocks =
            smallvec::SmallVec::<[Arc<gitcomet_core::history_index::HistoryRange>; 4]>::new();
        for id in ids {
            let block = indexed
                .range_index
                .as_ref()
                .and_then(|index| index.position(id.as_ref()))
                .map(|raw| {
                    raw / gitcomet_core::history_index::HISTORY_BLOCK_SIZE
                        * gitcomet_core::history_index::HISTORY_BLOCK_SIZE
                });
            let block = block.and_then(|block| indexed.ranges.get(&block));
            block
                .map(|range| Arc::as_ptr(range) as usize)
                .hash(&mut hasher);
            if let Some(block) = block
                && !blocks.iter().any(|old| Arc::ptr_eq(old, block))
            {
                blocks.push(block.clone());
            }
        }
        let key = hasher.finish();
        if let Some(cache) = &*self.range_comparison_commits_cache.borrow()
            && cache.key == key
        {
            return cache.cards.clone();
        }
        let cards: std::rc::Rc<[CommitCard]> = ids
            .iter()
            .zip(Self::comparison_commits(repo, ids))
            .map(|(id, commit)| {
                gitcomet_core::history_perf::record(
                    gitcomet_core::history_perf::Work::ComparisonCard,
                );
                commit.map_or_else(
                    || CommitCard::unloaded(id),
                    |commit| CommitCard::new(commit.clone()),
                )
            })
            .collect();
        *self.range_comparison_commits_cache.borrow_mut() = Some(ComparisonCardCache {
            key,
            cards: cards.clone(),
            _blocks: blocks.into_vec(),
        });
        cards
    }

    /// One selected/compared-commit preview card: avatar, summary, an author +
    /// relative-time line, and the short SHA. Shared by the multi-selection and
    /// range-comparison lists so both read identically.
    fn commit_card_element(
        &self,
        ix: usize,
        card: &CommitCard,
        now: std::time::SystemTime,
        show_border: bool,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let scaled_px = crate::ui_scale::scaler(self.ui_scale_percent);

        let short_sha = card.short_sha.clone();
        let summary = card.summary.clone();
        let author = card.author.clone();
        let when: SharedString = card
            .unix_secs
            .map(|unix_secs| {
                format!(
                    "{} · {}",
                    author,
                    crate::view::date_time::format_relative_time(unix_secs, now)
                )
            })
            .unwrap_or_default()
            .into();

        div()
            .id(("commit_multi_row", ix))
            .debug_selector(move || format!("commit_multi_row_{ix}"))
            .h(scaled_px(MULTI_COMMIT_ROW_HEIGHT_PX))
            .w_full()
            .flex()
            .items_center()
            .gap(scaled_px(8.0))
            .px(scaled_px(8.0))
            // The last card sits directly above the files section's own top
            // separator, so it omits its bottom border to avoid a double line.
            .when(show_border, |row| {
                row.border_b_1().border_color(theme.colors.stroke.default)
            })
            .when(card.unix_secs.is_some(), |row| {
                row.child(components::author_avatar(theme, ui_scale, author.as_ref()))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(scaled_px(2.0))
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .line_clamp(1)
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .child(when),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.foreground.secondary)
                    .child(short_sha),
            )
            .into_any_element()
    }

    /// Rows for both the comparison view's endpoint cards and the plain
    /// multi-selection list. `range_comparison_commit_ids` already resolves to the
    /// multi-selection when that is what is being compared, so one renderer
    /// serves both and the two views cannot drift apart.
    pub(in crate::view) fn render_multi_commit_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        this.ensure_comparison_order(cx);
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let key = Self::comparison_order_key(repo);
        let Some(order) = this
            .comparison_order
            .as_ref()
            .filter(|cache| cache.key == key)
        else {
            return Vec::new();
        };
        let ordered = &order.ids;
        let start = range.start.min(ordered.len());
        let end = range.end.min(ordered.len());
        let cards = this.comparison_cards(repo, &ordered[start..end], start);
        let last_ix = ordered.len().saturating_sub(1);
        let now = std::time::SystemTime::now();
        range
            .filter_map(|ix| cards.get(ix - start).map(|card| (ix, card)))
            .map(|(ix, card)| this.commit_card_element(ix, card, now, ix != last_ix))
            .collect()
    }

    /// The scrolling column of commit preview cards, shared by the plain
    /// multi-selection view (where it fills the pane) and the comparison view
    /// (where it is capped and sits above the changed-file list).
    fn commit_cards_list(
        &mut self,
        repo_id: RepoId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        Self::vertical_scroll_frame(
            self.theme,
            ("commit_multi_container", repo_id.0),
            ("commit_multi_scrollbar", repo_id.0),
            &self.commit_multi_scroll,
            uniform_list(
                ("commit_multi_list", repo_id.0),
                count,
                cx.processor(Self::render_multi_commit_rows),
            ),
        )
    }

    pub(super) fn multi_commit_details_view(
        &mut self,
        repo_id: RepoId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();

        let header = components::content_header_bar(theme, ui_scale)
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(1)
                    .child(SharedString::from(format!("{count} commits selected"))),
            )
            .child(
                components::Button::new("commit_details_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        if let Some(repo_id) = this.active_repo_id() {
                            this.store.dispatch(Msg::ClearCommitSelection {
                                request_id: None,
                                repo_id,
                            });
                        }
                        cx.notify();
                    })
                    .gitcomet_tooltip(theme, "Close commit details".into()),
            );

        let body = self.commit_cards_list(repo_id, count, cx);

        div()
            .id("commit_details_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("commit_details_body_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .p_2()
                    .child(body),
            )
            .into_any_element()
    }

    /// The details-pane view shown while two points are being compared: the
    /// selected commit cards, a "viewing diff between" subheader, and the list
    /// of files that differ between them. The diff pane starts empty; clicking a
    /// file loads that file's range diff in the main pane.
    pub(super) fn range_comparison_view(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();

        /// What the files section has to say for itself, kept separate from the
        /// count so a failed load can't read as an empty comparison.
        enum RangeFilesState {
            Loading,
            Failed(String),
            Loaded(usize),
        }

        let (card_count, is_merged_selection, range, files_state) = {
            let Some(repo) = self.active_repo() else {
                return div().into_any_element();
            };
            let Some(range) = repo.history_state.range_selection.clone() else {
                return div().into_any_element();
            };
            let card_count = Self::comparison_count(repo);
            // Only a genuine multi-selection is a "merged diff of N commits";
            // every other flow compares two named points, however many of them
            // happen to resolve to a card.
            let is_merged_selection = repo.history_state.multi_selection.is_multi();
            let files_state = match &repo.history_state.range_files {
                Loadable::Ready(files) => RangeFilesState::Loaded(files.len()),
                Loadable::Error(e) => RangeFilesState::Failed(e.clone()),
                Loadable::Loading | Loadable::NotLoaded => RangeFilesState::Loading,
            };
            (card_count, is_merged_selection, range, files_state)
        };

        let header_title: SharedString = if is_merged_selection {
            format!("{card_count} commits selected").into()
        } else {
            "Comparison".into()
        };
        let subheader: SharedString = if is_merged_selection {
            format!("Viewing merged diff of {card_count} commits").into()
        } else {
            format!("Viewing diff: {} → {}", range.from_label, range.to_label).into()
        };

        let header = components::content_header_bar(theme, ui_scale)
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(1)
                    .child(header_title),
            )
            .child(
                components::Button::new("range_comparison_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        if let Some(repo_id) = this.active_repo_id() {
                            this.store.dispatch(Msg::ClearComparison { repo_id });
                        }
                        cx.notify();
                    })
                    .gitcomet_tooltip(theme, "Close comparison".into()),
            );

        // Compared-commit preview cards. A two-point comparison has one or two,
        // but a multi-selection has one per selected commit, so the section
        // grows with the selection only up to half the comparison body and
        // scrolls past that — an even split with the changed-file list below,
        // rather than crowding it out.
        //
        // The cap is relative to the body, so it tracks the pane at whatever
        // height the splitter leaves it. The requested height stays definite
        // though: the card list is a `uniform_list`, which paints nothing when
        // its viewport height is indefinite, so `max_h` does the capping rather
        // than the height itself being content-derived.
        let card_row_height = ui_scale.px(MULTI_COMMIT_ROW_HEIGHT_PX);
        let cards = (card_count > 0).then(|| {
            div()
                .debug_selector(|| "range_comparison_cards".to_string())
                .flex()
                .flex_col()
                .w_full()
                .h(card_row_height * card_count as f32)
                .max_h(relative(COMPARISON_CARDS_MAX_BODY_FRACTION))
                .min_h(card_row_height)
                .child(self.commit_cards_list(repo_id, card_count, cx))
        });

        // No count until there is one: claiming "0 changed" while the diff is
        // still running states a number that is usually about to be wrong. The
        // selector names which of the two the label is, so a test can tell them
        // apart without reading painted text.
        let (files_label, files_label_selector): (SharedString, &'static str) = match &files_state {
            RangeFilesState::Loading | RangeFilesState::Failed(_) => {
                ("Changed files".into(), "range_files_label_pending")
            }
            RangeFilesState::Loaded(count) => {
                (format!("{count} changed").into(), "range_files_label_count")
            }
        };
        let (range_row_count, range_counts) = self
            .active_repo()
            .and_then(|repo| {
                let Loadable::Ready(files) = &repo.history_state.range_files else {
                    return None;
                };
                let rev = repo.history_state.range_files_rev;
                let projection = self.cached_range_file_projection(repo_id, rev, files);
                let plan = self.cached_range_file_plan(repo_id, rev, files);
                Some((plan.row_len(), projection.counts))
            })
            .unwrap_or((0, Default::default()));
        let range_controls = self.file_list_controls(
            crate::view::rows::FileListId::RangeFiles,
            repo_id,
            "range_file",
            range_counts.all == 0,
            cx,
        );
        let range_filters_width = self
            .range_filter_bounds_ref
            .borrow()
            .as_ref()
            .map(|b| b.size.width)
            .unwrap_or(Pixels::MAX);
        let range_filters = self.commit_file_filter_tabs(
            crate::view::rows::FileListId::RangeFiles,
            "range_file",
            range_filters_width,
            range_counts,
            cx,
        );

        let files_body: AnyElement = match &files_state {
            RangeFilesState::Loading => div()
                .debug_selector(|| "range_files_loading".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("Loading")
                .into_any_element(),
            // An error must not render as "No files." — that is exactly what a
            // pair of identical commits looks like, so the user would read a
            // failed comparison as a successful, empty one.
            RangeFilesState::Failed(message) => div()
                .debug_selector(|| "range_files_error".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.status.danger.foreground)
                .child(SharedString::from(message.clone()))
                .into_any_element(),
            RangeFilesState::Loaded(0) => div()
                .debug_selector(|| "range_files_empty".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child("No files.")
                .into_any_element(),
            RangeFilesState::Loaded(_) => Self::vertical_scroll_frame_content(
                theme,
                ("range_files_container", repo_id.0),
                ("range_files_scrollbar", repo_id.0),
                &self.range_files_scroll,
                self.changed_file_list(
                    repo_id,
                    crate::view::rows::FileListId::RangeFiles,
                    range_row_count,
                    cx,
                ),
            )
            .into_any_element(),
        };

        div()
            .id("range_comparison_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("range_comparison_body")
                    .debug_selector(|| "range_comparison_body".to_string())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .p_2()
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .child(subheader),
                    )
                    .children(cards)
                    .child(
                        div()
                            .debug_selector(|| "range_comparison_files".to_string())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .h_full()
                            // A real floor, not `px(0.0)`: the cards above are
                            // sized from the window, so on a pane shorter than
                            // that this is what stops them from taking the whole
                            // pane and collapsing the list to nothing.
                            .min_h(ui_scale.px(RANGE_FILES_SECTION_MIN_HEIGHT_PX))
                            .border_t_1()
                            .border_color(theme.colors.stroke.subtle)
                            .pt_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .w_full()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .debug_selector(move || {
                                                files_label_selector.to_string()
                                            })
                                            .text_size(theme.ui_text(14.0))
                                            .text_color(theme.colors.foreground.secondary)
                                            .child(files_label),
                                    )
                                    .child(range_controls),
                            )
                            .child({
                                let bounds = std::rc::Rc::clone(&self.range_filter_bounds_ref);
                                let pane = cx.weak_entity();
                                div()
                                    .relative()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .on_children_prepainted(move |children, _window, app| {
                                        let next = children.first().copied();
                                        let mut measured = bounds.borrow_mut();
                                        if *measured != next {
                                            *measured = next;
                                            // Cached panes must be notified after prepaint.
                                            let pane = pane.clone();
                                            app.defer(move |app| {
                                                let _ = pane.update(app, |_pane, cx| cx.notify());
                                            });
                                        }
                                    })
                                    .child(visible_bounds_probe())
                                    .child(range_filters)
                            })
                            .child(files_body),
                    ),
            )
            .into_any_element()
    }
}
