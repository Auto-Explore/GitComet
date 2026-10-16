//! Building the history list's base and decoration caches off the UI thread.

use super::{
    AppTheme, Arc, Branch, Commit, CommitId, FxHashMap, FxHashSet, HistoryAuthorVm,
    HistoryBaseCache, HistoryBaseCacheRequest, HistoryBaseRowVm, HistoryDecorationCache,
    HistoryDecorationCacheRequest, HistoryDecorationRowVm, HistoryRefListItem,
    HistoryRefListItemKind, HistoryShortShaVm, HistoryTextVm, HistoryVisibleIndices, HistoryWhenVm,
    LogPage, LogScope, RemoteBranch, SharedString, SmallVec, StashEntry, Tag,
    analyze_history_stashes, build_history_branch_chips_by_target,
    build_history_branch_containment_bits, build_history_branch_ref_items_by_target,
    build_history_branch_text_by_target, build_history_tag_names_by_target,
    build_history_visible_indices, graph_branch_heads, history_graph,
    history_ref_items_from_displayed_refs, next_history_stash_tip_for_commit_ix,
    related_commit_contains,
};

#[cfg(test)]
pub(super) fn is_probable_stash_tip(commit: &Commit) -> bool {
    crate::view::caches::history_commit_is_probable_stash_tip(commit)
}

/// Shared with find, which matches stash rows on what they show.
#[cfg(test)]
pub(super) fn stash_summary_from_log_summary(summary: &str) -> Option<&str> {
    gitcomet_core::history_find::stash_summary_tail(summary)
}

pub(super) fn resolve_history_head_target<'a>(
    history_scope: LogScope,
    detached_head_commit: Option<&'a CommitId>,
    head_branch: Option<&'a str>,
    branches: &'a [Branch],
    visible_indices: &HistoryVisibleIndices,
    commits: &'a [Commit],
) -> Option<&'a str> {
    match head_branch {
        Some("HEAD") => detached_head_commit.map(AsRef::as_ref).or_else(|| {
            history_scope
                .guarantees_head_visibility()
                .then(|| {
                    visible_indices
                        .first()
                        .and_then(|ix| commits.get(ix))
                        .map(|commit| commit.id.as_ref())
                })
                .flatten()
        }),
        Some(head) => branches
            .iter()
            .find(|branch| branch.name == head)
            .map(|branch| branch.target.as_ref()),
        None => None,
    }
}

pub(super) fn build_history_base_cache(
    request: HistoryBaseCacheRequest,
    page: &LogPage,
    theme: AppTheme,
    head_branch: Option<&str>,
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
    stashes: &[StashEntry],
) -> HistoryBaseCache {
    let stash_analysis = analyze_history_stashes(&page.commits, stashes);
    let stash_tips = stash_analysis.stash_tips;
    let stash_helper_ids = stash_analysis.stash_helper_ids;

    let visible_indices = build_history_visible_indices(&page.commits, &stash_helper_ids);
    let head_target = resolve_history_head_target(
        request.history_scope,
        request.detached_head_commit.as_ref(),
        head_branch,
        branches,
        &visible_indices,
        &page.commits,
    );

    let branch_heads = graph_branch_heads(request.history_scope, branches, remote_branches);
    let graph_rows: Arc<[history_graph::GraphRow]> = if stash_helper_ids.is_empty() {
        history_graph::compute_graph(&page.commits, theme, branch_heads, head_target).into()
    } else {
        let visible_commit_refs = visible_indices
            .iter()
            .map(|ix| &page.commits[ix])
            .collect::<Vec<_>>();
        history_graph::compute_graph_refs(&visible_commit_refs, theme, branch_heads, head_target)
            .into()
    };
    let has_stash_tips = !stash_tips.is_empty();
    let mut author_cache: FxHashMap<&str, HistoryAuthorVm> =
        FxHashMap::with_capacity_and_hasher(64, Default::default());
    let mut row_vms = Vec::with_capacity(visible_indices.len());
    if has_stash_tips {
        let mut next_stash_tip_ix = 0usize;
        for ix in visible_indices.iter() {
            let Some(commit) = page.commits.get(ix) else {
                continue;
            };
            let commit_id = commit.id.as_ref();
            let author = author_cache
                .entry(commit.author.as_ref())
                .or_insert_with(|| HistoryAuthorVm::new(commit.author.clone().into()))
                .clone();
            let (is_stash, summary) =
                match next_history_stash_tip_for_commit_ix(&stash_tips, &mut next_stash_tip_ix, ix)
                {
                    Some(stash_tip) => (
                        true,
                        SharedString::new(gitcomet_core::history_find::stash_row_summary(
                            stash_tip.message.map(|message| message.as_ref()),
                            &commit.summary,
                        )),
                    ),
                    None => (false, commit.summary.clone().into()),
                };

            row_vms.push(HistoryBaseRowVm {
                author,
                author_run_interior: false,
                summary: HistoryTextVm::new(summary),
                when: HistoryWhenVm::deferred(commit.time),
                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                is_head: head_target == Some(commit_id),
                is_stash,
            });
        }
    } else {
        for ix in visible_indices.iter() {
            let Some(commit) = page.commits.get(ix) else {
                continue;
            };
            let author = author_cache
                .entry(commit.author.as_ref())
                .or_insert_with(|| HistoryAuthorVm::new(commit.author.clone().into()))
                .clone();
            row_vms.push(HistoryBaseRowVm {
                author,
                author_run_interior: false,
                summary: HistoryTextVm::new(commit.summary.clone().into()),
                when: HistoryWhenVm::deferred(commit.time),
                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                is_head: head_target == Some(commit.id.as_ref()),
                is_stash: false,
            });
        }
    }

    // One entry per visible commit, built here so its readers can look up an id
    // during layout without walking the page.
    let mut visible_ix_by_commit: FxHashMap<CommitId, usize> =
        FxHashMap::with_capacity_and_hasher(visible_indices.len(), Default::default());
    for (visible_ix, commit_ix) in visible_indices.iter().enumerate() {
        if let Some(commit) = page.commits.get(commit_ix) {
            visible_ix_by_commit
                .entry(commit.id.clone())
                .or_insert(visible_ix);
        }
    }

    let mut base = HistoryBaseCache {
        request,
        visible_indices,
        visible_ix_by_commit: Arc::new(visible_ix_by_commit),
        graph_rows,
        row_vms,
    };
    crate::view::caches::mark_history_author_runs(&mut base, page);
    base
}

pub(super) fn build_history_decoration_cache(
    request: HistoryDecorationCacheRequest,
    page: &LogPage,
    base: &HistoryBaseCache,
    head_branch: Option<&str>,
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
    tags: &[Tag],
) -> HistoryDecorationCache {
    let head_target = resolve_history_head_target(
        request.base_request.history_scope,
        request.detached_head_commit.as_ref(),
        head_branch,
        branches,
        &base.visible_indices,
        &page.commits,
    );
    let (mut branch_text_by_target, head_branches_text) =
        build_history_branch_text_by_target(branches, remote_branches, head_branch, head_target);
    let (mut branch_chips_by_target, head_branch_chips) =
        build_history_branch_chips_by_target(branches, remote_branches, head_branch, head_target);
    let (mut branch_ref_items_by_target, head_branch_ref_items) =
        build_history_branch_ref_items_by_target(
            branches,
            remote_branches,
            head_branch,
            head_target,
        );
    let mut tag_names_by_target = build_history_tag_names_by_target(tags);
    let mut row_vms = Vec::with_capacity(base.visible_indices.len());

    // Branch attribution per lane column, carried downwards: a lane is started
    // by a branch head, and every commit below inherits it until the lane ends.
    //
    // Correct only because lane columns are stable for a lane's whole life (see
    // `history_graph::Lanes`) -- against shifting columns the carried name would
    // follow whichever lane slid into the column.
    let mut branch_names: Vec<SharedString> = Vec::new();
    // Owned keys: the names come from per-row `ref_items` that do not outlive
    // the iteration. Only ever written on a *miss*, so the allocations are
    // bounded by the number of distinct branch names rather than by rows.
    let mut branch_name_ix: FxHashMap<String, u16> = FxHashMap::default();
    // Local branches with an upstream, so attribution can prefer shared history
    // over a branch that only exists on this machine.
    let tracked_local_branches: FxHashSet<&str> = branches
        .iter()
        .filter(|branch| branch.upstream.is_some())
        .map(|branch| branch.name.as_str())
        .collect();
    // Index into `branch_names`, plus the row its branch head was seen on. The
    // row is what breaks ties where several branches contain the same commit.
    let mut lane_branch_by_col: SmallVec<[Option<(u16, usize)>; 8]> = SmallVec::new();

    // Integration branches present in this repo, each with the set of commits it
    // contains. A commit that is *in* `dev` is dev's, however the graph happens
    // to draw the lane it sits on -- carrying names down lanes alone gets this
    // wrong the moment a feature branch diverges, because the shared history
    // below the fork keeps whichever lane won the node.
    //
    // The names are interned up front, so the per-row lookup below yields an
    // index straight away rather than cloning a `String` on every row.
    let integration_containment: Vec<(u16, Arc<[u64]>)> = {
        let tips = integration_branch_tips(branches, remote_branches);
        let containment =
            build_history_branch_containment_bits(&page.commits, tips.iter().map(|(_, tip)| tip));
        tips.iter()
            .zip(containment)
            .filter_map(|((name, _), bits)| {
                let ix = intern_branch_name(&mut branch_names, &mut branch_name_ix, name)?;
                Some((ix, bits))
            })
            .collect()
    };

    for (visible_ix, (commit_ix, base_row)) in base
        .visible_indices
        .iter()
        .zip(base.row_vms.iter())
        .enumerate()
    {
        let Some(commit) = page.commits.get(commit_ix) else {
            continue;
        };
        let commit_id = commit.id.as_ref();
        let branches_text = if base_row.is_head {
            head_branches_text.clone().unwrap_or_default()
        } else {
            branch_text_by_target
                .remove(commit_id)
                .unwrap_or_else(HistoryTextVm::default)
        };
        let branch_items = if base_row.is_head {
            head_branch_ref_items.clone().unwrap_or_default()
        } else {
            branch_ref_items_by_target
                .remove(commit_id)
                .unwrap_or_default()
        };
        let branch_chips = if base_row.is_head {
            head_branch_chips.clone().unwrap_or_default()
        } else {
            branch_chips_by_target.remove(commit_id).unwrap_or_default()
        };
        let tag_names = tag_names_by_target.remove(commit_id).unwrap_or_default();
        let ref_items = history_ref_items_from_displayed_refs(&tag_names, branch_items);

        let graph_row = base.graph_rows.get(visible_ix);
        let node_col = graph_row.map_or(0, |row| usize::from(row.node_col));

        // Where lanes converge -- a fork point, where a feature branch rejoins
        // the branch it was cut from -- the commit is contained by every
        // converging branch, and taking whichever lane happens to own the node
        // is arbitrary. Prefer the branch head seen *nearest above* this commit,
        // which for the usual "feature cut from dev" shape is the base branch:
        // the feature's head sits further up the log, dev's nearer the shared
        // history. Both answers are true -- git would list both -- but this is
        // the one that matches how people read the graph.
        let mut resolved = lane_branch_by_col.get(node_col).copied().flatten();
        if let Some(graph_row) = graph_row {
            for edge in graph_row.joins_in.iter() {
                let candidate = lane_branch_by_col
                    .get(usize::from(edge.from_col))
                    .copied()
                    .flatten();
                if let Some(candidate) = candidate
                    && resolved.is_none_or(|(_, seeded_at)| candidate.1 > seeded_at)
                {
                    resolved = Some(candidate);
                }
            }
        }

        // Containment in an integration branch outranks everything: the commit
        // genuinely belongs to that branch, whatever lane it is drawn on. The
        // name is already interned, so the common case allocates nothing.
        let contained_in = integration_containment
            .iter()
            .find(|(_, bits)| related_commit_contains(bits, commit_ix))
            .map(|(ix, _)| *ix);

        // Otherwise a branch ref on this row beats anything inherited: the row
        // *is* that branch's head.
        let attributed = contained_in.or_else(|| {
            let name = history_row_attribution_branch(&ref_items, &tracked_local_branches)?;
            intern_branch_name(&mut branch_names, &mut branch_name_ix, name)
        });
        if let Some(ix) = attributed {
            resolved = Some((ix, visible_ix));
        }

        // The surviving lane carries whatever the convergence resolved to, so
        // the rest of the shared history follows the same branch.
        if lane_branch_by_col.len() <= node_col {
            lane_branch_by_col.resize(node_col + 1, None);
        }
        lane_branch_by_col[node_col] = resolved;

        let lane_branch = resolved.map(|(ix, _)| ix);

        // Carry the attribution into the next row: a lane born at this node
        // inherits the node's branch, and a column left empty forgets its own.
        if let Some(graph_row) = graph_row {
            if lane_branch_by_col.len() < graph_row.lanes_next.len() {
                lane_branch_by_col.resize(graph_row.lanes_next.len(), None);
            }
            for (col, lane) in graph_row.lanes_next.iter().enumerate() {
                if !lane.is_active() {
                    lane_branch_by_col[col] = None;
                } else if lane.starts_at_node() {
                    lane_branch_by_col[col] = resolved;
                }
            }
        }

        row_vms.push(HistoryDecorationRowVm {
            branches_text,
            tag_names,
            branch_chips,
            ref_items,
            lane_branch,
        });
    }

    HistoryDecorationCache {
        request,
        row_vms: row_vms.into(),
        branch_names: branch_names.into(),
    }
}

/// Records `name` in the decoration cache's shared name table and returns its
/// index, reusing the index when the name is already there.
///
/// `None` once the table is full. The index is a `u16`, and saturating at
/// `u16::MAX` instead would hand the same slot to every name past the cap while
/// the table kept growing, so rows would be labelled with someone else's branch.
pub(super) fn intern_branch_name(
    names: &mut Vec<SharedString>,
    ix_by_name: &mut FxHashMap<String, u16>,
    name: &str,
) -> Option<u16> {
    // Probed by `&str` first: on the hit path -- which is nearly every row in a
    // repo with an integration branch -- this must not allocate a key.
    if let Some(ix) = ix_by_name.get(name) {
        return Some(*ix);
    }
    let ix = u16::try_from(names.len()).ok()?;
    let owned = name.to_owned();
    names.push(SharedString::from(owned.clone()));
    ix_by_name.insert(owned, ix);
    Some(ix)
}

/// Branch name a rendered ref stands for, or `None` for tags and detached HEAD.
pub(super) fn history_ref_branch_name(item: &HistoryRefListItem) -> Option<&str> {
    match &item.kind {
        HistoryRefListItemKind::AttachedHead { branch } => Some(branch.as_str()),
        HistoryRefListItemKind::LocalBranch { name } => Some(name.as_str()),
        HistoryRefListItemKind::RemoteBranch { name, .. } => Some(name.as_str()),
        HistoryRefListItemKind::Tag { .. } | HistoryRefListItemKind::DetachedHead => None,
    }
}

/// Integration branches present in the repo, highest priority first, as
/// `(display name, tip)`. A local branch is preferred over the remote of the
/// same name so the label matches what the ref column shows.
pub(super) fn integration_branch_tips(
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
) -> Vec<(String, CommitId)> {
    let mut found: Vec<(String, CommitId)> = Vec::new();
    for wanted in INTEGRATION_BRANCH_NAMES {
        if let Some(branch) = branches.iter().find(|branch| branch.name == wanted) {
            found.push((branch.name.clone(), branch.target.clone()));
            continue;
        }
        if let Some(remote) = remote_branches
            .iter()
            .find(|remote| remote.name == wanted && remote.remote == "origin")
        {
            found.push((
                format!("{}/{}", remote.remote, remote.name),
                remote.target.clone(),
            ));
        }
    }
    found
}

/// Branch names that conventionally carry shared history. A commit sitting on
/// one of these belongs to it, not to whatever short-lived branch happens to be
/// parked on the same commit.
pub(super) const INTEGRATION_BRANCH_NAMES: [&str; 5] =
    ["main", "master", "dev", "develop", "trunk"];

/// Which of several branch refs on one commit names the history *below* it.
///
/// Several branches pointing at the same commit are structurally identical --
/// there is no graph signal to separate them -- so this ranks them on what the
/// refs themselves say. Lower is better:
///
/// 0. a conventional integration branch (`main`, `dev`, ...);
/// 1. a branch that is tracked on a remote, so its history is shared;
/// 2. anything else, i.e. a purely local branch.
///
/// The case this exists for: cutting a feature branch and not committing yet
/// leaves `HEAD -> feature` and `dev` on the same commit, and the entire history
/// beneath would otherwise be labelled with the brand-new feature branch.
pub(super) fn branch_attribution_rank(name: &str, tracked: bool) -> u8 {
    // `origin/dev` ranks as `dev`.
    let leaf = name.rsplit('/').next().unwrap_or(name);
    if INTEGRATION_BRANCH_NAMES.contains(&leaf) {
        0
    } else if tracked {
        1
    } else {
        2
    }
}

/// Best branch ref on a row to attribute the history below it to, or `None`
/// when the row carries no branch ref. Ties keep the rendered ref order.
pub(super) fn history_row_attribution_branch<'a>(
    ref_items: &'a [HistoryRefListItem],
    tracked_local_branches: &FxHashSet<&str>,
) -> Option<&'a str> {
    ref_items
        .iter()
        .enumerate()
        .filter_map(|(order, item)| {
            let name = history_ref_branch_name(item)?;
            // A remote branch is shared by definition; a local one only if it
            // has an upstream.
            let tracked = match &item.kind {
                HistoryRefListItemKind::RemoteBranch { .. } => true,
                _ => tracked_local_branches.contains(name),
            };
            Some((branch_attribution_rank(name, tracked), order, name))
        })
        .min_by_key(|(rank, order, _)| (*rank, *order))
        .map(|(_, _, name)| name)
}
