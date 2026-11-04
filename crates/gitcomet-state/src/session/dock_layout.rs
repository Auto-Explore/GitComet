//! Where the window's panels sit and which sections each panel shows.
//!
//! The model is plain data with pure operations, so every user action (a
//! drag, a menu item, a shortcut, a palette command) becomes one call here
//! and the renderer only reads the result. Its shape follows the tool window
//! layout of IntelliJ Community (`WindowInfoImpl`, `DesktopLayout`,
//! `ToolWindowDefaultLayoutManager`; Apache-2.0) and OpenIDEA's tool window
//! store (Apache-2.0): three docks with a primary and a split half each, one
//! visible panel per half, a stash for "Hide all", and versioned snapshots.
//!
//! The factory default reproduces the window as it was before layouts were
//! configurable: Branches, Files and File history on the left, Changes on
//! the right, Terminal and Reflog at the bottom.

use super::*;

/// Bumped when a saved layout needs a migration step to load.
pub const DOCK_LAYOUT_VERSION: u32 = 1;

/// The smallest and largest share of a dock the primary half may take, in
/// thousandths. OpenIDEA clamps the same ratio to 0.1..=0.9.
pub const MIN_SPLIT_PERMILLE: u16 = 100;
pub const MAX_SPLIT_PERMILLE: u16 = 900;
pub const DEFAULT_SPLIT_PERMILLE: u16 = 500;

/// Dock sizes are design units; OpenIDEA clamps pixel sizes to 80..=4000.
pub const MIN_DOCK_SIZE: u32 = 80;
pub const MAX_DOCK_SIZE: u32 = 4000;

/// Built-in panel ids. Custom panels use [`CUSTOM_PANEL_PREFIX`] and
/// extension panels [`EXTENSION_ID_PREFIX`].
pub mod panel_ids {
    pub const BRANCHES: &str = "branches";
    pub const FILES: &str = "files";
    pub const FILE_HISTORY: &str = "file_history";
    pub const CHANGES: &str = "changes";
    pub const TERMINAL: &str = "terminal";
    pub const REFLOG: &str = "reflog";
}

/// Built-in section ids. Extension sections use [`EXTENSION_ID_PREFIX`].
pub mod section_ids {
    pub const LOCAL_BRANCHES: &str = "local_branches";
    pub const REMOTE_BRANCHES: &str = "remote_branches";
    pub const TAGS: &str = "tags";
    pub const WORKTREES: &str = "worktrees";
    pub const SUBMODULES: &str = "submodules";
    pub const ANNEX: &str = "annex";
    pub const STASHES: &str = "stashes";
    pub const UNSTAGED: &str = "unstaged";
    pub const UNTRACKED: &str = "untracked";
    pub const STAGED: &str = "staged";
    pub const COMMIT_MESSAGE: &str = "commit_message";
}

pub const CUSTOM_PANEL_PREFIX: &str = "custom:";
pub const EXTENSION_ID_PREFIX: &str = "ext:";

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DockSide {
    #[default]
    Left,
    Right,
    Bottom,
}

impl DockSide {
    pub const ALL: [Self; 3] = [Self::Left, Self::Right, Self::Bottom];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Bottom => "Bottom",
        }
    }
}

/// A dock shows up to two panels at once: the primary half and, below or
/// beside it, the split half (IntelliJ's `isSplit`).
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum DockHalf {
    #[default]
    Primary,
    Split,
}

impl DockHalf {
    pub const ALL: [Self; 2] = [Self::Primary, Self::Split];
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PanelId(String);

impl PanelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_custom(&self) -> bool {
        self.0.starts_with(CUSTOM_PANEL_PREFIX)
    }

    pub fn is_extension(&self) -> bool {
        self.0.starts_with(EXTENSION_ID_PREFIX)
    }
}

impl From<&str> for PanelId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for PanelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SectionId(String);

impl SectionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_extension(&self) -> bool {
        self.0.starts_with(EXTENSION_ID_PREFIX)
    }

    /// The renderer that draws a built-in section; extension sections render
    /// with the reference list, as they do in today's sidebar.
    pub fn kind(&self) -> SectionKind {
        match self.as_str() {
            section_ids::UNSTAGED
            | section_ids::UNTRACKED
            | section_ids::STAGED
            | section_ids::COMMIT_MESSAGE => SectionKind::Changes,
            _ => SectionKind::References,
        }
    }

    /// The panel a built-in section belongs to in the factory layout.
    pub fn default_panel(&self) -> PanelId {
        match self.kind() {
            SectionKind::Changes => PanelId::from(panel_ids::CHANGES),
            SectionKind::References => PanelId::from(panel_ids::BRANCHES),
        }
    }

    pub fn title(&self) -> &str {
        match self.as_str() {
            section_ids::LOCAL_BRANCHES => "Local Branches",
            section_ids::REMOTE_BRANCHES => "Remote Branches",
            section_ids::TAGS => "Tags",
            section_ids::WORKTREES => "Worktrees",
            section_ids::SUBMODULES => "Submodules",
            section_ids::ANNEX => "git-annex",
            section_ids::STASHES => "Stashes",
            section_ids::UNSTAGED => "Unstaged",
            section_ids::UNTRACKED => "Untracked",
            section_ids::STAGED => "Staged",
            section_ids::COMMIT_MESSAGE => "Commit message",
            other => other.strip_prefix(EXTENSION_ID_PREFIX).unwrap_or(other),
        }
    }
}

impl From<&str> for SectionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Display for SectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which host draws a section: the reference list (branches, stashes, ...)
/// or the change list (unstaged, staged, the commit box).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SectionKind {
    References,
    Changes,
}

/// The reference sections in the order today's sidebar lists them.
pub const DEFAULT_BRANCHES_SECTIONS: [&str; 6] = [
    section_ids::LOCAL_BRANCHES,
    section_ids::REMOTE_BRANCHES,
    section_ids::WORKTREES,
    section_ids::SUBMODULES,
    section_ids::ANNEX,
    section_ids::STASHES,
];

/// The change sections in the order today's details pane lists them.
pub const DEFAULT_CHANGES_SECTIONS: [&str; 4] = [
    section_ids::UNSTAGED,
    section_ids::UNTRACKED,
    section_ids::STAGED,
    section_ids::COMMIT_MESSAGE,
];

/// Sections the factory layout knows about but does not show.
pub const DEFAULT_HIDDEN_SECTIONS: [&str; 1] = [section_ids::TAGS];

/// Every built-in section, in catalogue order.
pub fn builtin_sections() -> impl Iterator<Item = SectionId> {
    DEFAULT_BRANCHES_SECTIONS
        .iter()
        .chain(DEFAULT_HIDDEN_SECTIONS.iter())
        .chain(DEFAULT_CHANGES_SECTIONS.iter())
        .map(|id| SectionId::from(*id))
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct PanelPlacement {
    pub side: DockSide,
    pub half: DockHalf,
    /// Position among the panels of the same half; renumbered in steps of 10.
    pub order: u32,
}

impl PanelPlacement {
    pub const fn new(side: DockSide, half: DockHalf, order: u32) -> Self {
        Self { side, half, order }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct DockState {
    /// Width for the left and right docks, height for the bottom one, in
    /// design units. `None` keeps the built-in default.
    pub size: Option<u32>,
    /// Share of the dock the primary half takes when both halves show.
    pub split_permille: u16,
    /// Collapsed docks keep their active panels for when they come back.
    pub hidden: bool,
    pub active: Option<PanelId>,
    pub active_split: Option<PanelId>,
}

impl Default for DockState {
    fn default() -> Self {
        Self {
            size: None,
            split_permille: DEFAULT_SPLIT_PERMILLE,
            hidden: false,
            active: None,
            active_split: None,
        }
    }
}

impl DockState {
    pub fn active_in(&self, half: DockHalf) -> Option<&PanelId> {
        match half {
            DockHalf::Primary => self.active.as_ref(),
            DockHalf::Split => self.active_split.as_ref(),
        }
    }

    fn active_mut(&mut self, half: DockHalf) -> &mut Option<PanelId> {
        match half {
            DockHalf::Primary => &mut self.active,
            DockHalf::Split => &mut self.active_split,
        }
    }
}

/// A panel the user made. Its sections come from other panels.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct CustomPanel {
    pub id: PanelId,
    pub title: String,
}

/// How the centre shows history and the diff.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CentreSplitMode {
    /// One view at a time, as before layouts were configurable.
    #[default]
    Single,
    /// Side by side on wide windows, stacked on tall ones (lazygit's
    /// `mainPanelSplitMode: flexible`).
    Auto,
    SideBySide,
    Stacked,
}

impl CentreSplitMode {
    /// Resolves `Auto` against the centre's size: side by side when it is at
    /// least 1.6 times as wide as it is tall.
    pub fn resolve(self, width: f32, height: f32) -> Self {
        match self {
            Self::Auto if width >= height * 1.6 => Self::SideBySide,
            Self::Auto => Self::Stacked,
            other => other,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct CentreLayout {
    pub mode: CentreSplitMode,
    /// Share of the centre the history takes when split.
    pub split_permille: u16,
}

impl Default for CentreLayout {
    fn default() -> Self {
        Self {
            mode: CentreSplitMode::Single,
            split_permille: DEFAULT_SPLIT_PERMILLE,
        }
    }
}

/// The whole arrangement of one window.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct DockLayout {
    pub version: u32,
    /// Placement for every panel this layout has seen, including ones that
    /// are not registered right now (an uninstalled extension keeps its spot).
    pub panels: BTreeMap<PanelId, PanelPlacement>,
    /// Each panel's sections, top to bottom.
    pub contents: BTreeMap<PanelId, Vec<SectionId>>,
    /// Sections the user (or the factory layout) chose not to show.
    pub hidden_sections: BTreeSet<SectionId>,
    /// Relative heights inside a panel's section stack.
    pub section_weights: BTreeMap<SectionId, u32>,
    pub custom_panels: Vec<CustomPanel>,
    pub docks: BTreeMap<DockSide, DockState>,
    /// Docks that were visible before "Hide all"; the next "Hide all" brings
    /// them back.
    pub hide_all_stash: Option<BTreeSet<DockSide>>,
    pub centre: CentreLayout,
}

impl Default for DockLayout {
    fn default() -> Self {
        Self::factory()
    }
}

/// The result of asking where a section lives.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SectionLocation {
    InPanel { panel: PanelId, index: usize },
    Hidden,
    Unknown,
}

impl DockLayout {
    /// Today's window: the layout every user starts from and returns to.
    pub fn factory() -> Self {
        let mut panels = BTreeMap::new();
        let mut place = |id: &str, side, order| {
            panels.insert(
                PanelId::from(id),
                PanelPlacement::new(side, DockHalf::Primary, order),
            );
        };
        place(panel_ids::BRANCHES, DockSide::Left, 10);
        place(panel_ids::FILES, DockSide::Left, 20);
        place(panel_ids::FILE_HISTORY, DockSide::Left, 30);
        place(panel_ids::CHANGES, DockSide::Right, 10);
        place(panel_ids::TERMINAL, DockSide::Bottom, 10);
        place(panel_ids::REFLOG, DockSide::Bottom, 20);

        let mut contents = BTreeMap::new();
        contents.insert(
            PanelId::from(panel_ids::BRANCHES),
            DEFAULT_BRANCHES_SECTIONS
                .iter()
                .map(|id| SectionId::from(*id))
                .collect(),
        );
        contents.insert(
            PanelId::from(panel_ids::CHANGES),
            DEFAULT_CHANGES_SECTIONS
                .iter()
                .map(|id| SectionId::from(*id))
                .collect(),
        );

        let mut docks = BTreeMap::new();
        docks.insert(
            DockSide::Left,
            DockState {
                active: Some(PanelId::from(panel_ids::BRANCHES)),
                ..DockState::default()
            },
        );
        docks.insert(
            DockSide::Right,
            DockState {
                active: Some(PanelId::from(panel_ids::CHANGES)),
                ..DockState::default()
            },
        );
        // The bottom dock opens when the terminal or reflog is asked for.
        docks.insert(
            DockSide::Bottom,
            DockState {
                hidden: true,
                active: Some(PanelId::from(panel_ids::TERMINAL)),
                ..DockState::default()
            },
        );

        Self {
            version: DOCK_LAYOUT_VERSION,
            panels,
            contents,
            hidden_sections: DEFAULT_HIDDEN_SECTIONS
                .iter()
                .map(|id| SectionId::from(*id))
                .collect(),
            section_weights: BTreeMap::new(),
            custom_panels: Vec::new(),
            docks,
            hide_all_stash: None,
            centre: CentreLayout::default(),
        }
    }

    /// The factory layout carrying the sizes a window saved before layouts
    /// were configurable.
    pub fn from_legacy(legacy: &WorkspaceLayout) -> Self {
        let mut layout = Self::factory();
        layout.dock_mut(DockSide::Left).size = legacy.sidebar_width;
        layout.dock_mut(DockSide::Left).hidden = legacy.sidebar_collapsed;
        layout.dock_mut(DockSide::Right).size = legacy.details_width;
        if let Some(height) = legacy.change_tracking_height {
            layout
                .section_weights
                .insert(SectionId::from(section_ids::UNSTAGED), height);
        }
        if let Some(height) = legacy.untracked_height {
            layout
                .section_weights
                .insert(SectionId::from(section_ids::UNTRACKED), height);
        }
        layout
    }

    pub fn dock(&self, side: DockSide) -> &DockState {
        static DEFAULT: OnceLock<DockState> = OnceLock::new();
        self.docks
            .get(&side)
            .unwrap_or_else(|| DEFAULT.get_or_init(DockState::default))
    }

    pub fn dock_mut(&mut self, side: DockSide) -> &mut DockState {
        self.docks.entry(side).or_default()
    }

    // ----- panels -------------------------------------------------------

    pub fn placement(&self, panel: &PanelId) -> Option<&PanelPlacement> {
        self.panels.get(panel)
    }

    /// The panels of one half, in tab order.
    pub fn panels_in(&self, side: DockSide, half: DockHalf) -> Vec<PanelId> {
        let mut panels: Vec<(&PanelId, &PanelPlacement)> = self
            .panels
            .iter()
            .filter(|(_, placement)| placement.side == side && placement.half == half)
            .collect();
        panels.sort_by(|(a_id, a), (b_id, b)| a.order.cmp(&b.order).then_with(|| a_id.cmp(b_id)));
        panels.into_iter().map(|(id, _)| id.clone()).collect()
    }

    /// The panel a half shows, falling back to its first tab.
    pub fn shown_panel(&self, side: DockSide, half: DockHalf) -> Option<PanelId> {
        let tabs = self.panels_in(side, half);
        let active = self.dock(side).active_in(half);
        match active {
            Some(active) if tabs.contains(active) => Some(active.clone()),
            Some(_) | None if half == DockHalf::Primary => tabs.into_iter().next(),
            _ => None,
        }
    }

    /// Whether a dock takes space: it is not collapsed and holds a panel.
    pub fn is_dock_visible(&self, side: DockSide) -> bool {
        !self.dock(side).hidden
            && (self.shown_panel(side, DockHalf::Primary).is_some()
                || self.shown_panel(side, DockHalf::Split).is_some())
    }

    /// Whether a panel is on screen: its dock is shown and its half shows it.
    pub fn is_panel_visible(&self, panel: &PanelId) -> bool {
        let Some(placement) = self.panels.get(panel) else {
            return false;
        };
        !self.dock(placement.side).hidden
            && self.shown_panel(placement.side, placement.half).as_ref() == Some(panel)
    }

    /// Adds a panel this layout has not seen at its default placement.
    pub fn ensure_panel(&mut self, panel: &PanelId, default: PanelPlacement) -> bool {
        if self.panels.contains_key(panel) {
            return false;
        }
        let order = self
            .next_order(default.side, default.half)
            .max(default.order);
        self.panels
            .insert(panel.clone(), PanelPlacement { order, ..default });
        true
    }

    fn next_order(&self, side: DockSide, half: DockHalf) -> u32 {
        self.panels
            .values()
            .filter(|placement| placement.side == side && placement.half == half)
            .map(|placement| placement.order)
            .max()
            .map_or(10, |order| order + 10)
    }

    /// Moves a panel to a half, before `before` or at the end. A panel that
    /// was showing keeps showing in its new place.
    pub fn move_panel(
        &mut self,
        panel: &PanelId,
        side: DockSide,
        half: DockHalf,
        before: Option<&PanelId>,
    ) -> bool {
        let Some(old) = self.panels.get(panel).cloned() else {
            return false;
        };
        let was_visible = self.is_panel_visible(panel);
        let mut tabs: Vec<PanelId> = self
            .panels_in(side, half)
            .into_iter()
            .filter(|id| id != panel)
            .collect();
        let index = before
            .and_then(|before| tabs.iter().position(|id| id == before))
            .unwrap_or(tabs.len());
        tabs.insert(index, panel.clone());
        for (position, id) in tabs.iter().enumerate() {
            if let Some(placement) = self.panels.get_mut(id) {
                placement.side = side;
                placement.half = half;
                placement.order = (position as u32 + 1) * 10;
            }
        }
        // The half it left falls back to its first remaining tab.
        if old.side != side || old.half != half {
            let left = self.dock_mut(old.side).active_mut(old.half);
            if left.as_ref() == Some(panel) {
                *left = None;
            }
        }
        if was_visible {
            self.show_panel(panel);
        }
        let new = self.panels.get(panel).cloned();
        new.as_ref() != Some(&old) || was_visible != self.is_panel_visible(panel)
    }

    /// Shows a panel: opens its dock and selects it in its half.
    pub fn show_panel(&mut self, panel: &PanelId) -> bool {
        let Some(placement) = self.panels.get(panel).cloned() else {
            return false;
        };
        let before = self.dock(placement.side).clone();
        let dock = self.dock_mut(placement.side);
        dock.hidden = false;
        *dock.active_mut(placement.half) = Some(panel.clone());
        self.hide_all_stash = None;
        *self.dock(placement.side) != before
    }

    /// Hides a panel. The primary half collapses its whole dock, as the old
    /// sidebar toggle did; the split half just closes.
    pub fn hide_panel(&mut self, panel: &PanelId) -> bool {
        if !self.is_panel_visible(panel) {
            return false;
        }
        let Some(placement) = self.panels.get(panel).cloned() else {
            return false;
        };
        let dock = self.dock_mut(placement.side);
        match placement.half {
            DockHalf::Primary => dock.hidden = true,
            DockHalf::Split => dock.active_split = None,
        }
        true
    }

    pub fn toggle_panel(&mut self, panel: &PanelId) -> bool {
        if self.is_panel_visible(panel) {
            self.hide_panel(panel)
        } else {
            self.show_panel(panel)
        }
    }

    pub fn set_dock_hidden(&mut self, side: DockSide, hidden: bool) -> bool {
        let dock = self.dock_mut(side);
        if dock.hidden == hidden {
            return false;
        }
        dock.hidden = hidden;
        if !hidden {
            self.hide_all_stash = None;
        }
        true
    }

    pub fn toggle_dock(&mut self, side: DockSide) -> bool {
        let hidden = !self.dock(side).hidden;
        self.set_dock_hidden(side, hidden)
    }

    /// Hides every dock, remembering which were open; a second call brings
    /// them back.
    pub fn toggle_hide_all(&mut self) -> bool {
        let visible: BTreeSet<DockSide> = DockSide::ALL
            .into_iter()
            .filter(|side| self.is_dock_visible(*side))
            .collect();
        if !visible.is_empty() {
            for side in &visible {
                self.dock_mut(*side).hidden = true;
            }
            self.hide_all_stash = Some(visible);
            return true;
        }
        let Some(stash) = self.hide_all_stash.take() else {
            return false;
        };
        for side in stash {
            self.dock_mut(side).hidden = false;
        }
        true
    }

    pub fn set_dock_size(&mut self, side: DockSide, size: Option<u32>) -> bool {
        let size = size.map(|size| size.clamp(MIN_DOCK_SIZE, MAX_DOCK_SIZE));
        let dock = self.dock_mut(side);
        if dock.size == size {
            return false;
        }
        dock.size = size;
        true
    }

    pub fn set_dock_split(&mut self, side: DockSide, permille: u16) -> bool {
        let permille = permille.clamp(MIN_SPLIT_PERMILLE, MAX_SPLIT_PERMILLE);
        let dock = self.dock_mut(side);
        if dock.split_permille == permille {
            return false;
        }
        dock.split_permille = permille;
        true
    }

    // ----- sections -----------------------------------------------------

    pub fn sections_in(&self, panel: &PanelId) -> &[SectionId] {
        self.contents.get(panel).map_or(&[], Vec::as_slice)
    }

    pub fn section_location(&self, section: &SectionId) -> SectionLocation {
        for (panel, sections) in &self.contents {
            if let Some(index) = sections.iter().position(|id| id == section) {
                return SectionLocation::InPanel {
                    panel: panel.clone(),
                    index,
                };
            }
        }
        if self.hidden_sections.contains(section) {
            SectionLocation::Hidden
        } else {
            SectionLocation::Unknown
        }
    }

    pub fn section_panel(&self, section: &SectionId) -> Option<PanelId> {
        match self.section_location(section) {
            SectionLocation::InPanel { panel, .. } => Some(panel),
            SectionLocation::Hidden | SectionLocation::Unknown => None,
        }
    }

    /// Whether a section is on screen (its panel is visible). Collapsing a
    /// section is per repository and is checked by the caller.
    pub fn is_section_visible(&self, section: &SectionId) -> bool {
        self.section_panel(section)
            .is_some_and(|panel| self.is_panel_visible(&panel))
    }

    /// Panels that can hold sections: the two built-in hosts and custom ones.
    pub fn is_section_host(&self, panel: &PanelId) -> bool {
        panel.as_str() == panel_ids::BRANCHES
            || panel.as_str() == panel_ids::CHANGES
            || self.custom_panels.iter().any(|custom| &custom.id == panel)
    }

    fn take_section(&mut self, section: &SectionId) {
        for sections in self.contents.values_mut() {
            sections.retain(|id| id != section);
        }
        self.hidden_sections.remove(section);
    }

    /// Moves a section into a panel, before `before` or at the end.
    pub fn move_section(
        &mut self,
        section: &SectionId,
        panel: &PanelId,
        before: Option<&SectionId>,
    ) -> bool {
        if !self.is_section_host(panel) || before == Some(section) {
            return false;
        }
        let old = self.section_location(section);
        self.take_section(section);
        let sections = self.contents.entry(panel.clone()).or_default();
        let index = before
            .and_then(|before| sections.iter().position(|id| id == before))
            .unwrap_or(sections.len());
        sections.insert(index, section.clone());
        self.section_location(section) != old
    }

    /// Hides a section; [`Self::show_section`] brings it back.
    pub fn hide_section(&mut self, section: &SectionId) -> bool {
        if self.hidden_sections.contains(section) {
            return false;
        }
        self.take_section(section);
        self.hidden_sections.insert(section.clone());
        true
    }

    /// Shows a hidden or new section in its default panel, at the position
    /// the factory layout gives it among the sections already there.
    pub fn show_section(&mut self, section: &SectionId, default_panel: &PanelId) -> bool {
        if matches!(
            self.section_location(section),
            SectionLocation::InPanel { .. }
        ) {
            return false;
        }
        let panel = if self.is_section_host(default_panel) {
            default_panel.clone()
        } else {
            section.default_panel()
        };
        self.hidden_sections.remove(section);
        let factory_order = factory_section_rank(section);
        let sections = self.contents.entry(panel).or_default();
        let index = sections
            .iter()
            .position(|other| factory_section_rank(other) > factory_order)
            .unwrap_or(sections.len());
        sections.insert(index, section.clone());
        true
    }

    pub fn set_section_weight(&mut self, section: &SectionId, weight: Option<u32>) -> bool {
        match weight {
            Some(weight) => self.section_weights.insert(section.clone(), weight) != Some(weight),
            None => self.section_weights.remove(section).is_some(),
        }
    }

    // ----- custom panels ------------------------------------------------

    /// Creates an empty panel at the end of a dock's primary half and shows it.
    pub fn add_custom_panel(&mut self, title: &str, side: DockSide) -> PanelId {
        let next = self
            .custom_panels
            .iter()
            .filter_map(|panel| {
                panel
                    .id
                    .as_str()
                    .strip_prefix(CUSTOM_PANEL_PREFIX)?
                    .parse::<u32>()
                    .ok()
            })
            .max()
            .map_or(1, |n| n + 1);
        let id = PanelId::new(format!("{CUSTOM_PANEL_PREFIX}{next}"));
        let title = title.trim();
        self.custom_panels.push(CustomPanel {
            id: id.clone(),
            title: if title.is_empty() {
                format!("Panel {next}")
            } else {
                title.to_owned()
            },
        });
        let order = self.next_order(side, DockHalf::Primary);
        self.panels.insert(
            id.clone(),
            PanelPlacement::new(side, DockHalf::Primary, order),
        );
        self.contents.insert(id.clone(), Vec::new());
        self.show_panel(&id);
        id
    }

    pub fn rename_custom_panel(&mut self, panel: &PanelId, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        match self
            .custom_panels
            .iter_mut()
            .find(|custom| &custom.id == panel)
        {
            Some(custom) if custom.title != title => {
                custom.title = title.to_owned();
                true
            }
            _ => false,
        }
    }

    pub fn custom_panel_title(&self, panel: &PanelId) -> Option<&str> {
        self.custom_panels
            .iter()
            .find(|custom| &custom.id == panel)
            .map(|custom| custom.title.as_str())
    }

    /// Removes a custom panel. Its sections go back to their default panels,
    /// so nothing the user moved there disappears.
    pub fn remove_custom_panel(&mut self, panel: &PanelId) -> bool {
        let Some(index) = self
            .custom_panels
            .iter()
            .position(|custom| &custom.id == panel)
        else {
            return false;
        };
        self.custom_panels.remove(index);
        let sections = self.contents.remove(panel).unwrap_or_default();
        self.panels.remove(panel);
        for dock in self.docks.values_mut() {
            for half in DockHalf::ALL {
                let active = dock.active_mut(half);
                if active.as_ref() == Some(panel) {
                    *active = None;
                }
            }
        }
        for section in sections {
            let default = section.default_panel();
            self.show_section(&section, &default);
        }
        true
    }

    // ----- resets -------------------------------------------------------

    /// Puts a built-in panel back the way the factory layout has it: its
    /// place in the window and its sections in their original order. This is
    /// how the user rebuilds the old sidebar after rearranging it. Sections
    /// that were added to it (Tags, a custom choice) stay at the end.
    pub fn restore_panel(&mut self, panel: &PanelId) -> bool {
        let factory = Self::factory();
        let before = self.clone();
        if let Some(placement) = factory.panels.get(panel).cloned() {
            self.move_panel(panel, placement.side, placement.half, None);
            self.normalize_factory_order(placement.side, placement.half, &factory);
        }
        let defaults = factory.sections_in(panel).to_vec();
        if !defaults.is_empty() {
            let extras: Vec<SectionId> = self
                .sections_in(panel)
                .iter()
                .filter(|id| !defaults.contains(id))
                .cloned()
                .collect();
            for section in defaults.iter().chain(extras.iter()) {
                self.take_section(section);
            }
            let sections = self.contents.entry(panel.clone()).or_default();
            sections.extend(defaults);
            sections.extend(extras);
        }
        self.show_panel(panel);
        *self != before
    }

    /// Restores factory tab order inside one half for the built-in panels.
    fn normalize_factory_order(&mut self, side: DockSide, half: DockHalf, factory: &Self) {
        let tabs = self.panels_in(side, half);
        let mut ordered: Vec<PanelId> = tabs.clone();
        ordered.sort_by_key(|id| {
            factory
                .panels
                .get(id)
                .filter(|placement| placement.side == side && placement.half == half)
                .map_or(u32::MAX, |placement| placement.order)
        });
        for (position, id) in ordered.iter().enumerate() {
            if let Some(placement) = self.panels.get_mut(id) {
                placement.order = (position as u32 + 1) * 10;
            }
        }
    }

    /// The factory layout, keeping the sizes the user chose. Custom panels go
    /// away; [`Self::custom_panels`] tells the caller which ones before it
    /// asks the user.
    pub fn restore_default(&mut self) -> bool {
        let mut factory = Self::factory();
        for side in DockSide::ALL {
            factory.dock_mut(side).size = self.dock(side).size;
        }
        factory.section_weights = std::mem::take(&mut self.section_weights);
        // Extension panels and sections keep their registered defaults.
        for (panel, placement) in &self.panels {
            if panel.is_extension() {
                factory.panels.insert(panel.clone(), placement.clone());
            }
        }
        let changed = factory != *self;
        *self = factory;
        changed
    }

    // ----- registration and loading ------------------------------------

    /// Adds panels and sections that registered after the layout was saved,
    /// at their defaults. Sections the user hid stay hidden, and placements
    /// of panels that are not registered now are kept for later.
    pub fn reconcile(
        &mut self,
        panels: &[(PanelId, PanelPlacement)],
        sections: &[(SectionId, PanelId)],
    ) -> bool {
        let mut changed = false;
        for (panel, default) in panels {
            changed |= self.ensure_panel(panel, default.clone());
        }
        for (section, default_panel) in sections {
            if matches!(self.section_location(section), SectionLocation::Unknown) {
                changed |= self.show_section(section, default_panel);
            }
        }
        changed
    }

    /// Repairs a layout read from disk: clamps sizes and ratios, drops
    /// duplicate sections, removes contents of panels that cannot hold
    /// sections, and puts back built-in panels and sections it lost.
    pub fn sanitize(&mut self) {
        self.version = DOCK_LAYOUT_VERSION;
        for dock in self.docks.values_mut() {
            dock.size = dock
                .size
                .map(|size| size.clamp(MIN_DOCK_SIZE, MAX_DOCK_SIZE));
            dock.split_permille = dock
                .split_permille
                .clamp(MIN_SPLIT_PERMILLE, MAX_SPLIT_PERMILLE);
        }
        self.centre.split_permille = self
            .centre
            .split_permille
            .clamp(MIN_SPLIT_PERMILLE, MAX_SPLIT_PERMILLE);
        let custom: BTreeSet<PanelId> = self
            .custom_panels
            .iter()
            .map(|panel| panel.id.clone())
            .collect();
        self.panels
            .retain(|panel, _| !panel.is_custom() || custom.contains(panel));
        let mut orphans = Vec::new();
        let hosts: Vec<PanelId> = self.contents.keys().cloned().collect();
        for panel in hosts {
            if !self.is_section_host(&panel) {
                orphans.extend(self.contents.remove(&panel).unwrap_or_default());
            }
        }
        let mut seen = BTreeSet::new();
        for sections in self.contents.values_mut() {
            sections.retain(|section| seen.insert(section.clone()));
        }
        self.hidden_sections
            .retain(|section| !seen.contains(section));
        for custom in &self.custom_panels {
            self.contents.entry(custom.id.clone()).or_default();
            if !self.panels.contains_key(&custom.id) {
                self.panels.insert(
                    custom.id.clone(),
                    PanelPlacement::new(DockSide::Left, DockHalf::Primary, u32::MAX / 2),
                );
            }
        }
        let factory = Self::factory();
        for (panel, placement) in &factory.panels {
            self.panels
                .entry(panel.clone())
                .or_insert_with(|| placement.clone());
        }
        for section in orphans.into_iter().chain(builtin_sections()) {
            if matches!(self.section_location(&section), SectionLocation::Unknown) {
                if factory.hidden_sections.contains(&section) {
                    self.hidden_sections.insert(section);
                } else {
                    let default = section.default_panel();
                    self.show_section(&section, &default);
                }
            }
        }
    }

    /// Sections that are on screen, in layout order. The caller removes the
    /// ones collapsed in the current repository.
    pub fn visible_sections(&self) -> Vec<SectionId> {
        self.contents
            .iter()
            .filter(|(panel, _)| self.is_panel_visible(panel))
            .flat_map(|(_, sections)| sections.iter().cloned())
            .collect()
    }

    /// Applies a preset's dock visibility and centre mode, keeping where the
    /// user put panels and sections.
    pub fn apply_preset(&mut self, preset: LayoutPreset) -> bool {
        let before = self.clone();
        let (left, right, bottom, centre) = preset.shape();
        self.dock_mut(DockSide::Left).hidden = !left;
        self.dock_mut(DockSide::Right).hidden = !right;
        self.dock_mut(DockSide::Bottom).hidden = !bottom;
        self.centre.mode = centre;
        self.hide_all_stash = None;
        *self != before
    }
}

/// Built-in arrangements for common git tasks. They change which docks show
/// and how the centre splits, never where panels or sections are.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LayoutPreset {
    BrowseHistory,
    Commit,
    Review,
    ResolveConflicts,
}

impl LayoutPreset {
    pub const ALL: [Self; 4] = [
        Self::BrowseHistory,
        Self::Commit,
        Self::Review,
        Self::ResolveConflicts,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::BrowseHistory => "Browse history",
            Self::Commit => "Commit",
            Self::Review => "Review",
            Self::ResolveConflicts => "Resolve conflicts",
        }
    }

    /// (left shown, right shown, bottom shown, centre mode)
    const fn shape(self) -> (bool, bool, bool, CentreSplitMode) {
        match self {
            Self::BrowseHistory => (true, false, false, CentreSplitMode::Single),
            Self::Commit => (false, true, false, CentreSplitMode::Single),
            Self::Review => (true, true, false, CentreSplitMode::Auto),
            Self::ResolveConflicts => (false, true, true, CentreSplitMode::Single),
        }
    }
}

/// Layouts the user saved by name, shared by every window.
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(default)]
pub struct NamedLayouts {
    pub layouts: BTreeMap<String, DockLayout>,
    /// Whether the one-time "panels can move" hint was shown.
    pub hint_seen: bool,
}

impl NamedLayouts {
    pub fn save(&mut self, name: &str, layout: &DockLayout) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let mut layout = layout.clone();
        layout.hide_all_stash = None;
        self.layouts.insert(name.to_owned(), layout);
        Some(name.to_owned())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let to = to.trim();
        if to.is_empty() || from == to || self.layouts.contains_key(to) {
            return false;
        }
        match self.layouts.remove(from) {
            Some(layout) => {
                self.layouts.insert(to.to_owned(), layout);
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, name: &str) -> bool {
        self.layouts.remove(name).is_some()
    }

    /// Names in the order the switcher and Ctrl+Alt+1..9 use.
    pub fn names(&self) -> Vec<String> {
        self.layouts.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<&DockLayout> {
        self.layouts.get(name)
    }
}

/// Where the named layouts live: beside the session file, so tests and
/// `GITCOMET_DISABLE_SESSION_PERSIST` runs never touch them.
fn named_layouts_file_path() -> Option<PathBuf> {
    Some(default_session_file_path()?.with_file_name("layouts.json"))
}

/// The user's saved layouts; empty when none were saved or the file is
/// unreadable.
pub fn load_named_layouts() -> NamedLayouts {
    named_layouts_file_path()
        .map(|path| load_named_layouts_from_path(&path))
        .unwrap_or_default()
}

pub fn load_named_layouts_from_path(path: &Path) -> NamedLayouts {
    let Ok(contents) = fs::read(path) else {
        return NamedLayouts::default();
    };
    let mut layouts: NamedLayouts = serde_json::from_slice(&contents).unwrap_or_default();
    for layout in layouts.layouts.values_mut() {
        layout.sanitize();
    }
    layouts
}

pub fn persist_named_layouts(layouts: &NamedLayouts) -> io::Result<()> {
    match named_layouts_file_path() {
        Some(path) => persist_named_layouts_to_path(layouts, &path),
        None => Ok(()),
    }
}

pub fn persist_named_layouts_to_path(layouts: &NamedLayouts, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    persist_to_path(path, layouts)
}

fn factory_section_rank(section: &SectionId) -> usize {
    builtin_sections()
        .position(|id| &id == section)
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests;
