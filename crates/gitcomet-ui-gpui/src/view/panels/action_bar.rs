use super::super::path_display;
use super::*;
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

const ACTION_BAR_HEIGHT_PX: f32 = components::CONTROL_HEIGHT_PX + 8.0;

pub(in super::super) fn action_bar_height<C>(cx: &mut C) -> Pixels
where
    C: gpui::BorrowAppContext,
{
    crate::ui_scale::design_px(ACTION_BAR_HEIGHT_PX, cx)
}

fn head_branch_has_tracking_upstream(
    head_branch: &Loadable<String>,
    branches: &Loadable<Arc<Vec<Branch>>>,
) -> bool {
    head_branch_tracking_upstream_name(head_branch, branches).is_some()
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
            repo.upstream_divergence_rev.hash(&mut hasher);
            repo.merge_message_rev.hash(&mut hasher);
            repo.ops_rev.hash(&mut hasher);
            repo.status_cache_rev().hash(&mut hasher);
            repo.loads_in_flight.any_in_flight().hash(&mut hasher);
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
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let action_bar_height = action_bar_height(cx);
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px =
            |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
        let hover_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.06 } else { 0.04 });
        let active_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.10 } else { 0.07 });
        let icon_primary = theme.colors.accent;
        let icon_muted = with_alpha(theme.colors.accent, if theme.is_dark { 0.72 } else { 0.82 });
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

        let repo_title: SharedString = self
            .active_repo()
            .map(|r| path_display::path_display_shared(&r.spec.workdir))
            .unwrap_or_else(|| "No repository".into());

        let branch: SharedString = self
            .active_repo()
            .map(|r| match &r.head_branch {
                Loadable::Ready(name) => name.clone().into(),
                Loadable::Loading => "".into(),
                Loadable::Error(_) => "error".into(),
                Loadable::NotLoaded => "—".into(),
            })
            .unwrap_or_else(|| "—".into());

        let is_merging = self
            .active_repo()
            .is_some_and(|r| matches!(&r.merge_commit_message, Loadable::Ready(Some(_))));
        let is_rebase_or_apply_in_progress = self
            .active_repo()
            .is_some_and(|r| matches!(&r.rebase_in_progress, Loadable::Ready(true)));

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
        let pull_default_enabled = self.active_repo().is_some_and(|repo| {
            head_branch_has_tracking_upstream(&repo.head_branch, &repo.branches)
        });

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

        let repo_busy = self.active_repo().is_some_and(|repo| {
            matches!(repo.open, Loadable::Loading)
                || repo.loads_in_flight.any_in_flight()
                || repo.local_actions_in_flight > 0
                || repo.pull_in_flight > 0
                || repo.push_in_flight > 0
        });

        let repo_picker = div()
            .id("repo_picker")
            .debug_selector(|| "repo_picker".to_string())
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(hover_bg))
            .active(move |s| s.bg(active_bg))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Repository"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if repo_busy {
                                spinner(
                                    ("repo_busy_spinner", active_repo_key),
                                    with_alpha(
                                        theme.colors.text,
                                        if theme.is_dark { 0.72 } else { 0.62 },
                                    ),
                                )
                                .into_any_element()
                            } else {
                                div().into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_sm()
                            .text_color(theme.colors.text_muted)
                            .child(components::TruncatedText::path(repo_title).render(cx)),
                    ),
            )
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                this.open_popover_at(PopoverKind::RepoPicker, e.position(), window, cx);
            }))
            .gitcomet_tooltip(theme, "Select repository".into());

        let branch_picker = div()
            .id("branch_picker")
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(theme.radii.row))
            .hover(move |s| s.bg(hover_bg))
            .active(move |s| s.bg(active_bg))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child("Branch"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.colors.text_muted)
                    .child(branch),
            )
            .on_click(cx.listener(|this, e: &ClickEvent, window, cx| {
                this.open_popover_at(PopoverKind::BranchPicker, e.position(), window, cx);
            }))
            .gitcomet_tooltip(theme, "Select branch".into());

        let pull_color = if pull_count > 0 {
            theme.colors.warning
        } else {
            icon_muted
        };
        let menu_selected_bg =
            with_alpha(theme.colors.accent, if theme.is_dark { 0.26 } else { 0.20 });
        let mut pull_main = components::Button::new("pull_main", "Pull")
            .borderless()
            .start_slot(if pull_loading {
                spinner(("pull_spinner", active_repo_key), pull_color).into_any_element()
            } else {
                icon("icons/arrow_down.svg", pull_color).into_any_element()
            })
            .style(components::ButtonStyle::Subtle)
            .no_hover_border()
            .disabled(!pull_default_enabled);
        if pull_count > 0 {
            pull_main = pull_main.end_slot(count_badge(pull_count, pull_color));
        }
        let pull_picker_invoker: SharedString = "pull_picker".into();
        let pull_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == pull_picker_invoker.as_ref());
        let pull_tracking_branch_name = tracking_branch_name.clone();
        let pull_menu_icon_color = if pull_picker_active {
            theme.colors.accent
        } else {
            icon_muted
        };
        let pull_menu = components::Button::new("pull_menu", "")
            .borderless()
            .start_slot(icon("icons/chevron_down.svg", pull_menu_icon_color))
            .style(components::ButtonStyle::Subtle)
            .no_hover_border()
            .selected(pull_picker_active)
            .selected_bg(menu_selected_bg);

        let pull = div()
            .id("pull")
            .child(
                components::SplitButton::new(
                    pull_main.on_click(theme, cx, |this, _e, _w, _cx| {
                        if let Some(repo_id) = this.active_repo_id() {
                            this.store.dispatch(Msg::Pull {
                                repo_id,
                                mode: PullMode::Default,
                            });
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
                .style(components::SplitButtonStyle::Outlined)
                .render(theme, ui_scale_percent),
            )
            .gitcomet_tooltip(
                theme,
                pull_tooltip_text(pull_count, pull_tracking_branch_name.as_deref()),
            );

        let push_color = if push_count > 0 {
            theme.colors.success
        } else {
            icon_muted
        };
        let mut push_main = components::Button::new("push_main", "Push")
            .borderless()
            .start_slot(if push_loading {
                spinner(("push_spinner", active_repo_key), push_color).into_any_element()
            } else {
                icon("icons/arrow_up.svg", push_color).into_any_element()
            })
            .style(components::ButtonStyle::Subtle)
            .no_hover_border();
        if push_count > 0 {
            push_main = push_main.end_slot(count_badge(push_count, push_color));
        }
        let push_picker_invoker: SharedString = "push_picker".into();
        let push_picker_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == push_picker_invoker.as_ref());
        let push_tracking_branch_name = tracking_branch_name.clone();
        let push_menu_icon_color = if push_picker_active {
            theme.colors.accent
        } else {
            icon_muted
        };
        let push_menu = components::Button::new("push_menu", "")
            .borderless()
            .start_slot(icon("icons/chevron_down.svg", push_menu_icon_color))
            .style(components::ButtonStyle::Subtle)
            .no_hover_border()
            .selected(push_picker_active)
            .selected_bg(menu_selected_bg);

        let push = div()
            .id("push")
            .child(
                components::SplitButton::new(
                    push_main.on_click(theme, cx, |this, e, window, cx| {
                        let Some(repo) = this.active_repo() else {
                            return;
                        };
                        let repo_id = repo.id;
                        let head = match &repo.head_branch {
                            Loadable::Ready(head) => head.clone(),
                            _ => {
                                this.store.dispatch(Msg::Push { repo_id });
                                return;
                            }
                        };

                        let upstream_missing = match &repo.branches {
                            Loadable::Ready(branches) => branches
                                .iter()
                                .find(|b| b.name == head)
                                .is_some_and(|b| b.upstream.is_none()),
                            _ => false,
                        };

                        if upstream_missing {
                            let remote = match &repo.remotes {
                                Loadable::Ready(remotes) => {
                                    if remotes.is_empty() {
                                        None
                                    } else if remotes.iter().any(|r| r.name == "origin") {
                                        Some("origin".to_string())
                                    } else {
                                        Some(remotes[0].name.clone())
                                    }
                                }
                                _ => Some("origin".to_string()),
                            };

                            if let Some(remote) = remote {
                                this.open_popover_at(
                                    PopoverKind::PushSetUpstreamPrompt { repo_id, remote },
                                    e.position(),
                                    window,
                                    cx,
                                );
                                return;
                            }

                            this.push_toast(
                                components::ToastKind::Error,
                                "Cannot push: no remotes configured".to_string(),
                                cx,
                            );
                            return;
                        }

                        this.store.dispatch(Msg::Push { repo_id });
                    }),
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
                .style(components::SplitButtonStyle::Outlined)
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
        let stash = components::Button::new("stash", "Stash")
            .start_slot(icon(crate::view::icons::STASH_ICON_PATH, icon_primary))
            .style(components::ButtonStyle::Outlined)
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
            );

        let create_branch_invoker: SharedString = "create_branch_btn".into();
        let create_branch_active = self
            .active_context_menu_invoker
            .as_ref()
            .is_some_and(|id| id.as_ref() == create_branch_invoker.as_ref());
        let create_branch = components::Button::new("create_branch", "Branch")
            .start_slot(icon("icons/git_branch.svg", icon_primary))
            .style(components::ButtonStyle::Outlined)
            .selected(create_branch_active)
            .selected_bg(menu_selected_bg)
            .on_click_with_bounds(theme, cx, move |this, _e, bounds, window, cx| {
                this.activate_context_menu_invoker(create_branch_invoker.clone(), cx);
                this.open_popover_for_bounds(PopoverKind::CreateBranch, bounds, window, cx);
            })
            .gitcomet_tooltip(theme, "Create branch".into());

        div()
            .w_full()
            .h(action_bar_height)
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .py_1()
            .bg(theme.colors.active_section)
            .border_b_1()
            .border_color(theme.colors.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .child(repo_picker)
                    .child(branch_picker)
                    .when(is_merging, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.warning)
                                        .font_weight(FontWeight::BOLD)
                                        .child("MERGING"),
                                )
                                .child(
                                    components::Button::new("abort_merge", "Abort merge")
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
                    .when(!is_merging && is_rebase_or_apply_in_progress, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors.warning)
                                        .font_weight(FontWeight::BOLD)
                                        .child("APPLY/REBASE"),
                                )
                                .child(
                                    components::Button::new("abort_rebase_or_apply", "Abort")
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
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(pull)
                    .child(push)
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
    fn head_branch_has_tracking_upstream_returns_true_for_configured_head() {
        let head_branch = Loadable::Ready("main".to_string());
        let branches = Loadable::Ready(Arc::new(vec![test_branch(
            "main",
            Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
        )]));
        assert!(head_branch_has_tracking_upstream(&head_branch, &branches));
    }

    #[test]
    fn head_branch_has_tracking_upstream_returns_false_without_upstream() {
        let head_branch = Loadable::Ready("feat/no-upstream".to_string());
        let branches = Loadable::Ready(Arc::new(vec![test_branch("feat/no-upstream", None)]));
        assert!(!head_branch_has_tracking_upstream(&head_branch, &branches));
    }

    #[test]
    fn head_branch_has_tracking_upstream_returns_false_when_head_not_loaded() {
        let head_branch = Loadable::NotLoaded;
        let branches = Loadable::Ready(Arc::new(vec![test_branch(
            "main",
            Some(Upstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            }),
        )]));
        assert!(!head_branch_has_tracking_upstream(&head_branch, &branches));
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
}
