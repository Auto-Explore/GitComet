use super::*;
use rustc_hash::FxHashSet;
use std::sync::Arc;

const ALL_AUTHORS_LABEL: &str = "All authors";

/// What decides the author list: the repository, its loaded log and author
/// catalog revisions, and the history scope. [`suggestions`] rebuilds the list
/// when it changes, and the cached rows key on it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct AuthorListKey {
    repo_id: RepoId,
    log_rev: u64,
    authors_rev: u64,
    scope: gitcomet_core::domain::HistoryMode,
}

impl AuthorListKey {
    fn of(repo: &RepoState) -> Self {
        Self {
            repo_id: repo.id,
            log_rev: repo.history_state.log_rev,
            authors_rev: repo.history_state.authors.rev,
            scope: repo.history_state.history_scope,
        }
    }
}

pub(super) struct SuggestionCache {
    key: AuthorListKey,
    names: Arc<[SharedString]>,
}

/// What activating a row does: clear the filter, or filter to one author.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AuthorTarget {
    All,
    Author(SharedString),
}

/// Author names from the loaded log commits: trimmed, non-empty, deduplicated
/// case-insensitively and sorted.
///
/// Runs over the whole accumulated log, which grows without bound as pages are
/// appended, so it avoids per-commit allocation: the exact-spelling gate
/// absorbs nearly every commit for the cost of one hash and a refcount bump
/// (the gix backend interns repeated author names, and history comes in runs by
/// the same person), leaving one lowercase key per distinct spelling.
fn collect_author_suggestions(commits: &[gitcomet_core::domain::Commit]) -> Vec<SharedString> {
    collect_author_names(commits.iter().map(|commit| &commit.author))
}

fn collect_author_names<'a>(names: impl IntoIterator<Item = &'a Arc<str>>) -> Vec<SharedString> {
    let mut seen_spellings: FxHashSet<Arc<str>> = FxHashSet::default();
    let mut seen_folded: FxHashSet<String> = FxHashSet::default();
    let mut authors: Vec<SharedString> = Vec::new();

    for name in names {
        if !seen_spellings.insert(name.clone()) {
            continue;
        }
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if seen_folded.insert(name.to_ascii_lowercase()) {
            authors.push(SharedString::from(name.to_owned()));
        }
    }

    // ASCII folding throughout — the same rule the dedup above, the active-filter
    // mark below, and the backend's own matcher apply. A Unicode fold here would
    // order names by a key nothing else in the feature agrees with.
    //
    // `sort_by_cached_key` builds one key per author; `sort_by_key` would
    // rebuild it on every comparison.
    authors.sort_by_cached_key(|author| author.to_ascii_lowercase());
    authors
}

/// Reuse the complete scope's author catalog; loaded commits provide initial
/// suggestions while discovery runs. Neither source is rescanned per frame.
pub(super) fn suggestions(this: &mut PopoverHost, repo_id: RepoId) -> Arc<[SharedString]> {
    let empty: Arc<[SharedString]> = Arc::from(Vec::new());
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return empty;
    };

    let key = AuthorListKey::of(repo);
    let catalog = &repo.history_state.authors;
    let scope = key.scope;
    if let Some(cached) = &this.history_author_suggestions
        && cached.key == key
    {
        return cached.names.clone();
    }

    let filtered = repo.history_state.history_author_filter.is_some();
    let cached_for_repo = this
        .history_author_suggestions
        .as_ref()
        .filter(|cached| cached.key.repo_id == repo_id && cached.key.scope == scope)
        .map(|cached| cached.names.clone());

    // A filtered log only describes the author already selected, so keep the
    // last list instead — unless there is nothing to keep (a filter restored
    // from the session), where a one-name list still beats an empty one.
    let refreshing_catalog = catalog.matches_scope(scope)
        && matches!(catalog.names, Loadable::Loading | Loadable::Error(_));
    let authors = match &catalog.names {
        Loadable::Ready(names) if catalog.matches_scope(scope) => {
            Arc::from(collect_author_names(names.iter()))
        }
        _ if (filtered || refreshing_catalog) && cached_for_repo.is_some() => {
            cached_for_repo.unwrap()
        }
        _ => match &repo.history_state.log {
            Loadable::Ready(page) => Arc::from(collect_author_suggestions(&page.commits)),
            _ => cached_for_repo.unwrap_or(empty),
        },
    };

    this.history_author_suggestions = Some(SuggestionCache {
        key,
        names: authors.clone(),
    });
    authors
}

/// Every row the dropdown can offer, and the target each one applies.
/// `items` and `targets` are index-aligned, so a `PickerPrompt` selection index
/// (which is the original, pre-filter index) reads straight out of `targets`.
///
/// Narrowing by the typed query is left to [`components::PickerPrompt`], which
/// filters the items it is handed anyway — doing it here first would scan every
/// author twice per keystroke and give the two passes a chance to disagree.
pub(super) struct AuthorRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) targets: Vec<AuthorTarget>,
    pub(super) marked_index: Option<usize>,
}

pub(super) fn rows(authors: &[SharedString], current: Option<&str>) -> AuthorRows {
    // "All authors" is always offered; `PickerPrompt` drops it from the
    // rendered rows when it does not match the query, and the navigation
    // targets are derived from that same layout.
    let mut items = Vec::with_capacity(authors.len() + 1);
    let mut targets = Vec::with_capacity(authors.len() + 1);
    items.push(components::PickerPromptItem::from(SharedString::from(
        ALL_AUTHORS_LABEL,
    )));
    targets.push(AuthorTarget::All);
    let mut marked_index = current.is_none().then_some(0);

    for author in authors {
        if marked_index.is_none()
            && current.is_some_and(|current| current.eq_ignore_ascii_case(author))
        {
            marked_index = Some(items.len());
        }
        items.push(components::PickerPromptItem::from(author.clone()));
        targets.push(AuthorTarget::Author(author.clone()));
    }

    AuthorRows {
        items,
        targets,
        marked_index,
    }
}

/// Digests what the rows are built from: the author list's key, and the active
/// filter that marks a row.
fn rows_signature(this: &PopoverHost, repo_id: RepoId) -> u64 {
    use std::hash::Hash;
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    super::rows_cache::signature(|hasher| {
        repo_id.hash(hasher);
        if let Some(repo) = repo {
            AuthorListKey::of(repo).hash(hasher);
            repo.history_state.history_author_filter.hash(hasher);
        }
    })
}

/// The rows for `query`, built once per change to the author list or the
/// active filter, though the popover redraws on every blink of its search box.
/// The panel and the arrow keys both read them from here.
pub(super) fn cached(
    this: &mut PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> std::rc::Rc<super::rows_cache::CachedRows<AuthorTarget>> {
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::AuthorFilter,
        rows_signature(this, repo_id),
        query,
    );
    let authors = suggestions(this, repo_id);
    let current = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.history_state.history_author_filter.clone());
    super::rows_cache::get_or_build(&this.history_author_rows_cache, key, |_now| {
        let rows = rows(&authors, current.as_deref());
        (rows.items, rows.targets, rows.marked_index)
    })
}

#[cfg(test)]
thread_local! {
    static NAV_TARGET_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`nav_targets`] calls since the last call, which resets the count.
#[cfg(test)]
pub(super) fn take_nav_target_builds_for_tests() -> usize {
    NAV_TARGET_BUILDS.with(|count| count.replace(0))
}

/// What the arrow keys walk, in the order the rows are rendered: the filtered
/// order of the same cached layout the panel draws, so Enter cannot apply a
/// different author than the highlighted row.
pub(super) fn nav_targets(
    this: &mut PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> Vec<AuthorTarget> {
    #[cfg(test)]
    NAV_TARGET_BUILDS.with(|count| count.set(count.get() + 1));
    cached(this, repo_id, query).filtered_payloads()
}

pub(super) fn apply(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: AuthorTarget,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let author = match target {
        AuthorTarget::All => None,
        AuthorTarget::Author(name) => {
            let name = name.trim();
            (!name.is_empty()).then(|| name.to_owned())
        }
    };
    this.store
        .dispatch(Msg::SetHistoryAuthorFilter { repo_id, author });
    this.close_popover(cx);
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    if this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_some_and(|repo| repo.history_state.authors.needs_load(repo))
    {
        this.store.dispatch(Msg::HistoryAuthors(
            gitcomet_state::history_authors::HistoryAuthorsMsg::Ensure {
                repo_id,
                retry: false,
            },
        ));
    }
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
    let width = super::HISTORY_AUTHOR_FILTER_WIDTH;

    let Some(search) = this.history_author_filter_search_input.clone() else {
        return components::context_menu(
            theme,
            div().w(width.preferred_px(ui_scale)).child(
                components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    "Search input not initialized",
                    Some(this.tooltip_host.clone()),
                    cx,
                )
                .into_any_element(),
            ),
        );
    };

    // A chromeless input with a leading magnifier, matching the repository
    // picker's search field.
    search.update(cx, |input, cx| {
        input.set_chromeless(true, cx);
        input.set_leading_icon(Some("icons/zoom.svg"), cx);
    });

    let query = search.read(cx).text().trim().to_string();
    let rows = cached(this, repo_id, &query);

    // Free-form matching remains available while the full catalog is loading,
    // and for substrings rather than exact author names.
    let empty_text = if query.is_empty() {
        "No authors"
    } else {
        "No match — Enter filters on what you typed"
    };

    let prompt = components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
        .prebuilt_items(rows.items.clone(), rows.layout.clone())
        .tooltip_host(this.tooltip_host.clone())
        .empty_text(empty_text)
        .max_height(scaled_px(components::PICKER_LIST_MAX_HEIGHT_PX))
        .selected_index(this.history_author_filter_selected_index)
        .marked_index(rows.marked_index)
        // A busy repository has thousands of contributors, and the list windows
        // itself past a couple of viewports: only the rows on screen are built.
        // Keyboard navigation scrolls by the row geometry to match
        // (`scroll_picker_prompt_to_row`).
        .padded_query_row();

    let status = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| match &repo.history_state.authors.names {
            Loadable::Loading => Some("Loading authors…"),
            Loadable::Error(_) => Some("Could not load all authors. Reopen to retry."),
            _ => None,
        });

    components::context_menu(
        theme,
        div()
            .child(
                prompt.render(theme, ui_scale_percent, cx, move |this, ix, _e, _w, cx| {
                    let Some(target) = rows.payloads.get(ix).cloned() else {
                        return;
                    };
                    apply(this, repo_id, target, cx);
                }),
            )
            .when_some(status, |body, status| {
                body.child(components::context_menu_label(
                    theme,
                    ui_scale_percent,
                    status,
                    Some(this.tooltip_host.clone()),
                    cx,
                ))
            }),
    )
    // Fixed width: PickerPrompt rows size with `w_full`, which does not stretch
    // under fit-content parents.
    .w(width.preferred_px(ui_scale))
}

#[cfg(test)]
mod tests;
