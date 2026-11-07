//! The parent's side of a linked commit: while first-level submodules of the
//! open worktree hold uncommitted changes, the commit form shows one section
//! per changed submodule above the parent's own, and commits them all at
//! once, each submodule first and the parent recording their new commits.

use super::*;
use gitcomet_core::domain::{FileStatus, FileStatusKind, Submodule, SubmoduleChanges};
use gitcomet_state::model::{LinkedChild, submodule_display_name};
use rustc_hash::FxHashSet;
use std::path::{Path, PathBuf};

/// Files a section lists before it points to the submodule's own tab.
const LINKED_SECTION_MAX_FILES: usize = 100;
/// Tallest the submodule sections grow before they scroll.
const LINKED_SECTIONS_MAX_HEIGHT_PX: f32 = 320.0;

/// One changed first-level submodule in the parent's commit form.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::view) struct LinkedSection {
    pub(in crate::view) path: PathBuf,
    /// What the submodule holds, `None` until it is read.
    pub(in crate::view) changes: Option<SubmoduleChanges>,
    /// The commit the parent records for the submodule now, shortened.
    pub(in crate::view) recorded: String,
}

impl LinkedSection {
    pub(in crate::view) fn name(&self) -> String {
        submodule_display_name(&self.path)
    }

    pub(in crate::view) fn staged_count(&self) -> usize {
        self.changes
            .as_ref()
            .map_or(0, |changes| changes.staged.len())
    }

    /// A submodule takes part in the commit once something in it is staged.
    pub(in crate::view) fn included(&self) -> bool {
        self.staged_count() > 0
    }
}

/// The linked commit panel of a worktree tab.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::view) struct LinkedPanel {
    pub(in crate::view) parent: RepoId,
    pub(in crate::view) parent_name: String,
    pub(in crate::view) sections: Vec<LinkedSection>,
    /// Files staged in the parent itself, committed with the new pointers.
    pub(in crate::view) parent_staged: usize,
}

impl LinkedPanel {
    pub(in crate::view) fn included(&self) -> impl Iterator<Item = (usize, &LinkedSection)> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, section)| section.included())
    }

    #[cfg(test)]
    pub(in crate::view) fn included_count(&self) -> usize {
        self.included().count()
    }
}

/// A submodule the parent's commit form gives a section: a first-level one
/// with changes inside.
fn is_changed_first_level(row: &Submodule) -> bool {
    !row.nested && row.has_inner_changes
}

/// "a", "a and b", "a, b and c".
fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The linked commit panel for `repo`'s tab: a worktree that is not itself a
/// submodule and has changed first-level submodules.
pub(in crate::view) fn linked_panel(repo: &RepoState) -> Option<LinkedPanel> {
    if repo.is_bare() || repo.submodule_parent.is_some() {
        return None;
    }
    let Loadable::Ready(rows) = &repo.submodules else {
        return None;
    };
    let changes = match &repo.submodule_changes {
        Loadable::Ready(changes) => Some(changes.as_slice()),
        _ => None,
    };
    let sections = rows
        .iter()
        .filter(|row| is_changed_first_level(row))
        .map(|row| LinkedSection {
            path: row.path.clone(),
            changes: changes
                .and_then(|changes| changes.iter().find(|entry| entry.path == row.path))
                .cloned(),
            recorded: gitcomet_core::domain::short_commit_id(row.recorded_head.as_ref())
                .to_string(),
        })
        .collect::<Vec<_>>();
    if sections.is_empty() {
        return None;
    }
    // A committing submodule's gitlink shows as its own row. The parent's
    // other staged files, a gitlink of a submodule left out among them,
    // commit with it.
    let parent_staged = repo.staged_status_entries().map_or(0, |entries| {
        entries
            .iter()
            .filter(|entry| {
                !sections
                    .iter()
                    .any(|section| section.included() && section.path == entry.path)
            })
            .count()
    });
    Some(LinkedPanel {
        parent: repo.id,
        parent_name: repo
            .spec
            .workdir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "the repository".to_string()),
        sections,
        parent_staged,
    })
}

/// What a section header says about a branch and its upstream, and whether
/// it warns: a detached HEAD or a branch without an upstream cannot be pushed.
pub(in crate::view) fn tracking_label(
    branch: Option<&str>,
    upstream: Option<&gitcomet_core::domain::Upstream>,
) -> (String, bool) {
    match (branch, upstream) {
        (Some(branch), Some(upstream)) => (
            format!("{branch} → {}/{}", upstream.remote, upstream.branch),
            false,
        ),
        (Some(branch), None) => (format!("{branch} · no upstream"), true),
        (None, _) => ("detached HEAD".to_string(), true),
    }
}

fn parent_tracking(repo: &RepoState) -> Option<(String, bool)> {
    let Loadable::Ready(head) = &repo.head_branch else {
        return None;
    };
    if head == "HEAD" {
        return Some(tracking_label(None, None));
    }
    let upstream = match &repo.branches {
        Loadable::Ready(branches) => branches
            .iter()
            .find(|branch| branch.name == *head)
            .and_then(|branch| branch.upstream.as_ref()),
        _ => None,
    };
    Some(tracking_label(Some(head), upstream))
}

/// A file row's check state in a section: staged, partly staged, or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum LinkedFileCheck {
    Staged,
    Partly,
    Unstaged,
}

/// The file rows a section shows, and how many more it leaves out.
#[derive(Debug)]
pub(in crate::view) struct LinkedFileRows<'a> {
    pub(in crate::view) rows: Vec<(&'a FileStatus, LinkedFileCheck)>,
    pub(in crate::view) hidden: usize,
}

/// A section's file rows: staged files first, then the rest, each path once,
/// at most `max` of them. The form draws them on every keystroke and a
/// submodule can hold thousands of changes, so paths are looked up in sets
/// and rows past `max` are only counted.
pub(in crate::view) fn linked_file_rows(
    changes: &SubmoduleChanges,
    max: usize,
) -> LinkedFileRows<'_> {
    let staged_paths: FxHashSet<&Path> = changes
        .staged
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    let unstaged_paths: FxHashSet<&Path> = changes
        .unstaged
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    let staged = changes.staged.iter().map(|file| {
        let check = if unstaged_paths.contains(file.path.as_path()) {
            LinkedFileCheck::Partly
        } else {
            LinkedFileCheck::Staged
        };
        (file, check)
    });
    let unstaged = changes
        .unstaged
        .iter()
        .filter(|file| !staged_paths.contains(file.path.as_path()))
        .map(|file| (file, LinkedFileCheck::Unstaged));
    let mut all = staged.chain(unstaged);
    let rows = all.by_ref().take(max).collect();
    LinkedFileRows {
        rows,
        hidden: all.count(),
    }
}

/// A check box of the commit form: on, partly on (a dash), or off, and
/// greyed out while it cannot be changed.
pub(super) fn form_check_box(
    theme: AppTheme,
    on: bool,
    partly: bool,
    disabled: bool,
    box_px: Pixels,
    icon_px: Pixels,
    scaled_px: &impl Fn(f32) -> Pixels,
) -> gpui::Div {
    let mark = if disabled {
        theme.colors.foreground.disabled
    } else {
        theme.colors.accent.foreground
    };
    div()
        .flex_none()
        .size(box_px)
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(if on || disabled {
            mark
        } else {
            theme.colors.stroke.default
        })
        .rounded(scaled_px(theme.radii.control * 0.5))
        .when(on, |check_box| {
            check_box
                .bg(with_alpha(mark, if theme.is_dark { 0.18 } else { 0.12 }))
                .child(svg_icon(
                    if partly {
                        "icons/minus.svg"
                    } else {
                        "icons/check.svg"
                    },
                    mark,
                    icon_px,
                ))
        })
}

/// The commit button's label and tooltip for a linked commit of `panel`.
pub(in crate::view) fn linked_commit_label(panel: &LinkedPanel, push: bool) -> (String, String) {
    let names = panel
        .included()
        .map(|(_, section)| section.name())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return if push {
            (
                "Commit changes and Push".to_string(),
                "Commit staged changes and push".to_string(),
            )
        } else {
            ("Commit".to_string(), "Commit staged changes".to_string())
        };
    }
    let repos = names.len() + 1;
    let label = if push {
        format!("Commit {repos} repos and Push")
    } else {
        format!("Commit {repos} repos")
    };
    let names = name_list(&names);
    let tooltip = if push {
        format!(
            "Commit staged changes in {names}, record them in {parent}, then push {names} first and {parent} after",
            parent = panel.parent_name
        )
    } else {
        format!(
            "Commit staged changes in {names}, then commit {parent} with their new commits",
            parent = panel.parent_name
        )
    };
    (label, tooltip)
}

/// The note under the commit button while push after commit is on.
pub(in crate::view) fn linked_push_note(panel: &LinkedPanel) -> Option<String> {
    let names = panel
        .included()
        .map(|(_, section)| section.name())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return None;
    }
    let verb = if names.len() == 1 {
        "succeeds"
    } else {
        "succeed"
    };
    Some(format!(
        "Pushes {} first. {} is pushed only if {} {verb}.",
        name_list(&names),
        panel.parent_name,
        if names.len() == 1 {
            "that push"
        } else {
            "those pushes"
        },
    ))
}

impl DetailsPaneView {
    /// The active tab's changed submodules, unless the commit amends. The
    /// "Linked commit | <parent> only" toggle shows while there are any.
    pub(in crate::view) fn active_changed_submodules_panel(&self) -> Option<LinkedPanel> {
        if self.commit_amend_enabled {
            return None;
        }
        linked_panel(self.active_repo()?)
    }

    /// Whether the user chose a linked commit of `panel`, rather than one of
    /// the parent only.
    pub(in crate::view) fn links_commit(&self, panel: &LinkedPanel) -> bool {
        !self.linked_commit_off.contains(&panel.parent)
    }

    /// The active tab's linked commit panel, unless the user commits the
    /// parent only or the commit amends.
    pub(in crate::view) fn active_linked_panel(&self) -> Option<LinkedPanel> {
        self.active_changed_submodules_panel()
            .filter(|panel| self.links_commit(panel))
    }

    /// The submodule commits a linked commit of `panel` makes, or `None` when
    /// a submodule with staged files has no message yet.
    pub(in crate::view) fn linked_children(
        &self,
        panel: &LinkedPanel,
        cx: &gpui::App,
    ) -> Option<Vec<LinkedChild>> {
        panel
            .included()
            .map(|(_, section)| {
                let message = self
                    .linked_message_inputs
                    .get(&(panel.parent, section.path.clone()))
                    .map(|input| input.read(cx).text().trim().to_string())
                    .unwrap_or_default();
                (!message.is_empty()).then(|| LinkedChild {
                    path: section.path.clone(),
                    message,
                })
            })
            .collect()
    }

    pub(in crate::view) fn can_submit_linked_commit(
        &self,
        message: &str,
        children: Option<&[LinkedChild]>,
    ) -> bool {
        let Some(children) = children else {
            return false;
        };
        if children.is_empty() {
            return self.can_submit_commit(message);
        }
        let Some(repo) = self.active_repo() else {
            return false;
        };
        !message.trim().is_empty()
            && !repo.is_bare()
            && !self.pending_commits.contains_key(&repo.id)
            && repo.commit_in_flight == 0
            && self.state.linked_commit.is_none()
    }

    fn linked_message_input(
        &mut self,
        parent: RepoId,
        path: &Path,
        name: &str,
        cx: &mut gpui::Context<Self>,
    ) -> Entity<components::TextInput> {
        self.linked_message_inputs
            .entry((parent, path.to_path_buf()))
            .or_insert_with(|| {
                cx.new(|cx| {
                    components::TextInput::new_inert(
                        components::TextInputOptions {
                            placeholder: format!("Commit message for {name}").into(),
                            ..Default::default()
                        },
                        cx,
                    )
                })
            })
            .clone()
    }

    /// Forget the message boxes of closed tabs and of submodules that no
    /// longer hold changes, so a later change starts from an empty message
    /// rather than an old draft that would pass for the one it needs. Rows
    /// still loading keep their boxes. Runs for every state, whichever tab
    /// shows, since a submodule can go clean and change again meanwhile.
    pub(in crate::view) fn prune_linked_message_inputs(&mut self, state: &AppState) {
        if self.linked_message_inputs.is_empty() {
            return;
        }
        self.linked_message_inputs.retain(|(owner, path), _| {
            state
                .repos
                .iter()
                .find(|repo| repo.id == *owner)
                .is_some_and(|repo| match &repo.submodules {
                    Loadable::Ready(rows) => rows
                        .iter()
                        .any(|row| is_changed_first_level(row) && row.path == *path),
                    _ => true,
                })
        });
    }

    /// Clear the messages a finished linked commit used.
    pub(in crate::view) fn clear_linked_messages(
        &mut self,
        parent: RepoId,
        used: Vec<LinkedChild>,
        cx: &mut gpui::Context<Self>,
    ) {
        for child in used {
            if let Some(input) = self.linked_message_inputs.get(&(parent, child.path)) {
                let unchanged = input.read(cx).text().trim() == child.message;
                if unchanged {
                    input.update(cx, |input, cx| input.set_text(String::new(), cx));
                }
            }
        }
    }

    /// "Linked commit | <parent> only", shown while submodules hold changes.
    pub(in crate::view) fn linked_commit_toggle(
        &mut self,
        panel: &LinkedPanel,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let linked = self.links_commit(panel);
        let parent = panel.parent;
        let count = panel.sections.len();
        let summary = format!(
            "{} + {count} {}",
            panel.parent_name,
            if count == 1 {
                "submodule"
            } else {
                "submodules"
            }
        );
        div()
            .debug_selector(|| "linked_commit_toggle".to_string())
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(theme.ui_text(12.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.colors.foreground.primary)
                            .child("Commit"),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_color(theme.colors.foreground.secondary)
                            .child(summary),
                    ),
            )
            .child(
                div().flex().w_full().child(
                    components::SegmentedControl::new("linked_commit_mode")
                        .adaptive()
                        .segment(
                            components::Segment::new("linked_commit_mode_linked", "Linked commit")
                                .selected(linked)
                                .tooltip(
                                    "Commit the changed submodules and record them here",
                                    Vec::new(),
                                ),
                        )
                        .segment(
                            components::Segment::new(
                                "linked_commit_mode_parent_only",
                                format!("{} only", panel.parent_name),
                            )
                            .selected(!linked)
                            .tooltip(
                                "Commit only what is staged here; the submodules keep their changes",
                                Vec::new(),
                            ),
                        )
                        .render(theme, self.ui_scale(), cx, move |this, index, _w, cx| {
                            if index == 0 {
                                this.linked_commit_off.remove(&parent);
                            } else {
                                this.linked_commit_off.insert(parent);
                            }
                            cx.notify();
                        }),
                ),
            )
    }

    /// The submodule sections and the parent's section of a linked commit,
    /// the parent's message box inside its section.
    pub(in crate::view) fn linked_commit_panel(
        &mut self,
        panel: &LinkedPanel,
        commit_message: AnyElement,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let busy = self.state.linked_commit.is_some()
            || self
                .active_repo()
                .is_some_and(|repo| repo.commit_in_flight > 0);

        let mut sections = div().flex().flex_col().gap_2();
        for (index, section) in panel.sections.iter().enumerate() {
            let card = self.linked_section_card(panel, index, section, busy, cx);
            sections = sections.child(card);
            sections = sections.child(
                div()
                    .debug_selector(move || format!("linked_commit_record_note_{index}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(svg_icon(
                        "icons/arrow_down.svg",
                        theme.colors.foreground.secondary,
                        scaled_px(12.0),
                    ))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .child(if section.included() {
                                format!(
                                    "New {} commit is recorded in {} automatically",
                                    section.name(),
                                    panel.parent_name
                                )
                            } else {
                                format!(
                                    "Check files to commit {} with {}",
                                    section.name(),
                                    panel.parent_name
                                )
                            }),
                    ),
            );
        }
        let sections = components::ScrollContainer::vertical(
            ("linked_commit_sections_surface", panel.parent.0),
            ("linked_commit_sections_scrollbar", panel.parent.0),
            self.linked_commit_scroll.clone(),
            scaled_px(LINKED_SECTIONS_MAX_HEIGHT_PX),
        )
        .container_id(("linked_commit_sections", panel.parent.0))
        .debug_selector("linked_commit_sections")
        .render(theme, sections);

        let parent_card = self.linked_parent_card(panel, commit_message, cx);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(sections)
            .child(parent_card)
    }

    fn linked_card(theme: AppTheme) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_2()
            .py_2()
            .rounded(px(theme.radii.control))
            .border_1()
            .border_color(theme.colors.stroke.default)
            .bg(theme.colors.surface.raised)
    }

    fn linked_index_badge(
        theme: AppTheme,
        step: usize,
        scaled_px: &impl Fn(f32) -> Pixels,
    ) -> gpui::Div {
        div()
            .flex_none()
            .size(scaled_px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme.colors.surface.panel)
            .border_1()
            .border_color(theme.colors.stroke.default)
            .text_size(theme.ui_text(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.foreground.secondary)
            .child(step.to_string())
    }

    fn linked_card_header(
        theme: AppTheme,
        step: usize,
        icon: gpui::Svg,
        title: String,
        tracking: Option<(String, bool)>,
        scaled_px: &impl Fn(f32) -> Pixels,
    ) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .min_w(px(0.0))
            .child(Self::linked_index_badge(theme, step, scaled_px))
            .child(icon)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_size(theme.ui_text(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.foreground.primary)
                    .child(title),
            )
            .when_some(tracking, |header, (label, warning)| {
                header.child(
                    div()
                        .flex_none()
                        .max_w(scaled_px(200.0))
                        .truncate()
                        .text_size(theme.ui_text(11.0))
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_color(if warning {
                            theme.colors.status.warning.foreground
                        } else {
                            theme.colors.foreground.secondary
                        })
                        .child(label),
                )
            })
    }

    fn linked_path_label(theme: AppTheme, path: &Path) -> gpui::Div {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(|dir| format!("{}/", dir.display()));
        div()
            .flex()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .text_size(theme.ui_text(12.0))
            .whitespace_nowrap()
            .when_some(dir, |label, dir| {
                label.child(
                    div()
                        .min_w(px(0.0))
                        .truncate()
                        .text_color(theme.colors.foreground.secondary)
                        .child(dir),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_color(theme.colors.foreground.primary)
                    .child(name),
            )
    }

    fn linked_section_card(
        &mut self,
        panel: &LinkedPanel,
        index: usize,
        section: &LinkedSection,
        busy: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let name = section.name();
        let tracking = section
            .changes
            .as_ref()
            .map(|changes| tracking_label(changes.branch(), changes.upstream()));
        let header = Self::linked_card_header(
            theme,
            index + 1,
            svg_icon(
                "icons/corner_down_right.svg",
                theme.colors.foreground.secondary,
                scaled_px(12.0),
            ),
            section.path.display().to_string(),
            tracking,
            &scaled_px,
        );
        let mut card = Self::linked_card(theme)
            .debug_selector(move || format!("linked_commit_section_{index}"))
            .child(header);

        match &section.changes {
            None => {
                let text = match &self.active_repo().map(|repo| &repo.submodule_changes) {
                    Some(Loadable::Error(error)) => format!("Could not read {name}: {error}"),
                    _ => format!("Reading {name}…"),
                };
                card = card.child(
                    div()
                        .px_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(text),
                );
            }
            Some(changes) => {
                let LinkedFileRows { rows, hidden } =
                    linked_file_rows(changes, LINKED_SECTION_MAX_FILES);
                let parent = panel.parent;
                for (row_ix, (file, check)) in rows.into_iter().enumerate() {
                    let visuals = crate::view::rows::commit_file_kind_visuals(file.kind);
                    let kind_color = visuals.color(&theme);
                    // Staging a conflicted file tells git it is resolved, and
                    // its markers would then be committed and recorded here.
                    // Every other stage path asks first; this one sends the
                    // user to the submodule's own tab instead.
                    let conflicted =
                        file.kind == FileStatusKind::Conflicted || file.conflict.is_some();
                    let disabled = busy || conflicted;
                    let tooltip: SharedString = match check {
                        _ if conflicted => {
                            format!("Conflicted: resolve it in {name}'s tab first").into()
                        }
                        LinkedFileCheck::Staged => "Staged: click to leave it out".into(),
                        LinkedFileCheck::Partly => "Partly staged: click to stage the rest".into(),
                        LinkedFileCheck::Unstaged => "Click to stage it".into(),
                    };
                    let mut row = div()
                        .id(SharedString::from(format!(
                            "linked_commit_file_{index}_{row_ix}"
                        )))
                        .debug_selector(move || format!("linked_commit_file_{index}_{row_ix}"))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_1()
                        .h(scaled_px(22.0))
                        .rounded(px(theme.radii.row))
                        .when(conflicted, |row| {
                            row.bg(with_alpha(theme.colors.status.danger.foreground, 0.12))
                        })
                        .control_interaction(
                            InteractionStyle::new(theme),
                            InteractionState::default().disabled(disabled),
                        )
                        .child(form_check_box(
                            theme,
                            check != LinkedFileCheck::Unstaged,
                            check == LinkedFileCheck::Partly,
                            conflicted,
                            scaled_px(14.0),
                            scaled_px(9.0),
                            &scaled_px,
                        ))
                        .child(svg_icon(visuals.icon, kind_color, scaled_px(12.0)))
                        .child(Self::linked_path_label(theme, &file.path));
                    if !disabled {
                        let submodule = section.path.clone();
                        let file_path = file.path.clone();
                        row = row.on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                let files = vec![file_path.clone()];
                                let msg = if check == LinkedFileCheck::Staged {
                                    Msg::UnstageInSubmodule {
                                        repo_id: parent,
                                        path: submodule.clone(),
                                        files,
                                    }
                                } else {
                                    Msg::StageInSubmodule {
                                        repo_id: parent,
                                        path: submodule.clone(),
                                        files,
                                    }
                                };
                                this.store.dispatch(msg);
                                cx.notify();
                            }),
                        );
                    }
                    card = card.child(row.gitcomet_tooltip(theme, tooltip));
                }
                if hidden > 0 {
                    card = card.child(
                        div()
                            .px_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(format!(
                                "{hidden} more {}. Open {name} to see them all.",
                                if hidden == 1 { "file" } else { "files" }
                            )),
                    );
                }
            }
        }

        let input = self.linked_message_input(panel.parent, &section.path, &name, cx);
        card.child(
            div()
                .pt_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child("Message"),
                )
                .child(
                    div()
                        .debug_selector(move || format!("linked_commit_message_{index}"))
                        .child(input),
                ),
        )
    }

    fn linked_parent_card(
        &mut self,
        panel: &LinkedPanel,
        commit_message: AnyElement,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
        let step = panel.sections.len() + 1;
        let tracking = self.active_repo().and_then(parent_tracking);
        let header = Self::linked_card_header(
            theme,
            step,
            svg_icon(
                "icons/git_commit.svg",
                theme.colors.accent.foreground,
                scaled_px(12.0),
            ),
            panel.parent_name.clone(),
            tracking,
            &scaled_px,
        );
        let mut card = Self::linked_card(theme)
            .debug_selector(|| "linked_commit_parent_section".to_string())
            .child(header);
        for (index, section) in panel.included() {
            let pointer = format!("{} → step {}", section.recorded, index + 1);
            card = card.child(
                div()
                    .id(SharedString::from(format!("linked_commit_gitlink_{index}")))
                    .debug_selector(move || format!("linked_commit_gitlink_{index}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_1()
                    .h(scaled_px(22.0))
                    .rounded(px(theme.radii.row))
                    .bg(theme.colors.surface.panel)
                    .child(svg_icon(
                        "icons/link.svg",
                        theme.colors.accent.foreground,
                        scaled_px(12.0),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.primary)
                            .child(section.path.display().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme.ui_text(11.0))
                            .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                            .text_color(theme.colors.foreground.secondary)
                            .child(pointer),
                    )
                    .gitcomet_tooltip(
                        theme,
                        format!(
                            "{} records the new {} commit; this row cannot be unchecked",
                            panel.parent_name,
                            section.name()
                        )
                        .into(),
                    ),
            );
        }
        let staged_note = match panel.parent_staged {
            0 => None,
            1 => Some(format!(
                "Also commits 1 file staged in {}.",
                panel.parent_name
            )),
            count => Some(format!(
                "Also commits {count} files staged in {}.",
                panel.parent_name
            )),
        };
        card = card.when_some(staged_note, |card, note| {
            card.child(
                div()
                    .debug_selector(|| "linked_commit_parent_staged".to_string())
                    .px_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(note),
            )
        });
        card.child(
            div()
                .pt_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child("Message"),
                )
                .child(commit_message),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{
        CommitId, HeadTracking, RepoSpec, Submodule, SubmoduleStatus, Upstream,
    };
    use std::sync::Arc;

    fn file(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind,
            conflict: None,
        }
    }

    fn changes(path: &str, staged: Vec<FileStatus>, unstaged: Vec<FileStatus>) -> SubmoduleChanges {
        SubmoduleChanges {
            path: PathBuf::from(path),
            head: HeadTracking::Tracking {
                branch: "main".to_string(),
                upstream: Upstream {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                },
                divergence: None,
            },
            staged: Arc::new(staged),
            unstaged: Arc::new(unstaged),
        }
    }

    fn row(path: &str, changed: bool) -> Submodule {
        let mut row = Submodule::new(
            PathBuf::from(path),
            CommitId("a1b2c3d4e5f6".into()),
            Some(CommitId("a1b2c3d4e5f6".into())),
            SubmoduleStatus::UpToDate,
        );
        row.has_inner_changes = changed;
        row
    }

    fn parent_repo() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/work/GitComet"),
            },
        )
    }

    #[test]
    fn a_worktree_without_changed_submodules_has_no_panel() {
        let mut repo = parent_repo();
        assert_eq!(linked_panel(&repo), None);
        repo.submodules = Loadable::Ready(Arc::new(vec![row("fixtures/clean", false)]));
        assert_eq!(linked_panel(&repo), None);
    }

    #[test]
    fn the_panel_lists_changed_first_level_submodules_with_what_they_hold() {
        let mut repo = parent_repo();
        let mut nested = row("fixtures/syntax_test/deeper", true);
        nested.nested = true;
        repo.submodules = Loadable::Ready(Arc::new(vec![
            row("fixtures/syntax_test", true),
            nested,
            row("vendor/lib", true),
            row("fixtures/clean", false),
        ]));
        repo.submodule_changes = Loadable::Ready(Arc::new(vec![changes(
            "fixtures/syntax_test",
            vec![file("manifest.toml", FileStatusKind::Modified)],
            Vec::new(),
        )]));
        let panel = linked_panel(&repo).expect("panel");
        assert_eq!(panel.parent_name, "GitComet");
        let paths = panel
            .sections
            .iter()
            .map(|section| section.path.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("fixtures/syntax_test"),
                PathBuf::from("vendor/lib")
            ]
        );
        assert!(panel.sections[0].included());
        assert_eq!(panel.sections[0].recorded, "a1b2c3d");
        // Not read yet, so nothing in it is staged.
        assert_eq!(panel.sections[1].changes, None);
        assert!(!panel.sections[1].included());
        assert_eq!(panel.included_count(), 1);
    }

    #[test]
    fn a_submodule_tab_has_no_panel() {
        let mut repo = parent_repo();
        repo.submodules = Loadable::Ready(Arc::new(vec![row("inner", true)]));
        repo.submodule_parent = Some(gitcomet_state::model::SubmoduleParentLink {
            workdir: PathBuf::from("/work"),
            path: PathBuf::from("GitComet"),
            repo_id: None,
        });
        assert_eq!(linked_panel(&repo), None);
    }

    #[test]
    fn file_rows_list_staged_files_first_and_mark_partly_staged_ones() {
        let changes = changes(
            "sub",
            vec![
                file("a.txt", FileStatusKind::Added),
                file("b.txt", FileStatusKind::Modified),
            ],
            vec![
                file("b.txt", FileStatusKind::Modified),
                file("c.txt", FileStatusKind::Untracked),
            ],
        );
        let rows = linked_file_rows(&changes, LINKED_SECTION_MAX_FILES);
        assert_eq!(rows.hidden, 0);
        let rows = rows
            .rows
            .into_iter()
            .map(|(file, check)| (file.path.display().to_string(), check))
            .collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                ("a.txt".to_string(), LinkedFileCheck::Staged),
                ("b.txt".to_string(), LinkedFileCheck::Partly),
                ("c.txt".to_string(), LinkedFileCheck::Unstaged),
            ]
        );
    }

    #[test]
    fn file_rows_stop_at_the_cap_and_count_the_rest() {
        // 150 staged and 150 unstaged files, half of each partly staged:
        // 225 distinct paths.
        let staged = (0..150)
            .map(|ix| file(&format!("f{ix:03}"), FileStatusKind::Modified))
            .collect();
        let unstaged = (75..225)
            .map(|ix| file(&format!("f{ix:03}"), FileStatusKind::Modified))
            .collect();
        let changes = changes("sub", staged, unstaged);
        let rows = linked_file_rows(&changes, LINKED_SECTION_MAX_FILES);
        assert_eq!(rows.rows.len(), LINKED_SECTION_MAX_FILES);
        assert_eq!(rows.hidden, 225 - LINKED_SECTION_MAX_FILES);
        for (ix, (file, check)) in rows.rows.iter().enumerate() {
            assert_eq!(file.path, PathBuf::from(format!("f{ix:03}")));
            let expected = if ix < 75 {
                LinkedFileCheck::Staged
            } else {
                LinkedFileCheck::Partly
            };
            assert_eq!(*check, expected, "row {ix}");
        }

        // Past the staged files, the rest are the unstaged-only ones.
        let rows = linked_file_rows(&changes, 200);
        assert_eq!(rows.hidden, 25);
        assert_eq!(
            rows.rows[150],
            (&changes.unstaged[75], LinkedFileCheck::Unstaged)
        );
        assert_eq!(rows.rows[150].0.path, PathBuf::from("f150"));
    }

    #[test]
    fn the_parent_counts_a_staged_gitlink_of_a_submodule_left_out() {
        let mut repo = parent_repo();
        repo.submodules = Loadable::Ready(Arc::new(vec![
            row("fixtures/syntax_test", true),
            row("vendor/lib", true),
        ]));
        repo.submodule_changes = Loadable::Ready(Arc::new(vec![
            changes(
                "fixtures/syntax_test",
                vec![file("manifest.toml", FileStatusKind::Modified)],
                Vec::new(),
            ),
            changes(
                "vendor/lib",
                Vec::new(),
                vec![file("lib.rs", FileStatusKind::Modified)],
            ),
        ]));
        repo.status = Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: Arc::new(vec![
                    file("fixtures/syntax_test", FileStatusKind::Modified),
                    file("vendor/lib", FileStatusKind::Modified),
                    file("README.md", FileStatusKind::Modified),
                ]),
                unstaged: Arc::new(Vec::new()),
            }
            .into(),
        );
        let panel = linked_panel(&repo).expect("panel");
        // The committing submodule's gitlink has its own row; the left-out
        // one's staged gitlink commits with README.md.
        assert_eq!(panel.parent_staged, 2);
    }

    #[test]
    fn tracking_labels_warn_about_what_cannot_be_pushed() {
        let upstream = Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        };
        assert_eq!(
            tracking_label(Some("main"), Some(&upstream)),
            ("main → origin/main".to_string(), false)
        );
        assert_eq!(
            tracking_label(Some("topic"), None),
            ("topic · no upstream".to_string(), true)
        );
        assert_eq!(
            tracking_label(None, None),
            ("detached HEAD".to_string(), true)
        );
    }

    fn panel_with(included: &[&str], excluded: &[&str]) -> LinkedPanel {
        let section = |path: &str, staged: bool| LinkedSection {
            path: PathBuf::from(path),
            changes: Some(changes(
                path,
                if staged {
                    vec![file("x", FileStatusKind::Modified)]
                } else {
                    Vec::new()
                },
                vec![file("y", FileStatusKind::Modified)],
            )),
            recorded: "a1b2c3d".to_string(),
        };
        LinkedPanel {
            parent: RepoId(1),
            parent_name: "GitComet".to_string(),
            sections: included
                .iter()
                .map(|path| section(path, true))
                .chain(excluded.iter().map(|path| section(path, false)))
                .collect(),
            parent_staged: 0,
        }
    }

    #[test]
    fn the_button_counts_the_repos_it_commits() {
        let panel = panel_with(&["fixtures/syntax_test"], &["vendor/lib"]);
        assert_eq!(linked_commit_label(&panel, false).0, "Commit 2 repos");
        assert_eq!(
            linked_commit_label(&panel, true).0,
            "Commit 2 repos and Push"
        );
        let panel = panel_with(&["a", "b"], &[]);
        let (label, tooltip) = linked_commit_label(&panel, true);
        assert_eq!(label, "Commit 3 repos and Push");
        assert_eq!(
            tooltip,
            "Commit staged changes in a and b, record them in GitComet, then push a and b first and GitComet after"
        );
        let panel = panel_with(&[], &["a"]);
        assert_eq!(linked_commit_label(&panel, false).0, "Commit");
    }

    #[test]
    fn the_push_note_names_the_submodules_pushed_first() {
        assert_eq!(
            linked_push_note(&panel_with(&["fixtures/syntax_test"], &[])).as_deref(),
            Some("Pushes syntax_test first. GitComet is pushed only if that push succeeds.")
        );
        assert_eq!(
            linked_push_note(&panel_with(&["a", "b", "c"], &[])).as_deref(),
            Some("Pushes a, b and c first. GitComet is pushed only if those pushes succeed.")
        );
        assert_eq!(linked_push_note(&panel_with(&[], &["a"])), None);
    }
}
