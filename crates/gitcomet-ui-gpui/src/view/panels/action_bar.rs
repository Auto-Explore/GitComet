use super::*;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const ACTION_BAR_HEIGHT_PX: f32 = components::CONTROL_HEIGHT_PX + 12.0;

// The main window supports widths down to 820 design pixels. At that size the
// branch/upstream run and the fixed actions on the right cannot all retain full
// text. Pull/Push always retain their labels because they describe the remote
// operation beside the upstream badge. Only compact mode hides the remaining
// action labels; condensed mode keeps them while using tighter spacing.
// Tooltips keep omitted labels available in compact mode.
const COMPACT_ACTION_BAR_MAX_WIDTH_PX: f32 = 1120.0;
const CONDENSED_ACTION_BAR_MAX_WIDTH_PX: f32 = 1400.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super) enum ActionBarDensity {
    Compact,
    Condensed,
    Wide,
}

pub(in super::super) fn action_bar_density(
    window_width: Pixels,
    ui_scale_percent: u32,
) -> ActionBarDensity {
    if window_width
        <= crate::ui_scale::design_px_from_percent(
            COMPACT_ACTION_BAR_MAX_WIDTH_PX,
            ui_scale_percent,
        )
    {
        ActionBarDensity::Compact
    } else if window_width
        <= crate::ui_scale::design_px_from_percent(
            CONDENSED_ACTION_BAR_MAX_WIDTH_PX,
            ui_scale_percent,
        )
    {
        ActionBarDensity::Condensed
    } else {
        ActionBarDensity::Wide
    }
}

fn secondary_action_label(density: ActionBarDensity, label: &'static str) -> &'static str {
    if density == ActionBarDensity::Compact {
        ""
    } else {
        label
    }
}

pub(in super::super) fn action_bar_height<C>(cx: &mut C) -> Pixels
where
    C: gpui::BorrowAppContext,
{
    crate::ui_scale::design_px(ACTION_BAR_HEIGHT_PX, cx)
}

/// Longest badge label rendered before eliding. `components::Button` takes a
/// plain string with no truncation of its own, so an unbounded branch name or
/// folder name would squeeze other actions off the right edge. The full value
/// stays available in each badge's tooltip.
const BADGE_LABEL_MAX_CHARS: usize = 28;
const CONDENSED_BADGE_LABEL_MAX_CHARS: usize = 16;
const COMPACT_BADGE_LABEL_MAX_CHARS: usize = 10;

fn truncate_badge_label_to(label: &str, max_chars: usize) -> SharedString {
    let mut chars = label.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…").into()
    } else {
        head.into()
    }
}

#[cfg(test)]
fn truncate_badge_label(label: &str) -> SharedString {
    truncate_badge_label_to(label, BADGE_LABEL_MAX_CHARS)
}

fn head_branch_tracking_upstream_name(
    head_branch: &Loadable<String>,
    branches: &Loadable<Arc<Vec<Branch>>>,
) -> Option<String> {
    let Loadable::Ready(head) = head_branch else {
        return None;
    };
    let Loadable::Ready(branches) = branches else {
        return None;
    };
    branches
        .iter()
        .find(|branch| branch.name == *head)
        .and_then(|branch| branch.upstream.as_ref())
        .map(|upstream| format!("{}/{}", upstream.remote, upstream.branch))
}

/// Returns the local branch that owns the upstream badge and its optional
/// tracking target. Badge visibility intentionally depends only on having a
/// checked-out local branch: an absent (or not-yet-loaded) upstream is the
/// state in which the badge offers to configure one, not a reason to hide it.
fn upstream_badge_state<'a>(
    head_branch: &'a Loadable<String>,
    branches: &Loadable<Arc<Vec<Branch>>>,
) -> Option<(&'a str, Option<String>)> {
    let Loadable::Ready(branch) = head_branch else {
        return None;
    };
    if branch.is_empty() || branch == "HEAD" {
        return None;
    }
    Some((
        branch.as_str(),
        head_branch_tracking_upstream_name(head_branch, branches),
    ))
}

fn pull_tooltip_text(pull_count: usize, tracking_branch_name: Option<&str>) -> SharedString {
    match tracking_branch_name {
        Some(name) => format!("Pull {pull_count} behind\n{name}").into(),
        None => format!("Pull {pull_count} behind").into(),
    }
}

fn push_tooltip_text(push_count: usize, tracking_branch_name: Option<&str>) -> SharedString {
    match tracking_branch_name {
        Some(name) => format!("Push {push_count} ahead\n{name}").into(),
        None => format!("Push {push_count} ahead").into(),
    }
}

pub(in super::super) struct ActionBarView {
    store: Arc<AppStore>,
    state: Arc<AppState>,
    theme: AppTheme,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    notify_fingerprint: u64,
    active_context_menu_invoker: Option<SharedString>,
    open_terminal_repo_ids: FxHashSet<RepoId>,
    action_bar_terminal_target: ActionBarTerminalTarget,
}

impl ActionBarView {
    fn notify_fingerprint(state: &AppState) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.open_rev.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.remotes_rev.hash(&mut hasher);
            repo.remote_branches_rev.hash(&mut hasher);
            repo.upstream_divergence_rev.hash(&mut hasher);
            repo.merge_message_rev.hash(&mut hasher);
            repo.ops_rev.hash(&mut hasher);
            repo.status_cache_rev().hash(&mut hasher);
            // The historical-browse badge keys off the file browser source.
            repo.file_browser.file_browser_rev.hash(&mut hasher);
            repo.loads_in_flight.any_in_flight().hash(&mut hasher);
            // Global back/forward buttons enable/disable with nav stack position.
            repo.navigation.main_history.cursor.hash(&mut hasher);
            repo.navigation.main_history.entries.len().hash(&mut hasher);
        }

        hasher.finish()
    }

    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let notify_fingerprint = Self::notify_fingerprint(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint(&next);

            this.state = next;
            if next_fingerprint != this.notify_fingerprint {
                this.notify_fingerprint = next_fingerprint;
                cx.notify();
            }
        });

        Self {
            store,
            state,
            theme,
            _ui_model_subscription: subscription,
            root_view,
            notify_fingerprint,
            active_context_menu_invoker: None,
            open_terminal_repo_ids: FxHashSet::default(),
            action_bar_terminal_target: ActionBarTerminalTarget::default(),
        }
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    pub(in super::super) fn set_open_terminal_repo_ids(
        &mut self,
        next: FxHashSet<RepoId>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.open_terminal_repo_ids == next {
            return;
        }
        self.open_terminal_repo_ids = next;
        cx.notify();
    }

    pub(in super::super) fn set_action_bar_terminal_target(
        &mut self,
        target: ActionBarTerminalTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.action_bar_terminal_target == target {
            return;
        }
        self.action_bar_terminal_target = target;
        cx.notify();
    }

    fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_at(kind, anchor, window, cx);
        });
    }

    fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
        });
    }

    fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    fn push_toast(
        &mut self,
        kind: components::ToastKind,
        message: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.push_toast(kind, message, cx);
        });
    }
}

impl Render for ActionBarView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let action_bar_height = action_bar_height(cx);
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let density = action_bar_density(window.viewport_size().width, ui_scale_percent);
        let dense_spacing = density != ActionBarDensity::Wide;
        let badge_label_max_chars = match density {
            ActionBarDensity::Compact => COMPACT_BADGE_LABEL_MAX_CHARS,
            ActionBarDensity::Condensed => CONDENSED_BADGE_LABEL_MAX_CHARS,
            ActionBarDensity::Wide => BADGE_LABEL_MAX_CHARS,
        };
        let action_label = |label: &'static str| secondary_action_label(density, label);
        let action_group_gap = if dense_spacing {
            scaled_px(4.0)
        } else {
            scaled_px(8.0)
        };
        let tracking_action_gap = if dense_spacing {
            scaled_px(2.0)
        } else {
            scaled_px(4.0)
        };
        let action_bar_padding_x = if dense_spacing {
            scaled_px(4.0)
        } else {
            scaled_px(8.0)
        };
        let merge_abort_label = if dense_spacing {
            "Abort"
        } else {
            "Abort merge"
        };
        let icon_primary = theme.colors.accent.foreground;
        let icon_muted = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.72 } else { 0.82 },
        );
        let icon = |path: &'static str, color: gpui::Rgba| svg_icon(path, color, scaled_px(14.0));
        let spinner =
            |id: (&'static str, u64), color: gpui::Rgba| svg_spinner(id, color, scaled_px(14.0));
        let count_badge = |count: usize, color: gpui::Rgba| {
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(count.to_string())
                .into_any_element()
        };

        // Workspace, branch and historical badges all light up while their own
        // picker is open, so they need the active invoker before any of them.
        let menu_selected_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.26 } else { 0.20 },
        );
        let active_invoker = self.active_context_menu_invoker.clone();

        // Badge shown next to the selectors when the file directory is pinned to
        // a historical commit (not the live state). Click → back to live. Same
        // geometry and behaviour as the workspace/branch badges, in the fixed
        // "off-live" purple rather than the theme accent.
        let historical_badge = self
            .active_repo()
            .and_then(|repo| {
                repo.browsing_commit().map(|commit_id| {
                    let sha = commit_id.as_ref().to_string();
                    let short: SharedString = sha.get(0..8).unwrap_or(&sha).to_string().into();
                    (repo.id, sha, short)
                })
            })
            .map(|(repo_id, sha, short)| {
                let purple = crate::theme::historical_outline(theme.is_dark);
                let invoker: SharedString = "historical_browse_badge".into();
                let is_active = active_invoker
                    .as_ref()
                    .is_some_and(|id| id.as_ref() == invoker.as_ref());
                components::Button::new("historical_browse_badge", short)
                    .start_slot(icon("icons/history.svg", purple))
                    .style(components::ButtonStyle::Subtle)
                    .text_color(purple)
                    .bg(with_alpha(purple, 0.12))
                    .hover_bg(with_alpha(purple, if theme.is_dark { 0.22 } else { 0.18 }))
                    .selected(is_active)
                    .selected_bg(with_alpha(purple, if theme.is_dark { 0.30 } else { 0.24 }))
                    .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                        this.activate_context_menu_invoker(invoker.clone(), cx);
                        this.open_popover_for_bounds(
                            PopoverKind::BrowseHistoryMenu { repo_id },
                            bounds,
                            window,
                            cx,
                        );
                    })
                    .debug_selector(|| "historical_browse_badge".to_string())
                    .gitcomet_tooltip(
                        theme,
                        format!("Browsing commit {sha} — click for history / go live").into(),
                    )
            });

        let is_merging = self
            .active_repo()
            .is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))));
        let sequencer_state = self
            .active_repo()
            .map(|repo| match repo.sequencer_state {
                Loadable::Ready(state) => state,
                _ if matches!(&repo.rebase_in_progress, Loadable::Ready(true)) => {
                    gitcomet_core::services::SequencerState::RebaseOrApply
                }
                _ => gitcomet_core::services::SequencerState::None,
            })
            .unwrap_or_default();
        let is_cherry_pick_in_progress =
            sequencer_state == gitcomet_core::services::SequencerState::CherryPick;
        let is_rebase_or_apply_in_progress =
            sequencer_state == gitcomet_core::services::SequencerState::RebaseOrApply;
        let sequencer_label = if is_cherry_pick_in_progress {
            "CHERRY-PICKING"
        } else {
            "APPLY/REBASE"
        };
        let sequencer_abort_id = if is_cherry_pick_in_progress {
            "abort_cherry_pick"
        } else {
            "abort_rebase_or_apply"
        };
        let sequencer_continue_id = if is_cherry_pick_in_progress {
            "continue_cherry_pick"
        } else {
            "continue_rebase_or_apply"
        };
        let sequencer_continue_tooltip = if is_cherry_pick_in_progress {
            "Continue the in-progress cherry-pick"
        } else {
            "Continue the in-progress rebase or apply"
        };
        let rebase_has_unstaged_conflicts =
            self.active_repo().is_some_and(|r| r.has_unstaged_conflicts);

        let (pull_count, push_count) = self
            .active_repo()
            .and_then(|r| match &r.upstream_divergence {
                Loadable::Ready(Some(d)) => Some((d.behind, d.ahead)),
                _ => None,
            })
            .unwrap_or((0, 0));
        let (pull_loading, push_loading) = self
            .active_repo()
            .map(|r| (r.pull_in_flight > 0, r.push_in_flight > 0))
            .unwrap_or((false, false));
        let tracking_branch_name = self
            .active_repo()
            .and_then(|repo| head_branch_tracking_upstream_name(&repo.head_branch, &repo.branches));
        let active_repo_key = self.active_repo_id().map(|id| id.0).unwrap_or(0);
        let pull_default_enabled = self
            .active_repo()
            .is_some_and(head_branch_has_live_upstream);

        let can_stash = self
            .active_repo()
            .map(|repo| {
                repo.worktree_status_entries()
                    .is_some_and(|entries| !entries.is_empty())
                    || repo
                        .staged_status_entries()
                        .is_some_and(|entries| !entries.is_empty())
            })
            .unwrap_or(false);

        // Global back/forward navigation, mirroring the mouse side-buttons and
        // Alt+Left / Alt+Right (Option on macOS). Sits at the very start of the action bar.
        let (nav_can_back, nav_can_forward) = self
            .active_repo()
            .map(|repo| {
                (
                    repo.navigation.main_history.can_back(),
                    repo.navigation.main_history.can_forward(),
                )
            })
            .unwrap_or((false, false));
        let nav_back = components::Button::new("global_nav_back", "")
            .start_slot(icon(
                "icons/arrow_left.svg",
                if nav_can_back {
                    theme.colors.foreground.primary
                } else {
                    theme.colors.foreground.secondary
                },
            ))
            .style(components::ButtonStyle::Transparent)
            .disabled(!nav_can_back)
            .on_click(theme, cx, |this, _e, _w, _cx| {
                if let Some(repo_id) = this.active_repo_id() {
                    this.store.dispatch(Msg::GlobalNavBack { repo_id });
                }
            })
            .gitcomet_tooltip(
                theme,
                format!(
                    "Navigate Back ({})",
                    crate::view::shortcut_labels::alt_shortcut("Left")
                )
                .into(),
            );
        let nav_forward = components::Button::new("global_nav_forward", "")
            .start_slot(icon(
                "icons/arrow_right.svg",
                if nav_can_forward {
                    theme.colors.foreground.primary
                } else {
                    theme.colors.foreground.secondary
                },
            ))
            .style(components::ButtonStyle::Transparent)
            .disabled(!nav_can_forward)
            .on_click(theme, cx, |this, _e, _w, _cx| {
                if let Some(repo_id) = this.active_repo_id() {
                    this.store.dispatch(Msg::GlobalNavForward { repo_id });
                }
            })
            .gitcomet_tooltip(
                theme,
                format!(
                    "Navigate Forward ({})",
                    crate::view::shortcut_labels::alt_shortcut("Right")
                )
                .into(),
            );
        let global_nav = div()
            .id("global_nav")
            .debug_selector(|| "global_nav".to_string())
            .flex()
            .items_center()
            .gap(px(2.0))
            .flex_none()
            .child(nav_back)
            .child(nav_forward);

        // Workspace (worktree) and branch badges show the current state at a
        // glance, with each opening a filterable picker.
        let workspace_badge = self.active_repo().map(|repo| {
            let repo_id = repo.id;
            let label = truncate_badge_label_to(
                &crate::view::path_display::repo_path_name(&repo.spec.workdir),
                badge_label_max_chars,
            );
            let workdir = repo.spec.workdir.display().to_string();
            let invoker: SharedString = "workspace_badge".into();
            let is_active = active_invoker
                .as_ref()
                .is_some_and(|id| id.as_ref() == invoker.as_ref());
            components::Button::new("workspace_badge", label.clone())
                .start_slot(icon("icons/git_worktree.svg", icon_primary))
                .style(components::ButtonStyle::Subtle)
                .selected(is_active)
                .selected_bg(menu_selected_bg)
                .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                    this.activate_context_menu_invoker(invoker.clone(), cx);
                    this.open_popover_for_bounds(
                        PopoverKind::worktree(repo_id, WorktreePopoverKind::BadgePicker),
                        bounds,
                        window,
                        cx,
                    );
                })
                .debug_selector(|| "workspace_badge".to_string())
                .gitcomet_tooltip(theme, format!("Switch worktree\n{workdir}").into())
        });

        let branch_badge = self.active_repo().and_then(|repo| {
            let Loadable::Ready(head) = &repo.head_branch else {
                return None;
            };
            // Detached HEAD surfaces as the literal "HEAD"; label it as such
            // rather than pretending it is a branch.
            let detached = head == "HEAD";
            let label: SharedString = if detached {
                "detached".into()
            } else {
                truncate_badge_label_to(head, badge_label_max_chars)
            };
            let invoker: SharedString = "branch_badge".into();
            let is_active = active_invoker
                .as_ref()
                .is_some_and(|id| id.as_ref() == invoker.as_ref());
            let tooltip: SharedString = if detached {
                "Detached HEAD — click to check out a branch".into()
            } else {
                format!("On branch {head} — click to switch").into()
            };
            Some(
                components::Button::new("branch_badge", label)
                    .start_slot(icon("icons/git_branch.svg", icon_primary))
                    .style(components::ButtonStyle::Subtle)
                    .selected(is_active)
                    .selected_bg(menu_selected_bg)
                    .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                        this.activate_context_menu_invoker(invoker.clone(), cx);
                        this.open_popover_for_bounds(
                            PopoverKind::BranchPicker {
                                purpose: BranchPickerPurpose::Checkout,
                            },
                            bounds,
                            window,
                            cx,
                        );
                    })
                    .debug_selector(|| "branch_badge".to_string())
                    .gitcomet_tooltip(theme, tooltip),
            )
        });

        let upstream_badge = self.active_repo().and_then(|repo| {
            let (branch, upstream) = upstream_badge_state(&repo.head_branch, &repo.branches)?;
            let has_upstream = upstream.is_some();
            let repo_id = repo.id;
            let local_branch = branch.to_string();
            let label = upstream
                .as_deref()
                .map(|upstream| truncate_badge_label_to(upstream, badge_label_max_chars))
                .unwrap_or_else(|| "No upstream".into());
            let badge_color = if has_upstream {
                icon_primary
            } else {
                theme.colors.foreground.secondary
            };
            let tooltip: SharedString = upstream.as_deref().map_or_else(
                || "No upstream branch configured\nClick to select one".into(),
                |upstream| {
                    format!("Tracking upstream {upstream}\nClick to change or unlink").into()
                },
            );
            let invoker: SharedString = "upstream_badge".into();
            let is_active = active_invoker
                .as_ref()
                .is_some_and(|id| id.as_ref() == invoker.as_ref());
            Some(
                components::Button::new("upstream_badge", label)
                    .start_slot(icon("icons/cloud.svg", badge_color))
                    .style(components::ButtonStyle::Subtle)
                    .text_color(if has_upstream {
                        theme.colors.foreground.primary
                    } else {
                        theme.colors.foreground.secondary
                    })
                    .selected(is_active)
                    .selected_bg(menu_selected_bg)
                    .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                        this.activate_context_menu_invoker(invoker.clone(), cx);
                        this.open_popover_for_bounds(
                            PopoverKind::UpstreamPicker {
                                repo_id,
                                branch: local_branch.clone(),
                            },
                            bounds,
                            window,
                            cx,
                        );
                    })
                    .debug_selector(|| "upstream_badge".to_string())
                    .gitcomet_tooltip(theme, tooltip),
            )
        });
        let upstream_arrow = upstream_badge.is_some().then(|| {
            div()
                .debug_selector(|| "upstream_arrow".to_string())
                .flex()
                .items_center()
                .child(svg_icon(
                    "icons/arrow_right.svg",
                    theme.colors.foreground.secondary,
                    scaled_px(12.0),
                ))
        });

        let pull_color = if pull_count > 0 {
            theme.colors.status.warning.foreground
        } else {
            icon_muted
        };
        let mut pull_main = components::Button::new("pull_main", "Pull")
            .rounded_left()
            .start_slot(if pull_loading {
                spinner(("pull_spinner", active_repo_key), pull_color).into_any_element()
            } else {
                icon("icons/arrow_down.svg", pull_color).into_any_element()
            })
            .style(components::ButtonStyle::Subtle);
        if pull_count > 0 {
            pull_main = pull_main.end_slot(count_badge(pull_count, pull_color));
        }
        let pull_picker_invoker: SharedString = "pull_picker".into();
        let pull_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == pull_picker_invoker.as_ref());
        let pull_tracking_branch_name = tracking_branch_name.clone();
        let pull_request_enabled = self
            .active_repo()
            .is_some_and(|repo| matches!(pull_request(repo), PullRequest::Pull));
        let pull_menu_icon_color = if pull_picker_active {
            theme.colors.accent.foreground
        } else {
            icon_muted
        };
        let pull_menu = components::Button::new("pull_menu", "")
            .rounded_right()
            .start_slot(icon("icons/chevron_down.svg", pull_menu_icon_color))
            .style(components::ButtonStyle::Subtle)
            .selected(pull_picker_active)
            .selected_bg(menu_selected_bg);

        let pull = div()
            .id("pull")
            .debug_selector(|| "pull".to_string())
            .child(
                components::SplitButton::new(
                    pull_main
                        .disabled(!pull_default_enabled || !pull_request_enabled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            let Some(repo) = this.active_repo() else {
                                return;
                            };
                            let repo_id = repo.id;
                            match pull_request(repo) {
                                PullRequest::Pull => this.store.dispatch(Msg::Pull {
                                    repo_id,
                                    mode: PullMode::Default,
                                }),
                                PullRequest::NoRemotes => this.push_toast(
                                    components::ToastKind::Error,
                                    "Cannot pull: no remotes configured".to_string(),
                                    cx,
                                ),
                                PullRequest::NotReady => {}
                            }
                        }),
                    pull_menu.on_click_with_bounds(
                        theme,
                        cx,
                        move |this, _e, bounds, window, cx| {
                            this.activate_context_menu_invoker(pull_picker_invoker.clone(), cx);
                            this.open_popover_for_bounds(
                                PopoverKind::PullPicker,
                                bounds,
                                window,
                                cx,
                            );
                        },
                    ),
                )
                .style(components::SplitButtonStyle::Borderless)
                .render(theme, ui_scale_percent),
            )
            .gitcomet_tooltip(
                theme,
                pull_tooltip_text(pull_count, pull_tracking_branch_name.as_deref()),
            );

        let push_color = if push_count > 0 {
            theme.colors.status.success.foreground
        } else {
            icon_muted
        };
        let terminal_opens_external =
            self.action_bar_terminal_target == ActionBarTerminalTarget::External;
        let terminal_is_open = !terminal_opens_external
            && self
                .active_repo_id()
                .is_some_and(|repo_id| self.open_terminal_repo_ids.contains(&repo_id));
        let terminal_tooltip: SharedString = if terminal_opens_external {
            "Open external terminal".into()
        } else if terminal_is_open {
            "Hide terminal".into()
        } else {
            "Show terminal".into()
        };
        let terminal = div().debug_selector(|| "terminal".to_string()).child(
            components::Button::new("terminal", action_label("Terminal"))
                .start_slot(icon("icons/terminal.svg", icon_primary))
                .style(components::ButtonStyle::Subtle)
                .selected(terminal_is_open)
                .selected_bg(menu_selected_bg)
                .disabled(self.active_repo_id().is_none())
                .on_click(theme, cx, move |this, _e, window, cx| {
                    let _ = this.root_view.update(cx, |root, cx| {
                        root.activate_terminal_button_for_active_repo(window, cx);
                    });
                })
                .gitcomet_tooltip(theme, terminal_tooltip),
        );
        let mut push_main = components::Button::new("push_main", "Push")
            .rounded_left()
            .start_slot(if push_loading {
                spinner(("push_spinner", active_repo_key), push_color).into_any_element()
            } else {
                icon("icons/arrow_up.svg", push_color).into_any_element()
            })
            .style(components::ButtonStyle::Subtle);
        if push_count > 0 {
            push_main = push_main.end_slot(count_badge(push_count, push_color));
        }
        let push_picker_invoker: SharedString = "push_picker".into();
        let push_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == push_picker_invoker.as_ref());
        let push_tracking_branch_name = tracking_branch_name.clone();
        let push_request_ready = self
            .active_repo()
            .is_some_and(|repo| !matches!(push_request(repo), PushRequest::NotReady));
        let push_menu_icon_color = if push_picker_active {
            theme.colors.accent.foreground
        } else {
            icon_muted
        };
        let push_menu = components::Button::new("push_menu", "")
            .rounded_right()
            .start_slot(icon("icons/chevron_down.svg", push_menu_icon_color))
            .style(components::ButtonStyle::Subtle)
            .selected(push_picker_active)
            .selected_bg(menu_selected_bg);

        let push = div()
            .id("push")
            .debug_selector(|| "push".to_string())
            .child(
                components::SplitButton::new(
                    push_main.disabled(!push_request_ready).on_click(
                        theme,
                        cx,
                        |this, e, window, cx| {
                            let Some(repo) = this.active_repo() else {
                                return;
                            };
                            let repo_id = repo.id;
                            match push_request(repo) {
                                PushRequest::Push => this.store.dispatch(Msg::Push { repo_id }),
                                PushRequest::SetUpstream { remote } => this.open_popover_at(
                                    PopoverKind::PushSetUpstreamPrompt {
                                        repo_id,
                                        remote,
                                        configure_only_for: None,
                                    },
                                    e.position(),
                                    window,
                                    cx,
                                ),
                                PushRequest::NoRemotes => {
                                    this.push_toast(
                                        components::ToastKind::Error,
                                        "Cannot push: no remotes configured".to_string(),
                                        cx,
                                    );
                                }
                                PushRequest::NotReady => {}
                            }
                        },
                    ),
                    push_menu.on_click_with_bounds(
                        theme,
                        cx,
                        move |this, _e, bounds, window, cx| {
                            this.activate_context_menu_invoker(push_picker_invoker.clone(), cx);
                            this.open_popover_for_bounds(
                                PopoverKind::PushPicker,
                                bounds,
                                window,
                                cx,
                            );
                        },
                    ),
                )
                .style(components::SplitButtonStyle::Borderless)
                .render(theme, ui_scale_percent),
            )
            .gitcomet_tooltip(
                theme,
                push_tooltip_text(push_count, push_tracking_branch_name.as_deref()),
            );

        let stash_prompt_invoker: SharedString = "stash_btn".into();
        let stash_prompt_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == stash_prompt_invoker.as_ref());
        let stash = div().debug_selector(|| "stash".to_string()).child(
            components::Button::new("stash", action_label("Stash"))
                .start_slot(icon(crate::view::icons::STASH_ICON_PATH, icon_primary))
                .style(components::ButtonStyle::Subtle)
                .selected(stash_prompt_active)
                .selected_bg(menu_selected_bg)
                .disabled(!can_stash)
                .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                    this.activate_context_menu_invoker(stash_prompt_invoker.clone(), cx);
                    this.open_popover_for_bounds(PopoverKind::StashPrompt, bounds, window, cx);
                })
                .gitcomet_tooltip(
                    theme,
                    if can_stash {
                        "Create stash".into()
                    } else {
                        "No changes to stash".into()
                    },
                ),
        );

        let create_branch_invoker: SharedString = "create_branch_btn".into();
        let create_branch_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == create_branch_invoker.as_ref());
        let create_branch = div().debug_selector(|| "create_branch".to_string()).child(
            components::Button::new("create_branch", action_label("Branch"))
                .start_slot(icon("icons/git_branch.svg", icon_primary))
                .style(components::ButtonStyle::Subtle)
                .selected(create_branch_active)
                .selected_bg(menu_selected_bg)
                .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                    this.activate_context_menu_invoker(create_branch_invoker.clone(), cx);
                    if let Some(repo_id) = this.state.active_repo {
                        let target = this
                            .active_repo()
                            .and_then(|repo| {
                                if let Loadable::Ready(head) = &repo.head_branch {
                                    Some(head.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| "HEAD".to_string());
                        this.open_popover_for_bounds(
                            PopoverKind::CreateBranchFromRefPrompt {
                                repo_id,
                                target,
                                source_selectable: true,
                                name_prefix: String::new(),
                            },
                            bounds,
                            window,
                            cx,
                        );
                    }
                })
                .gitcomet_tooltip(theme, "Create branch".into()),
        );

        // Keep the branch, its tracking target, and the operations that use
        // that target together. Pull/Push stay in this run even before the
        // branch has an upstream, so the action bar does not jump around after
        // the first push.
        let tracking_actions = div()
            .id("tracking_actions")
            .debug_selector(|| "tracking_actions".to_string())
            .flex()
            .flex_none()
            .items_center()
            .gap(tracking_action_gap)
            .children(branch_badge)
            .children(upstream_arrow)
            .children(upstream_badge)
            .child(pull)
            .child(push);

        div()
            .debug_selector(|| "action_bar".to_string())
            .w_full()
            .h(action_bar_height)
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(action_bar_padding_x)
            .bg(theme.colors.surface.chrome)
            .child(
                div()
                    .debug_selector(|| "left_action_group".to_string())
                    .flex()
                    .items_center()
                    .gap(action_group_gap)
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(global_nav)
                    .children(workspace_badge)
                    .child(tracking_actions)
                    .children(historical_badge)
                    .when(is_merging, |d| {
                        d.child(
                            div()
                                .debug_selector(|| "merge_controls".to_string())
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.status.warning.foreground)
                                        .font_weight(FontWeight::BOLD)
                                        .child("MERGING"),
                                )
                                .child(
                                    components::Button::new("abort_merge", merge_abort_label)
                                        .style(components::ButtonStyle::Danger)
                                        .on_click(theme, cx, |this, e: &ClickEvent, window, cx| {
                                            if let Some(repo_id) = this.active_repo_id() {
                                                this.open_popover_at(
                                                    PopoverKind::MergeAbortConfirm { repo_id },
                                                    e.position(),
                                                    window,
                                                    cx,
                                                );
                                            }
                                        }),
                                ),
                        )
                    })
                    .when(
                        !is_merging
                            && (is_rebase_or_apply_in_progress || is_cherry_pick_in_progress),
                        |d| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.colors.status.warning.foreground)
                                            .font_weight(FontWeight::BOLD)
                                            .child(sequencer_label),
                                    )
                                    .child(
                                        components::Button::new(sequencer_abort_id, "Abort")
                                            .style(components::ButtonStyle::Danger)
                                            .on_click(
                                                theme,
                                                cx,
                                                |this, e: &ClickEvent, window, cx| {
                                                    if let Some(repo_id) = this.active_repo_id() {
                                                        this.open_popover_at(
                                                            PopoverKind::MergeAbortConfirm {
                                                                repo_id,
                                                            },
                                                            e.position(),
                                                            window,
                                                            cx,
                                                        );
                                                    }
                                                },
                                            ),
                                    )
                                    .child(
                                        components::Button::new(sequencer_continue_id, "Continue")
                                            .style(components::ButtonStyle::Outlined)
                                            .disabled(rebase_has_unstaged_conflicts)
                                            .on_click(theme, cx, |this, _e, _w, _cx| {
                                                if let Some(repo_id) = this.active_repo_id() {
                                                    this.store
                                                        .dispatch(Msg::RebaseContinue { repo_id });
                                                }
                                            })
                                            .gitcomet_tooltip(
                                                theme,
                                                if rebase_has_unstaged_conflicts {
                                                    "Resolve all conflicts before continuing".into()
                                                } else {
                                                    sequencer_continue_tooltip.into()
                                                },
                                            ),
                                    ),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "right_action_group".to_string())
                    .flex()
                    .items_center()
                    .gap(action_group_gap)
                    .flex_none()
                    .child(terminal)
                    .child(create_branch)
                    .child(stash),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::domain::Upstream;
    use std::path::PathBuf;

    fn test_branch(name: &str, upstream: Option<Upstream>) -> Branch {
        Branch {
            name: name.to_string(),
            target: CommitId("deadbeef".into()),
            upstream,
            divergence: None,
        }
    }

    #[test]
    fn truncate_badge_label_leaves_short_labels_alone() {
        assert_eq!(truncate_badge_label("main"), "main");
    }

    #[test]
    fn truncate_badge_label_elides_long_labels() {
        // Unbounded labels would push the Pull/Push controls off the right edge.
        let long = "feature/PROJ-1234-refactor-the-entire-rendering-pipeline";
        let out = truncate_badge_label(long);
        assert!(out.ends_with('\u{2026}'), "expected an ellipsis, got {out}");
        assert_eq!(out.chars().count(), BADGE_LABEL_MAX_CHARS + 1);
    }

    #[test]
    fn truncate_badge_label_counts_characters_not_bytes() {
        // Multi-byte names must not be cut mid-character.
        let label = "ä".repeat(BADGE_LABEL_MAX_CHARS + 5);
        let out = truncate_badge_label(&label);
        assert_eq!(out.chars().count(), BADGE_LABEL_MAX_CHARS + 1);
    }

    #[test]
    fn action_bar_density_changes_at_scaled_breakpoints() {
        assert_eq!(
            action_bar_density(px(960.0), 100),
            ActionBarDensity::Compact
        );
        assert_eq!(
            action_bar_density(px(961.0), 100),
            ActionBarDensity::Compact
        );
        assert_eq!(
            action_bar_density(px(1120.0), 100),
            ActionBarDensity::Compact
        );
        assert_eq!(
            action_bar_density(px(1121.0), 100),
            ActionBarDensity::Condensed
        );
        assert_eq!(
            action_bar_density(px(1400.0), 100),
            ActionBarDensity::Condensed
        );
        assert_eq!(action_bar_density(px(1401.0), 100), ActionBarDensity::Wide);
        assert_eq!(
            action_bar_density(px(1200.0), 125),
            ActionBarDensity::Compact
        );
    }

    #[test]
    fn secondary_action_labels_remain_visible_in_condensed_mode() {
        assert_eq!(
            secondary_action_label(ActionBarDensity::Compact, "Terminal"),
            ""
        );
        assert_eq!(
            secondary_action_label(ActionBarDensity::Condensed, "Terminal"),
            "Terminal"
        );
        assert_eq!(
            secondary_action_label(ActionBarDensity::Wide, "Terminal"),
            "Terminal"
        );
    }

    #[test]
    fn upstream_badge_state_remains_present_without_tracking_upstream() {
        let head_branch = Loadable::Ready("feat/no-upstream".to_string());
        let branches = Loadable::Ready(Arc::new(vec![test_branch("feat/no-upstream", None)]));

        assert_eq!(
            upstream_badge_state(&head_branch, &branches),
            Some(("feat/no-upstream", None))
        );
    }

    #[test]
    fn upstream_badge_state_remains_present_while_branch_metadata_loads() {
        let head_branch = Loadable::Ready("feat/no-upstream".to_string());

        assert_eq!(
            upstream_badge_state(&head_branch, &Loadable::Loading),
            Some(("feat/no-upstream", None))
        );
    }

    #[test]
    fn head_branch_tracking_upstream_name_returns_remote_tracking_name() {
        let head_branch = Loadable::Ready("main".to_string());
        let branches = Loadable::Ready(Arc::new(vec![test_branch(
            "main",
            Some(Upstream {
                remote: "origin".to_string(),
                branch: "feature/tooltip".to_string(),
            }),
        )]));

        assert_eq!(
            head_branch_tracking_upstream_name(&head_branch, &branches).as_deref(),
            Some("origin/feature/tooltip")
        );
    }

    #[test]
    fn pull_tooltip_text_includes_tracking_branch_on_second_line() {
        assert_eq!(
            pull_tooltip_text(3, Some("origin/main")).as_ref(),
            "Pull 3 behind\norigin/main"
        );
        assert_eq!(pull_tooltip_text(0, None).as_ref(), "Pull 0 behind");
    }

    #[test]
    fn push_tooltip_text_includes_tracking_branch_on_second_line() {
        assert_eq!(
            push_tooltip_text(2, Some("origin/main")).as_ref(),
            "Push 2 ahead\norigin/main"
        );
        assert_eq!(push_tooltip_text(0, None).as_ref(), "Push 0 ahead");
    }

    #[test]
    fn notify_fingerprint_changes_when_branches_rev_changes() {
        let repo_id = RepoId(1);
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::default()
        };
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));

        let before = ActionBarView::notify_fingerprint(&state);
        state.repos[0].branches = Loadable::Ready(Arc::new(vec![test_branch(
            "main",
            Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
        )]));
        state.repos[0].branches_rev = state.repos[0].branches_rev.wrapping_add(1);
        let after = ActionBarView::notify_fingerprint(&state);

        assert_ne!(before, after);
    }

    #[test]
    fn notify_fingerprint_changes_when_remote_branches_rev_changes() {
        let repo_id = RepoId(1);
        let mut state = AppState {
            active_repo: Some(repo_id),
            ..AppState::default()
        };
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));

        let before = ActionBarView::notify_fingerprint(&state);
        state.repos[0].remote_branches_rev = state.repos[0].remote_branches_rev.wrapping_add(1);
        let after = ActionBarView::notify_fingerprint(&state);

        assert_ne!(before, after);
    }
}
