use super::*;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSendError, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point as AlacPoint;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{
    Config, Osc52, Term, TermMode,
    cell::{Cell as AlacCell, Flags},
};
use alacritty_terminal::tty;
use gitcomet_ui_kit::theme::AppTheme;
use gpui::{App, Bounds, Pixels, Point, SharedString, TextRun, Window, px};
use palette::IntoColor;
use std::borrow::Cow;
use std::sync::Arc;

const TERMINAL_INITIAL_ROWS: u16 = 24;
const TERMINAL_INITIAL_COLS: u16 = 80;
pub(super) const TERMINAL_SCROLLBACK_ROWS: usize = 10_000;
const TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS: usize = 24;

#[derive(Clone, Copy, Debug)]
pub(super) struct TerminalAnsiPalette {
    pub foreground: gpui::Rgba,
    pub background: gpui::Rgba,
    pub black: gpui::Rgba,
    pub red: gpui::Rgba,
    pub green: gpui::Rgba,
    pub yellow: gpui::Rgba,
    pub blue: gpui::Rgba,
    pub magenta: gpui::Rgba,
    pub cyan: gpui::Rgba,
    pub white: gpui::Rgba,
    pub bright_black: gpui::Rgba,
    pub bright_red: gpui::Rgba,
    pub bright_green: gpui::Rgba,
    pub bright_yellow: gpui::Rgba,
    pub bright_blue: gpui::Rgba,
    pub bright_magenta: gpui::Rgba,
    pub bright_cyan: gpui::Rgba,
    pub bright_white: gpui::Rgba,
    pub dim_black: gpui::Rgba,
    pub dim_red: gpui::Rgba,
    pub dim_green: gpui::Rgba,
    pub dim_yellow: gpui::Rgba,
    pub dim_blue: gpui::Rgba,
    pub dim_magenta: gpui::Rgba,
    pub dim_cyan: gpui::Rgba,
    pub dim_white: gpui::Rgba,
}

impl TerminalAnsiPalette {
    pub(super) fn from_theme(theme: AppTheme) -> Self {
        let colors = theme.colors;
        let fg = colors.foreground.primary;
        let bg = colors.surface.canvas;

        let dark = theme.is_dark;
        if dark {
            Self {
                foreground: fg,
                background: bg,
                black: gpui::rgb(0x0d1016),
                red: gpui::rgb(0xef7177),
                green: gpui::rgb(0xaad84c),
                yellow: gpui::rgb(0xfeb454),
                blue: gpui::rgb(0x5ac1fe),
                magenta: gpui::rgb(0xde9fc1),
                cyan: gpui::rgb(0x78cce2),
                white: gpui::rgb(0xbfbdb6),
                bright_black: gpui::rgb(0x575b66),
                bright_red: gpui::rgb(0xff7777),
                bright_green: gpui::rgb(0xc6ff68),
                bright_yellow: gpui::rgb(0xffd56b),
                bright_blue: gpui::rgb(0x6dcaff),
                bright_magenta: gpui::rgb(0xf0b0d0),
                bright_cyan: gpui::rgb(0x95e6ff),
                bright_white: gpui::rgb(0xd9d7ce),
                dim_black: gpui::rgb(0x0a0b10),
                dim_red: gpui::rgb(0xb04b50),
                dim_green: gpui::rgb(0x709a30),
                dim_yellow: gpui::rgb(0xb07a35),
                dim_blue: gpui::rgb(0x3d80b0),
                dim_magenta: gpui::rgb(0x9b6a88),
                dim_cyan: gpui::rgb(0x508e9a),
                dim_white: gpui::rgb(0x808080),
            }
        } else {
            Self {
                foreground: fg,
                background: bg,
                black: gpui::rgb(0xfffffe),
                red: colors.status.danger.foreground,
                green: colors.status.success.foreground,
                yellow: colors.status.warning.foreground,
                blue: colors.accent.foreground,
                magenta: gpui::rgb(0x8e4c6f),
                cyan: gpui::rgb(0x1a7f82),
                white: colors.foreground.primary,
                bright_black: gpui::rgb(0x9aa0b0),
                bright_red: gpui::rgb(0xe06c75),
                bright_green: gpui::rgb(0x4caf50),
                bright_yellow: gpui::rgb(0xd4a03c),
                bright_blue: gpui::rgb(0x6e8be0),
                bright_magenta: gpui::rgb(0xae6e8c),
                bright_cyan: gpui::rgb(0x35a0a3),
                bright_white: gpui::rgb(0x383e4a),
                dim_black: gpui::rgb(0xd0d4dd),
                dim_red: gpui::rgb(0xa83b34),
                dim_green: gpui::rgb(0x256c29),
                dim_yellow: gpui::rgb(0x8d651c),
                dim_blue: gpui::rgb(0x3f5eb8),
                dim_magenta: gpui::rgb(0x73405b),
                dim_cyan: gpui::rgb(0x16686c),
                dim_white: gpui::rgb(0x808080),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

pub type AlacrittyTermLock = Arc<FairMutex<Term<GitCometListener>>>;

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GitCometListener {
    pub(super) events_tx: smol::channel::Sender<TerminalBackendEvent>,
}

impl EventListener for GitCometListener {
    fn send_event(&self, event: AlacEvent) {
        let _ = self.events_tx.try_send(event.into());
    }
}

// ---------------------------------------------------------------------------
// Events from Alacritty
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum TerminalBackendEvent {
    /// A response the emulator must write back to the PTY (e.g. cursor-position
    /// report `ESC[6n`, Device Attributes `ESC[c`). Alacritty's `Term` produces
    /// these via the event proxy and relies on us to forward them.
    PtyWrite(String),
    Title(String),
    /// The program printed: the emulator parsed what it read from the PTY.
    Output,
    /// Repaint for anything else, such as the mouse cursor's shape.
    Wakeup,
    Bell,
    Exit,
    ChildExit(Option<i32>),
    CursorBlinkingChange,
}

impl From<AlacEvent> for TerminalBackendEvent {
    fn from(event: AlacEvent) -> Self {
        match event {
            AlacEvent::PtyWrite(text) => Self::PtyWrite(text),
            AlacEvent::Title(title) => Self::Title(title),
            // OSC 52 is disabled in `terminal_config`, so the emulator never
            // raises these. They stay explicitly inert rather than growing a
            // handler: program output must not write to or read the system
            // clipboard, and a clipboard read pushed into the PTY would be
            // unbracketed keystrokes.
            AlacEvent::ClipboardStore(..) | AlacEvent::ClipboardLoad(..) => Self::Wakeup,
            AlacEvent::Wakeup => Self::Output,
            AlacEvent::Bell => Self::Bell,
            AlacEvent::Exit => Self::Exit,
            AlacEvent::ChildExit(status) => Self::ChildExit(status.code()),
            AlacEvent::CursorBlinkingChange => Self::CursorBlinkingChange,
            _ => Self::Wakeup,
        }
    }
}

// ---------------------------------------------------------------------------
// PTY spawning
// ---------------------------------------------------------------------------

pub struct SpawnedAlacTerminal {
    pub term_lock: AlacrittyTermLock,
    pub events_rx: smol::channel::Receiver<TerminalBackendEvent>,
    pub pty_sender: PtySender,
    pub child_pid: Option<u32>,
}

#[derive(Clone)]
pub enum PtySender {
    EventLoop(alacritty_terminal::event_loop::EventLoopSender),
    #[cfg(any(test, feature = "test-support"))]
    Recording(smol::channel::Sender<Msg>),
}

impl PtySender {
    fn send(&self, msg: Msg) -> Result<(), EventLoopSendError> {
        match self {
            Self::EventLoop(sender) => sender.send(msg),
            #[cfg(any(test, feature = "test-support"))]
            Self::Recording(sender) => {
                sender.try_send(msg).ok();
                Ok(())
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn recording() -> (Self, smol::channel::Receiver<Msg>) {
        let (sender, receiver) = smol::channel::unbounded();
        (Self::Recording(sender), receiver)
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Err(err) = self.send(Msg::Input(bytes.into())) {
            eprintln!("terminal: failed to write input to pty: {err}");
        }
    }

    pub fn resize(&self, columns: usize, screen_lines: usize) {
        self.send(Msg::Resize(alacritty_terminal::event::WindowSize {
            num_lines: screen_lines as u16,
            num_cols: columns as u16,
            cell_width: 1,
            cell_height: 1,
        }))
        .ok();
    }

    pub fn shutdown(&self) {
        self.send(Msg::Shutdown).ok();
    }
}

pub fn spawn_alacritty_terminal(
    shell_program: &std::path::Path,
    workdir: &std::path::Path,
    window_id: u64,
) -> Result<SpawnedAlacTerminal, String> {
    let shell_program_str = shell_program.to_string_lossy().to_string();

    let env: Vec<(String, String)> = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        (
            "TERM_PROGRAM".to_string(),
            gitcomet_base::identity::current()
                .display_name()
                .to_string(),
        ),
        (
            "TERM_PROGRAM_VERSION".to_string(),
            gitcomet_base::identity::current().version().to_string(),
        ),
    ];

    let (events_tx, events_rx) = smol::channel::unbounded();

    let pty_options = tty::Options {
        shell: Some(tty::Shell::new(shell_program_str, Vec::<String>::new())),
        working_directory: Some(workdir.to_path_buf()),
        drain_on_exit: true,
        #[cfg(target_os = "windows")]
        escape_args: true,
        env: env.into_iter().collect(),
    };

    let initial_bounds = TerminalDims {
        columns: TERMINAL_INITIAL_COLS as usize,
        screen_lines: TERMINAL_INITIAL_ROWS as usize,
        total_lines: TERMINAL_SCROLLBACK_ROWS,
    };

    let pty = tty::new(
        &pty_options,
        alacritty_terminal::event::WindowSize {
            num_lines: TERMINAL_INITIAL_ROWS,
            num_cols: TERMINAL_INITIAL_COLS,
            cell_width: 1,
            cell_height: 1,
        },
        window_id,
    )
    .map_err(|e| format!("failed to open PTY: {e}"))?;
    let child_pid = pty_child_pid(&pty);

    let config = terminal_config(TERMINAL_SCROLLBACK_ROWS);
    let term_lock = new_term(&config, &initial_bounds, events_tx.clone());

    let event_loop = EventLoop::new(
        term_lock.clone(),
        GitCometListener {
            events_tx: events_tx.clone(),
        },
        pty,
        true,
        false,
    )
    .map_err(|e| format!("failed to create event loop: {e}"))?;

    let event_loop_tx = event_loop.channel();
    let _io_thread = event_loop.spawn();

    Ok(SpawnedAlacTerminal {
        term_lock,
        events_rx,
        pty_sender: PtySender::EventLoop(event_loop_tx),
        child_pid,
    })
}

// ---------------------------------------------------------------------------
// Terminal dimensions
// ---------------------------------------------------------------------------

pub struct TerminalDims {
    pub columns: usize,
    pub screen_lines: usize,
    pub total_lines: usize,
}

impl Dimensions for TerminalDims {
    fn columns(&self) -> usize {
        self.columns
    }
    fn screen_lines(&self) -> usize {
        self.screen_lines
    }
    fn total_lines(&self) -> usize {
        self.total_lines
    }
}

// ---------------------------------------------------------------------------
// Terminal config
// ---------------------------------------------------------------------------

pub fn terminal_config(scrollback: usize) -> Config {
    Config {
        scrolling_history: scrollback,
        osc52: Osc52::Disabled,
        ..Default::default()
    }
}

pub fn new_term(
    config: &Config,
    bounds: &TerminalDims,
    events_tx: smol::channel::Sender<TerminalBackendEvent>,
) -> AlacrittyTermLock {
    let term = Term::new(config.clone(), bounds, GitCometListener { events_tx });
    Arc::new(FairMutex::new(term))
}

/// Clear both grids without resetting terminal modes or sending input to the
/// running process. In particular, leaving an alternate screen must not bring
/// back the primary screen's old output or scrollback.
pub(super) fn clear_terminal_screen_and_scrollback(term: &mut Term<GitCometListener>) {
    let mut cursor = term.grid().cursor.clone();
    let mut saved_cursor = term.grid().saved_cursor.clone();
    cursor.point = AlacPoint::default();
    cursor.input_needs_wrap = false;
    saved_cursor.point = AlacPoint::default();
    saved_cursor.input_needs_wrap = false;

    term.grid_mut().reset();
    // The inactive grid is private. Swapping twice restores the original
    // screen and its keyboard mode stack, and marks the terminal fully damaged.
    term.swap_alt();
    let mut inactive_cursor = term.grid().cursor.clone();
    let mut inactive_saved_cursor = term.grid().saved_cursor.clone();
    inactive_cursor.point = AlacPoint::default();
    inactive_cursor.input_needs_wrap = false;
    inactive_saved_cursor.point = AlacPoint::default();
    inactive_saved_cursor.input_needs_wrap = false;
    term.grid_mut().reset();
    term.grid_mut().cursor = inactive_cursor;
    term.grid_mut().saved_cursor = inactive_saved_cursor;
    term.swap_alt();

    // Entering the alternate screen copies the primary cursor, so restore the
    // active screen's rendition and character sets after the swaps.
    term.grid_mut().cursor = cursor;
    term.grid_mut().saved_cursor = saved_cursor;
    term.vi_mode_cursor.point = AlacPoint::default();
}

fn pty_child_pid(pty: &tty::Pty) -> Option<u32> {
    #[cfg(not(windows))]
    {
        Some(pty.child().id())
    }
    #[cfg(windows)]
    {
        pty.child_watcher().pid().map(std::num::NonZeroU32::get)
    }
}

// ---------------------------------------------------------------------------
// Content snapshot
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TerminalContent {
    pub cells: Vec<IndexedCell>,
    pub mode: TerminalModes,
    pub display_offset: usize,
    pub history_size: usize,
    pub cursor: TerminalCursor,
    pub cursor_char: char,
    pub terminal_bounds: AlacTerminalBounds,
}

#[derive(Clone, Debug)]
pub struct IndexedCell {
    pub point: AlacPoint,
    pub cell: AlacCell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct TerminalModes(u32);

impl TerminalModes {
    pub const APP_CURSOR: Self = Self(1 << 0);
    pub const APP_KEYPAD: Self = Self(1 << 1);
    pub const BRACKETED_PASTE: Self = Self(1 << 2);
    pub const SGR_MOUSE: Self = Self(1 << 3);
    pub const ALT_SCREEN: Self = Self(1 << 4);
    pub const MOUSE_REPORT_CLICK: Self = Self(1 << 5);
    pub const MOUSE_DRAG: Self = Self(1 << 6);
    pub const MOUSE_MOTION: Self = Self(1 << 7);
    pub const FOCUS_IN_OUT: Self = Self(1 << 8);
    pub const ALTERNATE_SCROLL: Self = Self(1 << 9);
    pub const SHOW_CURSOR: Self = Self(1 << 10);

    pub const MOUSE_MODE: Self =
        Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_DRAG.0 | Self::MOUSE_MOTION.0);

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn intersects(self, other: Self) -> bool {
        (self.0 & other.0) != 0
    }

    pub fn mouse_mode(self) -> bool {
        self.intersects(Self::MOUSE_MODE)
    }
}

impl std::ops::BitOr for TerminalModes {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl TerminalModes {
    fn from_term_mode(mode: TermMode) -> Self {
        let mut m = Self(0);
        if mode.contains(TermMode::APP_CURSOR) {
            m.0 |= Self::APP_CURSOR.0;
        }
        if mode.contains(TermMode::APP_KEYPAD) {
            m.0 |= Self::APP_KEYPAD.0;
        }
        if mode.contains(TermMode::BRACKETED_PASTE) {
            m.0 |= Self::BRACKETED_PASTE.0;
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            m.0 |= Self::SGR_MOUSE.0;
        }
        if mode.contains(TermMode::ALT_SCREEN) {
            m.0 |= Self::ALT_SCREEN.0;
        }
        if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            m.0 |= Self::MOUSE_REPORT_CLICK.0;
        }
        if mode.contains(TermMode::MOUSE_DRAG) {
            m.0 |= Self::MOUSE_DRAG.0;
        }
        if mode.contains(TermMode::MOUSE_MOTION) {
            m.0 |= Self::MOUSE_MOTION.0;
        }
        if mode.contains(TermMode::FOCUS_IN_OUT) {
            m.0 |= Self::FOCUS_IN_OUT.0;
        }
        if mode.contains(TermMode::ALTERNATE_SCROLL) {
            m.0 |= Self::ALTERNATE_SCROLL.0;
        }
        if mode.contains(TermMode::SHOW_CURSOR) {
            m.0 |= Self::SHOW_CURSOR.0;
        }
        m
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalCursor {
    pub point: AlacPoint,
    pub shape: TerminalCursorShape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCursorShape {
    Block,
    Underline,
    Beam,
    Hollow,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub struct AlacTerminalBounds {
    pub columns: usize,
    pub screen_lines: usize,
}

impl AlacTerminalBounds {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

// ---------------------------------------------------------------------------
// Cell / Color conversion
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TerminalCellStyle {
    pub fg: gpui::Rgba,
    pub bg: Option<gpui::Rgba>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl TerminalCellStyle {
    fn foreground(cell: &AlacCell, palette: &TerminalAnsiPalette) -> gpui::Rgba {
        color_to_rgba(cell.fg, palette, palette.foreground)
    }
}

pub(super) fn alacritty_cell_style(
    cell: &AlacCell,
    palette: &TerminalAnsiPalette,
) -> TerminalCellStyle {
    let mut fg = TerminalCellStyle::foreground(cell, palette);
    let mut bg = color_to_rgba(cell.bg, palette, palette.background);
    let flags = cell.flags;

    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }

    if flags.contains(Flags::DIM) {
        fg.alpha *= 0.7;
    }

    // Determine default-ness from the *effective* background, i.e. after the
    // inverse swap. For a reverse-video cell on default colors (e.g. the
    // PowerShell update banner) the visible background becomes the default
    // foreground, which must still be painted — testing the original `cell.bg`
    // would wrongly drop it and render dark-on-dark.
    let effective_bg_color = if flags.contains(Flags::INVERSE) {
        cell.fg
    } else {
        cell.bg
    };
    let is_default_bg = effective_bg_color
        == alacritty_terminal::vte::ansi::Color::Named(
            alacritty_terminal::vte::ansi::NamedColor::Background,
        );

    TerminalCellStyle {
        fg,
        bg: if is_default_bg { None } else { Some(bg) },
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline: flags.contains(Flags::UNDERLINE),
    }
}

fn color_to_rgba(
    color: alacritty_terminal::vte::ansi::Color,
    palette: &TerminalAnsiPalette,
    _default_val: gpui::Rgba,
) -> gpui::Rgba {
    use alacritty_terminal::vte::ansi::Color;
    use alacritty_terminal::vte::ansi::NamedColor;
    match color {
        Color::Named(name) => match name {
            NamedColor::Black => palette.black,
            NamedColor::Red => palette.red,
            NamedColor::Green => palette.green,
            NamedColor::Yellow => palette.yellow,
            NamedColor::Blue => palette.blue,
            NamedColor::Magenta => palette.magenta,
            NamedColor::Cyan => palette.cyan,
            NamedColor::White => palette.white,
            NamedColor::BrightBlack => palette.bright_black,
            NamedColor::BrightRed => palette.bright_red,
            NamedColor::BrightGreen => palette.bright_green,
            NamedColor::BrightYellow => palette.bright_yellow,
            NamedColor::BrightBlue => palette.bright_blue,
            NamedColor::BrightMagenta => palette.bright_magenta,
            NamedColor::BrightCyan => palette.bright_cyan,
            NamedColor::BrightWhite => palette.bright_white,
            NamedColor::DimBlack => palette.dim_black,
            NamedColor::DimRed => palette.dim_red,
            NamedColor::DimGreen => palette.dim_green,
            NamedColor::DimYellow => palette.dim_yellow,
            NamedColor::DimBlue => palette.dim_blue,
            NamedColor::DimMagenta => palette.dim_magenta,
            NamedColor::DimCyan => palette.dim_cyan,
            NamedColor::DimWhite => palette.dim_white,
            NamedColor::Foreground => palette.foreground,
            NamedColor::Background => palette.background,
            NamedColor::Cursor => palette.foreground,
            NamedColor::BrightForeground => palette.foreground,
            NamedColor::DimForeground => palette.foreground,
        },
        Color::Spec(rgb) => gpui::Rgba::new(
            rgb.r as f32 / 255.0,
            rgb.g as f32 / 255.0,
            rgb.b as f32 / 255.0,
            1.0,
        ),
        Color::Indexed(i) => get_color_at_index(i as usize, palette),
    }
}

fn get_color_at_index(index: usize, palette: &TerminalAnsiPalette) -> gpui::Rgba {
    match index {
        0..=15 => match index {
            0 => palette.black,
            1 => palette.red,
            2 => palette.green,
            3 => palette.yellow,
            4 => palette.blue,
            5 => palette.magenta,
            6 => palette.cyan,
            7 => palette.white,
            8 => palette.bright_black,
            9 => palette.bright_red,
            10 => palette.bright_green,
            11 => palette.bright_yellow,
            12 => palette.bright_blue,
            13 => palette.bright_magenta,
            14 => palette.bright_cyan,
            15 => palette.bright_white,
            _ => unreachable!(),
        },
        16..=231 => {
            let (r, g, b) = rgb_for_index((index - 16) as u8);
            let cube_level = |c: u8| {
                if c == 0 {
                    0.0
                } else {
                    (c * 40 + 55) as f32 / 255.0
                }
            };
            gpui::Rgba::new(cube_level(r), cube_level(g), cube_level(b), 1.0)
        }
        232..=255 => {
            let i = (index - 232) as u8;
            let v = (i as f32 * 10.0 + 8.0) / 255.0;
            gpui::Rgba::new(v, v, v, 1.0)
        }
        256 => palette.foreground,
        257 => palette.background,
        _ => palette.black,
    }
}

fn rgb_for_index(i: u8) -> (u8, u8, u8) {
    let r = (i - i % 36) / 36;
    let g = ((i % 36) - (i % 6)) / 6;
    let b = (i % 36) % 6;
    (r, g, b)
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

pub fn terminal_default_background(theme: AppTheme) -> gpui::Rgba {
    let palette = TerminalAnsiPalette::from_theme(theme);
    palette.background
}

pub fn terminal_default_foreground(theme: AppTheme) -> gpui::Rgba {
    let palette = TerminalAnsiPalette::from_theme(theme);
    palette.foreground
}

// ---------------------------------------------------------------------------
// Build content snapshot from Alacritty Term
// ---------------------------------------------------------------------------

/// Read the *current* terminal modes straight from the live `Term`, bypassing the
/// painted [`TerminalContent`] snapshot. Mouse-mode forwarding must consult the live
/// mode at the instant of the click so a TUI that just enabled mouse reporting is
/// honoured immediately, independent of paint timing.
pub(super) fn terminal_live_modes(term: &Term<GitCometListener>) -> TerminalModes {
    TerminalModes::from_term_mode(*term.mode())
}

pub(super) fn make_terminal_content(term: &Term<GitCometListener>) -> TerminalContent {
    let content = term.renderable_content();

    let cells: Vec<IndexedCell> = content
        .display_iter
        .map(|ic| IndexedCell {
            point: ic.point,
            cell: ic.cell.clone(),
        })
        .collect();

    let mode = TerminalModes::from_term_mode(content.mode);
    let cursor = content.cursor;
    let cursor_point = cursor.point;
    let cursor_char = term.grid()[cursor_point].c;

    let columns = term.columns();
    let screen_lines = term.screen_lines();

    TerminalContent {
        cells,
        mode,
        display_offset: content.display_offset,
        history_size: term.grid().history_size(),
        cursor: TerminalCursor {
            point: cursor_point,
            shape: terminal_cursor_shape(content.cursor.shape),
        },
        cursor_char,
        terminal_bounds: AlacTerminalBounds::new(columns, screen_lines),
    }
}

// ---------------------------------------------------------------------------
// Alt screen scroll bytes
// ---------------------------------------------------------------------------

pub(super) fn terminal_alt_screen_scroll_bytes(
    delta_y: Pixels,
    step_rows: usize,
    app_cursor: bool,
) -> Vec<u8> {
    let delta_gt_zero = delta_y > px(0.0);
    let sequence: &[u8] = if delta_gt_zero {
        if app_cursor { b"\x1bOA" } else { b"\x1b[A" }
    } else if app_cursor {
        b"\x1bOB"
    } else {
        b"\x1b[B"
    };
    let repeats = step_rows.clamp(1, TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS);
    let mut bytes = Vec::with_capacity(sequence.len() * repeats);
    for _ in 0..repeats {
        bytes.extend_from_slice(sequence);
    }
    bytes
}

// ---------------------------------------------------------------------------
// Key encoding
// ---------------------------------------------------------------------------

pub(super) fn encode_alacritty_key_input(
    keystroke: &gpui::Keystroke,
    app_cursor: bool,
    option_as_meta: bool,
) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    let mods = keystroke.modifiers;

    if mods.platform || mods.function {
        return None;
    }

    // Alt-as-Meta is intentionally NOT handled up front: special keys (arrows,
    // home/end, function keys) must flow through the `modifier_code` block below
    // so `Alt+Left` becomes `ESC[1;3D`, and character keys are Meta-prefixed by
    // the `ESC`+key fallback at the end (`Alt+f` -> `ESC f`). Handling Alt here
    // first — and routing letters through `encode_control_key` — produced
    // `ESC Ctrl-F` for `Alt+f` and swallowed `Alt`+special-key combos entirely.

    if mods.shift && !mods.control && !mods.alt {
        match key {
            "tab" => return Some(b"\x1b[Z".to_vec()),
            "up" => return Some(b"\x1b[1;2A".to_vec()),
            "down" => return Some(b"\x1b[1;2B".to_vec()),
            "right" => return Some(b"\x1b[1;2C".to_vec()),
            "left" => return Some(b"\x1b[1;2D".to_vec()),
            _ => {}
        }
    }

    let modifier_code = terminal_modifier_code(keystroke);
    if modifier_code > 1 {
        let modified = match key {
            "up" => Some(format!("\x1b[1;{modifier_code}A")),
            "down" => Some(format!("\x1b[1;{modifier_code}B")),
            "right" => Some(format!("\x1b[1;{modifier_code}C")),
            "left" => Some(format!("\x1b[1;{modifier_code}D")),
            "home" => Some(format!("\x1b[1;{modifier_code}H")),
            "end" => Some(format!("\x1b[1;{modifier_code}F")),
            "insert" => Some(format!("\x1b[2;{modifier_code}~")),
            "delete" => Some(format!("\x1b[3;{modifier_code}~")),
            "pageup" => Some(format!("\x1b[5;{modifier_code}~")),
            "pagedown" => Some(format!("\x1b[6;{modifier_code}~")),
            "f1" => Some(format!("\x1b[1;{modifier_code}P")),
            "f2" => Some(format!("\x1b[1;{modifier_code}Q")),
            "f3" => Some(format!("\x1b[1;{modifier_code}R")),
            "f4" => Some(format!("\x1b[1;{modifier_code}S")),
            "f5" => Some(format!("\x1b[15;{modifier_code}~")),
            "f6" => Some(format!("\x1b[17;{modifier_code}~")),
            "f7" => Some(format!("\x1b[18;{modifier_code}~")),
            "f8" => Some(format!("\x1b[19;{modifier_code}~")),
            "f9" => Some(format!("\x1b[20;{modifier_code}~")),
            "f10" => Some(format!("\x1b[21;{modifier_code}~")),
            "f11" => Some(format!("\x1b[23;{modifier_code}~")),
            "f12" => Some(format!("\x1b[24;{modifier_code}~")),
            _ => None,
        };
        if let Some(bytes) = modified {
            return Some(bytes.into_bytes());
        }
    }

    if mods.control
        && !mods.alt
        && let Some(control) = encode_control_key(key)
    {
        return Some(vec![control]);
    }

    // Meta (Alt) + editing keys that have no CSI-with-modifier form: prefix ESC.
    if mods.alt && !mods.control && option_as_meta {
        match key {
            "backspace" => return Some(vec![0x1b, 0x7f]),
            "enter" => return Some(vec![0x1b, b'\r']),
            _ => {}
        }
    }

    match key {
        "enter" => Some(vec![b'\r']),
        "tab" => Some(vec![b'\t']),
        "space" => Some(vec![b' ']),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "insert" => Some(b"\x1b[2~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "home" => {
            if app_cursor {
                Some(b"\x1bOH".to_vec())
            } else {
                Some(b"\x1b[H".to_vec())
            }
        }
        "end" => {
            if app_cursor {
                Some(b"\x1bOF".to_vec())
            } else {
                Some(b"\x1b[F".to_vec())
            }
        }
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "up" => {
            if app_cursor {
                Some(b"\x1bOA".to_vec())
            } else {
                Some(b"\x1b[A".to_vec())
            }
        }
        "down" => {
            if app_cursor {
                Some(b"\x1bOB".to_vec())
            } else {
                Some(b"\x1b[B".to_vec())
            }
        }
        "right" => {
            if app_cursor {
                Some(b"\x1bOC".to_vec())
            } else {
                Some(b"\x1b[C".to_vec())
            }
        }
        "left" => {
            if app_cursor {
                Some(b"\x1bOD".to_vec())
            } else {
                Some(b"\x1b[D".to_vec())
            }
        }
        "f1" => Some(b"\x1bOP".to_vec()),
        "f2" => Some(b"\x1bOQ".to_vec()),
        "f3" => Some(b"\x1bOR".to_vec()),
        "f4" => Some(b"\x1bOS".to_vec()),
        "f5" => Some(b"\x1b[15~".to_vec()),
        "f6" => Some(b"\x1b[17~".to_vec()),
        "f7" => Some(b"\x1b[18~".to_vec()),
        "f8" => Some(b"\x1b[19~".to_vec()),
        "f9" => Some(b"\x1b[20~".to_vec()),
        "f10" => Some(b"\x1b[21~".to_vec()),
        "f11" => Some(b"\x1b[23~".to_vec()),
        "f12" => Some(b"\x1b[24~".to_vec()),
        _ if mods.alt && !mods.control && option_as_meta && key.len() == 1 => {
            let base = key.as_bytes();
            let mut bytes = Vec::with_capacity(1 + base.len());
            bytes.push(0x1b);
            bytes.extend_from_slice(base);
            Some(bytes)
        }
        _ if key.len() == 1 && !mods.control && !mods.alt => {
            let ch = keystroke.key_char.as_deref().unwrap_or(key);
            Some(ch.as_bytes().to_vec())
        }
        _ => None,
    }
}

pub(super) fn encode_control_key(key: &str) -> Option<u8> {
    if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if ch.is_ascii_lowercase() {
            return Some(ch - b'a' + 1);
        }
    }
    match key {
        "space" => Some(0),
        "[" => Some(0x1b),
        "\\" => Some(0x1c),
        "]" => Some(0x1d),
        "^" => Some(0x1e),
        "_" => Some(0x1f),
        _ => None,
    }
}

fn terminal_modifier_code(keystroke: &gpui::Keystroke) -> u32 {
    let mut modifier_code = 0;
    if keystroke.modifiers.shift {
        modifier_code |= 1;
    }
    if keystroke.modifiers.alt {
        modifier_code |= 1 << 1;
    }
    if keystroke.modifiers.control {
        modifier_code |= 1 << 2;
    }
    modifier_code + 1
}

fn terminal_cursor_shape(shape: alacritty_terminal::vte::ansi::CursorShape) -> TerminalCursorShape {
    use alacritty_terminal::vte::ansi::CursorShape;

    match shape {
        CursorShape::Block => TerminalCursorShape::Block,
        CursorShape::Underline => TerminalCursorShape::Underline,
        CursorShape::Beam => TerminalCursorShape::Beam,
        CursorShape::HollowBlock => TerminalCursorShape::Hollow,
        CursorShape::Hidden => TerminalCursorShape::Hidden,
    }
}

// ---------------------------------------------------------------------------
// Build terminal row for rendering
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(super) struct TerminalBackgroundRect {
    pub row: i32,
    pub col: i32,
    pub num_cells: usize,
    pub num_rows: usize,
    pub color: gpui::Rgba,
}

pub(super) fn build_alacritty_row(
    cells: &[IndexedCell],
    row: i32,
    cols: usize,
    base_style: &gpui::TextStyle,
    theme: AppTheme,
) -> (SharedString, Vec<TextRun>, Vec<TerminalBackgroundRect>) {
    let palette = TerminalAnsiPalette::from_theme(theme);

    let mut text = String::with_capacity(cols);
    // `runs` holds *merged* style spans (a handful for ordinary output) and
    // `background_rects` only non-default backgrounds, so neither tracks `cols`.
    // `terminal_panel` retains both per cached row, so an over-estimate is held
    // for the lifetime of the render cache rather than just this call.
    let mut runs = Vec::new();
    let mut background_rects: Vec<TerminalBackgroundRect> = Vec::new();
    let mut active_style: Option<TerminalCellStyle> = None;
    let mut active_len = 0usize;

    let mut row_cells = cells.iter().filter(|cell| cell.point.line.0 == row);
    let mut next_cell = row_cells.next();

    let mut col = 0usize;
    let mut prev_had_extras = false;

    while col < cols {
        let cell = if next_cell.is_some_and(|cell| cell.point.column.0 == col) {
            let cell = next_cell.take().expect("matching terminal cell is present");
            next_cell = row_cells.next();
            cell
        } else {
            col += 1;
            continue;
        };

        if cell.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            col += 1;
            continue;
        }

        let ch = cell.cell.c;

        let had_extras = prev_had_extras;
        prev_had_extras = matches!(cell.cell.zerowidth(), Some(chars) if !chars.is_empty());

        let style = alacritty_cell_style(&cell.cell, &palette);

        if let Some(ref bg_color) = style.bg {
            if let Some(last) = background_rects.last_mut()
                && last.row == row
                && last.col + last.num_cells as i32 == col as i32
                && last.color == *bg_color
            {
                last.num_cells += 1;
            } else {
                background_rects.push(TerminalBackgroundRect {
                    row,
                    col: col as i32,
                    num_cells: 1,
                    num_rows: 1,
                    color: *bg_color,
                });
            }
        }

        // Skip spaces that follow cells with extras (emoji variation sequences),
        // but still push a space to maintain grid alignment.
        if ch == ' ' && had_extras {
            text.push(' ');
            if active_style
                .as_ref()
                .is_some_and(|current| *current == style)
            {
                active_len += 1;
            } else {
                if let Some(previous) = active_style.take() {
                    runs.push(terminal_text_run(base_style, &previous, active_len));
                }
                active_style = Some(style.clone());
                active_len = 1;
            }
            col += 1;
            continue;
        }

        // Push cell content to text (always, including spaces for grid alignment)
        if ch == ' ' || ch == '\0' {
            text.push(' ');
        } else {
            text.push(ch);
            if let Some(zw_chars) = cell.cell.zerowidth() {
                for &zc in zw_chars {
                    text.push(zc);
                }
            }
        }

        let pushed_len = if ch == ' ' || ch == '\0' {
            1
        } else {
            ch.len_utf8()
                + cell
                    .cell
                    .zerowidth()
                    .map(|zw| zw.iter().map(|c| c.len_utf8()).sum::<usize>())
                    .unwrap_or(0)
        };

        if active_style
            .as_ref()
            .is_some_and(|current| *current == style)
        {
            active_len += pushed_len;
        } else {
            if let Some(previous) = active_style.take() {
                runs.push(terminal_text_run(base_style, &previous, active_len));
            }
            active_style = Some(style);
            active_len = pushed_len;
        }

        // A WIDE_CHAR occupies two columns, but the second one has its own
        // WIDE_CHAR_SPACER cell. Advancing `col` past it here would leave that
        // cell unconsumed, pinning the cursor behind `col` for the rest of the
        // row; let the spacer branch above skip it instead.
        col += 1;
    }

    if let Some(previous) = active_style.take()
        && active_len > 0
    {
        runs.push(terminal_text_run(base_style, &previous, active_len));
    }

    (text.into(), runs, background_rects)
}

pub(super) fn terminal_text_run(
    base_style: &gpui::TextStyle,
    style: &TerminalCellStyle,
    len: usize,
) -> TextRun {
    let mut text_style = base_style.clone();
    text_style.color = style.fg.into_color();
    if style.bold {
        text_style.font_weight = gpui::FontWeight::BOLD;
    }
    if style.italic {
        text_style.font_style = gpui::FontStyle::Italic;
    }
    if style.underline {
        text_style.underline = Some(gpui::UnderlineStyle {
            thickness: px(1.0),
            color: Some(style.fg.into_color()),
            ..Default::default()
        });
    }
    TextRun {
        len,
        font: text_style.font(),
        color: text_style.color,
        background_color: None,
        underline: text_style.underline,
        strikethrough: None,
        letter_spacing: text_style.letter_spacing,
    }
}

// ---------------------------------------------------------------------------
// Bracketed paste sanitization
// ---------------------------------------------------------------------------

pub(super) fn sanitize_bracketed_paste(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
        .collect()
}

// ---------------------------------------------------------------------------
// Word selection boundaries
// ---------------------------------------------------------------------------

/// A char is part of a "word" for double-click selection when it is
/// alphanumeric or one of a few path/identifier punctuation characters.
fn is_terminal_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '~')
}

/// Inclusive word boundaries `(left, right)` around `col` within a single grid
/// row's characters, or `None` when the cell at `col` is not a word character.
pub(super) fn terminal_word_bounds(chars: &[char], col: usize) -> Option<(usize, usize)> {
    if col >= chars.len() || !is_terminal_word_char(chars[col]) {
        return None;
    }
    let mut left = col;
    while left > 0 && is_terminal_word_char(chars[left - 1]) {
        left -= 1;
    }
    let mut right = col;
    while right + 1 < chars.len() && is_terminal_word_char(chars[right + 1]) {
        right += 1;
    }
    Some((left, right))
}

// ---------------------------------------------------------------------------
// Mouse encoding (port of Alacritty/Zed mouse report protocol)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum MouseButtonCode {
    LeftButton = 0,
    MiddleButton = 1,
    RightButton = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoneMove = 35,
    ScrollUp = 64,
    ScrollDown = 65,
}

impl MouseButtonCode {
    /// Returns `None` for buttons the mouse protocol does not report (Navigate
    /// back/forward), so they are suppressed rather than reported as left-clicks.
    fn from_button(e: gpui::MouseButton) -> Option<Self> {
        match e {
            gpui::MouseButton::Left => Some(MouseButtonCode::LeftButton),
            gpui::MouseButton::Middle => Some(MouseButtonCode::MiddleButton),
            gpui::MouseButton::Right => Some(MouseButtonCode::RightButton),
            gpui::MouseButton::Navigate(_) => None,
        }
    }

    /// `None` for the held button means "suppress" (Navigate); `Some(NoneMove)`
    /// means no button is held (plain motion).
    fn from_move_button(e: Option<gpui::MouseButton>) -> Option<Self> {
        match e {
            Some(gpui::MouseButton::Left) => Some(MouseButtonCode::LeftMove),
            Some(gpui::MouseButton::Middle) => Some(MouseButtonCode::MiddleMove),
            Some(gpui::MouseButton::Right) => Some(MouseButtonCode::RightMove),
            Some(gpui::MouseButton::Navigate(_)) => None,
            None => Some(MouseButtonCode::NoneMove),
        }
    }

    fn from_scroll(delta_y: Pixels) -> Self {
        if delta_y > px(0.0) {
            MouseButtonCode::ScrollUp
        } else {
            MouseButtonCode::ScrollDown
        }
    }
}

#[derive(Clone, Copy)]
enum MouseFormat {
    Sgr,
    Normal(bool),
}

impl MouseFormat {
    fn from_mode(mode: TerminalModes) -> Self {
        if mode.contains(TerminalModes::SGR_MOUSE) {
            MouseFormat::Sgr
        } else {
            MouseFormat::Normal(false)
        }
    }
}

pub(super) fn terminal_grid_point(
    mouse_pos: gpui::Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    display_offset: usize,
    cols: u16,
) -> Option<(i32, usize)> {
    let rel_x = mouse_pos.x - bounds.left();
    let rel_y = mouse_pos.y - bounds.top();
    if rel_x < px(0.0) || rel_y < px(0.0) {
        return None;
    }
    let col = ((rel_x / cell_width).floor() as usize).min((cols as usize).saturating_sub(1));
    let row = (rel_y / line_height).floor() as i32;
    let grid_row = row - display_offset as i32;
    Some((grid_row, col))
}

/// Whether `row` soft-wraps into the next one, i.e. the two are halves of a
/// single logical line. Alacritty flags the last cell of a row that ran out of
/// columns. Copying must not insert a newline there, or a wrapped command
/// pastes back as two lines and a shell runs only the truncated first half.
pub(super) fn terminal_row_wraps(
    row: &alacritty_terminal::grid::Row<AlacCell>,
    cols: usize,
) -> bool {
    cols > 0
        && row[alacritty_terminal::index::Column(cols - 1)]
            .flags
            .contains(Flags::WRAPLINE)
}

/// Clamps a grid row into the range alacritty can actually address:
/// `-(history_size) ..= screen_lines - 1`. Indexing a `Grid` outside that range
/// is only `debug_assert`ed upstream, so in release it panics on the backing
/// slice or silently reads a wrong row.
pub(super) fn terminal_clamp_grid_row(row: i32, history_size: usize, screen_lines: usize) -> i32 {
    let top = -(history_size as i32);
    let bottom = screen_lines.saturating_sub(1) as i32;
    row.clamp(top, bottom.max(top))
}

/// Resolves a mouse position to a grid cell for *text selection*.
///
/// Unlike [`terminal_grid_point`] (which mouse reporting relies on returning
/// `None` outside the viewport) this never fails: the position is first clamped
/// into `bounds`, so a drag above or left of the terminal resolves to the edge
/// cell instead of freezing the selection. The row is then clamped into the
/// addressable buffer range so scrollback rows stay representable as negatives.
#[allow(clippy::too_many_arguments)]
pub(super) fn terminal_selection_grid_point(
    mouse_pos: gpui::Point<Pixels>,
    bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    display_offset: usize,
    history_size: usize,
    columns: usize,
    screen_lines: usize,
) -> Option<TerminalGridPoint> {
    if columns == 0 || screen_lines == 0 || cell_width <= px(0.0) || line_height <= px(0.0) {
        return None;
    }
    let x = mouse_pos.x.clamp(bounds.left(), bounds.right());
    let y = mouse_pos.y.clamp(bounds.top(), bounds.bottom());
    let rel_x: f32 = (x - bounds.left()).into();
    let rel_y: f32 = (y - bounds.top()).into();
    let cw: f32 = cell_width.into();
    let lh: f32 = line_height.into();
    // Sitting exactly on the right/bottom edge floors to one past the last
    // cell, hence the two `min`s: a drag past the edge must resolve to the last
    // *visible* row, not to the row below it.
    let col = ((rel_x / cw).floor().max(0.0) as usize).min(columns - 1);
    let screen_row = ((rel_y / lh).floor().max(0.0) as usize).min(screen_lines - 1);
    let grid_row = terminal_clamp_grid_row(
        screen_row as i32 - display_offset as i32,
        history_size,
        screen_lines,
    );
    Some(TerminalGridPoint::new(grid_row, col as u16))
}

/// Lines to scroll per autoscroll tick while a selection drag is outside the
/// viewport vertically. Positive scrolls into history (matching
/// `Scroll::Delta`'s sign), negative scrolls toward the live tail, zero means
/// the pointer is still inside.
pub(super) fn terminal_autoscroll_lines(
    mouse_y: Pixels,
    top: Pixels,
    bottom: Pixels,
    line_height: Pixels,
) -> i32 {
    const MAX_LINES: f32 = 6.0;
    if line_height <= px(0.0) {
        return 0;
    }
    let lh: f32 = line_height.into();
    let ramp = |distance: Pixels| -> i32 {
        let d: f32 = distance.into();
        (d / lh).ceil().clamp(1.0, MAX_LINES) as i32
    };
    if mouse_y < top {
        ramp(top - mouse_y)
    } else if mouse_y > bottom {
        -ramp(mouse_y - bottom)
    } else {
        0
    }
}

/// The portion of a selection's grid-row span that is currently on screen, or
/// `None` when the whole selection is scrolled out of view. Painting iterates
/// this instead of the raw span so a scrollback-wide selection costs O(screen)
/// rather than O(`TERMINAL_SCROLLBACK_ROWS`) per frame.
pub(super) fn terminal_selection_visible_rows(
    start_row: i32,
    end_row: i32,
    display_offset: usize,
    screen_lines: usize,
) -> Option<std::ops::RangeInclusive<i32>> {
    if screen_lines == 0 {
        return None;
    }
    let (lo, hi) = if start_row <= end_row {
        (start_row, end_row)
    } else {
        (end_row, start_row)
    };
    // A grid row is visible when `row + display_offset` lands in `0..screen_lines`.
    let offset = display_offset as i32;
    let visible_top = -offset;
    let visible_bottom = screen_lines as i32 - 1 - offset;
    let first = lo.max(visible_top);
    let last = hi.min(visible_bottom);
    (first <= last).then_some(first..=last)
}

pub(super) fn terminal_mouse_button_report(
    grid_row: i32,
    grid_col: usize,
    button: gpui::MouseButton,
    modifiers: gpui::Modifiers,
    pressed: bool,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let code = MouseButtonCode::from_button(button)?;
    mouse_report(
        grid_row,
        grid_col,
        code,
        pressed,
        modifiers,
        MouseFormat::from_mode(mode),
    )
}

pub(super) fn terminal_mouse_moved_report(
    grid_row: i32,
    grid_col: usize,
    held_button: Option<gpui::MouseButton>,
    modifiers: gpui::Modifiers,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    if !mode.intersects(TerminalModes::MOUSE_MOTION | TerminalModes::MOUSE_DRAG) {
        return None;
    }
    let code = MouseButtonCode::from_move_button(held_button)?;
    if mode.contains(TerminalModes::MOUSE_DRAG) && matches!(code, MouseButtonCode::NoneMove) {
        return None;
    }
    mouse_report(
        grid_row,
        grid_col,
        code,
        true,
        modifiers,
        MouseFormat::from_mode(mode),
    )
}

pub(super) fn terminal_scroll_report(
    grid_row: i32,
    grid_col: usize,
    modifiers: gpui::Modifiers,
    delta_y: Pixels,
    step_rows: usize,
    mode: TerminalModes,
) -> Vec<Vec<u8>> {
    let code = MouseButtonCode::from_scroll(delta_y);
    let mut reports = Vec::with_capacity(step_rows);
    if let Some(report) = mouse_report(
        grid_row,
        grid_col,
        code,
        true,
        modifiers,
        MouseFormat::from_mode(mode),
    ) {
        for _ in 0..step_rows {
            reports.push(report.clone());
        }
    }
    reports
}

fn mouse_report(
    grid_row: i32,
    grid_col: usize,
    button: MouseButtonCode,
    pressed: bool,
    modifiers: gpui::Modifiers,
    format: MouseFormat,
) -> Option<Vec<u8>> {
    if grid_row < 0 {
        return None;
    }
    let mut mods: u8 = 0;
    if modifiers.shift {
        mods += 4;
    }
    if modifiers.alt {
        mods += 8;
    }
    if modifiers.control {
        mods += 16;
    }
    match format {
        MouseFormat::Sgr => {
            let c = if pressed { 'M' } else { 'm' };
            Some(
                format!(
                    "\x1b[<{};{};{}{}",
                    button as u8 + mods,
                    grid_col + 1,
                    grid_row + 1,
                    c
                )
                .into_bytes(),
            )
        }
        MouseFormat::Normal(_utf8) => {
            if pressed {
                normal_mouse_report(grid_row, grid_col, button as u8 + mods)
            } else {
                normal_mouse_report(grid_row, grid_col, 3 + mods)
            }
        }
    }
}

fn normal_mouse_report(grid_row: i32, grid_col: usize, button: u8) -> Option<Vec<u8>> {
    let max_point = 223;
    if grid_row >= max_point || grid_col >= max_point as usize {
        return None;
    }
    let mut msg = vec![b'\x1b', b'[', b'M', 32 + button];
    msg.push(32 + 1 + grid_col as u8);
    msg.push(32 + 1 + grid_row as u8);
    Some(msg)
}

pub(super) fn terminal_mouse_event_at(
    position: gpui::Point<Pixels>,
    viewport_bounds: Option<Bounds<Pixels>>,
    layout_cache: &Option<TerminalLayoutCache>,
    last_content: &Option<TerminalContent>,
    button: gpui::MouseButton,
    modifiers: gpui::Modifiers,
    pressed: bool,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let bounds = viewport_bounds?;
    let cache = layout_cache.as_ref()?;
    let content = last_content.as_ref()?;
    if !mode.mouse_mode() {
        return None;
    }
    let (grid_row, grid_col) = terminal_grid_point(
        position,
        bounds,
        cache.metrics.cell_width,
        cache.metrics.line_height,
        content.display_offset,
        content.terminal_bounds.columns as u16,
    )?;
    terminal_mouse_button_report(grid_row, grid_col, button, modifiers, pressed, mode)
}

pub(super) fn terminal_mouse_moved_report_at(
    position: gpui::Point<Pixels>,
    viewport_bounds: Option<Bounds<Pixels>>,
    layout_cache: &Option<TerminalLayoutCache>,
    last_content: &Option<TerminalContent>,
    held_button: Option<gpui::MouseButton>,
    modifiers: gpui::Modifiers,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let bounds = viewport_bounds?;
    let cache = layout_cache.as_ref()?;
    let content = last_content.as_ref()?;
    let (grid_row, grid_col) = terminal_grid_point(
        position,
        bounds,
        cache.metrics.cell_width,
        cache.metrics.line_height,
        content.display_offset,
        content.terminal_bounds.columns as u16,
    )?;
    terminal_mouse_moved_report(grid_row, grid_col, held_button, modifiers, mode)
}

// ---------------------------------------------------------------------------
// Background rect merging across rows
// ---------------------------------------------------------------------------

pub(super) fn merge_background_rects(
    rects: &[TerminalBackgroundRect],
) -> Vec<TerminalBackgroundRect> {
    if rects.is_empty() {
        return Vec::new();
    }
    let mut merged: Vec<TerminalBackgroundRect> = rects.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < merged.len() {
            let mut j = i + 1;
            while j < merged.len() {
                let can_merge = merged[i].color == merged[j].color
                    && merged[i].col == merged[j].col
                    && merged[i].num_cells == merged[j].num_cells
                    && merged[i].row + merged[i].num_rows as i32 == merged[j].row;
                if can_merge {
                    merged[i].num_rows += merged[j].num_rows;
                    merged.swap_remove(j);
                    changed = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
    }
    merged
}

// ---------------------------------------------------------------------------
// IME State
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(super) struct TerminalImeState {
    pub marked_text: String,
}

// ---------------------------------------------------------------------------
// IME InputHandler
// ---------------------------------------------------------------------------

pub(super) struct TerminalTextInputHandler {
    pub(super) focus_handle: gpui::FocusHandle,
    pub(super) pty_sender: Option<PtySender>,
    pub(super) ime_state: Option<TerminalImeState>,
}

impl gpui::InputHandler for TerminalTextInputHandler {
    fn accepts_text_input(&mut self, window: &mut Window, _cx: &mut App) -> bool {
        gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window)
            && self.pty_sender.is_some()
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<gpui::UTF16Selection> {
        Some(gpui::UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<std::ops::Range<usize>> {
        self.ime_state
            .as_ref()
            .filter(|s| !s.marked_text.is_empty())
            .map(|s| {
                let len = s.marked_text.encode_utf16().count();
                0..len
            })
    }

    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if !gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window) {
            return;
        }
        if let Some(ref pty) = self.pty_sender {
            self.ime_state = None;
            pty.write(text.as_bytes().to_vec());
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if !gitcomet_ui_kit::window_focus::is_active(&self.focus_handle, window) {
            return;
        }
        self.ime_state = Some(TerminalImeState {
            marked_text: new_text.to_string(),
        });
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut App) {
        self.ime_state = None;
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<usize> {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
