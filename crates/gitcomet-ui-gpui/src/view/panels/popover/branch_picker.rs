use super::*;
use std::rc::Rc;

const LOCAL_SECTION: &str = "Local Branches";
const REMOTE_SECTION: &str = "Remote Branches";

/// What activating a row in the branch picker does. Shared with the branch
/// search input, whose subscription is generic over a single payload type and
/// also serves the delete picker, the create-branch source field and the
/// worktree-add ref field — those always produce [`Ref`](Self::Ref).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum BranchPickerNavTarget {
    /// A ref name: check it out, or fill in the field that asked for a ref.
    Ref(String),
    /// A remote-tracking branch; checking it out needs a local branch name.
    RemoteBranch { remote: String, branch: String },
    /// Create (and check out) a branch with this name.
    CreateBranch(String),
}

pub(super) struct BranchRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) rows: Vec<BranchPickerNavTarget>,
    /// Index of the checked-out branch **before filtering** — `PickerPrompt`
    /// compares `marked_index` against the pre-filter index.
    pub(super) marked_index: Option<usize>,
}

/// True for the action-bar branch badge's checkout picker, which renders
/// sectioned rows with metadata and a create row rather than a plain list.
/// Shared with the keyboard side so the rendered list and the Enter list are
/// gated by the very same condition.
pub(super) fn is_checkout_picker(this: &PopoverHost) -> bool {
    matches!(
        this.popover,
        Some(PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Checkout
        })
    )
}

/// Author / relative date / summary parts for `refname`, when ref metadata has
/// loaded. These form the row's secondary line. All non-searchable: filtering is
/// by branch name, so typing an author name must not pull in unrelated branches.
fn metadata_parts(
    repo: &RepoState,
    refname: &str,
    now: std::time::SystemTime,
) -> Vec<components::PickerPromptItemPart> {
    let Loadable::Ready(metadata) = &repo.ref_metadata else {
        // Still loading: no detail line yet rather than a placeholder that would
        // be replaced a moment later.
        return Vec::new();
    };
    let Some(entry) = metadata.get(refname) else {
        if metadata.is_empty() {
            // A backend without ref metadata latches an empty map rather than an
            // error (`ref_metadata_loaded`), and the trait documents that callers
            // fall back to name-only rows. Every row claiming "no commits" would
            // be a worse answer than no detail line at all.
            return Vec::new();
        }
        // Metadata did load and this ref simply has none: say so rather than
        // dropping the line, so one branch does not leave a short row in a list
        // of tall ones.
        return vec![
            components::PickerPromptItemPart::new("No commits found")
                .searchable(false)
                .tooltip(false),
        ];
    };

    let mut parts = Vec::new();
    let mut push = |text: String| {
        if text.is_empty() {
            return;
        }
        if !parts.is_empty() {
            parts.push(components::PickerPromptItemPart::separator("  •  "));
        }
        // Fixed-width and never squeezed, so there is no truncation for a
        // tooltip to reveal.
        parts.push(
            components::PickerPromptItemPart::new(text)
                .searchable(false)
                .flexible(false)
                .tooltip(false),
        );
    };

    push(entry.author.clone());
    push(crate::view::date_time::format_relative_time(
        entry.committed_at,
        now,
    ));

    if !entry.summary.is_empty() {
        if !parts.is_empty() {
            parts.push(components::PickerPromptItemPart::separator("  •  "));
        }
        // Last and flexible, so the summary is what gives way when the row is
        // narrower than the whole detail line.
        parts.push(
            components::PickerPromptItemPart::new(entry.summary.clone())
                .searchable(false)
                .profile(components::TextTruncationProfile::End),
        );
    }

    parts
}

fn branch_row(
    repo: &RepoState,
    display_name: String,
    lookup_name: &str,
    icon: &'static str,
    section: &'static str,
    now: std::time::SystemTime,
) -> components::PickerPromptItem {
    // The ref name is the row's title; who touched it last and what they said is
    // supporting detail on a second, quieter line.
    let item = components::PickerPromptItem::from_parts([components::PickerPromptItemPart::new(
        display_name,
    )
    .profile(components::TextTruncationProfile::End)
    .flexible(false)])
    .icon(icon)
    .section(section);
    item.secondary_parts(metadata_parts(repo, lookup_name, now))
}

/// Rows for the checkout picker: local branches, remote branches, and a create
/// row. Both the panel and keyboard navigation go through this, so the rendered
/// list and the list Enter walks can never disagree.
///
/// Takes the repository rather than the host so the result is a pure function of
/// its inputs: [`rows_cache`](super::rows_cache) memoises it across frames, and
/// benchmarks build it without a popover. `now` is passed in for the same reason
/// — the relative dates on the detail line would otherwise make every call
/// distinct.
pub(super) fn rows(repo: &RepoState, query: &str, now: std::time::SystemTime) -> BranchRows {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    let mut marked_index = None;

    let head_branch = match &repo.head_branch {
        Loadable::Ready(head) if head != "HEAD" => Some(head.as_str()),
        _ => None,
    };

    let mut local_names: Vec<&str> = Vec::new();
    let branches_ready = matches!(repo.branches, Loadable::Ready(_));
    if let Loadable::Ready(branches) = &repo.branches {
        for branch in branches.iter() {
            local_names.push(branch.name.as_str());
            if head_branch == Some(branch.name.as_str()) {
                marked_index = Some(items.len());
            }
            items.push(branch_row(
                repo,
                branch.name.clone(),
                &branch.name,
                "icons/git_branch.svg",
                LOCAL_SECTION,
                now,
            ));
            rows.push(BranchPickerNavTarget::Ref(branch.name.clone()));
        }
    }

    if let Loadable::Ready(remote_branches) = &repo.remote_branches {
        for remote_branch in remote_branches.iter() {
            // `refs/remotes/<remote>/HEAD` is a symref, not a branch anyone
            // checks out by that name.
            if remote_branch.name == "HEAD" {
                continue;
            }
            let display = format!("{}/{}", remote_branch.remote, remote_branch.name);
            items.push(branch_row(
                repo,
                display.clone(),
                &display,
                "icons/cloud.svg",
                REMOTE_SECTION,
                now,
            ));
            rows.push(BranchPickerNavTarget::RemoteBranch {
                remote: remote_branch.remote.clone(),
                branch: remote_branch.name.clone(),
            });
        }
    }

    // Offer creation whenever the typed name is not already a local branch —
    // not merely when nothing matched, since a query hitting only a remote
    // branch still leaves creating a local branch of that name legal.
    //
    // Gated on the branch list being loaded: with an empty `local_names` an
    // in-flight (or failed) load would turn "switch to main" into "create main",
    // which git then rejects. The comparison is case-insensitive to match how
    // `match_items` filters, so `MAIN` does not offer to create a near-duplicate
    // of `main` (a hard failure on case-insensitive filesystems).
    let query = query.trim();
    let already_exists = local_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(query));
    if branches_ready && !query.is_empty() && !already_exists {
        // Section-less, so it renders last with no header. Its searchable text
        // is the query itself, so no filter can drop it. The base ref sits on a
        // detail line, which also keeps this row the same height as the branch
        // rows above it.
        let base = head_branch.unwrap_or("HEAD");
        items.push(
            components::PickerPromptItem::from_parts([
                components::PickerPromptItemPart::new("Create branch ")
                    .flexible(false)
                    .searchable(false)
                    .tooltip(false),
                components::PickerPromptItemPart::new(query.to_string()).flexible(false),
            ])
            .secondary_parts([
                components::PickerPromptItemPart::new(format!("Based off {base}"))
                    .searchable(false),
            ])
            .icon("icons/plus.svg"),
        );
        rows.push(BranchPickerNavTarget::CreateBranch(query.to_string()));
    }

    BranchRows {
        items,
        rows,
        marked_index,
    }
}

/// The cached view of the rows for `query`: the items, the filtered layout, and
/// the payload of every row that survived the filter.
///
/// Every caller goes through the cache, so a frame that only repaints (a hover
/// moving between rows is the common one) reuses the rows instead of rebuilding
/// every branch label.
pub(super) fn cached(
    this: &PopoverHost,
    query: &str,
) -> Rc<super::rows_cache::CachedRows<BranchPickerNavTarget>> {
    let Some(repo) = this.active_repo() else {
        return super::rows_cache::CachedRows::empty();
    };
    let key = super::rows_cache::RowsCacheKey::for_branch_checkout(repo, query);
    super::rows_cache::get_or_build(&this.branch_picker_rows_cache, key, |now| {
        let built = rows(repo, query, now);
        (built.items, built.rows, built.marked_index)
    })
}

pub(super) fn nav_targets(this: &PopoverHost, query: &str) -> Vec<BranchPickerNavTarget> {
    cached(this, query).filtered_payloads()
}

/// Activates a checkout-picker row.
pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: BranchPickerNavTarget,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match target {
        BranchPickerNavTarget::Ref(name) => {
            this.handle_inline_branch_picker_select(name, repo_id, window, cx);
        }
        BranchPickerNavTarget::RemoteBranch { remote, branch } => {
            // Hand off to the existing prompt, which names the local branch and
            // already guards against colliding with an existing one.
            this.open_popover_centered(
                PopoverKind::CheckoutRemoteBranchPrompt {
                    repo_id,
                    remote,
                    branch,
                },
                window,
                cx,
            );
        }
        BranchPickerNavTarget::CreateBranch(name) => {
            let target = this
                .active_repo()
                .and_then(|repo| match &repo.head_branch {
                    Loadable::Ready(head) => Some(head.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "HEAD".to_string());
            this.store.dispatch(Msg::CreateBranchAndCheckout {
                repo_id,
                name,
                target,
            });
            this.close_popover(cx);
        }
    }
}

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);
    let is_checkout = is_checkout_picker(this);
    let width = if is_checkout {
        super::LARGE_PICKER_WIDTH
    } else {
        super::PICKER_WIDTH
    };
    let is_delete = matches!(
        this.popover,
        Some(PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::Delete
        })
    );
    let is_rebase_onto = matches!(
        this.popover,
        Some(PopoverKind::BranchPicker {
            purpose: BranchPickerPurpose::RebaseOnto
        })
    );
    let title = if is_delete {
        "Delete Branch"
    } else if is_rebase_onto {
        "Rebase Onto"
    } else {
        "Checkout Branch"
    };

    let mut menu = div()
        .flex()
        .flex_col()
        .min_w(width.min_px(ui_scale))
        .max_w(width.max_px(ui_scale))
        .child(popover_title(title))
        .child(div().border_t_1().border_color(theme.colors.border));

    // The checkout picker renders sectioned, metadata-bearing rows and a create
    // row, so it drives PickerPrompt directly rather than through
    // BranchRefPicker (which only builds plain single-part rows).
    if is_checkout {
        let Some(search) = this.branch_picker_search_input.clone() else {
            return components::context_menu(theme, menu).w(width.preferred_px(ui_scale));
        };
        let repo_id = this.active_repo().map(|repo| repo.id);
        // Read the query from the same input PickerPrompt filters with, so the
        // rows built here are the rows it displays.
        let query = search.read_with(cx, |input, _| input.text().trim().to_string());
        let built = cached(this, &query);
        let row_payloads = Rc::clone(&built.payloads);
        let empty_text = match this.active_repo().map(|repo| &repo.branches) {
            Some(Loadable::Loading) | Some(Loadable::NotLoaded) => "Loading",
            Some(Loadable::Error(_)) => "Could not list branches",
            _ => "No branches",
        };

        menu = menu.child(
            components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
                // Prebuilt items and layout: the cache already filtered and
                // sorted them, so `render` must not repeat that work.
                .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
                // Long ref lists render only what is on screen; keyboard
                // navigation scrolls by the row geometry to match
                // (`scroll_picker_prompt_to_row`).
                .windowed_rows()
                .tooltip_host(this.tooltip_host.clone())
                .empty_text(empty_text)
                .max_height(scaled_px(components::PICKER_LIST_MAX_HEIGHT_PX))
                .selected_index(this.branch_picker_selected_index)
                .marked_index(built.marked_index)
                .render(
                    theme,
                    ui_scale_percent,
                    cx,
                    move |this, ix, _e, window, cx| {
                        let (Some(repo_id), Some(target)) =
                            (repo_id, row_payloads.get(ix).cloned())
                        else {
                            return;
                        };
                        activate(this, repo_id, target, window, cx);
                    },
                ),
        );

        return components::context_menu(theme, menu).w(width.preferred_px(ui_scale));
    }

    if let Some(repo) = this.active_repo() {
        match &repo.branches {
            Loadable::Ready(branches) => {
                if let Some(search) = this.branch_picker_search_input.clone() {
                    let repo_id = repo.id;
                    let head_branch = match &repo.head_branch {
                        Loadable::Ready(head) => Some(head.as_str()),
                        _ => None,
                    };
                    let branch_names = branches
                        .iter()
                        .filter_map(|b| {
                            // The current branch cannot be rebased onto itself;
                            // deleting it is impossible too.
                            if (is_delete || is_rebase_onto) && head_branch == Some(b.name.as_str())
                            {
                                None
                            } else {
                                Some(b.name.clone())
                            }
                        })
                        .collect::<Vec<_>>();
                    let items = branch_names
                        .iter()
                        .cloned()
                        .map(components::BranchRefPickerItem::branch)
                        .collect::<Vec<_>>();
                    let marked_name = head_branch.map(str::to_string);

                    menu = menu.child(
                        components::BranchRefPicker::new(
                            search,
                            this.picker_prompt_scroll.clone(),
                            items,
                        )
                        .tooltip_host(this.tooltip_host.clone())
                        .empty_text("No branches")
                        .max_height(scaled_px(240.0))
                        .selected_index(this.branch_picker_selected_index)
                        .marked_name(marked_name)
                        .render(
                            theme,
                            ui_scale_percent,
                            cx,
                            move |this, name, _e, window, cx| {
                                this.handle_inline_branch_picker_select(name, repo_id, window, cx);
                            },
                        ),
                    );
                } else {
                    for (ix, branch) in branches.iter().enumerate() {
                        let repo_id = repo.id;
                        let name = branch.name.clone();
                        let label: SharedString = name.clone().into();
                        menu = menu.child(
                            components::ContextMenuEntry::new(
                                ("branch_item", ix),
                                components::ContextMenuText::new(label)
                                    .max_lines(1)
                                    .tooltip_mode(
                                        components::TruncatedTextTooltipMode::FullTextIfTruncated,
                                    ),
                            )
                            .tooltip_host(this.tooltip_host.clone())
                            .render(theme, ui_scale_percent, cx)
                            .on_click(cx.listener(
                                move |this, _e: &ClickEvent, window, cx| {
                                    this.handle_inline_branch_picker_select(
                                        name.clone(),
                                        repo_id,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        );
                    }
                }
            }
            Loadable::Loading => {
                menu = menu.child(branch_picker_status_panel(this, "Loading", cx));
            }
            Loadable::Error(e) => {
                menu = menu.child(branch_picker_status_panel(this, e.clone(), cx));
            }
            Loadable::NotLoaded => {
                menu = menu.child(branch_picker_status_panel(this, "Not loaded", cx));
            }
        }
    }

    // Fixed width: PickerPrompt rows size with `w_full`, which does not
    // stretch under fit-content parents.
    components::context_menu(theme, menu).w(width.preferred_px(ui_scale))
}

fn branch_picker_status_panel(
    this: &mut PopoverHost,
    empty_text: impl Into<SharedString>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = |value: f32| super::popover_scaled_px_from_percent(value, ui_scale_percent);

    if let Some(search) = this.branch_picker_search_input.clone() {
        components::BranchRefPicker::new(search, this.picker_prompt_scroll.clone(), Vec::new())
            .tooltip_host(this.tooltip_host.clone())
            .empty_text(empty_text)
            .max_height(scaled_px(240.0))
            .selected_index(this.branch_picker_selected_index)
            .render(theme, ui_scale_percent, cx, |_, _, _, _, _| {})
    } else {
        components::context_menu_label(
            theme,
            ui_scale_percent,
            empty_text.into(),
            Some(this.tooltip_host.clone()),
            cx,
        )
    }
}
