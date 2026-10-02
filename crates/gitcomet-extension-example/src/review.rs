//! The example's extension: marking repositories reviewed.
//!
//! It registers one of every contribution through the public API only:
//! repository views (one of them hosting diff panes and a file list, see
//! [`crate::changes`]), a bottom panel, a details tab, a sidebar section and
//! sidebar rows, a history annotator, a status item, the edition strip and
//! title-bar brand, a settings page, commands with a key binding and menu
//! entries, an asset, a window gate, a repository-entry gate, a close guard,
//! and a hosted dialog. Review counts are per window and saved in that
//! window's workspace, so they survive a restart.

use crate::review_counter::ReviewCounter;
use gitcomet_core::domain::Commit;
use gitcomet_core::identity::WindowKind;
use gitcomet_extension_api::{
    BottomPanelDescriptor, ChromeDescriptor, CloseDecision, CloseRequest, CloseScope,
    CommandContext, CommandDescriptor, DetailsTabDescriptor, DialogHandle, EntryOrigin, Extension,
    ExtensionId, GateDecision, HistoryAnnotator, HistoryRowAnnotation, HostedAction,
    HostedMenuItem, MenuLocation, NotificationKind, Registrar, RepositoryEntryRequest,
    RepositoryViewContext, RepositoryViewDescriptor, RowGlyph, RowMark, SettingsPageContext,
    SettingsPageDescriptor, ShellEvent, SidebarProvider, SidebarRow, SidebarSectionDescriptor,
    SidebarSectionRows, SlotSignal, StatusItemDescriptor, WindowExtension, WindowGateDescriptor,
    WindowHost,
};
use gitcomet_ui_kit::components::{Button, ButtonStyle};
use gitcomet_ui_kit::gpui::prelude::*;
use gitcomet_ui_kit::gpui::{
    AnyView, App, Context, Entity, Global, Window, WindowId, div, rgb_to_hsla, svg,
};
use gitcomet_ui_kit::theme::AppTheme;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;

pub const EXTENSION_ID: &str = "com.example.review";
/// A repository holding this file cannot be opened: the example's gate.
pub const DENY_MARKER: &str = ".comet-example-deny";
/// Served at `extensions/com.example.review/icons/review.svg`.
pub const ICON_PATH: &str = "extensions/com.example.review/icons/review.svg";

const ICON_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2 8l4 4 8-8" stroke="currentColor" fill="none" stroke-width="2"/></svg>"#;

/// The Changes list's flag glyph, served at
/// `extensions/com.example.review/icons/flag.svg`.
pub const FLAG_ICON_PATH: &str = "extensions/com.example.review/icons/flag.svg";

const FLAG_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3 15V2h9l-2 3 2 3H3" stroke="currentColor" fill="none" stroke-width="1.5" stroke-linejoin="round"/></svg>"#;

/// The bottom panel listing this repository's reviews.
pub const REVIEW_LOG_PANEL: &str = "review-log";

/// Where the settings page's documentation link goes.
pub const DOCS_URL: &str = "https://example.com/comet-example/docs";

/// Set to `1` to open every window behind the example's gate.
pub const GATE_ENV: &str = "COMET_EXAMPLE_GATE";

/// The gate's revision. One per process is enough: the flag itself is per App.
static GATE_SIGNAL: LazyLock<SlotSignal> = LazyLock::new(SlotSignal::default);

struct Gate(bool);

impl Global for Gate {}

/// Whether windows show the gate: [`GATE_ENV`] until [`set_gated`] says
/// otherwise (tests cannot set environment variables).
pub fn is_gated(cx: &App) -> bool {
    cx.try_global::<Gate>().map_or_else(
        || std::env::var_os(GATE_ENV).is_some_and(|value| value == "1"),
        |gate| gate.0,
    )
}

/// Raises or lifts the gate in every window.
pub fn set_gated(gated: bool, cx: &mut App) {
    cx.set_global(Gate(gated));
    GATE_SIGNAL.bump();
    cx.refresh_windows();
}

pub fn extension_id() -> ExtensionId {
    ExtensionId::new(EXTENSION_ID).expect("the example id is valid")
}

/// Review counts per window and repository, shared by every view the
/// extension builds. Views observe it, so one change repaints each once.
#[derive(Default)]
pub struct Reviews {
    counts: HashMap<WindowId, BTreeMap<PathBuf, u64>>,
    /// Ask before closing a repository nobody has reviewed yet.
    pub confirm_close_unreviewed: bool,
}

impl Reviews {
    pub fn count(&self, window: WindowId, workdir: &Path) -> u64 {
        self.counts
            .get(&window)
            .and_then(|counts| counts.get(workdir))
            .copied()
            .unwrap_or(0)
    }

    pub fn window_total(&self, window: WindowId) -> u64 {
        self.counts
            .get(&window)
            .map_or(0, |counts| counts.values().sum())
    }

    /// Windows the model still tracks; closed ones are dropped.
    pub fn windows(&self) -> usize {
        self.counts.len()
    }
}

struct ReviewsGlobal(Entity<Reviews>);

impl Global for ReviewsGlobal {}

/// The shared model, created on first use.
pub fn reviews(cx: &mut App) -> Entity<Reviews> {
    if let Some(global) = cx.try_global::<ReviewsGlobal>() {
        return global.0.clone();
    }
    let entity = cx.new(|_| Reviews::default());
    cx.set_global(ReviewsGlobal(entity.clone()));
    entity
}

fn saved_counts(window: &WindowHost, cx: &App) -> BTreeMap<PathBuf, u64> {
    window
        .workspace_state(&extension_id(), cx)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_value(value.get("reviews")?.clone()).ok())
        .unwrap_or_default()
}

/// Marks the repository reviewed, saves the window's counts to its
/// workspace, and says so.
pub fn mark_reviewed(window: &WindowHost, workdir: &Path, cx: &mut App) {
    let model = reviews(cx);
    let window_id = window.id();
    let counts = model.update(cx, |reviews, cx| {
        let counts = reviews.counts.entry(window_id).or_default();
        *counts.entry(workdir.to_path_buf()).or_default() += 1;
        cx.notify();
        counts.clone()
    });
    let value = serde_json::json!({ "reviews": counts });
    let _ = window.set_workspace_state(&extension_id(), value, cx);
    let name = workdir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = window.notify(format!("Marked {name} reviewed"), cx);
}

/// The repository view: the count, and a button to add one.
pub struct ReviewView {
    context: RepositoryViewContext,
    reviews: Entity<Reviews>,
    _observe: gitcomet_ui_kit::gpui::Subscription,
}

impl ReviewView {
    fn new(context: RepositoryViewContext, cx: &mut Context<Self>) -> Self {
        let reviews = reviews(cx);
        let observe = cx.observe(&reviews, |_, _, cx| cx.notify());
        Self {
            context,
            reviews,
            _observe: observe,
        }
    }
}

impl Render for ReviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.window.theme(cx);
        let workdir = self.context.repository.workdir().to_path_buf();
        let count = self
            .reviews
            .read(cx)
            .count(self.context.window.id(), &workdir);
        let window = self.context.window.clone();
        div()
            .id("example_review_view")
            .debug_selector(|| "example_review_view".to_string())
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .text_color(theme.colors.foreground.primary)
            .child(format!("Reviewed {count} times"))
            .child(
                Button::new("example_review_mark", "Mark reviewed")
                    .style(ButtonStyle::Filled)
                    .on_click(theme, cx, move |_, _, _, cx| {
                        mark_reviewed(&window, &workdir, cx);
                    }),
            )
            .child(Button::new("example_review_more", "More").on_click(
                theme,
                cx,
                |this, event, _, cx| this.open_more(event.position(), cx),
            ))
    }
}

impl ReviewView {
    /// A menu of the view's actions, with the extension's own icon.
    fn open_more(
        &self,
        anchor: gitcomet_ui_kit::gpui::Point<gitcomet_ui_kit::gpui::Pixels>,
        cx: &mut App,
    ) {
        let window = self.context.window.clone();
        let workdir = self.context.repository.workdir().to_path_buf();
        let mark = HostedAction::new("Mark reviewed", move |cx| {
            mark_reviewed(&window, &workdir, cx)
        });
        let _ = self.context.window.open_menu(
            anchor,
            vec![HostedMenuItem::action(mark).with_icon(ICON_PATH)],
            cx,
        );
    }
}

/// The bottom panel: this repository's review count in the window.
pub struct ReviewLog {
    context: RepositoryViewContext,
    reviews: Entity<Reviews>,
    _observe: gitcomet_ui_kit::gpui::Subscription,
}

impl ReviewLog {
    fn new(context: RepositoryViewContext, cx: &mut Context<Self>) -> Self {
        let reviews = reviews(cx);
        let observe = cx.observe(&reviews, |_, _, cx| cx.notify());
        Self {
            context,
            reviews,
            _observe: observe,
        }
    }
}

impl Render for ReviewLog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.window.theme(cx);
        let workdir = self.context.repository.workdir();
        let count = self
            .reviews
            .read(cx)
            .count(self.context.window.id(), workdir);
        div()
            .id("example_review_log")
            .debug_selector(|| "example_review_log".to_string())
            .size_full()
            .p_2()
            .text_color(theme.colors.foreground.primary)
            .child(format!("{count} reviews recorded in this window"))
    }
}

/// The details tab and the sidebar section: the repository's count, under
/// `selector` so tests tell them apart.
pub struct ReviewCount {
    context: RepositoryViewContext,
    reviews: Entity<Reviews>,
    selector: &'static str,
    _observe: gitcomet_ui_kit::gpui::Subscription,
}

impl ReviewCount {
    fn new(context: RepositoryViewContext, selector: &'static str, cx: &mut Context<Self>) -> Self {
        let reviews = reviews(cx);
        let observe = cx.observe(&reviews, |_, _, cx| cx.notify());
        Self {
            context,
            reviews,
            selector,
            _observe: observe,
        }
    }
}

impl Render for ReviewCount {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.window.theme(cx);
        let workdir = self.context.repository.workdir();
        let count = self
            .reviews
            .read(cx)
            .count(self.context.window.id(), workdir);
        let selector = self.selector;
        div()
            .id(selector)
            .debug_selector(move || selector.to_string())
            .px_3()
            .py_1()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(format!("Reviewed {count} times"))
    }
}

fn toggle_review_log(context: CommandContext, _window: &mut Window, cx: &mut App) {
    let Some(repository) = &context.repository else {
        return;
    };
    let panel = extension_id()
        .contribution(REVIEW_LOG_PANEL)
        .expect("the panel id is valid");
    let _ = if context.window.is_bottom_panel_open(repository, &panel, cx) {
        context.window.close_bottom_panel(repository, &panel, cx)
    } else {
        context.window.open_bottom_panel(repository, &panel, cx)
    };
}

/// The status item: reviews in this window, out of its open repositories.
/// It follows the window's state through a subscription, not polling.
pub struct ReviewStatus {
    window: WindowHost,
    reviews: Entity<Reviews>,
    _observe: gitcomet_ui_kit::gpui::Subscription,
    _state: Option<gitcomet_extension_api::StateSubscription>,
}

impl ReviewStatus {
    fn new(window: WindowHost, cx: &mut Context<Self>) -> Self {
        let reviews = reviews(cx);
        let view = cx.weak_entity();
        let state = window
            .observe_state(move |_, cx| {
                let _ = view.update(cx, |_, cx| cx.notify());
            })
            .ok();
        Self {
            _observe: cx.observe(&reviews, |_, _, cx| cx.notify()),
            _state: state,
            window,
            reviews,
        }
    }
}

impl Render for ReviewStatus {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.window.theme(cx);
        let total = self.reviews.read(cx).window_total(self.window.id());
        let open = self.window.state(cx).map_or(0, |state| state.repos.len());
        div()
            .id("example_review_status")
            .debug_selector(|| "example_review_status".to_string())
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(format!("{total} reviewed · {open} open"))
    }
}

/// The settings page: whether closing an unreviewed repository asks first,
/// and a reset that asks before it forgets anything.
pub struct ReviewSettings {
    context: SettingsPageContext,
    reviews: Entity<Reviews>,
}

impl Render for ReviewSettings {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.context.theme;
        let enabled = self.reviews.read(cx).confirm_close_unreviewed;
        let label = if enabled {
            "Ask before closing unreviewed repositories: on"
        } else {
            "Ask before closing unreviewed repositories: off"
        };
        div()
            .id("example_review_settings")
            .debug_selector(|| "example_review_settings".to_string())
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                Button::new("example_review_settings_toggle", label).on_click(
                    theme,
                    cx,
                    |this, _, _, cx| {
                        this.reviews.update(cx, |reviews, cx| {
                            reviews.confirm_close_unreviewed = !reviews.confirm_close_unreviewed;
                            cx.notify();
                        });
                        let enabled = this.reviews.read(cx).confirm_close_unreviewed;
                        let value = serde_json::json!({ "confirm_close_unreviewed": enabled });
                        let _ = gitcomet_extension_api::storage::save(&extension_id(), Some(value));
                        let message = if enabled {
                            "Closing an unreviewed repository will ask first"
                        } else {
                            "Unreviewed repositories close without asking"
                        };
                        let _ = this.context.host.toast(
                            NotificationKind::Success,
                            message,
                            Vec::new(),
                            cx,
                        );
                        cx.notify();
                    },
                ),
            )
            .child(
                Button::new("example_review_reset", "Reset review counts…").on_click(
                    theme,
                    cx,
                    |this, _, _, cx| confirm_reset(&this.context.host, this.context.theme, cx),
                ),
            )
            .child(
                Button::new("example_review_docs", "Documentation").on_click(
                    theme,
                    cx,
                    |this, _, _, cx| {
                        let _ = this.context.host.open_url(DOCS_URL, cx);
                    },
                ),
            )
    }
}

/// Asks in a hosted dialog before forgetting every window's counts.
fn confirm_reset(host: &WindowHost, theme: AppTheme, cx: &mut App) {
    let handle: Rc<RefCell<Option<DialogHandle>>> = Rc::default();
    let shared = Rc::clone(&handle);
    let window = host.clone();
    let opened = host.open_dialog(
        "Reset review counts?",
        move |_, cx| -> AnyView {
            cx.new(|_| ResetConfirm {
                theme,
                window,
                handle: shared,
            })
            .into()
        },
        cx,
    );
    *handle.borrow_mut() = opened.ok();
}

/// The reset confirmation's body.
struct ResetConfirm {
    theme: AppTheme,
    window: WindowHost,
    handle: Rc<RefCell<Option<DialogHandle>>>,
}

impl Render for ResetConfirm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("example_reset_confirm")
            .debug_selector(|| "example_reset_confirm".to_string())
            .flex()
            .flex_col()
            .gap_2()
            .child("Every window forgets how often its repositories were reviewed.")
            .child(
                Button::new("example_reset_confirm_button", "Reset")
                    .style(ButtonStyle::Danger)
                    .on_click(self.theme, cx, |this, _, _, cx| {
                        reviews(cx).update(cx, |reviews, cx| {
                            reviews.counts.values_mut().for_each(BTreeMap::clear);
                            cx.notify();
                        });
                        if let Some(handle) = this.handle.borrow_mut().take() {
                            handle.close(cx);
                        }
                        let _ = this.window.toast(
                            NotificationKind::Success,
                            "Review counts reset",
                            Vec::new(),
                            cx,
                        );
                    }),
            )
    }
}

fn gate(request: &RepositoryEntryRequest, _cx: &App) -> GateDecision {
    // A restored workspace keeps its repositories; the marker only stops new
    // entries, the way a licence check might.
    if request.origin != EntryOrigin::WorkspaceRestore && request.path.join(DENY_MARKER).exists() {
        GateDecision::Deny {
            reason: format!(
                "{} is marked as not for review ({DENY_MARKER}).",
                request.path.display()
            )
            .into(),
        }
    } else {
        GateDecision::Allow
    }
}

fn close_guard(request: &CloseRequest, cx: &App) -> CloseDecision {
    let Some(global) = cx.try_global::<ReviewsGlobal>() else {
        return CloseDecision::Allow;
    };
    let reviews = global.0.read(cx);
    if request.scope != CloseScope::Repository || !reviews.confirm_close_unreviewed {
        return CloseDecision::Allow;
    }
    match &request.repository {
        Some(repository) if reviews.count(request.window.id(), repository.workdir()) == 0 => {
            CloseDecision::Confirm {
                reason: format!(
                    "{} has not been reviewed yet.",
                    repository.workdir().display()
                )
                .into(),
            }
        }
        _ => CloseDecision::Allow,
    }
}

/// The gate: the product is locked until someone continues.
pub struct GateView {
    window: WindowHost,
}

impl Render for GateView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.window.theme(cx);
        div()
            .id("example_gate")
            .debug_selector(|| "example_gate".to_string())
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .bg(theme.colors.surface.canvas)
            .text_color(theme.colors.foreground.primary)
            .child(format!("{} is locked", crate::DISPLAY_NAME))
            .child(
                Button::new("example_gate_continue", "Continue")
                    .style(ButtonStyle::Filled)
                    .on_click(theme, cx, |_, _, _, cx| set_gated(false, cx)),
            )
    }
}

/// The edition strip: replaces the status bar's default product row.
pub struct EditionStrip {
    window: WindowHost,
}

impl Render for EditionStrip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.window.theme(cx);
        div()
            .id("example_edition_strip")
            .debug_selector(|| "example_edition_strip".to_string())
            .px_2()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(format!(
                "{} {} · Example edition",
                crate::DISPLAY_NAME,
                gitcomet_core::identity::current().version()
            ))
    }
}

/// The title-bar brand: the example's mark beside the title.
pub struct TitleBrand {
    window: WindowHost,
}

impl Render for TitleBrand {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.window.theme(cx);
        div()
            .id("example_title_brand")
            .debug_selector(|| "example_title_brand".to_string())
            .flex()
            .items_center()
            .gap_1()
            .text_size(theme.ui_text(12.0))
            .text_color(theme.colors.foreground.secondary)
            .child(
                svg()
                    .path(ICON_PATH)
                    .size(theme.ui_text(12.0))
                    .text_color(theme.colors.accent.solid),
            )
            .child("Example")
    }
}

/// Marks commits whose summary mentions a review.
fn annotate(context: &RepositoryViewContext, commit: &Commit, cx: &App) -> HistoryRowAnnotation {
    let accent = rgb_to_hsla(context.window.theme(cx).colors.accent.solid);
    match review_mark(commit, accent) {
        Some(mark) => HistoryRowAnnotation::default().with_trailing(mark),
        None => HistoryRowAnnotation::default(),
    }
}

fn review_mark(commit: &Commit, color: gitcomet_ui_kit::gpui::Hsla) -> Option<RowMark> {
    commit.summary.to_lowercase().contains("review").then(|| {
        RowMark::new(color)
            .with_glyph(RowGlyph::Icon(ICON_PATH.into()))
            .with_label("review")
    })
}

/// The sidebar rows: one action marking the repository reviewed.
fn sidebar_rows(context: &RepositoryViewContext, _cx: &App) -> Vec<SidebarSectionRows> {
    let window = context.window.clone();
    let workdir = context.repository.workdir().to_path_buf();
    vec![SidebarSectionRows::new(
        "example-review-actions",
        "Review actions",
        vec![
            SidebarRow::new(
                "mark-reviewed",
                "Mark reviewed",
                HostedAction::new("Mark reviewed", move |cx| {
                    mark_reviewed(&window, &workdir, cx)
                }),
            )
            .with_icon(ICON_PATH),
        ],
    )]
}

fn show_summary(context: CommandContext, _window: &mut Window, cx: &mut App) {
    let total = reviews(cx).read(cx).window_total(context.window.id());
    let theme = context.window.theme(cx);
    let _ = context.window.open_dialog(
        "Review summary",
        move |_, cx| -> AnyView {
            cx.new(|_| {
                let mut counter = ReviewCounter::new(theme);
                counter.set_reviews(total as usize);
                counter
            })
            .into()
        },
        cx,
    );
}

/// One main window's share of the model: its counts leave with the window.
struct ReviewWindow {
    reviews: Entity<Reviews>,
}

impl WindowExtension for ReviewWindow {
    fn on_event(&mut self, event: &ShellEvent, host: &WindowHost, cx: &mut App) {
        if matches!(event, ShellEvent::WindowClosed) {
            let window_id = host.id();
            self.reviews.update(cx, |reviews, cx| {
                reviews.counts.remove(&window_id);
                cx.notify();
            });
        }
    }
}

pub struct ReviewExtension;

impl Extension for ReviewExtension {
    fn id(&self) -> ExtensionId {
        extension_id()
    }

    fn register(&self, registrar: &mut Registrar) {
        registrar
            .repository_view(
                "review",
                RepositoryViewDescriptor::new("Review", ICON_PATH, |context, _window, cx| {
                    cx.new(|cx| ReviewView::new(context, cx)).into()
                }),
            )
            .repository_view(
                "changes",
                RepositoryViewDescriptor::new("Changes", ICON_PATH, |context, _window, cx| {
                    cx.new(|cx| crate::changes::ChangesView::new(context, cx))
                        .into()
                })
                .with_action_bar(|context, _window, cx| {
                    cx.new(|cx| crate::changes::ChangesActions::new(context, cx))
                        .into()
                }),
            )
            .bottom_panel(
                REVIEW_LOG_PANEL,
                BottomPanelDescriptor::new("Review Log", ICON_PATH, |context, _window, cx| {
                    cx.new(|cx| ReviewLog::new(context, cx)).into()
                }),
            )
            .details_tab(
                "review-details",
                DetailsTabDescriptor::new("Review", |context, _window, cx| {
                    cx.new(|cx| ReviewCount::new(context, "example_review_details", cx))
                        .into()
                })
                .with_icon(ICON_PATH),
            )
            .sidebar_section(
                "review-sidebar",
                SidebarSectionDescriptor::new("Review", |context, _window, cx| {
                    cx.new(|cx| ReviewCount::new(context, "example_review_sidebar", cx))
                        .into()
                }),
            )
            .status_item(
                "review-status",
                StatusItemDescriptor::new(|window, _, cx| {
                    cx.new(|cx| ReviewStatus::new(window, cx)).into()
                }),
            )
            .settings_page(
                "review-settings",
                SettingsPageDescriptor::new("Review", ICON_PATH, |context, _, cx| {
                    let reviews = reviews(cx);
                    cx.new(|_| ReviewSettings { context, reviews }).into()
                })
                .with_keywords("review unreviewed close confirm reset"),
            )
            .command(
                "mark-reviewed",
                CommandDescriptor::new(
                    "Mark Repository Reviewed",
                    "Review",
                    |context, _window, cx| {
                        if let Some(repository) = &context.repository {
                            mark_reviewed(&context.window, repository.workdir(), cx);
                        }
                    },
                )
                .with_keywords("review approve")
                .requiring_repository(),
            )
            .command(
                "show-summary",
                CommandDescriptor::new("Show Review Summary", "Review", show_summary)
                    .with_keywords("review count"),
            )
            .command(
                "toggle-review-log",
                CommandDescriptor::new("Toggle Review Log", "Review", toggle_review_log)
                    .with_keywords("review log panel")
                    .requiring_repository(),
            )
            .key_binding("secondary-alt-r", "mark-reviewed", None)
            .menu_item(MenuLocation::Application, "show-summary")
            .menu_item(MenuLocation::RepositoryTab, "mark-reviewed")
            .asset("icons/review.svg", ICON_SVG)
            .asset("icons/flag.svg", FLAG_SVG)
            .repository_entry_gate("deny-marker", Rc::new(gate))
            .close_guard("unreviewed", Rc::new(close_guard))
            .window_gate(
                "locked",
                WindowGateDescriptor::new(
                    GATE_SIGNAL.clone(),
                    |_, cx| is_gated(cx),
                    |window, _, cx| cx.new(|_| GateView { window }).into(),
                ),
            )
            .edition_strip(
                "edition",
                ChromeDescriptor::new(|window, _, cx| cx.new(|_| EditionStrip { window }).into()),
            )
            .title_bar_brand(
                "brand",
                ChromeDescriptor::new(|window, _, cx| cx.new(|_| TitleBrand { window }).into()),
            )
            .history_annotator(
                "review-mentions",
                HistoryAnnotator::new(SlotSignal::default(), annotate),
            )
            .sidebar_provider(
                "review-actions",
                SidebarProvider::new(SlotSignal::default(), sidebar_rows),
            );
    }

    /// Restores a main window's saved counts; they leave with the window.
    fn window_opened(
        &self,
        host: WindowHost,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<Box<dyn WindowExtension>> {
        if host.kind() != WindowKind::Main {
            return None;
        }
        let counts = saved_counts(&host, cx);
        let window_id = host.id();
        let model = reviews(cx);
        model.update(cx, |reviews, cx| {
            reviews.counts.insert(window_id, counts);
            cx.notify();
        });
        if let Some(value) = gitcomet_extension_api::storage::load(&extension_id())
            && let Some(enabled) = value
                .get("confirm_close_unreviewed")
                .and_then(|value| value.as_bool())
        {
            model.update(cx, |reviews, _| reviews.confirm_close_unreviewed = enabled);
        }
        Some(Box::new(ReviewWindow { reviews: model }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_extension_api::Registry;

    #[test]
    fn every_contribution_registers_and_validates() {
        let registry =
            Registry::build(vec![Box::new(ReviewExtension)]).expect("valid registration");
        assert_eq!(registry.repository_views().len(), 2);
        assert_eq!(registry.status_items().len(), 1);
        assert_eq!(registry.settings_pages().len(), 1);
        assert_eq!(registry.commands().len(), 3);
        assert_eq!(registry.bottom_panels().len(), 1);
        assert_eq!(registry.details_tabs().len(), 1);
        assert_eq!(registry.sidebar_sections().len(), 1);
        assert_eq!(registry.key_bindings().len(), 1);
        assert_eq!(registry.menu_items(MenuLocation::Application).count(), 1);
        assert_eq!(registry.menu_items(MenuLocation::RepositoryTab).count(), 1);
        assert_eq!(registry.asset(ICON_PATH), Some(ICON_SVG));
        assert_eq!(registry.entry_gates().len(), 1);
        assert_eq!(registry.close_guards().len(), 1);
        assert_eq!(registry.window_gates().len(), 1);
        assert!(registry.edition_strip().is_some());
        assert!(registry.title_bar_brand().is_some());
        assert_eq!(registry.history_annotators().len(), 1);
        assert_eq!(registry.sidebar_providers().len(), 1);
    }

    #[test]
    fn history_rows_mentioning_a_review_are_marked() {
        let commit = |summary: &str| Commit {
            id: gitcomet_core::domain::CommitId("c0ffee".into()),
            parent_ids: Default::default(),
            summary: summary.into(),
            author: "A".into(),
            time: std::time::SystemTime::UNIX_EPOCH,
        };
        let color = gitcomet_ui_kit::gpui::red();
        assert!(review_mark(&commit("Review the parser"), color).is_some());
        assert!(review_mark(&commit("Fix the parser"), color).is_none());
    }

    #[test]
    fn registering_twice_is_refused() {
        let errors = Registry::build(vec![Box::new(ReviewExtension), Box::new(ReviewExtension)])
            .err()
            .expect("a duplicate extension id is an error");
        assert!(errors[0].message.contains("registered twice"), "{errors:?}");
    }

    #[gpui::test]
    fn the_gate_denies_marked_repositories_except_on_restore(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(DENY_MARKER), "").unwrap();
        let request = |origin| RepositoryEntryRequest {
            path: dir.path().to_path_buf(),
            origin,
        };
        cx.update(|cx| {
            assert!(matches!(
                gate(&request(EntryOrigin::Chooser), cx),
                GateDecision::Deny { .. }
            ));
            assert_eq!(
                gate(&request(EntryOrigin::WorkspaceRestore), cx),
                GateDecision::Allow
            );
        });
    }
}
