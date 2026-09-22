//! The Home page: what a window shows while it has no repository open.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use gitcomet_state::session::{Workspace, WorkspaceId};
use gpui::Stateful;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const HOME_MAX_WIDTH_PX: f32 = 720.0;

fn matches_query(query: &str, haystacks: &[&str]) -> bool {
    query.is_empty()
        || haystacks
            .iter()
            .any(|haystack| haystack.to_lowercase().contains(query))
}

fn repo_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(|| path.display().to_string(), ToOwned::to_owned)
}

/// Open workspaces first, then most recently used; the window's own one is left
/// out because it is already here.
pub(super) fn home_workspaces(cx: &App, own: Option<WorkspaceId>) -> Vec<Workspace> {
    let mut workspaces = crate::workspaces::workspaces(cx);
    workspaces.retain(|workspace| Some(workspace.id) != own);
    workspaces.sort_by_key(|workspace| {
        (
            !workspace.restore_on_launch,
            std::cmp::Reverse(workspace.last_activation_order),
        )
    });
    workspaces
}

/// Pinned repositories first, then recents, each listed once.
pub(super) fn home_repositories(pinned: &[PathBuf], recent: &[PathBuf]) -> Vec<PathBuf> {
    let mut repositories = pinned.to_vec();
    for path in recent {
        if !repositories.contains(path) {
            repositories.push(path.clone());
        }
    }
    repositories
}

impl GitCometView {
    /// Reload pinned and recent repositories from the session file. Called on
    /// entering Home and on window activation, never from render.
    pub(super) fn refresh_home_repositories(&mut self) {
        let session = session::load();
        self.home_pinned_repos = session.pinned_repos;
        self.home_recent_repos = session.recent_repos;
    }

    fn open_workspace_from_home(&mut self, id: WorkspaceId, cx: &mut gpui::Context<Self>) {
        let window_id = self.window_handle.window_id();
        // Adoption updates this view, so leave the listener first.
        cx.defer(move |cx| crate::app::open_workspace_in_window(cx, window_id, id));
    }

    fn home_section(&self, id: &'static str, title: &'static str, theme: AppTheme) -> gpui::Div {
        let _ = id;
        div()
            .pt(px(8.0))
            .pb(px(4.0))
            .px(px(4.0))
            .text_size(theme.ui_text(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.foreground.secondary)
            .child(title)
    }

    fn home_list(&self, id: &'static str, theme: AppTheme) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .w_full()
            .flex()
            .flex_col()
            .p(px(4.0))
            .gap(px(2.0))
            .rounded(px(theme.radii.panel))
            .border_1()
            .border_color(theme.colors.stroke.subtle)
            .bg(theme.colors.surface.panel)
    }

    fn home_empty(&self, text: &'static str, theme: AppTheme) -> gpui::Div {
        div()
            .px(px(8.0))
            .py(px(8.0))
            .text_size(theme.ui_text(13.0))
            .text_color(theme.colors.foreground.secondary)
            .child(text)
    }

    fn home_row(
        &self,
        id: SharedString,
        leading: AnyElement,
        title: String,
        detail: String,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let debug_id = id.clone();
        div()
            .id(id)
            .debug_selector(move || debug_id.to_string())
            .w_full()
            .min_w(px(0.0))
            .px(px(8.0))
            .py(px(6.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .control_interaction(
                components::InteractionStyle::new(theme),
                components::InteractionState::default(),
            )
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(theme.ui_text(13.0))
                            .text_color(theme.colors.foreground.primary)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .truncate()
                            .child(detail),
                    ),
            )
    }

    fn home_workspace_rows(
        &self,
        query: &str,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        home_workspaces(cx, self.workspace_id)
            .into_iter()
            .filter(|workspace| {
                let name = workspace.display_name();
                let mut haystacks = vec![name.as_str()];
                let paths: Vec<String> = workspace
                    .repositories
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect();
                haystacks.extend(paths.iter().map(String::as_str));
                matches_query(query, &haystacks)
            })
            .map(|workspace| {
                let id = workspace.id;
                let dot = div()
                    .size(px(10.0))
                    .mx(px(4.0))
                    .flex_none()
                    .rounded_full()
                    .bg(crate::view::chrome::workspace_color(workspace.color, theme))
                    .into_any_element();
                let state = if workspace.restore_on_launch {
                    "Open"
                } else {
                    "Saved"
                };
                let names = workspace
                    .repositories
                    .iter()
                    .map(|path| repo_name(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                let count = crate::workspaces::repository_count_label(workspace.repositories.len());
                let detail = if names.is_empty() {
                    format!("{state} · {count}")
                } else {
                    format!("{state} · {count} · {names}")
                };
                self.home_row(
                    format!("home_workspace_{id}").into(),
                    dot,
                    workspace.display_name(),
                    detail,
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        this.open_workspace_from_home(id, cx);
                    }),
                )
                .into_any_element()
            })
            .collect()
    }

    fn home_repository_rows(
        &self,
        query: &str,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let scale = crate::ui_scale::UiScale::from_percent(self.ui_scale_percent);
        home_repositories(&self.home_pinned_repos, &self.home_recent_repos)
            .into_iter()
            .filter(|path| {
                let name = repo_name(path);
                let full = path.display().to_string();
                matches_query(query, &[name.as_str(), full.as_str()])
            })
            .map(|path| {
                let name = repo_name(&path);
                let parent = path
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let pinned = self.home_pinned_repos.contains(&path);
                let detail = if pinned {
                    format!("Pinned · {parent}")
                } else {
                    parent
                };
                let badge = components::repository_initials_box(
                    theme,
                    scale,
                    components::repository_initials(&name).into(),
                    false,
                )
                .into_any_element();
                let row_id: SharedString =
                    format!("home_recent_{}", session::path_storage_key(&path)).into();
                self.home_row(row_id, badge, name, detail, theme)
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.open_repo_path(path.clone(), cx);
                        }),
                    )
                    .into_any_element()
            })
            .collect()
    }

    pub(super) fn home_screen(&mut self, cx: &mut gpui::Context<Self>) -> AnyElement {
        if matches!(
            self.state.git_runtime.availability,
            gitcomet_core::process::GitExecutableAvailability::Checking
        ) {
            return self.startup_repository_loading_screen();
        }
        if self.git_runtime_unavailable() {
            return self.git_unavailable_splash(cx);
        }

        let theme = self.theme;
        let scaled_px = crate::ui_scale::scaler(self.ui_scale_percent);
        let colors = self.splash_palette();
        let query = self.home_search_query.trim().to_lowercase();

        let open_button = Self::splash_cta_button(
            theme,
            "home_open_repo",
            "Open Repository",
            "icons/folder.svg",
            colors.primary,
            self.ui_scale_percent,
        )
        .gitcomet_tooltip(theme, "Open an existing repository".into())
        .on_activate(
            false,
            controls::ControlActivation::Action,
            cx.listener(|this, _e, window, cx| this.prompt_open_repo(window, cx)),
        );

        let clone_button = {
            let last_bounds: Rc<RefCell<Option<Bounds<Pixels>>>> = Rc::new(RefCell::new(None));
            let last_bounds_for_prepaint = Rc::clone(&last_bounds);
            let last_bounds_for_click = Rc::clone(&last_bounds);
            let button = Self::splash_cta_button(
                theme,
                "home_clone_repo",
                "Clone Repository",
                "icons/cloud.svg",
                colors.secondary,
                self.ui_scale_percent,
            )
            .gitcomet_tooltip(theme, "Clone a repository from a URL".into())
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(move |this, e: &ClickEvent, window, cx| {
                    let bounds = (*last_bounds_for_click.borrow())
                        .unwrap_or_else(|| Bounds::new(e.position(), size(px(0.0), px(0.0))));
                    this.open_popover_for_bounds(PopoverKind::CloneRepo, bounds, window, cx);
                }),
            );
            div()
                .on_children_prepainted(move |children_bounds, _window, _cx| {
                    if let Some(bounds) = children_bounds.first() {
                        *last_bounds_for_prepaint.borrow_mut() = Some(*bounds);
                    }
                })
                .child(button)
        };

        let init_button = (!self.blocks_repository_management_actions()).then(|| {
            Self::splash_cta_button(
                theme,
                "home_init_repo",
                "Initialize Repository",
                "icons/git_branch.svg",
                colors.secondary,
                self.ui_scale_percent,
            )
            .gitcomet_tooltip(theme, "Create a new repository in a folder".into())
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e, window, cx| this.prompt_init_repo(window, cx)),
            )
        });

        let open_repo_fallback = self.open_repo_panel.then(|| {
            div()
                .w_full()
                .child(
                    div()
                        .pb(scaled_px(8.0))
                        .text_size(theme.ui_text(11.0))
                        .text_color(colors.muted)
                        .text_center()
                        .child(
                            "Native folder picker unavailable. Enter a repository path manually.",
                        ),
                )
                .child(self.open_repo_panel(cx))
        });

        let workspace_rows = self.home_workspace_rows(&query, theme, cx);
        let repository_rows = self.home_repository_rows(&query, theme, cx);
        let filtering = !query.is_empty();
        let workspaces_list = if workspace_rows.is_empty() {
            self.home_list("home_workspaces_list", theme)
                .child(self.home_empty(
                    if filtering {
                        "No matching workspaces."
                    } else {
                        "No saved workspaces yet. Every window with repositories open is one."
                    },
                    theme,
                ))
        } else {
            self.home_list("home_workspaces_list", theme)
                .children(workspace_rows)
        };
        let repositories_list = if repository_rows.is_empty() {
            self.home_list("home_recent_list", theme)
                .child(self.home_empty(
                    if filtering {
                        "No matching repositories."
                    } else {
                        "Repositories you open appear here."
                    },
                    theme,
                ))
        } else {
            self.home_list("home_recent_list", theme)
                .children(repository_rows)
        };

        let own_workspace_name = self
            .workspace_id
            .and_then(|id| crate::workspaces::workspace(cx, id))
            .map(|workspace| workspace.display_name());
        let title = own_workspace_name.unwrap_or_else(|| "Home".to_string());

        div()
            .id("repository_entry_screen")
            .debug_selector(|| "repository_entry_screen".to_string())
            .relative()
            .flex()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(self.splash_backdrop_base())
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, _window, cx| {
                    this.submit_external_drag_payload_after_repo_drop(paths.clone(), cx);
                }),
            )
            .child(self.interstitial_backdrop())
            .child(
                div()
                    .id("home_scroll")
                    .relative()
                    .size_full()
                    .overflow_y_scroll()
                    .flex()
                    .justify_center()
                    .px_4()
                    .pt(scaled_px(40.0))
                    .pb(scaled_px(24.0))
                    .child(
                        div()
                            .w_full()
                            .max_w(scaled_px(HOME_MAX_WIDTH_PX))
                            .flex()
                            .flex_col()
                            .gap(scaled_px(12.0))
                            .child(
                                div()
                                    .id("home_title")
                                    .debug_selector(|| "home_title".to_string())
                                    .flex()
                                    .items_center()
                                    .gap(scaled_px(10.0))
                                    .child(Self::interstitial_logo(theme, scaled_px(28.0)))
                                    .child(
                                        div()
                                            .text_size(theme.ui_text(22.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(colors.text)
                                            .child(title),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap(scaled_px(10.0))
                                    .child(
                                        div()
                                            .id("home_open_repo_action")
                                            .debug_selector(|| "home_open_repo_action".to_string())
                                            .child(open_button),
                                    )
                                    .child(
                                        div()
                                            .id("home_clone_repo_action")
                                            .debug_selector(|| "home_clone_repo_action".to_string())
                                            .child(clone_button),
                                    )
                                    .children(init_button.map(|button| {
                                        div()
                                            .id("home_init_repo_action")
                                            .debug_selector(|| "home_init_repo_action".to_string())
                                            .child(button)
                                    })),
                            )
                            .children(open_repo_fallback)
                            .child(
                                div()
                                    .id("home_search")
                                    .debug_selector(|| "home_search".to_string())
                                    .w_full()
                                    .child(self.home_search_input.clone()),
                            )
                            .child(self.home_section(
                                "home_workspaces_heading",
                                "Workspaces",
                                theme,
                            ))
                            .child(workspaces_list)
                            .child(self.home_section(
                                "home_recent_heading",
                                "Recent repositories",
                                theme,
                            ))
                            .child(repositories_list),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_repositories_lead_and_recents_are_not_repeated() {
        let pinned = vec![PathBuf::from("/r/b")];
        let recent = vec![PathBuf::from("/r/a"), PathBuf::from("/r/b")];
        assert_eq!(
            home_repositories(&pinned, &recent),
            vec![PathBuf::from("/r/b"), PathBuf::from("/r/a")]
        );
    }

    #[test]
    fn an_empty_query_matches_everything_and_matching_ignores_case() {
        assert!(matches_query("", &["anything"]));
        assert!(matches_query("comet", &["GitComet"]));
        assert!(!matches_query("zzz", &["GitComet", "/home/repos"]));
    }
}
