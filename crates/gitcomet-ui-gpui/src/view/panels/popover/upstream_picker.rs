//! Picker opened by the action bar's upstream badge.

use super::*;
use gitcomet_core::domain::Upstream;
use std::rc::Rc;

pub(super) const UPSTREAM_PICKER_LIST_MAX_HEIGHT_PX: f32 = components::PICKER_LIST_MAX_HEIGHT_PX;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UpstreamTarget {
    pub(super) remote: String,
    pub(super) branch: String,
}

impl UpstreamTarget {
    fn display_name(&self) -> String {
        format!("{}/{}", self.remote, self.branch)
    }
}

impl From<UpstreamTarget> for Upstream {
    fn from(target: UpstreamTarget) -> Self {
        Self {
            remote: target.remote,
            branch: target.branch,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum UpstreamNavTarget {
    Branch(UpstreamTarget),
    CreateNew,
    Unlink,
}

fn repo_for(this: &PopoverHost, repo_id: RepoId) -> Option<&RepoState> {
    this.state.repos.iter().find(|repo| repo.id == repo_id)
}

fn current_upstream<'a>(repo: &'a RepoState, branch: &str) -> Option<&'a Upstream> {
    repo.branches
        .ready()?
        .iter()
        .find(|candidate| candidate.name == branch)?
        .upstream
        .as_ref()
}

fn can_unlink(repo: &RepoState, branch: &str) -> bool {
    captured_branch_is_current(repo, branch) && current_upstream(repo, branch).is_some()
}

fn can_create_new(repo: &RepoState, branch: &str) -> bool {
    captured_branch_is_current(repo, branch) && current_upstream(repo, branch).is_none()
}

fn rows_signature(repo: &RepoState, branch: &str) -> u64 {
    use std::hash::Hash;

    super::rows_cache::signature(|hasher| {
        repo.id.hash(hasher);
        branch.hash(hasher);
        repo.branches_rev.hash(hasher);
        repo.remotes_rev.hash(hasher);
        repo.remote_branches_rev.hash(hasher);
        repo.ref_metadata_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.branches).hash(hasher);
        super::rows_cache::loadable_kind(&repo.remotes).hash(hasher);
        super::rows_cache::loadable_kind(&repo.remote_branches).hash(hasher);
    })
}

pub(super) fn cached(
    this: &PopoverHost,
    repo_id: RepoId,
    branch: &str,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<UpstreamTarget>> {
    let Some(repo) = repo_for(this, repo_id) else {
        return super::rows_cache::CachedRows::empty();
    };
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Upstream,
        rows_signature(repo, branch),
        query,
    );
    super::rows_cache::get_or_build(&this.upstream_picker_rows_cache, key, |now| {
        let Some(remote_branches) = repo.remote_branches.ready() else {
            return (Vec::new(), Vec::new(), None);
        };
        let Some(remotes) = repo.remotes.ready() else {
            return (Vec::new(), Vec::new(), None);
        };
        let current = current_upstream(repo, branch);
        let mut items = Vec::with_capacity(remote_branches.len());
        let mut targets = Vec::with_capacity(remote_branches.len());

        for remote_branch in remote_branches.iter() {
            if !remotes
                .iter()
                .any(|remote| remote.name == remote_branch.remote)
            {
                continue;
            }
            if remote_branch.name == "HEAD" {
                continue;
            }
            targets.push(UpstreamTarget {
                remote: remote_branch.remote.clone(),
                branch: remote_branch.name.clone(),
            });
        }

        // The current upstream is both the first row and the initially focused
        // selection. Stable sorting leaves every other remote branch in the
        // same order as the checkout picker/backend supplied it.
        targets.sort_by_key(|target| {
            !current.is_some_and(|upstream| {
                upstream.remote == target.remote && upstream.branch == target.branch
            })
        });
        let marked_index = targets
            .first()
            .filter(|target| {
                current.is_some_and(|upstream| {
                    upstream.remote == target.remote && upstream.branch == target.branch
                })
            })
            .map(|_| 0);
        let mut display_name_counts = rustc_hash::FxHashMap::default();
        for target in &targets {
            *display_name_counts
                .entry(target.display_name())
                .or_insert(0usize) += 1;
        }
        for target in &targets {
            let lookup_name = target.display_name();
            let display_name = if display_name_counts.get(&lookup_name) == Some(&1) {
                lookup_name.clone()
            } else {
                format!("{} / {}", target.remote, target.branch)
            };
            items.push(branch_picker::remote_branch_row(
                repo,
                display_name,
                &lookup_name,
                now,
            ));
        }

        (items, targets, marked_index)
    })
}

pub(super) fn nav_targets(
    this: &PopoverHost,
    repo_id: RepoId,
    branch: &str,
    query: &str,
) -> Vec<UpstreamNavTarget> {
    let mut targets = Vec::new();
    if repo_for(this, repo_id).is_some_and(|repo| can_unlink(repo, branch)) {
        targets.push(UpstreamNavTarget::Unlink);
    } else if repo_for(this, repo_id).is_some_and(|repo| can_create_new(repo, branch)) {
        targets.push(UpstreamNavTarget::CreateNew);
    }
    targets.extend(
        cached(this, repo_id, branch, query)
            .filtered_payloads()
            .into_iter()
            .map(UpstreamNavTarget::Branch),
    );
    targets
}

/// A disappearing branch selection must never clamp onto a fixed action.
/// Keep explicit action selections so keyboard navigation still works without
/// matching branches. Apply this before both rendering and keyboard handling.
pub(super) fn clear_missing_branch_selection(
    selected: &mut Option<usize>,
    branch_count: usize,
    leading_action_count: usize,
) {
    if branch_count == 0 && selected.is_some_and(|index| index >= leading_action_count) {
        *selected = None;
    }
}

pub(super) fn leading_action_count(this: &PopoverHost, repo_id: RepoId, branch: &str) -> usize {
    usize::from(
        repo_for(this, repo_id)
            .is_some_and(|repo| can_unlink(repo, branch) || can_create_new(repo, branch)),
    )
}

/// Keep the current (first) remote branch selected when the picker opens. The
/// fixed action is visually first, but selecting Unlink by default would make
/// an accidental Enter destructive.
pub(super) fn initial_selected_index(
    this: &PopoverHost,
    repo_id: RepoId,
    branch: &str,
) -> Option<usize> {
    let leading_actions = leading_action_count(this, repo_id, branch);
    if !cached(this, repo_id, branch, "")
        .layout
        .item_indices
        .is_empty()
    {
        Some(leading_actions)
    } else {
        None
    }
}

fn captured_branch_is_current(repo: &RepoState, branch: &str) -> bool {
    matches!(&repo.head_branch, Loadable::Ready(head) if head == branch)
        && repo
            .branches
            .ready()
            .is_some_and(|branches| branches.iter().any(|candidate| candidate.name == branch))
}

fn activate_branch(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: &str,
    target: UpstreamTarget,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(repo) = repo_for(this, repo_id) else {
        this.close_popover(cx);
        return;
    };
    let target_is_live = repo
        .remotes
        .ready()
        .is_some_and(|remotes| remotes.iter().any(|remote| remote.name == target.remote))
        && repo.remote_branches.ready().is_some_and(|remote_branches| {
            remote_branches.iter().any(|candidate| {
                candidate.remote == target.remote && candidate.name == target.branch
            })
        });
    if !captured_branch_is_current(repo, branch) || !target_is_live {
        this.close_popover(cx);
        return;
    }
    if current_upstream(repo, branch).is_some_and(|upstream| {
        upstream.remote == target.remote && upstream.branch == target.branch
    }) {
        this.close_popover(cx);
        return;
    }

    this.store.dispatch(Msg::SetUpstreamBranch {
        repo_id,
        branch: branch.to_string(),
        upstream: target.into(),
    });
    this.close_popover(cx);
}

pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: &str,
    target: UpstreamNavTarget,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match target {
        UpstreamNavTarget::Branch(target) => activate_branch(this, repo_id, branch, target, cx),
        UpstreamNavTarget::CreateNew => create_new(this, repo_id, branch, window, cx),
        UpstreamNavTarget::Unlink => unlink(this, repo_id, branch, cx),
    }
}

fn create_new(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: &str,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let Some(remote) = repo_for(this, repo_id).and_then(|repo| {
        can_create_new(repo, branch)
            .then(|| push_set_upstream_prompt::selected_remote(repo, "origin").unwrap_or_default())
    }) else {
        this.close_popover(cx);
        return;
    };
    this.open_popover_centered(
        PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote,
            configure_only_for: Some(branch.to_string()),
        },
        window,
        cx,
    );
}

pub(super) fn unlink(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: &str,
    cx: &mut gpui::Context<PopoverHost>,
) {
    let can_unlink_branch = repo_for(this, repo_id).is_some_and(|repo| can_unlink(repo, branch));
    if can_unlink_branch {
        this.store.dispatch(Msg::UnsetUpstreamBranch {
            repo_id,
            branch: branch.to_string(),
        });
    }
    this.close_popover(cx);
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    branch: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::LARGE_PICKER_WIDTH;
    let can_unlink_branch = repo_for(this, repo_id).is_some_and(|repo| can_unlink(repo, &branch));
    let can_create_new_branch =
        repo_for(this, repo_id).is_some_and(|repo| can_create_new(repo, &branch));

    let mut content = div()
        .flex()
        .flex_col()
        .min_w(width.min_px(ui_scale))
        .max_w(width.max_px(ui_scale))
        .child(popover_title(if can_unlink_branch {
            "Change Upstream"
        } else {
            "Set Upstream"
        }))
        .child(div().border_t_1().border_color(theme.colors.stroke.default))
        .child(
            div()
                .px_2()
                .pt_1()
                .text_xs()
                .text_color(theme.colors.foreground.secondary)
                .child(format!("Local branch: {branch}")),
        );

    let Some(search) = this.remote_picker_search_input.clone() else {
        return components::context_menu(
            theme,
            content.child(components::context_menu_label(
                theme,
                ui_scale,
                "Search input not initialized",
                Some(this.tooltip_host.clone()),
                cx,
            )),
        )
        .w(width.preferred_px(ui_scale));
    };
    search.update(cx, |input, cx| {
        input.set_chromeless(false, cx);
        input.set_leading_icon(Some("icons/cloud.svg"), cx);
    });
    let query = search.read(cx).text().trim().to_string();
    let built = cached(this, repo_id, &branch, &query);
    let leading_action_count = usize::from(can_unlink_branch || can_create_new_branch);
    if query.is_empty()
        && this.upstream_picker_selected_index.is_none()
        && let Some(marked_index) = built.marked_index
        && built.layout.item_indices.contains(&marked_index)
    {
        // The picker can open before remote rows arrive. Keep the destructive
        // Unlink action neutral while loading, then select the current upstream
        // as soon as its row is available.
        this.upstream_picker_selected_index = Some(leading_action_count + marked_index);
    }
    let targets = Rc::clone(&built.payloads);
    let branch_count = built.layout.item_indices.len();
    let nav_count = branch_count + leading_action_count;
    clear_missing_branch_selection(
        &mut this.upstream_picker_selected_index,
        branch_count,
        leading_action_count,
    );
    let selected_index = this
        .upstream_picker_selected_index
        .filter(|_| nav_count > 0)
        .map(|selected| selected.min(nav_count - 1));
    let branch_selected_index = selected_index
        .and_then(|selected| selected.checked_sub(leading_action_count))
        .filter(|selected| *selected < branch_count);
    let empty_text = match repo_for(this, repo_id) {
        Some(repo)
            if matches!(repo.remotes, Loadable::Loading | Loadable::NotLoaded)
                || matches!(
                    repo.remote_branches,
                    Loadable::Loading | Loadable::NotLoaded
                ) =>
        {
            "Loading remote branches"
        }
        Some(repo)
            if matches!(repo.remotes, Loadable::Error(_))
                || matches!(repo.remote_branches, Loadable::Error(_)) =>
        {
            "Could not list remote branches"
        }
        Some(_) if !query.is_empty() => "No matching remote branches",
        Some(_) => "No remote branches",
        None => "No repository",
    };
    if can_unlink_branch {
        let unlink_branch = branch.clone();
        let unlink_entry = components::ContextMenuEntry::new(
            "upstream_unlink",
            components::ContextMenuText::new("Unlink upstream branch").max_lines(1),
        )
        .icon(components::ContextMenuIconSlot::Icon(
            "icons/unlink.svg".into(),
        ))
        .selected(selected_index == Some(0))
        .tooltip_host(this.tooltip_host.clone())
        .render(theme, ui_scale, cx)
        .debug_selector(|| "upstream_unlink".to_string())
        .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
            unlink(this, repo_id, &unlink_branch, cx);
        }));
        content = content.child(unlink_entry);
    } else if can_create_new_branch {
        let create_branch = branch.clone();
        let create_entry = components::ContextMenuEntry::new(
            "upstream_create_new",
            components::ContextMenuText::new("Create new").max_lines(1),
        )
        .icon(components::ContextMenuIconSlot::Icon(
            "icons/plus.svg".into(),
        ))
        .selected(selected_index == Some(0))
        .tooltip_host(this.tooltip_host.clone())
        .render(theme, ui_scale, cx)
        .debug_selector(|| "upstream_create_new".to_string())
        .on_click(cx.listener(move |this, _e: &ClickEvent, window, cx| {
            create_new(this, repo_id, &create_branch, window, cx);
        }));
        content = content.child(create_entry);
    }
    if leading_action_count > 0 {
        content = content
            .child(components::context_menu_separator(theme, ui_scale))
            .child(div().h(scaled_px(4.0)));
    }

    let select_branch = branch.clone();
    content = content.child(
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
            .tooltip_host(this.tooltip_host.clone())
            .empty_text(empty_text)
            .max_height(scaled_px(UPSTREAM_PICKER_LIST_MAX_HEIGHT_PX))
            .selected_index(branch_selected_index)
            .marked_index(built.marked_index)
            .accent_selection()
            .render(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, _e, window, cx| {
                    let Some(target) = targets.get(ix).cloned() else {
                        return;
                    };
                    activate(
                        this,
                        repo_id,
                        &select_branch,
                        UpstreamNavTarget::Branch(target),
                        window,
                        cx,
                    );
                },
            ),
    );

    components::context_menu(theme, content).w(width.preferred_px(ui_scale))
}
