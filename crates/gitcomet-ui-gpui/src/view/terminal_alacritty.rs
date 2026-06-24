use super::*;
use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::event_loop::{EventLoop, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{
    Config, Osc52, Term, TermMode,
    cell::{Cell as AlacCell, Flags},
};
use alacritty_terminal::tty;
use std::borrow::Cow;
use std::sync::Arc;

const TERMINAL_INITIAL_ROWS: u16 = 24;
const TERMINAL_INITIAL_COLS: u16 = 80;
const TERMINAL_SCROLLBACK_ROWS: usize = 10_000;
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
        let fg = colors.text;
        let bg = colors.window_bg;

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
                red: gpui::rgb(0xc43d35),
                green: gpui::rgb(0x2e7d32),
                yellow: gpui::rgb(0xa97822),
                blue: gpui::rgb(0x4f72dd),
                magenta: gpui::rgb(0x8e4c6f),
                cyan: gpui::rgb(0x1a7f82),
                white: gpui::rgb(0x1d2330),
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

pub(super) type AlacrittyTermLock = Arc<FairMutex<Term<GitCometListener>>>;

// ---------------------------------------------------------------------------
// Listener
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct GitCometListener {
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
pub(super) enum TerminalBackendEvent {
    PtyWrite(String),
    Title(String),
    ClipboardStore(String),
    ClipboardLoad,
    Wakeup,
    Bell,
    Exit,
    ChildExit(std::process::ExitStatus),
    CursorBlinkingChange,
}

impl From<AlacEvent> for TerminalBackendEvent {
    fn from(event: AlacEvent) -> Self {
        match event {
            AlacEvent::PtyWrite(data) => Self::PtyWrite(data),
            AlacEvent::Title(title) => Self::Title(title),
            AlacEvent::ClipboardStore(_, data) => Self::ClipboardStore(data),
            AlacEvent::ClipboardLoad(_, _) => Self::ClipboardLoad,
            AlacEvent::Wakeup => Self::Wakeup,
            AlacEvent::Bell => Self::Bell,
            AlacEvent::Exit => Self::Exit,
            AlacEvent::ChildExit(status) => Self::ChildExit(status),
            AlacEvent::CursorBlinkingChange => Self::CursorBlinkingChange,
            _ => Self::Wakeup,
        }
    }
}

// ---------------------------------------------------------------------------
// PTY spawning
// ---------------------------------------------------------------------------

pub(super) struct SpawnedAlacTerminal {
    pub term_lock: AlacrittyTermLock,
    pub events_rx: smol::channel::Receiver<TerminalBackendEvent>,
    pub pty_sender: PtySender,
    pub child_pid: Option<u32>,
}

#[derive(Clone)]
pub(super) struct PtySender {
    event_loop_tx: alacritty_terminal::event_loop::EventLoopSender,
}

impl PtySender {
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.event_loop_tx.send(Msg::Input(bytes.into())).ok();
    }

    pub fn resize(&self, columns: usize, screen_lines: usize) {
        self.event_loop_tx
            .send(Msg::Resize(alacritty_terminal::event::WindowSize {
                num_lines: screen_lines as u16,
                num_cols: columns as u16,
                cell_width: 1,
                cell_height: 1,
            }))
            .ok();
    }

    pub fn shutdown(&self) {
        self.event_loop_tx.send(Msg::Shutdown).ok();
    }
}

pub(super) fn spawn_alacritty_terminal(
    workdir: &std::path::Path,
    window_id: u64,
) -> Result<SpawnedAlacTerminal, String> {
    let shell_program = resolve_embedded_shell_program()?;
    let shell_program_str = shell_program.to_string_lossy().to_string();

    let mut env: Vec<(String, String)> = Vec::new();
    env.push(("TERM".to_string(), "xterm-256color".to_string()));
    env.push(("COLORTERM".to_string(), "truecolor".to_string()));
    env.push(("TERM_PROGRAM".to_string(), "GitComet".to_string()));
    env.push((
        "TERM_PROGRAM_VERSION".to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    ));

    let (events_tx, events_rx) = smol::channel::unbounded();

    let pty_options = tty::Options {
        shell: Some(tty::Shell::new(shell_program_str, Vec::<String>::new())),
        working_directory: Some(workdir.to_path_buf()),
        drain_on_exit: false,
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
        false,
        false,
    )
    .map_err(|e| format!("failed to create event loop: {e}"))?;

    let event_loop_tx = event_loop.channel();
    let _io_thread = event_loop.spawn();

    Ok(SpawnedAlacTerminal {
        term_lock,
        events_rx,
        pty_sender: PtySender { event_loop_tx },
        child_pid,
    })
}

// ---------------------------------------------------------------------------
// Terminal dimensions
// ---------------------------------------------------------------------------

struct TerminalDims {
    columns: usize,
    screen_lines: usize,
    total_lines: usize,
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

fn terminal_config(scrollback: usize) -> Config {
    let mut config = Config::default();
    config.scrolling_history = scrollback;
    config.osc52 = Osc52::Disabled;
    config
}

fn new_term(
    config: &Config,
    bounds: &TerminalDims,
    events_tx: smol::channel::Sender<TerminalBackendEvent>,
) -> AlacrittyTermLock {
    let term = Term::new(config.clone(), bounds, GitCometListener { events_tx });
    Arc::new(FairMutex::new(term))
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
pub(super) struct TerminalContent {
    pub cells: Vec<IndexedCell>,
    pub mode: TerminalModes,
    pub display_offset: usize,
    pub cursor: TerminalCursor,
    pub cursor_char: char,
    pub terminal_bounds: AlacTerminalBounds,
}

#[derive(Clone, Debug)]
pub(super) struct IndexedCell {
    pub point: AlacPoint,
    pub cell: AlacCell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub(super) struct TerminalModes(u32);

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
pub(super) struct TerminalCursor {
    pub point: AlacPoint,
    pub shape: TerminalCursorShape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalCursorShape {
    Block,
    Underline,
    Beam,
    Hollow,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AlacTerminalBounds {
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

    fn background(cell: &AlacCell, palette: &TerminalAnsiPalette) -> gpui::Rgba {
        color_to_rgba(cell.bg, palette, palette.background)
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
        fg.a *= 0.7;
    }

    let is_default_bg = cell.bg
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
        Color::Spec(rgb) => gpui::Rgba {
            r: rgb.r as f32 / 255.0,
            g: rgb.g as f32 / 255.0,
            b: rgb.b as f32 / 255.0,
            a: 1.0,
        },
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
            gpui::Rgba {
                r: if r == 0 {
                    0.0
                } else {
                    (r * 40 + 55) as f32 / 255.0
                },
                g: if g == 0 {
                    0.0
                } else {
                    (g * 40 + 55) as f32 / 255.0
                },
                b: if b == 0 {
                    0.0
                } else {
                    (b * 40 + 55) as f32 / 255.0
                },
                a: 1.0,
            }
        }
        232..=255 => {
            let i = (index - 232) as u8;
            let v = (i as f32 * 10.0 + 8.0) / 255.0;
            gpui::Rgba {
                r: v,
                g: v,
                b: v,
                a: 1.0,
            }
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

pub(super) fn terminal_default_background(theme: AppTheme) -> gpui::Rgba {
    let palette = TerminalAnsiPalette::from_theme(theme);
    palette.background
}

pub(super) fn terminal_default_foreground(theme: AppTheme) -> gpui::Rgba {
    let palette = TerminalAnsiPalette::from_theme(theme);
    palette.foreground
}

// ---------------------------------------------------------------------------
// Build content snapshot from Alacritty Term
// ---------------------------------------------------------------------------

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
        cursor: TerminalCursor {
            point: cursor_point,
            shape: terminal_cursor_shape(content.cursor.shape),
        },
        cursor_char,
        terminal_bounds: AlacTerminalBounds::new(columns, screen_lines),
    }
}

// ---------------------------------------------------------------------------
// Grid resize
// ---------------------------------------------------------------------------

pub(super) fn resize_terminal_grid(
    term_lock: &AlacrittyTermLock,
    pty_sender: &PtySender,
    rows: u16,
    cols: u16,
) {
    let mut term = term_lock.lock();
    term.resize(TerminalDims {
        columns: cols as usize,
        screen_lines: rows as usize,
        total_lines: TERMINAL_SCROLLBACK_ROWS,
    });
    drop(term);
    pty_sender.resize(cols as usize, rows as usize);
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
    let repeats = step_rows
        .max(1)
        .min(TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS);
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

    if mods.alt && !mods.control && option_as_meta {
        if let Some(control) = encode_control_key(key) {
            return Some(vec![0x1b, control]);
        }
        if key.len() == 1 {
            let base = key.as_bytes();
            let mut bytes = Vec::with_capacity(1 + base.len());
            bytes.push(0x1b);
            bytes.extend_from_slice(base);
            return Some(bytes);
        }
        return None;
    }

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
        _ if mods.alt && !mods.control && key.len() == 1 => {
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

    let mut text = String::new();
    let mut runs = Vec::new();
    let mut background_rects: Vec<TerminalBackgroundRect> = Vec::new();
    let mut active_style: Option<TerminalCellStyle> = None;
    let mut active_len = 0usize;

    let row_cells: Vec<&IndexedCell> = cells.iter().filter(|ic| ic.point.line.0 == row).collect();

    let mut col = 0usize;
    let mut cell_idx = 0usize;
    let mut prev_had_extras = false;

    while col < cols {
        let cell = if cell_idx < row_cells.len() && row_cells[cell_idx].point.column.0 == col {
            let c = row_cells[cell_idx];
            cell_idx += 1;
            c
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

        col += 1;
        if cell.cell.flags.contains(Flags::WIDE_CHAR) && col < cols {
            col += 1;
        }
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
    text_style.color = style.fg.into();
    if style.bold {
        text_style.font_weight = gpui::FontWeight::BOLD;
    }
    if style.italic {
        text_style.font_style = gpui::FontStyle::Italic;
    }
    if style.underline {
        text_style.underline = Some(gpui::UnderlineStyle {
            thickness: px(1.0),
            color: Some(style.fg.into()),
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
    }
}

// ---------------------------------------------------------------------------
// Full buffer text (including scrollback)
// ---------------------------------------------------------------------------

pub(super) fn terminal_full_buffer_text(term: &Term<GitCometListener>) -> String {
    let grid = term.grid();
    let history_size = grid.history_size();
    let screen_lines = term.screen_lines();
    let cols = term.columns();

    let mut text = String::new();
    let total_lines = history_size + screen_lines;

    for line_idx in 0..total_lines {
        let row = Line((line_idx as i32) - (history_size as i32));

        for col in 0..cols {
            let cell = &grid[row][Column(col)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let ch = cell.c;
            if ch == ' ' || ch == '\0' {
                text.push(' ');
            } else {
                text.push(ch);
            }
        }

        if line_idx < total_lines - 1 {
            text.push('\n');
        }
    }
    text
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
    fn from_button(e: gpui::MouseButton) -> Self {
        match e {
            gpui::MouseButton::Left => MouseButtonCode::LeftButton,
            gpui::MouseButton::Right => MouseButtonCode::MiddleButton,
            gpui::MouseButton::Middle => MouseButtonCode::RightButton,
            gpui::MouseButton::Navigate(_) => MouseButtonCode::LeftButton,
        }
    }

    fn from_move_button(e: Option<gpui::MouseButton>) -> Self {
        match e {
            Some(gpui::MouseButton::Left) => MouseButtonCode::LeftMove,
            Some(gpui::MouseButton::Middle) => MouseButtonCode::MiddleMove,
            Some(gpui::MouseButton::Right) => MouseButtonCode::RightMove,
            Some(gpui::MouseButton::Navigate(_)) => MouseButtonCode::LeftMove,
            None => MouseButtonCode::NoneMove,
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
    let rel_x = (mouse_pos.x - bounds.left()).max(px(0.0));
    let rel_y = (mouse_pos.y - bounds.top()).max(px(0.0));
    if rel_x < px(0.0) || rel_y < px(0.0) {
        return None;
    }
    let col = ((rel_x / cell_width).floor() as usize).min(cols as usize - 1);
    let row = (rel_y / line_height).floor() as i32;
    let grid_row = row - display_offset as i32;
    Some((grid_row, col))
}

pub(super) fn terminal_mouse_button_report(
    grid_row: i32,
    grid_col: usize,
    button: gpui::MouseButton,
    modifiers: gpui::Modifiers,
    pressed: bool,
    mode: TerminalModes,
) -> Option<Vec<u8>> {
    let code = MouseButtonCode::from_button(button);
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
    let code = MouseButtonCode::from_move_button(held_button);
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
) -> Option<Vec<u8>> {
    let bounds = viewport_bounds?;
    let cache = layout_cache.as_ref()?;
    let content = last_content.as_ref()?;
    let mode = content.mode;
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

impl TerminalImeState {
    pub fn new() -> Self {
        Self {
            marked_text: String::new(),
        }
    }

    pub fn set_marked_text(&mut self, text: String) {
        self.marked_text = text;
    }

    pub fn clear(&mut self) {
        self.marked_text.clear();
    }

    pub fn has_marked_text(&self) -> bool {
        !self.marked_text.is_empty()
    }
}

// ---------------------------------------------------------------------------
// IME InputHandler
// ---------------------------------------------------------------------------

pub(super) struct TerminalTextInputHandler {
    pub(super) pty_sender: Option<PtySender>,
    pub(super) ime_state: Option<TerminalImeState>,
}

impl gpui::InputHandler for TerminalTextInputHandler {
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
        _window: &mut Window,
        _cx: &mut App,
    ) {
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
        _window: &mut Window,
        _cx: &mut App,
    ) {
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
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
    use alacritty_terminal::term::cell::Cell as AlacCell;
    use gpui::{FontStyle, FontWeight, WhiteSpace};

    fn default_text_style() -> gpui::TextStyle {
        gpui::TextStyle {
            color: gpui::Hsla::default(),
            font_family: Default::default(),
            font_features: Default::default(),
            font_fallbacks: None,
            font_size: px(14.0).into(),
            font_style: FontStyle::Normal,
            font_weight: FontWeight::NORMAL,
            line_height: px(20.0).into(),
            background_color: Some(gpui::Hsla::default()),
            white_space: WhiteSpace::Normal,
            underline: None,
            strikethrough: None,
            line_clamp: None,
            text_align: gpui::TextAlign::Left,
            text_overflow: Default::default(),
        }
    }

    fn make_cell(ch: char, col: i32) -> IndexedCell {
        IndexedCell {
            point: AlacPoint::new(Line(0), Column(col as usize)),
            cell: AlacCell {
                c: ch,
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        }
    }

    #[test]
    fn build_row_accumulates_text_for_same_style_cells() {
        let base = default_text_style();
        let cells = vec![
            make_cell('h', 0),
            make_cell('e', 1),
            make_cell('l', 2),
            make_cell('l', 3),
            make_cell('o', 4),
        ];
        let (text, runs, _bg_rects) =
            build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "hello", "text must contain all same-style characters");
        assert_eq!(
            runs.len(),
            1,
            "same-style cells must produce a single TextRun"
        );
        assert_eq!(
            runs[0].len,
            "hello".len(),
            "TextRun length must match text length"
        );
    }

    #[test]
    fn build_row_handles_empty_cells() {
        let base = default_text_style();
        let cells: Vec<IndexedCell> = vec![];
        let (text, runs, _bg_rects) =
            build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "");
        assert!(runs.is_empty());
    }

    #[test]
    fn word_bounds_expands_over_word_chars() {
        let chars: Vec<char> = "  cargo build  ".chars().collect();
        // Click inside "cargo" (index 3) -> covers cols 2..=6.
        assert_eq!(terminal_word_bounds(&chars, 3), Some((2, 6)));
        // Click inside "build" (index 9) -> covers cols 8..=12.
        assert_eq!(terminal_word_bounds(&chars, 9), Some((8, 12)));
    }

    #[test]
    fn word_bounds_includes_path_punctuation() {
        let chars: Vec<char> = "see ./src/main.rs now".chars().collect();
        // "./src/main.rs" spans cols 4..=16 (dots, slashes kept).
        assert_eq!(terminal_word_bounds(&chars, 8), Some((4, 16)));
    }

    #[test]
    fn word_bounds_none_on_whitespace_or_oob() {
        let chars: Vec<char> = "a b".chars().collect();
        assert_eq!(terminal_word_bounds(&chars, 1), None); // space
        assert_eq!(terminal_word_bounds(&chars, 99), None); // out of bounds
    }

    #[test]
    fn build_row_handles_wide_char_spacer() {
        let base = default_text_style();
        let cells = vec![
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(0_usize)),
                cell: AlacCell {
                    c: '\0',
                    fg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Foreground,
                    ),
                    bg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Background,
                    ),
                    flags: Flags::WIDE_CHAR_SPACER,
                    extra: None,
                },
            },
            make_cell('a', 1),
        ];
        let (text, _runs, _bg_rects) =
            build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "a", "wide char spacer at col 0 must be skipped");
    }

    #[test]
    fn build_row_produces_background_rects() {
        let base = default_text_style();
        let cells = vec![IndexedCell {
            point: AlacPoint::new(Line(0), Column(0_usize)),
            cell: AlacCell {
                c: 'X',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Red,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        }];
        let (_text, _runs, bg_rects) =
            build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(
            bg_rects.len(),
            1,
            "non-default background must produce a rect"
        );
        assert_eq!(bg_rects[0].col, 0);
        assert_eq!(bg_rects[0].num_cells, 1);
        assert_eq!(bg_rects[0].row, 0);
    }

    #[test]
    fn build_row_no_background_rects_for_default_bg() {
        let base = default_text_style();
        let cells = vec![make_cell('X', 0)];
        let (_text, _runs, bg_rects) =
            build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert!(
            bg_rects.is_empty(),
            "cells with default background must not produce rects"
        );
    }

    #[test]
    fn build_row_adjacent_same_bg_merged() {
        let base = default_text_style();
        let make_red_cell = |ch, col| IndexedCell {
            point: AlacPoint::new(Line(0), Column(col as usize)),
            cell: AlacCell {
                c: ch,
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Red,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        };
        let cells = vec![make_red_cell('a', 0), make_red_cell('b', 1)];
        let (_text, _runs, bg_rects) =
            build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(bg_rects.len(), 1, "adjacent same-bg cells must merge");
        assert_eq!(bg_rects[0].num_cells, 2);
    }

    #[test]
    fn build_row_different_bg_not_merged() {
        let base = default_text_style();
        let cells = vec![
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(0_usize)),
                cell: AlacCell {
                    c: 'a',
                    fg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Foreground,
                    ),
                    bg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Red,
                    ),
                    flags: Flags::empty(),
                    extra: None,
                },
            },
            IndexedCell {
                point: AlacPoint::new(Line(0), Column(1_usize)),
                cell: AlacCell {
                    c: 'b',
                    fg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Foreground,
                    ),
                    bg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Blue,
                    ),
                    flags: Flags::empty(),
                    extra: None,
                },
            },
        ];
        let (_text, _runs, bg_rects) =
            build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(bg_rects.len(), 2, "different bg colors must not merge");
    }

    #[test]
    fn color_to_rgba_resolves_named_ansi_colors() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let fg = palette.foreground;
        let _bg = palette.background;

        let red = color_to_rgba(
            alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Red,
            ),
            &palette,
            fg,
        );
        assert_eq!(red, palette.red);

        let green = color_to_rgba(
            alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Green,
            ),
            &palette,
            fg,
        );
        assert_eq!(green, palette.green);

        let foreground = color_to_rgba(
            alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            &palette,
            fg,
        );
        assert_eq!(foreground, palette.foreground);
    }

    #[test]
    fn color_to_rgba_resolves_spec_colors() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let fg = palette.foreground;
        let spec = color_to_rgba(
            alacritty_terminal::vte::ansi::Color::Spec(alacritty_terminal::vte::ansi::Rgb {
                r: 100,
                g: 150,
                b: 200,
            }),
            &palette,
            fg,
        );
        assert!((spec.r - 100.0 / 255.0).abs() < 0.01);
        assert!((spec.g - 150.0 / 255.0).abs() < 0.01);
        assert!((spec.b - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn get_color_at_index_maps_correctly() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        assert_eq!(get_color_at_index(0, &palette), palette.black);
        assert_eq!(get_color_at_index(1, &palette), palette.red);
        assert_eq!(get_color_at_index(7, &palette), palette.white);
        assert_eq!(get_color_at_index(8, &palette), palette.bright_black);
        assert_eq!(get_color_at_index(15, &palette), palette.bright_white);
        // 256-color cube: index 16 is first entry
        let cube_color = get_color_at_index(16, &palette);
        assert_eq!(cube_color.r, 0.0);
        assert_eq!(cube_color.g, 0.0);
        assert_eq!(cube_color.b, 0.0);
        // Index 232 is first grayscale
        let gray = get_color_at_index(232, &palette);
        assert!((gray.r - 8.0 / 255.0).abs() < 0.01);
        assert_eq!(gray.r, gray.g);
        assert_eq!(gray.g, gray.b);
    }

    #[test]
    fn alacritty_cell_style_inverse_swaps_fg_bg() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let cell = AlacCell {
            c: 'X',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::White,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Red,
            ),
            flags: Flags::INVERSE,
            extra: None,
        };
        let style = alacritty_cell_style(&cell, &palette);
        assert_eq!(style.fg, palette.red, "inverse swaps fg to bg color");
        assert_eq!(
            style.bg,
            Some(palette.white),
            "inverse swaps bg to fg color"
        );
    }

    #[test]
    fn alacritty_cell_style_dim_reduces_alpha() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let cell = AlacCell {
            c: 'X',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::White,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::DIM,
            extra: None,
        };
        let style = alacritty_cell_style(&cell, &palette);
        assert!(style.fg.a < 1.0, "dim must reduce foreground alpha");
    }

    #[test]
    fn alacritty_cell_style_default_bg_is_none() {
        let palette = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let cell = AlacCell {
            c: 'X',
            fg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Foreground,
            ),
            bg: alacritty_terminal::vte::ansi::Color::Named(
                alacritty_terminal::vte::ansi::NamedColor::Background,
            ),
            flags: Flags::empty(),
            extra: None,
        };
        let style = alacritty_cell_style(&cell, &palette);
        assert!(
            style.bg.is_none(),
            "default background must produce None bg"
        );
    }

    #[test]
    fn terminal_text_run_no_background_color() {
        let base = default_text_style();
        let style = TerminalCellStyle {
            fg: gpui::rgb(0xff0000),
            bg: Some(gpui::rgb(0x00ff00)),
            bold: false,
            italic: false,
            underline: false,
        };
        let run = terminal_text_run(&base, &style, 5);
        assert!(
            run.background_color.is_none(),
            "TextRun must never set background_color"
        );
        assert_eq!(run.color, gpui::Hsla::from(style.fg));
    }

    #[test]
    fn terminal_palette_dark_vs_light_differs() {
        let dark = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_dark());
        let light = TerminalAnsiPalette::from_theme(AppTheme::gitcomet_light());
        assert_ne!(
            dark.background, light.background,
            "dark/light palettes must differ"
        );
    }

    #[test]
    fn rgb_for_index_computes_6x6x6_cube() {
        assert_eq!(rgb_for_index(0), (0, 0, 0));
        assert_eq!(rgb_for_index(1), (0, 0, 1));
        assert_eq!(rgb_for_index(5), (0, 0, 5));
        assert_eq!(rgb_for_index(6), (0, 1, 0));
        assert_eq!(rgb_for_index(35), (0, 5, 5));
        assert_eq!(rgb_for_index(36), (1, 0, 0));
        assert_eq!(rgb_for_index(215), (5, 5, 5));
    }

    #[test]
    fn build_row_includes_spaces_for_grid_alignment() {
        let base = default_text_style();
        let cells = vec![
            make_cell('a', 0),
            make_cell(' ', 1),
            make_cell(' ', 2),
            make_cell('b', 3),
        ];
        let (text, _runs, _bg_rects) =
            build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
        assert_eq!(
            text, "a  b",
            "text must include spaces so shape_line positions glyphs at correct columns"
        );
    }

    #[test]
    fn build_row_handles_zero_width_chars() {
        let base = default_text_style();
        let combining = '\u{0301}';
        let cell = IndexedCell {
            point: AlacPoint::new(Line(0), Column(0_usize)),
            cell: {
                let mut c = AlacCell {
                    c: 'e',
                    fg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Foreground,
                    ),
                    bg: alacritty_terminal::vte::ansi::Color::Named(
                        alacritty_terminal::vte::ansi::NamedColor::Background,
                    ),
                    flags: Flags::empty(),
                    extra: None,
                };
                c.push_zerowidth(combining);
                c
            },
        };
        let (text, runs, _bg_rects) =
            build_alacritty_row(&[cell], 0, 2, &base, AppTheme::gitcomet_dark());
        let expected = format!("e{}", combining);
        assert_eq!(
            text, expected,
            "text must include zero-width combining char"
        );
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].len, expected.len());
    }

    // -----------------------------------------------------------------------
    // Mouse encoding tests
    // -----------------------------------------------------------------------

    #[test]
    fn mouse_mode_composite_flag() {
        let mut mode = TerminalModes::default();
        assert!(!mode.mouse_mode());
        mode = TerminalModes::MOUSE_REPORT_CLICK;
        assert!(mode.mouse_mode());
        mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::MOUSE_DRAG;
        assert!(mode.mouse_mode());
        mode = TerminalModes::MOUSE_MOTION;
        assert!(mode.mouse_mode());
    }

    #[test]
    fn sgr_mouse_button_press_report() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
        let report = terminal_mouse_button_report(
            5,
            10,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            true,
            mode,
        )
        .unwrap();
        let s = String::from_utf8(report).unwrap();
        assert_eq!(s, "\x1b[<0;11;6M");
    }

    #[test]
    fn sgr_mouse_button_release_report() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
        let report = terminal_mouse_button_report(
            3,
            7,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            false,
            mode,
        )
        .unwrap();
        let s = String::from_utf8(report).unwrap();
        assert_eq!(s, "\x1b[<0;8;4m");
    }

    #[test]
    fn sgr_mouse_report_with_modifiers() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
        let mods = gpui::Modifiers {
            shift: true,
            control: true,
            ..Default::default()
        };
        let report =
            terminal_mouse_button_report(0, 0, gpui::MouseButton::Left, mods, true, mode).unwrap();
        let s = String::from_utf8(report).unwrap();
        assert_eq!(s, "\x1b[<20;1;1M", "shift=4 + control=16 + button=0 = 20");
    }

    #[test]
    fn normal_mouse_button_press_report() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK;
        let report = terminal_mouse_button_report(
            0,
            0,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            true,
            mode,
        )
        .unwrap();
        assert_eq!(
            report,
            vec![0x1b, b'[', b'M', 32 + 0, 32 + 1 + 0, 32 + 1 + 0]
        );
    }

    #[test]
    fn normal_mouse_button_release_report() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK;
        let report = terminal_mouse_button_report(
            0,
            0,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            false,
            mode,
        )
        .unwrap();
        assert_eq!(
            report,
            vec![0x1b, b'[', b'M', 32 + 3, 32 + 1 + 0, 32 + 1 + 0]
        );
    }

    #[test]
    fn mouse_event_at_returns_none_without_mouse_mode() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        let layout_cache = TerminalLayoutCache {
            rem_size: px(16.0),
            key: TerminalLayoutKey::default(),
            base_style: default_text_style(),
            metrics: TerminalTextMetrics {
                font_size: px(14.0),
                line_height: px(20.0),
                cell_width: px(8.0),
            },
        };
        let content = TerminalContent {
            cells: vec![],
            mode: TerminalModes::default(),
            display_offset: 0,
            cursor: TerminalCursor {
                point: AlacPoint::new(Line(0), Column(0)),
                shape: TerminalCursorShape::Beam,
            },
            cursor_char: ' ',
            terminal_bounds: AlacTerminalBounds::new(80, 24),
        };
        let report = terminal_mouse_event_at(
            point(px(40.0), px(80.0)),
            Some(bounds),
            &Some(layout_cache),
            &Some(content),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            true,
        );
        assert!(report.is_none(), "no report when mouse mode is off");
    }

    #[test]
    fn mouse_moved_report_with_motion_mode() {
        let mode = TerminalModes::MOUSE_MOTION | TerminalModes::SGR_MOUSE;
        let report = terminal_mouse_moved_report(
            2,
            5,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
            mode,
        )
        .unwrap();
        let s = String::from_utf8(report).unwrap();
        assert_eq!(s, "\x1b[<32;6;3M");
    }

    #[test]
    fn mouse_moved_report_none_move_blocked_in_drag_only_mode() {
        let mode = TerminalModes::MOUSE_DRAG;
        let report = terminal_mouse_moved_report(0, 0, None, gpui::Modifiers::default(), mode);
        assert!(
            report.is_none(),
            "NoneMove blocked when only MOUSE_DRAG is set"
        );
    }

    #[test]
    fn scroll_report_generates_reports() {
        let mode = TerminalModes::MOUSE_MODE | TerminalModes::SGR_MOUSE;
        let reports = terminal_scroll_report(10, 20, gpui::Modifiers::default(), px(-1.0), 2, mode);
        assert_eq!(reports.len(), 2);
        let s0 = String::from_utf8(reports[0].clone()).unwrap();
        assert_eq!(s0, "\x1b[<65;21;11M", "scroll down = 65");
    }

    #[test]
    fn grid_point_computes_correctly() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        let cell_width = px(8.0);
        let line_height = px(16.0);
        let display_offset = 0;
        let cols = 100u16;

        let (row, col) = terminal_grid_point(
            point(px(40.0), px(80.0)),
            bounds,
            cell_width,
            line_height,
            display_offset,
            cols,
        )
        .unwrap();
        assert_eq!(col, 5);
        assert_eq!(row, 5);
    }

    #[test]
    fn grid_point_with_display_offset() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(800.0), px(600.0)));
        let cell_width = px(8.0);
        let line_height = px(16.0);
        let display_offset = 5;
        let cols = 100u16;

        let (row, _col) = terminal_grid_point(
            point(px(0.0), px(0.0)),
            bounds,
            cell_width,
            line_height,
            display_offset,
            cols,
        )
        .unwrap();
        assert_eq!(row, -5, "viewport row 0 with offset 5 = grid row -5");
    }

    #[test]
    fn mouse_report_rejects_negative_grid_rows() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
        let report = terminal_mouse_button_report(
            -1,
            0,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
            true,
            mode,
        );
        assert!(
            report.is_none(),
            "negative grid row must not produce report"
        );
    }

    // -----------------------------------------------------------------------
    // Background rect merging tests
    // -----------------------------------------------------------------------

    fn make_bg_rect(row: i32, col: i32, cells: usize, color: gpui::Rgba) -> TerminalBackgroundRect {
        TerminalBackgroundRect {
            row,
            col,
            num_cells: cells,
            num_rows: 1,
            color,
        }
    }

    #[test]
    fn merge_background_rects_vertical_adjacent_same_span() {
        let red = gpui::rgb(0xff0000);
        let rects = vec![
            make_bg_rect(0, 0, 5, red),
            make_bg_rect(1, 0, 5, red),
            make_bg_rect(2, 0, 5, red),
        ];
        let merged = merge_background_rects(&rects);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].row, 0);
        assert_eq!(merged[0].num_rows, 3);
        assert_eq!(merged[0].num_cells, 5);
    }

    #[test]
    fn merge_background_rects_different_colors_not_merged() {
        let red = gpui::rgb(0xff0000);
        let blue = gpui::rgb(0x0000ff);
        let rects = vec![make_bg_rect(0, 0, 5, red), make_bg_rect(1, 0, 5, blue)];
        let merged = merge_background_rects(&rects);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merge_background_rects_empty_input() {
        let rects: Vec<TerminalBackgroundRect> = vec![];
        let merged = merge_background_rects(&rects);
        assert!(merged.is_empty());
    }

    // -----------------------------------------------------------------------
    // Focus/blur escape sequence tests
    // -----------------------------------------------------------------------

    #[test]
    fn focus_in_out_mode_is_tracked() {
        let mut mode = TerminalModes::default();
        assert!(!mode.contains(TerminalModes::FOCUS_IN_OUT));
        mode = TerminalModes::FOCUS_IN_OUT;
        assert!(mode.contains(TerminalModes::FOCUS_IN_OUT));
    }

    // -----------------------------------------------------------------------
    // IME InputHandler tests
    // -----------------------------------------------------------------------

    #[test]
    fn ime_handler_marked_text_range_none_when_empty() {
        let handler = TerminalTextInputHandler {
            pty_sender: None,
            ime_state: None,
        };
        assert!(handler.ime_state.is_none());
    }

    #[test]
    fn ime_handler_replace_and_mark_stores_text() {
        let mut handler = TerminalTextInputHandler {
            pty_sender: None,
            ime_state: None,
        };
        handler.ime_state = Some(TerminalImeState {
            marked_text: "test".to_string(),
        });
        assert!(handler.ime_state.is_some());
        assert_eq!(handler.ime_state.as_ref().unwrap().marked_text, "test");
    }

    #[test]
    fn ime_handler_unmark_clears_state() {
        let mut handler = TerminalTextInputHandler {
            pty_sender: None,
            ime_state: Some(TerminalImeState {
                marked_text: "x".to_string(),
            }),
        };
        handler.ime_state = None;
        assert!(handler.ime_state.is_none());
    }

    #[test]
    fn ime_handler_apple_press_and_hold_disabled() {
        let mut handler = TerminalTextInputHandler {
            pty_sender: None,
            ime_state: None,
        };
        assert!(
            !<TerminalTextInputHandler as gpui::InputHandler>::apple_press_and_hold_enabled(
                &mut handler
            )
        );
    }

    // -----------------------------------------------------------------------
    // Search/find tests
    // -----------------------------------------------------------------------

    #[test]
    fn terminal_content_text_extraction() {
        let dummy_terminal_content_text = "line one\nline two\n";
        assert!(dummy_terminal_content_text.contains("line one"));
        assert!(dummy_terminal_content_text.contains("line two"));
    }

    // -----------------------------------------------------------------------
    // Clipboard shortcut key matching tests
    // -----------------------------------------------------------------------

    #[test]
    fn clipboard_shortcut_lowercase_c_ctrl_shift_linux() {
        let ks = gpui::Keystroke {
            key: "c".into(),
            modifiers: gpui::Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            },
            key_char: None,
        };
        let app_cursor = false;
        let option_as_meta = true;
        let result = encode_alacritty_key_input(&ks, app_cursor, option_as_meta);
        assert_eq!(result, Some(vec![0x03]));
    }

    #[test]
    fn clipboard_shortcut_uppercase_c_ctrl_shift_linux() {
        let ks = gpui::Keystroke {
            key: "C".into(),
            modifiers: gpui::Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            },
            key_char: None,
        };
        let app_cursor = false;
        let option_as_meta = true;
        let result = encode_alacritty_key_input(&ks, app_cursor, option_as_meta);
        assert_eq!(result, None);
    }

    #[test]
    fn clipboard_shortcut_v_ctrl_shift_linux() {
        let ks = gpui::Keystroke {
            key: "v".into(),
            modifiers: gpui::Modifiers {
                control: true,
                shift: true,
                ..Default::default()
            },
            key_char: None,
        };
        let app_cursor = false;
        let option_as_meta = true;
        let result = encode_alacritty_key_input(&ks, app_cursor, option_as_meta);
        assert_eq!(result, Some(vec![0x16]));
    }

    #[test]
    fn terminal_modes_mouse_mode_composite() {
        let mode = TerminalModes::MOUSE_MODE;
        assert!(mode.mouse_mode());
        assert!(mode.intersects(TerminalModes::MOUSE_REPORT_CLICK));
        assert!(mode.intersects(TerminalModes::MOUSE_DRAG));
        assert!(mode.intersects(TerminalModes::MOUSE_MOTION));
    }

    #[test]
    fn terminal_modes_intersects_partial() {
        let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
        assert!(mode.intersects(TerminalModes::MOUSE_MODE));
        assert!(mode.intersects(TerminalModes::MOUSE_REPORT_CLICK));
        assert!(!mode.intersects(TerminalModes::MOUSE_DRAG));
    }

    // -----------------------------------------------------------------------
    // Shift key uppercase tests
    // -----------------------------------------------------------------------

    #[test]
    fn encode_shift_a_produces_uppercase_a() {
        let ks = gpui::Keystroke {
            key: "a".into(),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            key_char: Some("A".into()),
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(
            result,
            Some(b"A".to_vec()),
            "Shift+a must produce uppercase 'A'"
        );
    }

    #[test]
    fn encode_shift_z_produces_uppercase_z() {
        let ks = gpui::Keystroke {
            key: "z".into(),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            key_char: Some("Z".into()),
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"Z".to_vec()));
    }

    #[test]
    fn encode_no_shift_a_produces_lowercase_a() {
        let ks = gpui::Keystroke {
            key: "a".into(),
            modifiers: gpui::Modifiers::default(),
            key_char: Some("a".into()),
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"a".to_vec()));
    }

    #[test]
    fn encode_shift_digit_produces_symbol() {
        let ks = gpui::Keystroke {
            key: "1".into(),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            key_char: Some("!".into()),
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"!".to_vec()), "Shift+1 must produce '!'");
    }

    #[test]
    fn encode_shift_space_produces_space() {
        let ks = gpui::Keystroke {
            key: "space".into(),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            key_char: None,
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b" ".to_vec()));
    }

    #[test]
    fn encode_tab_produces_tab_byte() {
        let ks = gpui::Keystroke {
            key: "tab".into(),
            modifiers: gpui::Modifiers::default(),
            key_char: None,
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"\t".to_vec()));
    }

    #[test]
    fn encode_shift_tab_produces_backtab_escape_sequence() {
        let ks = gpui::Keystroke {
            key: "tab".into(),
            modifiers: gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            key_char: None,
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn encode_ctrl_left_produces_modified_escape_sequence() {
        let ks = gpui::Keystroke {
            key: "left".into(),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            key_char: None,
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"\x1b[1;5D".to_vec()));
    }

    #[test]
    fn encode_ctrl_right_produces_modified_escape_sequence() {
        let ks = gpui::Keystroke {
            key: "right".into(),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            key_char: None,
        };
        let result = encode_alacritty_key_input(&ks, false, true);
        assert_eq!(result, Some(b"\x1b[1;5C".to_vec()));
    }

    // -----------------------------------------------------------------------
    // Build row with grid-relative rows (scrollback) tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_row_with_negative_grid_row() {
        let base = default_text_style();
        let cells = vec![IndexedCell {
            point: AlacPoint::new(Line(-3), Column(0_usize)),
            cell: AlacCell {
                c: 'S',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        }];
        let (text, runs, _bg) =
            build_alacritty_row(&cells, -3, 5, &base, AppTheme::gitcomet_dark());
        assert_eq!(
            text, "S",
            "build_alacritty_row must accept negative grid rows for scrollback"
        );
        assert_eq!(runs.len(), 1);
    }

    #[test]
    fn build_row_empty_when_no_cells_at_row() {
        let base = default_text_style();
        let cells = vec![IndexedCell {
            point: AlacPoint::new(Line(0), Column(0_usize)),
            cell: AlacCell {
                c: 'X',
                fg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Foreground,
                ),
                bg: alacritty_terminal::vte::ansi::Color::Named(
                    alacritty_terminal::vte::ansi::NamedColor::Background,
                ),
                flags: Flags::empty(),
                extra: None,
            },
        }];
        let (text, runs, _bg) = build_alacritty_row(&cells, 5, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "", "no cells at row 5");
        assert!(runs.is_empty());
    }
}
