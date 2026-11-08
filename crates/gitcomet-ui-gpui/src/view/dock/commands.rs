//! Layout commands: what a palette row, a menu entry or a shortcut asks the
//! dock layout to do, and the palette rows that list them.
//!
//! Palette ids start with `layout:` so they never collide with the built-in
//! [`Command`](crate::view::commands::Command) ids or `extension:` ones.

use super::*;
use crate::kit::components::Shortcut;
use crate::view::command_palette::{CommandEntry, Needs};

const PREFIX: &str = "layout:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DockCommand {
    TogglePanel(PanelId),
    ShowPanel(PanelId),
    HidePanel(PanelId),
    MovePanel {
        panel: PanelId,
        side: DockSide,
        half: DockHalf,
    },
    /// Puts a built-in panel back where the factory layout has it, with its
    /// sections in today's order. For Branches this rebuilds today's
    /// left-hand menu.
    RestorePanel(PanelId),
    RestoreDefault,
    HideAll,
    Maximize,
    ShowSection(SectionId),
    HideSection(SectionId),
    MoveSection {
        section: SectionId,
        panel: PanelId,
    },
    MoveSectionBy {
        section: SectionId,
        delta: i32,
    },
    NewPanel(DockSide),
    NewPanelWithSection(SectionId),
    RemovePanel(PanelId),
    ApplyPreset(LayoutPreset),
    SaveLayout,
    /// Saved layout n ("Layout n"), counting from 1.
    ApplySaved(usize),
    /// Forgets saved layout n, counting from 1.
    DeleteSaved(usize),
    SetCentre(CentreSplitMode),
    PopOut(PanelId),
    ResetDockSize(DockSide),
    /// Grows (`grow`) or shrinks a dock by one step, for the keyboard.
    ResizeDock {
        side: DockSide,
        grow: bool,
    },
}

fn side_key(side: DockSide) -> &'static str {
    match side {
        DockSide::Left => "left",
        DockSide::Right => "right",
        DockSide::Bottom => "bottom",
    }
}

fn side_from_key(key: &str) -> Option<DockSide> {
    DockSide::ALL
        .into_iter()
        .find(|side| side_key(*side) == key)
}

fn half_key(half: DockHalf) -> &'static str {
    match half {
        DockHalf::Primary => "primary",
        DockHalf::Split => "split",
    }
}

fn half_from_key(key: &str) -> Option<DockHalf> {
    DockHalf::ALL
        .into_iter()
        .find(|half| half_key(*half) == key)
}

fn preset_key(preset: LayoutPreset) -> &'static str {
    match preset {
        LayoutPreset::BrowseHistory => "browse-history",
        LayoutPreset::Commit => "commit",
        LayoutPreset::Review => "review",
        LayoutPreset::ResolveConflicts => "resolve-conflicts",
    }
}

fn centre_key(mode: CentreSplitMode) -> &'static str {
    match mode {
        CentreSplitMode::Single => "single",
        CentreSplitMode::Auto => "auto",
        CentreSplitMode::SideBySide => "side-by-side",
        CentreSplitMode::Stacked => "stacked",
    }
}

const CENTRE_MODES: [CentreSplitMode; 4] = [
    CentreSplitMode::Single,
    CentreSplitMode::Auto,
    CentreSplitMode::SideBySide,
    CentreSplitMode::Stacked,
];

/// Panels that can open in a window of their own.
pub(in crate::view) fn can_pop_out(panel: &PanelId) -> bool {
    matches!(
        PanelKind::of(panel),
        PanelKind::Branches
            | PanelKind::Files
            | PanelKind::FileHistory
            | PanelKind::Changes
            | PanelKind::Reflog
    )
}

impl DockCommand {
    pub(in crate::view) fn palette_id(&self) -> Option<String> {
        Some(match self {
            Self::TogglePanel(panel) => format!("{PREFIX}toggle-panel:{panel}"),
            Self::MovePanel { panel, side, half } => format!(
                "{PREFIX}move-panel:{panel}:{}:{}",
                side_key(*side),
                half_key(*half)
            ),
            Self::RestorePanel(panel) => format!("{PREFIX}restore-panel:{panel}"),
            Self::RestoreDefault => format!("{PREFIX}restore-default"),
            Self::HideAll => format!("{PREFIX}hide-all"),
            Self::Maximize => format!("{PREFIX}maximize"),
            Self::ShowSection(section) => format!("{PREFIX}show-section:{section}"),
            Self::HideSection(section) => format!("{PREFIX}hide-section:{section}"),
            Self::MoveSection { section, panel } => {
                format!("{PREFIX}move-section:{section}:{panel}")
            }
            Self::NewPanel(side) => format!("{PREFIX}new-panel:{}", side_key(*side)),
            Self::ApplyPreset(preset) => format!("{PREFIX}preset:{}", preset_key(*preset)),
            Self::SaveLayout => format!("{PREFIX}save"),
            Self::ApplySaved(index) => format!("{PREFIX}apply-saved:{index}"),
            Self::DeleteSaved(index) => format!("{PREFIX}delete-saved:{index}"),
            Self::SetCentre(mode) => format!("{PREFIX}centre:{}", centre_key(*mode)),
            Self::PopOut(panel) => format!("{PREFIX}pop-out:{panel}"),
            Self::ResetDockSize(side) => format!("{PREFIX}reset-size:{}", side_key(*side)),
            Self::ResizeDock { side, grow } => format!(
                "{PREFIX}{}:{}",
                if *grow { "grow" } else { "shrink" },
                side_key(*side)
            ),
            Self::ShowPanel(_)
            | Self::HidePanel(_)
            | Self::MoveSectionBy { .. }
            | Self::NewPanelWithSection(_)
            | Self::RemovePanel(_) => return None,
        })
    }

    /// The command a `layout:` palette id stands for.
    pub(in crate::view) fn from_palette_id(id: &str) -> Option<Self> {
        let rest = id.strip_prefix(PREFIX)?;
        let mut parts = rest.split(':');
        let verb = parts.next()?;
        let args: Vec<&str> = parts.collect();
        // Custom panel ids contain a colon (`custom:1`); rejoin them.
        let joined = |from: usize| args[from..].join(":");
        Some(match (verb, args.as_slice()) {
            ("toggle-panel", [_, ..]) => Self::TogglePanel(PanelId::new(joined(0))),
            ("move-panel", [panel, side, half]) => Self::MovePanel {
                panel: PanelId::new(*panel),
                side: side_from_key(side)?,
                half: half_from_key(half)?,
            },
            ("restore-panel", [_, ..]) => Self::RestorePanel(PanelId::new(joined(0))),
            ("restore-default", []) => Self::RestoreDefault,
            ("hide-all", []) => Self::HideAll,
            ("maximize", []) => Self::Maximize,
            ("show-section", [section]) => Self::ShowSection(SectionId::new(*section)),
            ("hide-section", [section]) => Self::HideSection(SectionId::new(*section)),
            ("move-section", [section, _, ..]) => Self::MoveSection {
                section: SectionId::new(*section),
                panel: PanelId::new(joined(1)),
            },
            ("new-panel", [side]) => Self::NewPanel(side_from_key(side)?),
            ("preset", [preset]) => Self::ApplyPreset(
                LayoutPreset::ALL
                    .into_iter()
                    .find(|candidate| preset_key(*candidate) == *preset)?,
            ),
            ("save", []) => Self::SaveLayout,
            ("apply-saved", [index]) => Self::ApplySaved(index.parse().ok()?),
            ("delete-saved", [index]) => Self::DeleteSaved(index.parse().ok()?),
            ("centre", [mode]) => Self::SetCentre(
                CENTRE_MODES
                    .into_iter()
                    .find(|candidate| centre_key(*candidate) == *mode)?,
            ),
            ("pop-out", [_, ..]) => Self::PopOut(PanelId::new(joined(0))),
            ("reset-size", [side]) => Self::ResetDockSize(side_from_key(side)?),
            ("grow", [side]) => Self::ResizeDock {
                side: side_from_key(side)?,
                grow: true,
            },
            ("shrink", [side]) => Self::ResizeDock {
                side: side_from_key(side)?,
                grow: false,
            },
            _ => return None,
        })
    }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn entry(
    command: DockCommand,
    label: String,
    keywords: &'static str,
    shortcut: Shortcut,
) -> CommandEntry {
    CommandEntry {
        id: leak(command.palette_id().expect("palette command has an id")),
        label: leak(label),
        shortcut,
        category: "Layout",
        requires_repo: true,
        keywords,
        needs: Needs::Nothing,
    }
}

fn panel_shortcut(index: usize) -> Shortcut {
    const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match DIGITS.get(index) {
        Some(digit) => Shortcut::Platform {
            macos: leak(format!("Cmd+{digit}")),
            other: leak(format!("Alt+{digit}")),
        },
        None => Shortcut::None,
    }
}

fn saved_layout_shortcut(index: usize) -> Shortcut {
    Shortcut::Platform {
        macos: leak(format!("Cmd+Alt+{index}")),
        other: leak(format!("Ctrl+Alt+{index}")),
    }
}

fn build_entries() -> Vec<CommandEntry> {
    let factory = DockLayout::factory();
    let mut out = vec![
        entry(
            DockCommand::RestorePanel(PanelId::from(panel_ids::BRANCHES)),
            "Restore Branches Panel (Today's Sidebar)".into(),
            "reset sidebar left menu default sections local remote worktrees stashes",
            Shortcut::None,
        ),
        entry(
            DockCommand::RestoreDefault,
            "Restore Default Layout".into(),
            "reset panels docks factory original",
            Shortcut::None,
        ),
        entry(
            DockCommand::HideAll,
            "Hide All Panels".into(),
            "distraction free focus docks",
            Shortcut::Secondary("Shift+F12"),
        ),
        entry(
            DockCommand::Maximize,
            "Maximize Panel".into(),
            "zoom fill window dock",
            Shortcut::Secondary("Shift+'"),
        ),
    ];
    for (index, panel) in BUILTIN_PANELS.iter().enumerate() {
        let panel = PanelId::from(*panel);
        let title = panel_title(&factory, &panel);
        out.push(entry(
            DockCommand::TogglePanel(panel.clone()),
            format!("Show or Hide {title}"),
            "panel toggle dock",
            panel_shortcut(index),
        ));
    }
    for panel in BUILTIN_PANELS {
        let panel = PanelId::from(panel);
        let title = panel_title(&factory, &panel);
        for side in DockSide::ALL {
            for half in DockHalf::ALL {
                out.push(entry(
                    DockCommand::MovePanel {
                        panel: panel.clone(),
                        side,
                        half,
                    },
                    format!("Move {title} to {}", side_label(side, half)),
                    "panel dock position place",
                    Shortcut::None,
                ));
            }
        }
        if panel.as_str() != panel_ids::BRANCHES {
            out.push(entry(
                DockCommand::RestorePanel(panel.clone()),
                format!("Restore {title} Panel"),
                "reset default position",
                Shortcut::None,
            ));
        }
        if can_pop_out(&panel) {
            out.push(entry(
                DockCommand::PopOut(panel.clone()),
                format!("Open {title} in New Window"),
                "pop out detach float window",
                Shortcut::None,
            ));
        }
    }
    for section in ALL_SECTIONS {
        let section = SectionId::from(section);
        let title = section.title().to_owned();
        out.push(entry(
            DockCommand::ShowSection(section.clone()),
            format!("Show {title}"),
            "section jump go to reveal",
            Shortcut::None,
        ));
        out.push(entry(
            DockCommand::HideSection(section.clone()),
            format!("Hide {title}"),
            "section remove",
            Shortcut::None,
        ));
        for panel in [panel_ids::BRANCHES, panel_ids::CHANGES] {
            let panel = PanelId::from(panel);
            out.push(entry(
                DockCommand::MoveSection {
                    section: section.clone(),
                    panel: panel.clone(),
                },
                format!("Move {title} to {} Panel", panel_title(&factory, &panel)),
                "section place",
                Shortcut::None,
            ));
        }
    }
    for side in DockSide::ALL {
        out.push(entry(
            DockCommand::NewPanel(side),
            format!("New Panel on the {}", side.label()),
            "custom panel create add",
            Shortcut::None,
        ));
        out.push(entry(
            DockCommand::ResetDockSize(side),
            format!("Reset {} Dock Size", side.label()),
            "width height splitter",
            Shortcut::None,
        ));
        for grow in [true, false] {
            out.push(entry(
                DockCommand::ResizeDock { side, grow },
                format!(
                    "{} {} Dock",
                    if grow { "Grow" } else { "Shrink" },
                    side.label()
                ),
                "resize width height keyboard larger smaller",
                Shortcut::None,
            ));
        }
    }
    for preset in LayoutPreset::ALL {
        out.push(entry(
            DockCommand::ApplyPreset(preset),
            format!("Layout: {}", preset.label()),
            "preset workspace arrangement",
            Shortcut::None,
        ));
    }
    out.push(entry(
        DockCommand::SaveLayout,
        "Save Current Layout".into(),
        "named layout store remember",
        Shortcut::None,
    ));
    for index in 1..=9 {
        out.push(entry(
            DockCommand::ApplySaved(index),
            format!("Apply Saved Layout {index}"),
            "named layout switch",
            saved_layout_shortcut(index),
        ));
    }
    for index in 1..=9 {
        out.push(entry(
            DockCommand::DeleteSaved(index),
            format!("Delete Saved Layout {index}"),
            "named layout remove forget",
            Shortcut::None,
        ));
    }
    for mode in CENTRE_MODES {
        let label = match mode {
            CentreSplitMode::Single => "Centre: History or Diff",
            CentreSplitMode::Auto => "Centre: History and Diff, Automatic",
            CentreSplitMode::SideBySide => "Centre: History and Diff Side by Side",
            CentreSplitMode::Stacked => "Centre: History Above Diff",
        };
        out.push(entry(
            DockCommand::SetCentre(mode),
            label.into(),
            "split history diff centre center",
            Shortcut::None,
        ));
    }
    out
}

/// Layout rows followed by `extensions`, built once per extension list.
pub(in crate::view) fn palette_entries(
    extensions: &'static [CommandEntry],
) -> &'static [CommandEntry] {
    thread_local! {
        static CACHE: std::cell::RefCell<Vec<(usize, &'static [CommandEntry])>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    let key = extensions.as_ptr() as usize ^ extensions.len();
    CACHE.with(|cache| {
        if let Some((_, entries)) = cache.borrow().iter().find(|(k, _)| *k == key) {
            return *entries;
        }
        let mut entries = build_entries();
        entries.extend(extensions.iter().map(|entry| CommandEntry {
            id: entry.id,
            label: entry.label,
            shortcut: entry.shortcut,
            category: entry.category,
            requires_repo: entry.requires_repo,
            keywords: entry.keywords,
            needs: entry.needs,
        }));
        let leaked: &'static [CommandEntry] = Box::leak(entries.into_boxed_slice());
        cache.borrow_mut().push((key, leaked));
        leaked
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_layout_command_round_trips_through_its_palette_id() {
        for entry in build_entries() {
            let command = DockCommand::from_palette_id(entry.id)
                .unwrap_or_else(|| panic!("{} does not parse", entry.id));
            assert_eq!(command.palette_id().as_deref(), Some(entry.id));
        }
    }

    #[test]
    fn custom_panel_ids_survive_the_colon_in_their_id() {
        let command = DockCommand::MoveSection {
            section: SectionId::from(section_ids::TAGS),
            panel: PanelId::new("custom:3"),
        };
        let id = command.palette_id().unwrap();
        assert_eq!(DockCommand::from_palette_id(&id), Some(command));
        assert_eq!(
            DockCommand::from_palette_id("layout:toggle-panel:custom:2"),
            Some(DockCommand::TogglePanel(PanelId::new("custom:2")))
        );
        assert_eq!(DockCommand::from_palette_id("layout:nope"), None);
        assert_eq!(DockCommand::from_palette_id("commit"), None);
    }

    #[test]
    fn the_palette_offers_a_way_back_to_todays_sidebar() {
        let entries = build_entries();
        assert!(entries.iter().any(|entry| {
            entry.label.contains("Restore Branches Panel")
                && DockCommand::from_palette_id(entry.id)
                    == Some(DockCommand::RestorePanel(PanelId::from(
                        panel_ids::BRANCHES,
                    )))
        }));
        assert!(entries.iter().any(|entry| entry.label == "Show Tags"));
    }
}
