//! The Home page: what a window shows while it has no repository open.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::kit::{Scrollbar, ScrollbarAxis};
use crate::view::panels::popover::picker_nav::{
    PickerNavKeys, PickerNavOutcome, handle_picker_nav,
};
use gitcomet_state::session::{Workspace, WorkspaceId};
use gpui::{ScrollStrategy, Stateful, UniformListScrollHandle, uniform_list};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

const HOME_MAX_WIDTH_PX: f32 = 960.0;
/// Rows a list shows before it scrolls; also the Page Up/Down step.
pub(super) const HOME_LIST_MAX_ROWS: usize = 8;

/// The filtered Home rows, in keyboard order: workspaces, then repositories.
#[derive(Default)]
pub(super) struct HomeRows {
    pub(super) workspaces: Vec<Workspace>,
    pub(super) repositories: Vec<PathBuf>,
    inputs: Option<HomeRowsInputs>,
}

/// Repaints (including the search caret) reuse filtered rows. Only these
/// inputs can change their contents or ordering.
struct HomeRowsInputs {
    workspace_revision: Option<u64>,
    own: Option<WorkspaceId>,
    query: String,
    pinned: Vec<PathBuf>,
    recent: Vec<PathBuf>,
}

impl HomeRows {
    fn len(&self) -> usize {
        self.workspaces.len() + self.repositories.len()
    }
}

struct HomeColumn {
    title: &'static str,
    frame_id: &'static str,
    list_id: &'static str,
    scrollbar_id: &'static str,
    empty_text: &'static str,
}

type HomeRowsRenderer = fn(
    &mut GitCometView,
    std::ops::Range<usize>,
    &mut Window,
    &mut gpui::Context<GitCometView>,
) -> Vec<AnyElement>;

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
    crate::workspaces::sort_workspaces(&mut workspaces);
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

    fn home_section(&self, title: &'static str, theme: AppTheme) -> gpui::Div {
        div()
            .pt(px(8.0))
            .pb(px(4.0))
            .px(px(4.0))
            .text_size(theme.ui_text(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.foreground.secondary)
            .child(title)
    }

    fn home_list_frame(&self, id: &'static str, theme: AppTheme) -> Stateful<gpui::Div> {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .relative()
            .w_full()
            .min_w(px(0.0))
            .p(px(4.0))
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

    /// Every row has this height: `uniform_list` measures only the first one.
    /// Two text lines plus padding, in rems so it follows the font size.
    pub(super) fn home_row_height(&self) -> gpui::Rems {
        self.theme.ui_text(18.0 + 16.0 + 12.0)
    }

    fn compute_home_rows(&self, cx: &App) -> HomeRows {
        let query = self.home_search_query.trim().to_lowercase();
        let workspaces = home_workspaces(cx, self.workspace_id)
            .into_iter()
            .filter(|workspace| {
                let name = workspace.display_name();
                let paths: Vec<String> = workspace
                    .repositories
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect();
                let mut haystacks = vec![name.as_str()];
                haystacks.extend(paths.iter().map(String::as_str));
                matches_query(&query, &haystacks)
            })
            .collect();
        let repositories = home_repositories(&self.home_pinned_repos, &self.home_recent_repos)
            .into_iter()
            .filter(|path| {
                let name = repo_name(path);
                let full = path.display().to_string();
                matches_query(&query, &[name.as_str(), full.as_str()])
            })
            .collect();
        HomeRows {
            workspaces,
            repositories,
            inputs: Some(HomeRowsInputs {
                workspace_revision: crate::workspaces::revision(cx),
                own: self.workspace_id,
                query: self.home_search_query.clone(),
                pinned: self.home_pinned_repos.clone(),
                recent: self.home_recent_repos.clone(),
            }),
        }
    }

    /// Refresh the filtered rows and keep a row selected while any exist.
    pub(super) fn sync_home_rows(&mut self, cx: &App) {
        let current = self.home_rows.inputs.as_ref().is_some_and(|inputs| {
            inputs.workspace_revision == crate::workspaces::revision(cx)
                && inputs.own == self.workspace_id
                && inputs.query == self.home_search_query
                && inputs.pinned == self.home_pinned_repos
                && inputs.recent == self.home_recent_repos
        });
        if !current {
            self.home_rows = self.compute_home_rows(cx);
        }
        let count = self.home_rows.len();
        self.home_selected = (count > 0).then(|| self.home_selected.unwrap_or(0).min(count - 1));
    }

    fn scroll_home_selection_into_view(&self) {
        let Some(ix) = self.home_selected else {
            return;
        };
        let workspaces = self.home_rows.workspaces.len();
        if ix < workspaces {
            self.home_workspaces_scroll
                .scroll_to_item(ix, ScrollStrategy::Nearest);
        } else {
            self.home_repositories_scroll
                .scroll_to_item(ix - workspaces, ScrollStrategy::Nearest);
        }
    }

    fn activate_home_row(&mut self, ix: usize, cx: &mut gpui::Context<Self>) {
        let workspaces = self.home_rows.workspaces.len();
        if let Some(workspace) = self.home_rows.workspaces.get(ix) {
            let id = workspace.id;
            self.open_workspace_from_home(id, cx);
        } else if let Some(path) = self.home_rows.repositories.get(ix - workspaces).cloned() {
            self.open_repo_path(path, cx);
        }
    }

    /// The search box's keys: typing filters and selects the first match,
    /// Up/Down walk both lists as one run, Left/Right (with the caret at that
    /// edge) jump to the same position in the other list, Enter opens.
    pub(super) fn handle_home_search_input(
        &mut self,
        input: &Entity<components::TextInput>,
        cx: &mut gpui::Context<Self>,
    ) {
        let (keys, left, right, text) = input.update(cx, |input, _| {
            (
                PickerNavKeys::take(input),
                input.take_arrow_left_at_start_pressed(),
                input.take_arrow_right_at_end_pressed(),
                input.text().to_string(),
            )
        });
        if self.home_search_query != text {
            self.home_search_query = text;
            self.home_selected = None;
            self.sync_home_rows(cx);
            self.home_workspaces_scroll
                .scroll_to_item_strict(0, ScrollStrategy::Top);
            self.home_repositories_scroll
                .scroll_to_item_strict(0, ScrollStrategy::Top);
            cx.notify();
            return;
        }
        if !self.is_home_screen_active() {
            return;
        }
        self.sync_home_rows(cx);
        let workspaces = self.home_rows.workspaces.len();
        let repositories = self.home_rows.repositories.len();

        if left || right {
            if let Some(ix) = self.home_selected
                && workspaces > 0
                && repositories > 0
            {
                self.home_selected = Some(if right && ix < workspaces {
                    workspaces + ix.min(repositories - 1)
                } else if left && ix >= workspaces {
                    (ix - workspaces).min(workspaces - 1)
                } else {
                    ix
                });
                self.scroll_home_selection_into_view();
                cx.notify();
            }
            return;
        }

        match handle_picker_nav(
            &keys,
            &mut self.home_selected,
            workspaces + repositories,
            HOME_LIST_MAX_ROWS,
        ) {
            PickerNavOutcome::Navigated => {
                self.scroll_home_selection_into_view();
                cx.notify();
            }
            PickerNavOutcome::Enter => {
                if let Some(ix) = self.home_selected {
                    self.activate_home_row(ix, cx);
                }
            }
            PickerNavOutcome::Escape => {
                if !self.home_search_query.is_empty() {
                    input.update(cx, |input, cx| input.set_text("", cx));
                }
            }
            PickerNavOutcome::Idle => {}
        }
    }

    /// Focus the search box (entering Home) or release it (leaving Home) once
    /// the snapshot has been applied; this path has no `Window`.
    pub(super) fn defer_home_search_focus(&self, focus: bool, cx: &mut gpui::Context<Self>) {
        let handle = self.home_search_input.read(cx).focus_handle();
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_root, window, cx| {
                if focus {
                    window.focus(&handle, cx);
                } else if handle.is_focused(window) {
                    window.blur(cx);
                }
            });
        });
    }

    pub(super) fn focus_home_search(&self, window: &mut Window, cx: &mut App) {
        let focus = self.home_search_input.read(cx).focus_handle();
        window.focus(&focus, cx);
    }

    fn remove_home_workspace(&mut self, id: WorkspaceId, cx: &mut gpui::Context<Self>) {
        let view = cx.entity().downgrade();
        cx.defer(move |cx| {
            crate::app::delete_workspace(cx, id);
            let _ = view.update(cx, |this, cx| {
                this.sync_home_rows(cx);
                cx.notify();
            });
        });
    }

    /// Drop a repository from Home: forget it and, if pinned, unpin it too,
    /// since a pin would otherwise keep it listed.
    fn remove_home_repository(&mut self, path: &Path, cx: &mut gpui::Context<Self>) {
        if self.home_pinned_repos.iter().any(|pinned| pinned == path) {
            let _ = session::remove_pinned_repo(path);
            self.home_pinned_repos.retain(|pinned| pinned != path);
        }
        let _ = session::remove_recent_repo(path);
        self.home_recent_repos.retain(|recent| recent != path);
        self.sync_home_rows(cx);
        cx.notify();
    }

    /// The Enter pill (selected row only) and the remove cross, which shows on
    /// hover and stays visible on the selected row for keyboard users.
    fn home_row_trailing(
        &self,
        row_group: SharedString,
        index: usize,
        selected: bool,
        remove: Option<(
            &'static str,
            SharedString,
            Arc<components::OnRemoveFn<Self>>,
        )>,
        cx: &gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = self.theme;
        let ui_scale = crate::ui_scale::UiScale::from_percent(self.ui_scale_percent);
        let mut trailing = Vec::new();
        if selected {
            trailing.push(
                components::selected_hint_pill(theme, ui_scale, "Enter".into())
                    .debug_selector(|| "home_enter_hint".to_string())
                    .into_any_element(),
            );
        }
        if let Some((id_prefix, tooltip, on_remove)) = remove {
            trailing.push(
                components::remove_row_button(
                    id_prefix,
                    theme,
                    ui_scale,
                    index,
                    row_group,
                    selected,
                    Some(tooltip),
                    Some(self.tooltip_host.downgrade()),
                    on_remove,
                    cx,
                )
                .into_any_element(),
            );
        }
        trailing
    }

    fn home_row(
        &self,
        id: SharedString,
        leading: AnyElement,
        text: (String, String),
        selected: bool,
        trailing: Vec<AnyElement>,
        theme: AppTheme,
    ) -> Stateful<gpui::Div> {
        let (title, detail) = text;
        let debug_id = id.clone();
        let group = id.clone();
        div()
            .group(group)
            .id(id)
            .debug_selector(move || debug_id.to_string())
            .relative()
            .w_full()
            .min_w(px(0.0))
            .h(self.home_row_height())
            .px(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .rounded(px(theme.radii.row))
            .cursor(CursorStyle::PointingHand)
            .control_interaction(
                components::InteractionStyle::new(theme).selection_outline(false),
                components::InteractionState::default().selected(selected, theme.active_overlay()),
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
                            .line_height(theme.ui_text(18.0))
                            .text_color(theme.colors.foreground.primary)
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .line_height(theme.ui_text(16.0))
                            .text_color(theme.colors.foreground.secondary)
                            .truncate()
                            .child(detail),
                    ),
            )
            .children(trailing)
    }

    fn render_home_workspace_rows(
        this: &mut Self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let ui_scale = crate::ui_scale::UiScale::from_percent(this.ui_scale_percent);
        range
            .filter_map(|ix| this.home_rows.workspaces.get(ix).cloned().map(|w| (ix, w)))
            .map(|(ix, workspace)| {
                let id = workspace.id;
                let row_id: SharedString = format!("home_workspace_{id}").into();
                let selected = this.home_selected == Some(ix);
                // A workspace open in another window would reappear at once.
                let remove = (!crate::workspaces::is_open_in_a_window(cx, id)).then(|| {
                    let on_remove: Arc<components::OnRemoveFn<Self>> =
                        Arc::new(move |this: &mut Self, _ix, _window, cx| {
                            this.remove_home_workspace(id, cx);
                        });
                    (
                        "home_workspace_remove",
                        SharedString::from("Delete workspace"),
                        on_remove,
                    )
                });
                let trailing = this.home_row_trailing(row_id.clone(), ix, selected, remove, cx);
                components::picker_row(
                    theme,
                    ui_scale,
                    &crate::view::workspace_picker::workspace_picker_item(&workspace),
                    components::PickerRowSpec {
                        id: row_id.clone().into(),
                        selector_prefix: "home",
                        key: components::PickerRowKey::Text(id.to_string().into()),
                        row_selector: Some(row_id.clone()),
                        selected,
                        marked: false,
                        match_range: None,
                        leading_icon: None,
                    },
                    Some(this.tooltip_host.downgrade()),
                    cx,
                )
                .group(row_id)
                .children(trailing)
                .on_activate(
                    false,
                    controls::ControlActivation::PreserveFocus,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        this.home_selected = Some(ix);
                        this.open_workspace_from_home(id, cx);
                    }),
                )
                .into_any_element()
            })
            .collect()
    }

    fn render_home_repository_rows(
        this: &mut Self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let scale = crate::ui_scale::UiScale::from_percent(this.ui_scale_percent);
        let offset = this.home_rows.workspaces.len();
        range
            .filter_map(|ix| {
                this.home_rows
                    .repositories
                    .get(ix)
                    .cloned()
                    .map(|p| (ix, p))
            })
            .map(|(ix, path)| {
                let selected = this.home_selected == Some(offset + ix);
                let name = repo_name(&path);
                let parent = path
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let detail = if this.home_pinned_repos.contains(&path) {
                    format!("Pinned · {parent}")
                } else {
                    parent
                };
                let badge = components::repository_initials_box(
                    theme,
                    scale,
                    components::repository_initials(&name).into(),
                    selected,
                )
                .into_any_element();
                let row_id: SharedString =
                    format!("home_recent_{}", session::path_storage_key(&path)).into();
                let pinned = this.home_pinned_repos.contains(&path);
                let remove_path = path.clone();
                let on_remove: Arc<components::OnRemoveFn<Self>> =
                    Arc::new(move |this: &mut Self, _ix, _window, cx| {
                        this.remove_home_repository(&remove_path, cx);
                    });
                let tooltip = if pinned {
                    "Unpin and remove from recent repositories"
                } else {
                    "Remove from recent repositories"
                };
                let trailing = this.home_row_trailing(
                    row_id.clone(),
                    ix,
                    selected,
                    Some(("home_recent_remove", tooltip.into(), on_remove)),
                    cx,
                );
                this.home_row(row_id, badge, (name, detail), selected, trailing, theme)
                    .on_activate(
                        false,
                        controls::ControlActivation::PreserveFocus,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.home_selected = Some(offset + ix);
                            this.open_repo_path(path.clone(), cx);
                        }),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// A column: header plus a list capped at `HOME_LIST_MAX_ROWS` rows that
    /// renders only what is in view.
    fn home_column(
        &self,
        labels: HomeColumn,
        count: usize,
        row_height: gpui::AbsoluteLength,
        scroll: UniformListScrollHandle,
        rows: HomeRowsRenderer,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let frame = self.home_list_frame(labels.frame_id, theme);
        let body = if count == 0 {
            frame.child(self.home_empty(labels.empty_text, theme))
        } else {
            let visible = count.min(HOME_LIST_MAX_ROWS) as f32;
            let height: gpui::AbsoluteLength = match row_height {
                gpui::AbsoluteLength::Pixels(row) => (row * visible).into(),
                gpui::AbsoluteLength::Rems(row) => (row * visible).into(),
            };
            let gutter = Scrollbar::visible_gutter(scroll.clone(), ScrollbarAxis::Vertical);
            let list = uniform_list(labels.list_id, count, cx.processor(rows))
                .h(height)
                .pr(gutter)
                .track_scroll(&scroll);
            frame.child(
                div()
                    .relative()
                    .w_full()
                    .h(height)
                    .min_w(px(0.0))
                    .child(restrict_scroll_to_vertical_axis(list))
                    .child(Scrollbar::new(labels.scrollbar_id, scroll).render(theme)),
            )
        };
        div()
            .flex_1()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(self.home_section(labels.title, theme))
            .child(body)
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
        let colors = theme.colors.interstitial;
        let cta_scale = crate::ui_scale::UiScale::from_percent(self.ui_scale_percent);
        let query = self.home_search_query.trim().to_lowercase();

        let open_button = components::interstitial_cta_button(
            "home_open_repo",
            "Open Repository",
            "icons/folder.svg",
            true,
            theme,
            cta_scale,
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
            let button = components::interstitial_cta_button(
                "home_clone_repo",
                "Clone Repository",
                "icons/cloud.svg",
                false,
                theme,
                cta_scale,
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
            components::interstitial_cta_button(
                "home_init_repo",
                "Initialize Repository",
                "icons/git_branch.svg",
                false,
                theme,
                cta_scale,
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

        self.sync_home_rows(cx);
        let filtering = !query.is_empty();
        let workspaces_column = self.home_column(
            HomeColumn {
                title: "Workspaces",
                frame_id: "home_workspaces_list",
                list_id: "home_workspaces_rows",
                scrollbar_id: "home_workspaces_scrollbar",
                empty_text: if filtering {
                    "No matching workspaces."
                } else {
                    "No saved workspaces yet. Every window with repositories open is one."
                },
            },
            self.home_rows.workspaces.len(),
            // Workspace rows are the shared picker rows, repositories Home's own.
            components::picker_row_height(
                crate::ui_scale::UiScale::from_percent(self.ui_scale_percent),
                true,
            )
            .into(),
            self.home_workspaces_scroll.clone(),
            Self::render_home_workspace_rows,
            cx,
        );
        let repositories_column = self.home_column(
            HomeColumn {
                title: "Recent repositories",
                frame_id: "home_recent_list",
                list_id: "home_recent_rows",
                scrollbar_id: "home_recent_scrollbar",
                empty_text: if filtering {
                    "No matching repositories."
                } else {
                    "Repositories you open appear here."
                },
            },
            self.home_rows.repositories.len(),
            self.home_row_height().into(),
            self.home_repositories_scroll.clone(),
            Self::render_home_repository_rows,
            cx,
        );

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
                    this.submit_external_drag_payload_from_home(paths.clone(), cx);
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
                                            .child(crate::view::product_name()),
                                    ),
                            )
                            .when_some(
                                gitcomet_core::identity::current().branding().tagline,
                                |home, tagline| {
                                    home.child(
                                        div()
                                            .id("home_tagline")
                                            .debug_selector(|| "home_tagline".to_string())
                                            .mt(scaled_px(-8.0))
                                            .text_size(theme.ui_text(14.0))
                                            .text_color(colors.muted)
                                            .child(tagline),
                                    )
                                },
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
                            .child(
                                div()
                                    .id("home_columns")
                                    .debug_selector(|| "home_columns".to_string())
                                    .w_full()
                                    .flex()
                                    .items_start()
                                    .gap(scaled_px(16.0))
                                    .child(workspaces_column)
                                    .child(repositories_column),
                            ),
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
