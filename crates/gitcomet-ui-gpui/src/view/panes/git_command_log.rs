use super::super::*;
use crate::ui_scale::UiScale;
use gitcomet_core::git_execution_log::output::OutputPreview;
use gitcomet_core::git_execution_log::{
    ExecutionEntry, ExecutionKind, ExecutionOutcome, ExecutionState, GitExecutionLog,
    LogSubscription, RepoLogKey,
};

const ROW_HEIGHT: f32 = 28.0;
const TABLE_MIN_WIDTH: f32 = 1000.0;
/// Columns after the command, widths in UI-font units.
const COLUMNS: [(&str, f32); 5] = [
    ("Started", 170.0),
    ("Duration", 85.0),
    ("Status", 100.0),
    ("Backend", 64.0),
    ("Source", 180.0),
];
/// The Details tab's label column, in UI-font units.
const DETAIL_LABEL_WIDTH: f32 = 112.0;
const UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const RUNNING_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum BackendFilter {
    #[default]
    All,
    Git,
    Gix,
}

impl BackendFilter {
    fn label(self) -> &'static str {
        match self {
            Self::All => "All backends",
            Self::Git => "Git",
            Self::Gix => "gix",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::All => Self::Git,
            Self::Git => Self::Gix,
            Self::Gix => Self::All,
        }
    }
}

#[derive(Default)]
struct Rows {
    entries: Vec<ExecutionEntry>,
    visible: Vec<usize>,
    query: String,
    backend: BackendFilter,
    failures_only: bool,
    selected: Option<u64>,
}

impl Rows {
    fn filter(&mut self) {
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let backend_matches = match self.backend {
                    BackendFilter::All => true,
                    BackendFilter::Git => entry.kind == ExecutionKind::Git,
                    BackendFilter::Gix => entry.kind == ExecutionKind::Gix,
                };
                (backend_matches
                    && (!self.failures_only || entry.is_failure())
                    && (self.query.is_empty()
                        || entry.details().to_lowercase().contains(&self.query)))
                .then_some(index)
            })
            .collect();
        if self.selected.is_some_and(|id| {
            !self
                .visible
                .iter()
                .any(|index| self.entries[*index].id == id)
        }) {
            self.selected = None;
        }
    }

    fn selected(&self) -> Option<&ExecutionEntry> {
        self.entries
            .iter()
            .find(|entry| Some(entry.id) == self.selected)
    }

    fn copy_visible(&self) -> String {
        self.visible
            .iter()
            .map(|index| self.entries[*index].details())
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum DetailTab {
    #[default]
    Details,
    Stdout,
    Stderr,
}

/// One labelled value on the Details tab; the value stays selectable.
struct DetailField {
    label: &'static str,
    alert: bool,
    value: Entity<components::TextInput>,
}

struct Panel {
    key: RepoLogKey,
    rows: Rows,
    query_input: Entity<components::TextInput>,
    detail_fields: Vec<DetailField>,
    output_input: Entity<components::TextInput>,
    detail_scroll: ScrollHandle,
    detail_tab: DetailTab,
    scroll: UniformListScrollHandle,
    horizontal_scroll: ScrollHandle,
    _query_subscription: gpui::Subscription,
    _journal_subscription: LogSubscription,
    _updates: gpui::Task<()>,
    wake: smol::channel::Sender<()>,
}

pub(in crate::view) struct GitCommandLogPane {
    state: Arc<AppState>,
    theme: AppTheme,
    preferences: UiPreferences,
    panels: FxHashMap<RepoId, Panel>,
    visible_repo: Option<RepoId>,
    root_view: WeakEntity<GitCometView>,
    _model_subscription: gpui::Subscription,
}

impl GitCommandLogPane {
    pub(in crate::view) fn new(
        model: Entity<AppUiModel>,
        theme: AppTheme,
        root_view: WeakEntity<GitCometView>,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&model.read(cx).state);
        let preferences = model.read(cx).preferences.clone();
        let subscription = cx.observe(&model, |this, model, cx| {
            let model = model.read(cx);
            let changed = this.state.active_repo != model.state.active_repo
                || this.preferences.appearance != model.preferences.appearance
                || this
                    .panels
                    .keys()
                    .any(|id| !model.state.repos.iter().any(|repo| repo.id == *id));
            this.state = Arc::clone(&model.state);
            this.preferences = model.preferences.clone();
            this.panels
                .retain(|id, _| this.state.repos.iter().any(|repo| repo.id == *id));
            if changed {
                if let Some(repo) = this.state.active_repo {
                    this.sync_detail(repo, cx);
                }
                cx.notify();
            }
        });
        Self {
            state,
            theme,
            preferences,
            panels: FxHashMap::default(),
            visible_repo: None,
            root_view,
            _model_subscription: subscription,
        }
    }

    pub(in crate::view) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    pub(in crate::view) fn set_appearance(
        &mut self,
        appearance: preferences::AppearancePreferences,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.preferences.appearance == appearance {
            return;
        }
        self.preferences.appearance = appearance;
        if let Some(repo) = self.state.active_repo {
            self.sync_detail(repo, cx);
        }
        cx.notify();
    }

    pub(in crate::view) fn set_visible(&mut self, repo: Option<RepoId>) {
        if self.visible_repo != repo
            && let Some(panel) = repo.and_then(|id| self.panels.get(&id))
        {
            let _ = panel.wake.try_send(());
        }
        self.visible_repo = repo;
    }

    pub(in crate::view) fn is_open(&self, repo: RepoId) -> bool {
        self.panels.contains_key(&repo)
    }

    pub(in crate::view) fn close(&mut self, repo: RepoId, cx: &mut gpui::Context<Self>) {
        self.panels.remove(&repo);
        let root = self.root_view.clone();
        cx.defer(move |cx| {
            let _ = root.update(cx, |root, cx| {
                root.on_git_command_log_panel_closed(repo, cx)
            });
        });
        cx.notify();
    }

    pub(in crate::view) fn open(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        if self.panels.contains_key(&repo_id) {
            self.refresh(repo_id, cx);
            return;
        }
        let Some(repo) = self.state.repos.iter().find(|repo| repo.id == repo_id) else {
            return;
        };
        let key = RepoLogKey::new(&repo.spec.workdir);
        let (sender, receiver) = smol::channel::bounded(1);
        let wake = sender.clone();
        let journal_subscription = GitExecutionLog::global().subscribe(&key, move || {
            let _ = sender.try_send(());
        });
        let query_input = cx.new(|cx| {
            components::TextInput::new_inert(
                components::TextInputOptions {
                    placeholder: "Search commands, sources and output…".into(),
                    ..Default::default()
                },
                cx,
            )
        });
        let query_subscription = cx.observe(&query_input, move |this, input, cx| {
            let text = input.read(cx).text().trim().to_lowercase();
            let Some(panel) = this.panels.get_mut(&repo_id) else {
                return;
            };
            if panel.rows.query == text {
                return;
            }
            panel.rows.query = text;
            panel.rows.filter();
            panel.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
            this.sync_detail(repo_id, cx);
            cx.notify();
        });
        let detail_scroll = ScrollHandle::new();
        let output_input = cx.new(|cx| {
            let mut input = components::TextInput::new_inert(
                components::TextInputOptions::selectable_multiline(),
                cx,
            );
            input.set_vertical_scroll_handle(Some(detail_scroll.clone()));
            input
        });
        let updates = cx.spawn(async move |view, cx| {
            loop {
                let ticking = view
                    .read_with(cx, |this, _| {
                        this.visible_repo == Some(repo_id)
                            && this.panels.get(&repo_id).is_some_and(|panel| {
                                panel
                                    .rows
                                    .entries
                                    .iter()
                                    .any(|entry| matches!(entry.state, ExecutionState::Running))
                            })
                    })
                    .unwrap_or(false);
                let notification = if ticking {
                    let timer = cx.background_executor().timer(RUNNING_INTERVAL);
                    smol::future::or(async { receiver.recv().await.map(|_| true) }, async {
                        timer.await;
                        Ok(false)
                    })
                    .await
                } else {
                    receiver.recv().await.map(|_| true)
                };
                let Ok(changed) = notification else {
                    break;
                };
                cx.background_executor().timer(UPDATE_INTERVAL).await;
                while receiver.try_recv().is_ok() {}
                if view
                    .update(cx, |this, cx| {
                        if changed || this.visible_repo == Some(repo_id) {
                            this.refresh(repo_id, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.panels.insert(
            repo_id,
            Panel {
                key,
                rows: Rows::default(),
                query_input,
                detail_fields: Vec::new(),
                output_input,
                detail_scroll,
                detail_tab: DetailTab::Details,
                scroll: UniformListScrollHandle::default(),
                horizontal_scroll: ScrollHandle::new(),
                _query_subscription: query_subscription,
                _journal_subscription: journal_subscription,
                _updates: updates,
                wake,
            },
        );
        self.refresh(repo_id, cx);
    }

    fn refresh(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        let Some(panel) = self.panels.get_mut(&repo_id) else {
            return;
        };
        panel.rows.entries = GitExecutionLog::global().snapshot(&panel.key);
        panel.rows.filter();
        self.sync_detail(repo_id, cx);
        if self.visible_repo == Some(repo_id) {
            cx.notify();
        }
    }

    /// Updates the open tab's text; the details are hidden without a selection.
    fn sync_detail(&mut self, repo_id: RepoId, cx: &mut gpui::Context<Self>) {
        let Some(panel) = self.panels.get_mut(&repo_id) else {
            return;
        };
        let Some(entry) = panel.rows.selected() else {
            return;
        };
        let output = match panel.detail_tab {
            DetailTab::Details => None,
            DetailTab::Stdout => Some(entry.stdout.text.clone()),
            DetailTab::Stderr => Some(entry.stderr.text.clone()),
        };
        if let Some(output) = output {
            panel.output_input.update(cx, |input, cx| {
                input.set_text_preserving_selection_on_append(output, cx)
            });
            return;
        }
        let failure = entry.is_failure();
        let values = detail_values(entry, &self.preferences.appearance);
        panel.detail_fields.truncate(values.len());
        for (index, (label, text)) in values.into_iter().enumerate() {
            if index == panel.detail_fields.len() {
                let value = cx.new(|cx| {
                    let mut input = components::TextInput::new_inert(
                        components::TextInputOptions::selectable_multiline(),
                        cx,
                    );
                    input.set_display_text(cx);
                    input
                });
                panel.detail_fields.push(DetailField {
                    label,
                    alert: false,
                    value,
                });
            }
            let field = &mut panel.detail_fields[index];
            field.label = label;
            field.alert = failure && matches!(label, "Status" | "Error");
            field.value.update(cx, |input, cx| input.set_text(text, cx));
        }
    }

    fn render_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(repo_id) = this.state.active_repo else {
            return Vec::new();
        };
        let Some(panel) = this.panels.get(&repo_id) else {
            return Vec::new();
        };
        let selected = panel.rows.selected;
        let entries: Vec<_> = range
            .filter_map(|index| {
                panel
                    .rows
                    .visible
                    .get(index)
                    .map(|row| panel.rows.entries[*row].clone())
            })
            .collect();
        let theme = this.theme;
        let scale = UiScale::from_percent(this.preferences.appearance.ui_scale_percent)
            .with_appearance(theme.metrics);
        entries
            .into_iter()
            .map(|entry| {
                let id = entry.id;
                let mut time = String::new();
                let appearance = &this.preferences.appearance;
                format_datetime_into(
                    &mut time,
                    entry.started_at,
                    DateTimeFormat::YmdHms,
                    appearance.timezone,
                    appearance.show_timezone,
                );
                let values = [
                    time,
                    format!("{:.1} ms", entry.elapsed.as_secs_f64() * 1000.0),
                    entry.status(),
                    entry.kind.to_string(),
                    one_line(&entry.context.origin.to_string()),
                ];
                let color = if entry.is_failure() {
                    theme.colors.status.danger.foreground
                } else {
                    theme.colors.foreground.primary
                };
                div()
                    .id(("git_command_log_row", id as usize))
                    .debug_selector(move || format!("git_command_log_row_{id}"))
                    .flex()
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap(scale.px(8.0))
                    .px(scale.px(8.0))
                    .h(scale.row_height(ROW_HEIGHT, 36.0))
                    .text_size(theme.ui_text(12.0))
                    .text_color(color)
                    .bg(if selected == Some(id) {
                        theme.colors.interaction.selected_background
                    } else {
                        theme.colors.surface.canvas
                    })
                    .hover_style(|style| style.bg(theme.colors.interaction.hover_background))
                    .cursor_pointer()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .child(command_text(&entry, theme.colors.foreground.secondary)),
                    )
                    .children(
                        values
                            .into_iter()
                            .zip(COLUMNS)
                            .map(|(text, (_, width))| cell(text, width, scale)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(panel) = this.panels.get_mut(&repo_id) {
                            panel.rows.selected = Some(id);
                        }
                        this.sync_detail(repo_id, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect()
    }
}

/// Cells stay on one line; wrapped text spilled over the fixed-height rows.
fn cell(text: impl Into<SharedString>, width: f32, scale: UiScale) -> gpui::Div {
    div()
        .w(scale.px(scale.appearance.ui_text(width)))
        .flex_none()
        .truncate()
        .child(text.into())
}

/// Hard breaks still split a nowrap line, so row text drops them.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Git rows lead with the backend's readable label and mute the invocation.
fn command_text(entry: &ExecutionEntry, muted: gpui::Rgba) -> gpui::StyledText {
    if entry.kind == ExecutionKind::Gix {
        return gpui::StyledText::new(one_line(&entry.command));
    }
    let label = one_line(&entry.issuer);
    let text = format!("{label}  {}", one_line(&entry.command));
    let invocation = label.len()..text.len();
    gpui::StyledText::new(text).with_highlights([(
        invocation,
        gpui::HighlightStyle {
            color: Some(palette::IntoColor::into_color(muted)),
            ..Default::default()
        },
    )])
}

/// The Details tab's labelled values. Absent optional values are left out.
fn detail_values(
    entry: &ExecutionEntry,
    appearance: &preferences::AppearancePreferences,
) -> Vec<(&'static str, String)> {
    let mut started = String::new();
    format_datetime_into(
        &mut started,
        entry.started_at,
        DateTimeFormat::YmdHms,
        appearance.timezone,
        appearance.show_timezone,
    );
    let command = match entry.kind {
        ExecutionKind::Git => "Command",
        ExecutionKind::Gix => "Operation",
    };
    let mut values = vec![(command, entry.command.clone()), ("Status", entry.status())];
    if let ExecutionState::Finished {
        outcome: ExecutionOutcome::Failed(error),
        ..
    } = &entry.state
    {
        values.push(("Error", error.clone()));
    }
    if let Some(code) = entry.exit_code {
        values.push(("Exit code", code.to_string()));
    }
    values.extend([
        ("Started", started),
        (
            "Duration",
            format!("{:.3} ms", entry.elapsed.as_secs_f64() * 1000.0),
        ),
        ("Source", entry.context.origin.to_string()),
        ("Issuer", entry.issuer.clone()),
        ("Backend", entry.kind.to_string()),
        ("Thread", entry.thread.clone()),
        ("Repository", entry.repository.path().display().to_string()),
    ]);
    if let Some(directory) = &entry.working_directory {
        values.push(("Working directory", directory.display().to_string()));
    }
    values.push(("Execution ID", entry.id.to_string()));
    values.push(("Action ID", entry.context.action_id.to_string()));
    if let Some(parent) = entry.context.parent_execution {
        values.push(("Parent execution", parent.to_string()));
    }
    values
}

fn output_caption(output: &OutputPreview) -> String {
    match (output.total_bytes, output.omitted_bytes) {
        (0, _) => "No output captured.".into(),
        (total, 0) => format!("{total} bytes captured"),
        (total, omitted) => format!("{total} bytes captured · {omitted} omitted"),
    }
}

impl Render for GitCommandLogPane {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let scale = UiScale::from_percent(self.preferences.appearance.ui_scale_percent)
            .with_appearance(theme.metrics);
        let container = div()
            .id("git_command_log_panel")
            .debug_selector(|| "git_command_log_panel".to_string())
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .bg(theme.colors.surface.canvas);
        let Some(repo_id) = self.state.active_repo else {
            return container;
        };
        let Some(panel) = self.panels.get(&repo_id) else {
            return container;
        };
        let count = panel.rows.visible.len();
        let total = panel.rows.entries.len();
        let query = panel.query_input.clone();
        let scroll = panel.scroll.clone();
        let horizontal_scroll = panel.horizontal_scroll.clone();
        let detail_scroll = panel.detail_scroll.clone();
        let backend = panel.rows.backend;
        let failures = panel.rows.failures_only;
        let selected = panel.rows.selected().is_some();
        let detail_tab = panel.detail_tab;
        let detail_body =
            match (detail_tab, panel.rows.selected()) {
                (_, None) => div(),
                (DetailTab::Details, Some(_)) => div()
                    .flex()
                    .flex_col()
                    .gap(scale.px(4.0))
                    .py(scale.px(4.0))
                    .children(panel.detail_fields.iter().map(|field| {
                        div()
                            .flex()
                            .items_start()
                            .gap(scale.px(12.0))
                            .child(
                                div()
                                    .w(scale.px(scale.appearance.ui_text(DETAIL_LABEL_WIDTH)))
                                    .flex_none()
                                    .truncate()
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(field.label),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .text_color(if field.alert {
                                        theme.colors.status.danger.foreground
                                    } else {
                                        theme.colors.foreground.primary
                                    })
                                    .child(field.value.clone()),
                            )
                    })),
                (tab, Some(entry)) => div()
                    .flex()
                    .flex_col()
                    .gap(scale.px(4.0))
                    .pt(scale.px(4.0))
                    .child(div().text_color(theme.colors.foreground.secondary).child(
                        output_caption(if tab == DetailTab::Stderr {
                            &entry.stderr
                        } else {
                            &entry.stdout
                        }),
                    ))
                    .child(panel.output_input.clone()),
            };
        let close = components::on_nested_control_click(
            components::panel_tab_close(
                "git_command_log_close",
                theme,
                scale,
                theme.colors.interaction.selected_foreground,
            )
            .debug_selector(|| "git_command_log_close".into()),
            cx,
            move |this, _, _, cx| this.close(repo_id, cx),
        );
        let title = components::panel_tab(
            "git_command_log_title",
            theme,
            scale,
            "icons/history.svg",
            "Git Command Log",
            true,
        )
        .child(close);
        let controls = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(scale.px(6.0))
            .p(scale.px(4.0))
            .bg(theme.colors.surface.panel)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(title)
            .child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(format!("{count}/{total}")),
            )
            .child(div().flex_1().min_w(scale.px(180.0)).child(query))
            .child(
                components::Button::new("git_command_log_backend", backend.label())
                    .on_click_handler(
                        theme,
                        scale,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(panel) = this.panels.get_mut(&repo_id) {
                                panel.rows.backend = panel.rows.backend.next();
                                panel.rows.filter();
                            }
                            this.sync_detail(repo_id, cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                components::Button::new("git_command_log_failures", "Failures only")
                    .selected(failures)
                    .on_click_handler(
                        theme,
                        scale,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(panel) = this.panels.get_mut(&repo_id) {
                                panel.rows.failures_only = !panel.rows.failures_only;
                                panel.rows.filter();
                            }
                            this.sync_detail(repo_id, cx);
                            cx.notify();
                        }),
                    ),
            )
            .child(
                components::Button::new("git_command_log_copy_entry", "Copy entry")
                    .disabled(!selected)
                    .on_click_handler(
                        theme,
                        scale,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(entry) = this
                                .panels
                                .get(&repo_id)
                                .and_then(|panel| panel.rows.selected())
                            {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    entry.details(),
                                ));
                            }
                        }),
                    ),
            )
            .child(
                components::Button::new("git_command_log_copy_visible", "Copy visible")
                    .disabled(count == 0)
                    .on_click_handler(
                        theme,
                        scale,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(panel) = this.panels.get(&repo_id) {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    panel.rows.copy_visible(),
                                ));
                            }
                        }),
                    ),
            )
            .child(
                components::Button::new("git_command_log_clear", "Clear")
                    .disabled(total == 0)
                    .on_click_handler(
                        theme,
                        scale,
                        cx.listener(move |this, _, _, cx| {
                            if let Some(panel) = this.panels.get(&repo_id) {
                                GitExecutionLog::global().clear(&panel.key);
                            }
                            this.refresh(repo_id, cx);
                            cx.notify();
                        }),
                    ),
            );
        let table_header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(scale.px(8.0))
            .px(scale.px(8.0))
            .py(scale.px(3.0))
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .border_b_1()
            .border_color(theme.colors.stroke.subtle)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .child("Command / operation"),
            )
            .children(COLUMNS.map(|(label, width)| cell(label, width, scale)));
        let body = if count == 0 {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_color(theme.colors.foreground.secondary)
                .child(if total == 0 {
                    "No executions yet. New Git activity appears here."
                } else {
                    "No executions match these filters."
                })
                .into_any_element()
        } else {
            uniform_list(
                ("git_command_log_rows", repo_id.0 as usize),
                count,
                cx.processor(Self::render_rows),
            )
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(&scroll)
            .into_any_element()
        };
        let table = div()
            .id("git_command_log_table")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(
                div()
                    .id(("git_command_log_horizontal_scroll", repo_id.0 as usize))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_x_scroll()
                    .track_scroll(&horizontal_scroll)
                    .pb(components::Scrollbar::gutter(
                        components::ScrollbarAxis::Horizontal,
                    ))
                    .pr(components::Scrollbar::gutter(
                        components::ScrollbarAxis::Vertical,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h(px(0.0))
                            .min_w(scale.px(scale.appearance.ui_text(TABLE_MIN_WIDTH)))
                            .child(table_header)
                            .child(body),
                    ),
            )
            .child(
                components::Scrollbar::horizontal(
                    "git_command_log_horizontal_bar",
                    horizontal_scroll,
                )
                .render(theme),
            )
            .child(
                components::Scrollbar::new("git_command_log_vertical_bar", scroll).render(theme),
            );
        let close_details = components::Button::new("git_command_log_detail_close", "")
            .start_slot(svg_icon(
                "icons/generic_close.svg",
                theme.colors.foreground.secondary,
                scale.px(12.0),
            ))
            .style(components::ButtonStyle::Transparent)
            .on_click_handler(
                theme,
                scale,
                cx.listener(move |this, _, _, cx| {
                    if let Some(panel) = this.panels.get_mut(&repo_id) {
                        panel.rows.selected = None;
                    }
                    cx.notify();
                }),
            )
            .gitcomet_tooltip(theme, "Close details".into());
        let detail_tabs = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(scale.px(4.0))
            .p(scale.px(4.0))
            .children(
                [
                    (DetailTab::Details, "Details", "git_command_log_details"),
                    (DetailTab::Stdout, "stdout", "git_command_log_stdout"),
                    (DetailTab::Stderr, "stderr", "git_command_log_stderr"),
                ]
                .into_iter()
                .map(|(tab, label, id)| {
                    components::Button::new(id, label)
                        .selected(detail_tab == tab)
                        .on_click_handler(
                            theme,
                            scale,
                            cx.listener(move |this, _, _, cx| {
                                if let Some(panel) = this.panels.get_mut(&repo_id) {
                                    panel.detail_tab = tab;
                                }
                                this.sync_detail(repo_id, cx);
                                cx.notify();
                            }),
                        )
                }),
            )
            .child(div().flex_1())
            .child(close_details);
        let details = div()
            .flex()
            .flex_col()
            .w(gpui::relative(0.38))
            .min_w(scale.px(240.0))
            .min_h(px(0.0))
            .text_size(theme.ui_text(12.0))
            .border_l_1()
            .border_color(theme.colors.stroke.subtle)
            .child(detail_tabs)
            .child(
                div()
                    .id(("git_command_log_detail_scroll", repo_id.0 as usize))
                    .flex_1()
                    .min_h(px(0.0))
                    .px(scale.px(8.0))
                    .overflow_y_scroll()
                    .track_scroll(&detail_scroll)
                    .child(detail_body),
            );
        container.child(controls).child(
            div()
                .flex()
                .flex_1()
                .min_h(px(0.0))
                .child(table)
                .when(selected, |row| row.child(details)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::git_execution_log::context::{ExecutionContext, Origin, UiSurface};

    fn fixture() -> (GitExecutionLog, RepoLogKey) {
        let log = GitExecutionLog::default();
        let key = RepoLogKey::new(std::path::Path::new("/command-log-filter-test"));
        let source = ExecutionContext::ui(UiSurface::Button, "Fetch toolbar");
        let _scope = source.attach();
        log.begin(
            &key,
            ExecutionKind::Git,
            "git fetch origin",
            "Remote backend",
        )
        .finish(ExecutionOutcome::Success);
        log.begin(
            &key,
            ExecutionKind::Gix,
            "Read worktree status",
            "Status backend",
        )
        .finish(ExecutionOutcome::Failed("Missing index".into()));
        (log, key)
    }

    #[test]
    fn filters_search_sources_output_and_backend_and_copy_only_visible_entries() {
        let (log, key) = fixture();
        let mut rows = Rows {
            entries: log.snapshot(&key),
            ..Default::default()
        };
        rows.filter();
        assert_eq!(rows.visible.len(), 2);
        rows.query = "fetch toolbar".into();
        rows.filter();
        assert_eq!(rows.visible.len(), 2);
        rows.backend = BackendFilter::Gix;
        rows.failures_only = true;
        rows.filter();
        assert_eq!(rows.visible.len(), 1);
        assert!(rows.copy_visible().contains("Missing index"));
        assert!(!rows.copy_visible().contains("git fetch origin"));
        rows.query = "missing index".into();
        rows.filter();
        assert_eq!(rows.visible.len(), 1);
        rows.backend = BackendFilter::Git;
        rows.filter();
        assert!(rows.visible.is_empty());
    }

    #[test]
    fn selection_uses_execution_identity_and_is_removed_on_eviction_or_clear() {
        let (log, key) = fixture();
        let entries = log.snapshot(&key);
        let selected = entries[1].id;
        let mut rows = Rows {
            entries,
            selected: Some(selected),
            ..Default::default()
        };
        rows.filter();
        log.begin(&key, ExecutionKind::Git, "git status", "Status")
            .finish(ExecutionOutcome::Success);
        rows.entries = log.snapshot(&key);
        rows.filter();
        assert_eq!(rows.selected().unwrap().command, "git fetch origin");
        for _ in 0..100 {
            log.begin(&key, ExecutionKind::Git, "git status", "Status")
                .finish(ExecutionOutcome::Success);
        }
        rows.entries = log.snapshot(&key);
        rows.filter();
        assert!(rows.selected().is_none());
        rows.selected = Some(rows.entries[0].id);
        log.clear(&key);
        rows.entries = log.snapshot(&key);
        rows.filter();
        assert!(rows.selected().is_none());
        assert!(rows.copy_visible().is_empty());
    }

    /// Details were one text block with Debug values such as "Exit code: None"
    /// and "SystemTime { … }"; each value is now a labelled row.
    #[test]
    fn details_label_each_value_and_leave_out_absent_ones() {
        let (log, key) = fixture();
        let entries = log.snapshot(&key);
        let appearance = UiPreferences::default().appearance;
        let git = entries
            .iter()
            .find(|entry| entry.kind == ExecutionKind::Git)
            .unwrap();
        let values = detail_values(git, &appearance);
        assert_eq!(
            values.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            [
                "Command",
                "Status",
                "Started",
                "Duration",
                "Source",
                "Issuer",
                "Backend",
                "Thread",
                "Repository",
                "Execution ID",
                "Action ID",
            ]
        );
        assert_eq!(values[0].1, "git fetch origin");
        assert!(
            values
                .iter()
                .all(|(_, value)| !value.contains("None") && !value.contains("SystemTime"))
        );
        let gix = entries
            .iter()
            .find(|entry| entry.kind == ExecutionKind::Gix)
            .unwrap();
        let values = detail_values(gix, &appearance);
        assert_eq!(values[0], ("Operation", "Read worktree status".into()));
        assert_eq!(values[2], ("Error", "Missing index".into()));
    }

    struct Host {
        pane: Entity<GitCommandLogPane>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            div().size_full().flex().flex_col().child(self.pane.clone())
        }
    }

    #[gpui::test]
    fn journal_updates_do_not_publish_app_state_and_clear_does_not_resurrect_running_entries(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::view::test_support::NoopBackend;
        let directory = tempfile::tempdir().unwrap();
        let key = RepoLogKey::new(directory.path());
        let repo_id = RepoId(1);
        let (store, _events) = AppStore::new_test(Arc::new(NoopBackend));
        let store = Arc::new(store);
        let state = Arc::new(AppState {
            repos: vec![RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: directory.path().to_path_buf(),
                },
            )],
            active_repo: Some(repo_id),
            ..AppState::test_default()
        });
        let (host, cx) = cx.add_window_view(|_, cx| {
            let model = cx.new(|cx| {
                AppUiModel::new_with_preferences(
                    Arc::clone(&state),
                    UiPreferences::default(),
                    Arc::downgrade(&store),
                    cx,
                )
            });
            let pane = cx.new(|cx| {
                GitCommandLogPane::new(
                    model,
                    AppTheme::gitcomet_dark(),
                    WeakEntity::new_invalid(),
                    cx,
                )
            });
            pane.update(cx, |pane, cx| {
                pane.set_visible(Some(repo_id));
                pane.open(repo_id, cx);
            });
            Host { pane }
        });
        let pane = host.read_with(&*cx, |host, _| host.pane.clone());
        let context = ExecutionContext::new(Origin::Extension {
            command: "test.refresh\n\nstdout with a newline".into(),
        });
        let mut execution = {
            let _scope = context.attach();
            GitExecutionLog::global().begin(
                &key,
                ExecutionKind::Gix,
                "Native status",
                "Test backend",
            )
        };
        pump(cx);
        let id = pane.read_with(&*cx, |pane, _| {
            assert!(Arc::ptr_eq(&pane.state, &state));
            assert_eq!(pane.panels[&repo_id].rows.entries.len(), 1);
            pane.panels[&repo_id].rows.entries[0].id
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let row = cx
            .debug_bounds(format!("git_command_log_row_{id}").leak())
            .unwrap();
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        pane.read_with(&*cx, |pane, app| {
            let value = |label| {
                pane.panels[&repo_id]
                    .detail_fields
                    .iter()
                    .find(|field| field.label == label)
                    .map(|field| field.value.read(app).text().to_owned())
            };
            assert!(value("Source").unwrap().contains("stdout with a newline"));
            assert!(value("Duration").is_some());
            assert_eq!(value("Status").as_deref(), Some("Running"));
        });
        click(cx, "git_command_log_copy_entry");
        cx.update(|_, app| {
            let copied = app.read_from_clipboard().unwrap().text().unwrap();
            assert!(copied.contains("Native status"));
            assert!(copied.contains("test.refresh"));
        });
        execution.output().unwrap().append(
            gitcomet_core::git_operation::GitOutputStream::Stderr,
            b"Diagnostic output\n",
        );
        pump(cx);
        pane.read_with(&*cx, |pane, _| {
            assert!(
                pane.panels[&repo_id].rows.entries[0]
                    .stderr
                    .text
                    .contains("Diagnostic output")
            )
        });
        click(cx, "git_command_log_stderr");
        pane.read_with(&*cx, |pane, app| {
            assert!(
                pane.panels[&repo_id]
                    .output_input
                    .read(app)
                    .text()
                    .contains("Diagnostic output")
            )
        });
        click(cx, "git_command_log_detail_close");
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        pane.read_with(&*cx, |pane, _| {
            assert!(pane.panels[&repo_id].rows.selected.is_none())
        });
        assert!(cx.debug_bounds("git_command_log_detail_close").is_none());
        // Reselect, so Clear below has a selection to drop.
        cx.simulate_click(row.center(), gpui::Modifiers::default());
        pane.read_with(&*cx, |pane, _| {
            assert_eq!(pane.panels[&repo_id].rows.selected, Some(id))
        });
        pane.update(cx, |pane, _| pane.set_visible(None));
        pump(cx);
        let hidden_elapsed = pane.read_with(&*cx, |pane, _| {
            pane.panels[&repo_id].rows.entries[0].elapsed
        });
        cx.executor().advance_clock(RUNNING_INTERVAL * 4);
        cx.run_until_parked();
        assert_eq!(
            pane.read_with(&*cx, |pane, _| pane.panels[&repo_id].rows.entries[0]
                .elapsed),
            hidden_elapsed
        );
        pane.update(cx, |pane, _| pane.set_visible(Some(repo_id)));
        pump(cx);
        assert!(
            pane.read_with(&*cx, |pane, _| pane.panels[&repo_id].rows.entries[0]
                .elapsed)
                > hidden_elapsed
        );
        execution.finish(ExecutionOutcome::Success);
        pump(cx);
        pane.read_with(&*cx, |pane, _| {
            assert!(matches!(
                pane.panels[&repo_id].rows.entries[0].state,
                ExecutionState::Finished {
                    outcome: ExecutionOutcome::Success,
                    ..
                }
            ))
        });
        let mut running =
            GitExecutionLog::global().begin(&key, ExecutionKind::Git, "git status", "Test backend");
        pump(cx);
        click(cx, "git_command_log_clear");
        pane.read_with(&*cx, |pane, _| {
            assert!(pane.panels[&repo_id].rows.entries.is_empty());
            assert!(pane.panels[&repo_id].rows.selected.is_none());
            assert!(Arc::ptr_eq(&pane.state, &state));
        });
        running.finish(ExecutionOutcome::Success);
        pump(cx);
        pane.read_with(&*cx, |pane, _| {
            assert!(pane.panels[&repo_id].rows.entries.is_empty())
        });
        click(cx, "git_command_log_close");
        pane.read_with(&*cx, |pane, _| assert!(!pane.is_open(repo_id)));
    }

    fn pump(cx: &mut gpui::VisualTestContext) {
        for _ in 0..2 {
            cx.run_until_parked();
            cx.executor().advance_clock(UPDATE_INTERVAL);
        }
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }

    fn click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("Missing {selector}"));
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
    }
}
