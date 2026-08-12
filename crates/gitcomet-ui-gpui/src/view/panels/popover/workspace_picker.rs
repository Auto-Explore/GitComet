use super::*;

/// What activating a row in the workspace picker does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorkspaceRow {
    /// Open (or re-activate) the worktree at this path.
    Worktree(std::path::PathBuf),
    /// Hand off to the Add-worktree dialog, prefilled from the query.
    CreateNew,
}

/// Rows the picker displays, in render order, paired with what each one does
/// and which row carries the "current worktree" check.
///
/// Both the panel and keyboard navigation go through this, so the list the user
/// sees and the list Enter walks can never disagree.
pub(super) struct WorkspaceRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) rows: Vec<WorkspaceRow>,
    /// Index of the active worktree **before filtering** — `PickerPrompt`
    /// compares `marked_index` against the pre-filter index.
    pub(super) marked_index: Option<usize>,
}

/// Ref the create row bases a new worktree on: the current branch, or `HEAD`
/// when detached or not yet loaded.
fn create_base_ref(repo: &RepoState) -> String {
    match &repo.head_branch {
        Loadable::Ready(head) if head != "HEAD" => head.clone(),
        _ => "HEAD".to_string(),
    }
}

/// Suggested folder for a new worktree named `query`: a sibling of the current
/// workdir, which is where linked worktrees conventionally live. An empty query
/// leaves the dialog's path blank.
pub(super) fn suggested_worktree_path(repo: &RepoState, query: &str) -> String {
    let query = query.trim();
    if query.is_empty() {
        return String::new();
    }
    // Branch-shaped queries ("feat/x") would nest directories; flatten them.
    let folder = query.replace(['/', '\\'], "-");
    match repo.spec.workdir.parent() {
        Some(parent) => parent.join(folder).display().to_string(),
        None => folder,
    }
}

pub(super) fn rows(this: &PopoverHost, repo_id: RepoId, query: &str) -> WorkspaceRows {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    let mut marked_index = None;

    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
        return WorkspaceRows {
            items,
            rows,
            marked_index,
        };
    };

    // Create row first, mirroring the placeholder's "select or type to create".
    // Its searchable text is the query itself, so it survives any filter —
    // `match_items` drops rows whose match text does not contain the query.
    let base = create_base_ref(repo);
    let query = query.trim();
    // The base ref goes on the detail line rather than trailing the title, so this
    // row is the same height as the worktree rows below it and the list reads as one
    // block instead of a short row stacked on tall ones.
    let create_item = if query.is_empty() {
        components::PickerPromptItem::from_parts([components::PickerPromptItemPart::new(
            "Create new worktree",
        )
        .flexible(false)
        .searchable(false)])
    } else {
        components::PickerPromptItem::from_parts([
            components::PickerPromptItemPart::new("Create worktree ")
                .flexible(false)
                .searchable(false),
            components::PickerPromptItemPart::new(query.to_string()).flexible(false),
        ])
    };
    items.push(
        create_item
            .secondary_parts([
                components::PickerPromptItemPart::new(format!("Based off {base}"))
                    .searchable(false),
            ])
            .icon("icons/plus.svg"),
    );
    rows.push(WorkspaceRow::CreateNew);

    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return WorkspaceRows {
            items,
            rows,
            marked_index,
        };
    };

    for worktree in worktrees.iter() {
        // Title line: the folder the worktree lives in, then what is checked out
        // in it — the two things that identify a worktree at a glance.
        let name = crate::view::path_display::repo_path_name(&worktree.path);
        let mut primary = vec![
            components::PickerPromptItemPart::new(name.to_string())
                .profile(components::TextTruncationProfile::End)
                .flexible(false),
        ];

        if let Some(branch) = &worktree.branch {
            primary.push(components::PickerPromptItemPart::separator("  on  "));
            primary.push(
                components::PickerPromptItemPart::new(branch.clone())
                    .profile(components::TextTruncationProfile::End)
                    .flexible(false),
            );
        } else if worktree.detached {
            primary.push(components::PickerPromptItemPart::separator("  "));
            primary.push(
                components::PickerPromptItemPart::new("detached")
                    .flexible(false)
                    .searchable(false)
                    .dim(),
            );
        }

        // Detail line: where it is on disk, and which commit it sits on. The path
        // stays searchable so a path query still finds its row.
        let mut secondary = Vec::new();
        if let Some(head) = &worktree.head {
            let sha = head.as_ref();
            secondary.push(
                components::PickerPromptItemPart::new(sha.get(0..8).unwrap_or(sha).to_string())
                    .flexible(false)
                    .searchable(false),
            );
            secondary.push(components::PickerPromptItemPart::separator("  •  "));
        }
        secondary.push(components::PickerPromptItemPart::path(
            worktree.path.display().to_string(),
        ));

        if worktree.path == repo.spec.workdir {
            marked_index = Some(items.len());
        }
        items.push(
            components::PickerPromptItem::from_parts(primary)
                .secondary_parts(secondary)
                .icon("icons/git_worktree.svg"),
        );
        rows.push(WorkspaceRow::Worktree(worktree.path.clone()));
    }

    WorkspaceRows {
        items,
        rows,
        marked_index,
    }
}

/// Payloads for the rows surviving `query`, in the order the picker renders
/// them, alongside the scroll-child index of each (section-free here, but the
/// layout helper is what keeps nav and render in lockstep).
pub(super) fn filtered_layout(
    this: &PopoverHost,
    repo_id: RepoId,
    query: &str,
) -> (Vec<WorkspaceRow>, components::PickerPromptLayout) {
    let built = rows(this, repo_id, query);
    let layout = components::picker_prompt_layout(&built.items, query);
    let targets = layout
        .item_indices
        .iter()
        .filter_map(|ix| built.rows.get(*ix).cloned())
        .collect();
    (targets, layout)
}

pub(super) fn nav_targets(this: &PopoverHost, repo_id: RepoId, query: &str) -> Vec<WorkspaceRow> {
    filtered_layout(this, repo_id, query).0
}

/// Activates a row: opens the worktree, or hands the query to the Add dialog.
pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    row: WorkspaceRow,
    query: &str,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match row {
        WorkspaceRow::Worktree(path) => {
            let is_current = this
                .state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .is_some_and(|repo| repo.spec.workdir == path);
            // Re-opening the active worktree would only re-activate its own
            // tab; just dismiss instead.
            if !is_current {
                this.store.dispatch(Msg::OpenRepo(path));
            }
            this.close_popover(cx);
        }
        WorkspaceRow::CreateNew => {
            let path = this
                .state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .map(|repo| suggested_worktree_path(repo, query))
                .unwrap_or_default();
            // Deliberately no reference: `git worktree add <path>` branches off
            // HEAD into a new branch named after the folder, which is what the
            // "based on <head>" label promises. Passing the head branch itself
            // would fail — git refuses to check out a branch that is already
            // checked out in another worktree.
            this.pending_worktree_add_prefill = Some((path, String::new()));
            this.open_popover_centered(
                PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
                window,
                cx,
            );
        }
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let width = super::LARGE_PICKER_WIDTH;

    let Some(search) = this.workspace_picker_search_input.clone() else {
        return components::context_menu_label(
            theme,
            ui_scale_percent,
            "Search input not initialized",
            Some(this.tooltip_host.clone()),
            cx,
        );
    };

    // Read the query from the same input `PickerPrompt::render` filters with,
    // so the rows built here match the rows it displays.
    let query = search.read_with(cx, |input, _| input.text().trim().to_string());
    let built = rows(this, repo_id, &query);
    let row_payloads = built.rows.clone();

    components::context_menu(
        theme,
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            .items(built.items)
            .tooltip_host(this.tooltip_host.clone())
            // Only reachable when the repo is gone: the create row always
            // matches, so a present repo never yields an empty list.
            .empty_text("No repository")
            .max_height(scaled_px(300.0))
            .selected_index(this.workspace_picker_selected_index)
            .marked_index(built.marked_index)
            .render(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, _e, window, cx| {
                    let Some(row) = row_payloads.get(ix).cloned() else {
                        return;
                    };
                    let query = this
                        .workspace_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    activate(this, repo_id, row, &query, window, cx);
                },
            ),
    )
    .w(width.preferred_px(ui_scale))
}
