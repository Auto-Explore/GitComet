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
const TERMINAL_DEFAULT_BG_HEX: u32 = 0x000000;
const TERMINAL_DEFAULT_FG_HEX: u32 = 0xffffff;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[derive(Default)]
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

    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
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

pub(super) fn alacritty_cell_style(
    cell: &AlacCell,
    foreground: gpui::Rgba,
    background: gpui::Rgba,
) -> TerminalCellStyle {
    let default_cell = AlacCell::default();
    let is_default_bg = cell.bg == default_cell.bg;

    let mut fg = color_to_rgba(cell.fg, foreground);
    let bg = if is_default_bg {
        None
    } else {
        Some(color_to_rgba(cell.bg, background))
    };
    let flags = cell.flags;

    if flags.contains(Flags::INVERSE) {
        fg = bg.unwrap_or(background);
    }

    if flags.contains(Flags::DIM) {
        fg = mix_rgba(fg, background, 0.35);
    }

    TerminalCellStyle {
        fg,
        bg: if flags.contains(Flags::INVERSE) && is_default_bg {
            Some(foreground)
        } else if flags.contains(Flags::INVERSE) {
            Some(foreground)
        } else {
            bg
        },
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline: flags.contains(Flags::UNDERLINE),
    }
}

fn color_to_rgba(
    color: alacritty_terminal::vte::ansi::Color,
    default_val: gpui::Rgba,
) -> gpui::Rgba {
    use alacritty_terminal::vte::ansi::Color;
    match color {
        Color::Named(_) => default_val,
        Color::Spec(rgb) => gpui::Rgba {
            r: rgb.r as f32 / 255.0,
            g: rgb.g as f32 / 255.0,
            b: rgb.b as f32 / 255.0,
            a: 1.0,
        },
        Color::Indexed(_) => default_val,
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

pub(super) fn terminal_default_background() -> gpui::Rgba {
    rgba_from_hex(TERMINAL_DEFAULT_BG_HEX)
}

pub(super) fn terminal_default_foreground() -> gpui::Rgba {
    rgba_from_hex(TERMINAL_DEFAULT_FG_HEX)
}

fn rgba_from_hex(hex: u32) -> gpui::Rgba {
    gpui::Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn mix_rgba(fg: gpui::Rgba, bg: gpui::Rgba, t: f32) -> gpui::Rgba {
    gpui::Rgba {
        r: fg.r * (1.0 - t) + bg.r * t,
        g: fg.g * (1.0 - t) + bg.g * t,
        b: fg.b * (1.0 - t) + bg.b * t,
        a: fg.a,
    }
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
            shape: TerminalCursorShape::Beam,
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

    if mods.control && !mods.alt
        && let Some(control) = encode_control_key(key) {
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
        _ if key.len() == 1 && !mods.control && !mods.alt => Some(key.as_bytes().to_vec()),
        _ => None,
    }
}

pub(super) fn encode_control_key(key: &str) -> Option<u8> {
    if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if (b'a'..=b'z').contains(&ch) {
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

// ---------------------------------------------------------------------------
// Build terminal row for rendering
// ---------------------------------------------------------------------------

pub(super) fn build_alacritty_row(
    cells: &[IndexedCell],
    row: i32,
    cols: usize,
    base_style: &gpui::TextStyle,
    _theme: AppTheme,
) -> (SharedString, Vec<TextRun>) {
    let terminal_fg = terminal_default_foreground();
    let terminal_bg = terminal_default_background();

    let mut text = String::new();
    let mut runs = Vec::new();
    let mut active_style: Option<TerminalCellStyle> = None;
    let mut active_len = 0usize;

    let row_cells: Vec<&IndexedCell> = cells.iter().filter(|ic| ic.point.line.0 == row).collect();

    let mut col = 0usize;
    let mut cell_idx = 0usize;

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
        let contents: SharedString = if ch == ' ' || ch == '\0' {
            " ".into()
        } else {
            ch.to_string().into()
        };

        let style = alacritty_cell_style(&cell.cell, terminal_fg, terminal_bg);

        text.push_str(&contents);
        if active_style
            .as_ref()
            .is_some_and(|current| current == &style)
        {
            active_len += contents.len();
        } else {
            if let Some(previous) = active_style.take() {
                runs.push(terminal_text_run(base_style, &previous, active_len));
            }
            active_style = Some(style);
            active_len = contents.len();
        }

        col += 1;
        if cell.cell.flags.contains(Flags::WIDE_CHAR) && col < cols {
            col += 1;
        }
    }

    if let Some(previous) = active_style.take()
        && active_len > 0 {
            runs.push(terminal_text_run(base_style, &previous, active_len));
        }

    (text.into(), runs)
}

pub(super) fn terminal_text_run(
    base_style: &gpui::TextStyle,
    style: &TerminalCellStyle,
    len: usize,
) -> TextRun {
    let mut text_style = base_style.clone();
    text_style.color = style.fg.into();
    if let Some(bg) = style.bg {
        text_style.background_color = Some(bg.into());
    }
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
        background_color: text_style.background_color,
        underline: text_style.underline,
        strikethrough: None,
    }
}

// ---------------------------------------------------------------------------
// Extract text from cells
// ---------------------------------------------------------------------------

pub(super) fn row_cell_text(cells: &[IndexedCell], row: i32, cols: usize) -> String {
    let mut text = String::new();
    let row_cells: Vec<&IndexedCell> = cells.iter().filter(|ic| ic.point.line.0 == row).collect();

    let mut col = 0usize;
    let mut cell_idx = 0usize;

    while col < cols {
        if cell_idx < row_cells.len() && row_cells[cell_idx].point.column.0 == col {
            let cell = row_cells[cell_idx];
            cell_idx += 1;
            if !cell.cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                let ch = cell.cell.c;
                if ch == ' ' || ch == '\0' {
                    text.push(' ');
                } else {
                    text.push(ch);
                }
                if cell.cell.flags.contains(Flags::WIDE_CHAR) {
                    col += 1;
                }
            }
        } else {
            text.push(' ');
        }
        col += 1;
    }
    text
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
        let (text, runs) = build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
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
        let (text, runs) = build_alacritty_row(&cells, 0, 5, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "");
        assert!(runs.is_empty());
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
        let (text, _runs) = build_alacritty_row(&cells, 0, 3, &base, AppTheme::gitcomet_dark());
        assert_eq!(text, "a", "wide char spacer at col 0 must be skipped");
    }
}
