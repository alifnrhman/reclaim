use crate::delete;
use crate::format;
use crate::model::{Candidate, Risk, ScanWarnings};
use anyhow::Result;
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::mpsc;
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const ACTIVE: &str = "\x1b[48;5;237m";

#[derive(Debug)]
struct App {
    candidates: Vec<Candidate>,
    cursor: usize,
    scroll: usize,
    filter: Filter,
    descending: bool,
    message: String,
    warnings: ScanWarnings,
    confirm: bool,
    deleting: Option<DeleteProgress>,
}

#[derive(Debug)]
struct DeleteProgress {
    frame: usize,
    current: usize,
    total: usize,
    path: String,
}

enum DeleteEvent {
    Started {
        current: usize,
        total: usize,
        path: String,
    },
    Done(Vec<delete::DeleteReport>),
}

#[derive(Debug, Clone, Copy)]
enum Filter {
    All,
    Safe,
    Review,
    Protected,
}

impl Filter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Safe,
            Self::Safe => Self::Review,
            Self::Review => Self::Protected,
            Self::Protected => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Safe => "safe",
            Self::Review => "review",
            Self::Protected => "protected",
        }
    }

    fn includes(self, risk: Risk) -> bool {
        match self {
            Self::All => true,
            Self::Safe => risk == Risk::Safe,
            Self::Review => risk == Risk::Review,
            Self::Protected => risk == Risk::Protected,
        }
    }
}

impl App {
    fn visible(&self) -> Vec<usize> {
        let mut indices: Vec<_> = self
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| self.filter.includes(candidate.risk))
            .map(|(index, _)| index)
            .collect();
        indices.sort_by_key(|index| self.candidates[*index].bytes);
        if self.descending {
            indices.reverse();
        }
        indices
    }

    fn selected_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.selected && candidate.risk != Risk::Protected)
            .count()
    }

    fn selected_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .filter(|candidate| candidate.selected && candidate.risk != Risk::Protected)
            .map(|candidate| candidate.bytes)
            .sum()
    }

    fn safe_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .filter(|candidate| candidate.risk == Risk::Safe)
            .map(|candidate| candidate.bytes)
            .sum()
    }

    fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(self.visible().len().saturating_sub(1));
        self.ensure_cursor_visible();
    }

    fn move_cursor(&mut self, amount: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        self.cursor = self
            .cursor
            .saturating_add_signed(amount)
            .min(len.saturating_sub(1));
        self.ensure_cursor_visible();
    }

    fn ensure_cursor_visible(&mut self) {
        let rows = viewport_rows();
        let len = self.visible().len();
        self.scroll = self.scroll.min(len.saturating_sub(rows));
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll.saturating_add(rows) {
            self.scroll = self.cursor + 1 - rows;
        }
    }
}

#[derive(Debug)]
#[cfg_attr(not(unix), allow(dead_code))]
enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    Backspace,
    Escape,
    Char(char),
    #[cfg(not(unix))]
    Text(String),
    Unknown,
}

pub fn run(
    candidates: Vec<Candidate>,
    warnings: ScanWarnings,
    startup_message: Option<String>,
) -> Result<()> {
    let _terminal = TerminalSession::enter()?;
    let mut app = App {
        candidates,
        cursor: 0,
        scroll: 0,
        filter: Filter::All,
        descending: true,
        message: startup_message.unwrap_or_default(),
        warnings,
        confirm: false,
        deleting: None,
    };

    loop {
        draw(&app)?;
        let key = read_key()?;
        if app.confirm {
            handle_confirmation(&mut app, key)?;
            continue;
        }
        match key {
            // Exit only on an explicit q. Some terminals deliver an arrow's
            // ESC prefix separately, so treating bare ESC as quit is unsafe.
            Key::Char('q') => break,
            Key::Down | Key::Char('j') => app.move_cursor(1),
            Key::Up | Key::Char('k') => app.move_cursor(-1),
            Key::PageDown => app.move_cursor(viewport_rows() as isize),
            Key::PageUp => app.move_cursor(-(viewport_rows() as isize)),
            Key::Char(' ') | Key::Char('x') => toggle_selected(&mut app),
            Key::Char('f') => {
                app.filter = app.filter.next();
                app.cursor = 0;
                app.scroll = 0;
            }
            Key::Char('s') => app.descending = !app.descending,
            Key::Char('a') => {
                for candidate in &mut app.candidates {
                    if candidate.risk == Risk::Safe {
                        candidate.selected = true;
                    }
                }
            }
            Key::Char('n') => {
                for candidate in &mut app.candidates {
                    candidate.selected = false;
                }
            }
            Key::Char('w') if !app.warnings.0.is_empty() => {
                app.message = app.warnings.0.join("  ·  ");
            }
            Key::Char('d') | Key::Enter => begin_delete(&mut app),
            #[cfg(not(unix))]
            Key::Text(text) => handle_cooked_command(&mut app, &text),
            _ => {}
        }
    }
    Ok(())
}

fn toggle_selected(app: &mut App) {
    if let Some(index) = app.visible().get(app.cursor).copied()
        && app.candidates[index].risk != Risk::Protected
    {
        app.candidates[index].selected = !app.candidates[index].selected;
    }
}

#[cfg(not(unix))]
fn handle_cooked_command(app: &mut App, text: &str) {
    match text.trim().to_ascii_lowercase().as_str() {
        "j" | "down" => {
            app.move_cursor(1);
        }
        "k" | "up" => app.move_cursor(-1),
        "x" | "select" => toggle_selected(app),
        "f" => {
            app.filter = app.filter.next();
            app.cursor = 0;
            app.scroll = 0;
        }
        "s" => app.descending = !app.descending,
        "d" => begin_delete(app),
        _ => {}
    }
}

fn begin_delete(app: &mut App) {
    if app.selected_count() == 0 {
        app.message = "Nothing selected".into();
    } else {
        app.confirm = true;
    }
}

fn handle_confirmation(app: &mut App, key: Key) -> Result<()> {
    match key {
        Key::Escape | Key::Char('q') => app.confirm = false,
        Key::Char('d') | Key::Enter => finish_delete(app)?,
        #[cfg(not(unix))]
        Key::Text(text) => {
            if matches!(text.trim().to_ascii_lowercase().as_str(), "" | "d") {
                finish_delete(app)?;
            } else if text.trim().eq_ignore_ascii_case("q") {
                app.confirm = false;
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish_delete(app: &mut App) -> Result<()> {
    app.confirm = false;
    let selected: Vec<_> = app
        .candidates
        .iter()
        .filter(|candidate| candidate.selected && candidate.risk != Risk::Protected)
        .cloned()
        .collect();
    app.deleting = Some(DeleteProgress {
        frame: 0,
        current: 0,
        total: selected.len(),
        path: "Preparing…".into(),
    });
    draw(app)?;

    let reports = std::thread::scope(|scope| -> Result<Vec<delete::DeleteReport>> {
        let (sender, receiver) = mpsc::channel();
        let worker = scope.spawn(move || {
            let reports =
                delete::delete_selected_with_progress(&selected, |current, total, candidate| {
                    let _ = sender.send(DeleteEvent::Started {
                        current,
                        total,
                        path: candidate.display_path(),
                    });
                });
            let _ = sender.send(DeleteEvent::Done(reports));
        });

        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(DeleteEvent::Started {
                    current,
                    total,
                    path,
                }) => {
                    app.deleting = Some(DeleteProgress {
                        frame: 0,
                        current,
                        total,
                        path,
                    });
                    draw(app)?;
                }
                Ok(DeleteEvent::Done(reports)) => {
                    worker
                        .join()
                        .map_err(|_| anyhow::anyhow!("deletion worker stopped unexpectedly"))?;
                    break Ok(reports);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(progress) = &mut app.deleting {
                        progress.frame = progress.frame.wrapping_add(1);
                    }
                    draw(app)?;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    worker
                        .join()
                        .map_err(|_| anyhow::anyhow!("deletion worker stopped unexpectedly"))?;
                    break Err(anyhow::anyhow!("deletion worker disconnected"));
                }
            }
        }
    })?;
    app.deleting = None;
    let successful: HashSet<_> = reports
        .iter()
        .filter(|report| report.result.is_ok())
        .map(|report| report.id)
        .collect();
    let reclaimed: u64 = reports
        .iter()
        .filter(|report| report.result.is_ok())
        .map(|report| report.estimated_bytes)
        .sum();
    let failures: Vec<_> = reports
        .iter()
        .filter_map(|report| {
            report
                .result
                .as_ref()
                .err()
                .map(|error| format!("{}: {error}", report.display))
        })
        .collect();
    app.candidates
        .retain(|candidate| !successful.contains(&candidate.id));
    for (id, candidate) in app.candidates.iter_mut().enumerate() {
        candidate.id = id;
    }
    app.message = if failures.is_empty() {
        format!("Reclaimed {}", format::bytes(reclaimed))
    } else {
        format!("{} removed · {}", successful.len(), failures.join(" · "))
    };
    app.clamp_cursor();
    Ok(())
}

fn draw(app: &App) -> Result<()> {
    let (width, height) = terminal_size();
    let visible = app.visible();
    let list_height = viewport_rows_for(height);
    let page_start = app.scroll.min(visible.len().saturating_sub(list_height));
    let page_end = (page_start + list_height).min(visible.len());

    let mut output = String::from("\x1b[2J\x1b[H");
    output.push_str(&format!("  {BOLD}reclaim{RESET}\n"));
    output.push_str(&format!(
        "  {GREEN}{}{RESET} safe   {CYAN}{}{RESET} selected   {DIM}{} item{} · {} · {}–{}/{}{RESET}\n\n",
        format::bytes(app.safe_bytes()),
        format::bytes(app.selected_bytes()),
        app.candidates.len(),
        if app.candidates.len() == 1 { "" } else { "s" },
        app.filter.label(),
        if visible.is_empty() { 0 } else { page_start + 1 },
        page_end,
        visible.len()
    ));

    if visible.is_empty() {
        output.push_str(&format!("  {DIM}Nothing in this view.{RESET}\n"));
    } else {
        for (position, index) in visible
            .iter()
            .enumerate()
            .skip(page_start)
            .take(list_height)
        {
            let candidate = &app.candidates[*index];
            let current = position == app.cursor;
            let pointer = if current { "›" } else { " " };
            let selected = if candidate.risk == Risk::Protected {
                "×"
            } else if candidate.selected {
                "●"
            } else {
                "○"
            };
            let color = risk_color(candidate.risk);
            let prefix = if current { ACTIVE } else { "" };
            // The visible fixed columns before PATH occupy 41 cells. Keep a
            // four-cell right margin so no terminal can auto-wrap the row and
            // silently double its height.
            let path_width = width.saturating_sub(45);
            let path = sanitize_terminal(&candidate.display_path());
            let path = truncate_middle(&path, path_width);
            output.push_str(&format!(
                "{prefix}  {pointer} {color}{selected}{RESET}{prefix} {:>10}  {color}{:<9}{RESET}{prefix}  {:<10}  {}{RESET}\x1b[K\n",
                format::bytes(candidate.bytes),
                candidate.risk.label().to_ascii_lowercase(),
                candidate.source.label(),
                path
            ));
        }
    }

    output.push('\n');
    if let Some(index) = visible.get(app.cursor) {
        let candidate = &app.candidates[*index];
        output.push_str(&format!(
            "  {}\n  {DIM}{}{RESET}\n",
            truncate(
                &sanitize_terminal(&candidate.reason),
                width.saturating_sub(4)
            ),
            truncate(
                &sanitize_terminal(&candidate.restore),
                width.saturating_sub(4)
            )
        ));
    }

    if let Some(progress) = &app.deleting {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][progress.frame % 10];
        let path = sanitize_terminal(&progress.path);
        let path = truncate(&path, width.saturating_sub(8));
        output.push_str(&format!(
            "\n  {CYAN}{BOLD}{spinner} Deleting {}/{}{RESET}  {}\n  {DIM}Please wait. Do not close this terminal.{RESET}",
            progress.current, progress.total, path
        ));
    } else if app.confirm {
        output.push_str(&format!(
            "\n  {RED}{BOLD}Permanently delete{RESET} {} item{} ({})\n  press {BOLD}d{RESET} again or {BOLD}Enter{RESET} to confirm · Esc to cancel",
            app.selected_count(),
            if app.selected_count() == 1 { "" } else { "s" },
            format::bytes(app.selected_bytes())
        ));
    } else {
        let hints = truncate(
            "↑↓/wheel move   space select   a all-safe   n clear   f filter   s sort   d delete   q quit",
            width.saturating_sub(4),
        );
        let status = if !app.message.is_empty() {
            let message = sanitize_terminal(&app.message);
            format!("  {}\n", truncate(&message, width.saturating_sub(4)))
        } else if !app.warnings.0.is_empty() {
            format!(
                "  {YELLOW}{} scan warning(s) · w to view{RESET}\n",
                app.warnings.0.len()
            )
        } else {
            String::new()
        };
        output.push_str(&format!("\n{status}  {DIM}{hints}{RESET}"));
    }

    print!("{output}");
    io::stdout().flush()?;
    Ok(())
}

fn viewport_rows() -> usize {
    viewport_rows_for(terminal_size().1)
}

fn viewport_rows_for(height: usize) -> usize {
    // Title, summary, spacing, two detail rows, status, and key hints use the
    // remaining lines. Keeping one spare row prevents terminal auto-scroll.
    height.saturating_sub(10).max(3)
}

fn risk_color(risk: Risk) -> &'static str {
    match risk {
        Risk::Safe => GREEN,
        Risk::Review => YELLOW,
        Risk::Protected => RED,
    }
}

fn truncate(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    value.chars().take(width - 1).collect::<String>() + "…"
}

fn truncate_middle(value: &str, width: usize) -> String {
    let count = value.chars().count();
    if count <= width {
        return value.to_owned();
    }
    if width < 5 {
        return truncate(value, width);
    }
    let left = (width - 1) / 2;
    let right = width - left - 1;
    let start: String = value.chars().take(left).collect();
    let end: String = value.chars().skip(count - right).collect();
    format!("{start}…{end}")
}

fn sanitize_terminal(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || is_bidi_control(character) {
                '�'
            } else {
                character
            }
        })
        .collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(unix)]
fn terminal_size() -> (usize, usize) {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: ioctl writes only to the valid winsize pointer supplied here.
    let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    if result == 0 && size.ws_col > 0 && size.ws_row > 0 {
        (size.ws_col as usize, size.ws_row as usize)
    } else {
        (100, 24)
    }
}

#[cfg(not(unix))]
fn terminal_size() -> (usize, usize) {
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let height = std::env::var("LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24);
    (width, height)
}

#[cfg(unix)]
fn read_key() -> Result<Key> {
    let first = read_raw_byte()?;
    Ok(match first {
        b'\x03' => Key::Char('q'),
        b'\r' | b'\n' => Key::Enter,
        8 | 127 => Key::Backspace,
        27 => read_escape_sequence()?,
        byte if byte.is_ascii() => Key::Char(byte as char),
        _ => Key::Unknown,
    })
}

#[cfg(unix)]
fn read_escape_sequence() -> Result<Key> {
    // PTYs and multiplexed terminals can split a single cursor-key sequence
    // across scheduler ticks. A slightly generous timeout is only observable
    // when Escape is pressed by itself.
    if !stdin_ready(200) {
        return Ok(Key::Escape);
    }

    let mut sequence = Vec::with_capacity(8);
    while sequence.len() < 32 && stdin_ready(50) {
        let byte = read_raw_byte()?;
        sequence.push(byte);

        // CSI and SS3 sequences end with a byte in 0x40..=0x7e. The
        // introducer itself (`[` or `O`) must not be treated as the end.
        if sequence.len() >= 2 && (0x40..=0x7e).contains(&byte) {
            break;
        }
    }
    Ok(parse_escape_sequence(&sequence))
}

#[cfg(unix)]
fn read_raw_byte() -> io::Result<u8> {
    let mut byte = 0_u8;
    // SAFETY: read receives a valid pointer to a one-byte writable buffer.
    let count = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            (&mut byte as *mut u8).cast::<libc::c_void>(),
            1,
        )
    };
    if count == 1 {
        Ok(byte)
    } else if count == 0 {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "terminal input closed",
        ))
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn stdin_ready(timeout_ms: i32) -> bool {
    let mut descriptor = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll receives a valid pointer to one pollfd value.
    unsafe { libc::poll(&mut descriptor, 1, timeout_ms) > 0 }
}

#[cfg(unix)]
fn parse_escape_sequence(sequence: &[u8]) -> Key {
    match sequence {
        // Normal cursor mode and application cursor mode.
        [b'[', b'A'] | [b'O', b'A'] => Key::Up,
        [b'[', b'B'] | [b'O', b'B'] => Key::Down,
        // Page Up / Page Down are useful for terminals that map wheel events
        // to paging rather than mouse-reporting sequences.
        [b'[', b'5', b'~'] => Key::PageUp,
        [b'[', b'6', b'~'] => Key::PageDown,
        sequence if sgr_wheel_direction(sequence) == Some(true) => Key::Up,
        sequence if sgr_wheel_direction(sequence) == Some(false) => Key::Down,
        // Mouse reports and every other escape sequence are ignored. They
        // must never be interpreted as a bare Escape key.
        _ => Key::Unknown,
    }
}

#[cfg(unix)]
fn sgr_wheel_direction(sequence: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(sequence).ok()?;
    let button = text
        .strip_prefix("[<")?
        .split(';')
        .next()?
        .parse::<u16>()
        .ok()?;
    match button & 0b1100_0001 {
        64 => Some(true),
        65 => Some(false),
        _ => None,
    }
}

#[cfg(not(unix))]
fn read_key() -> Result<Key> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let value = line.trim().to_owned();
    Ok(if value.chars().count() == 1 {
        Key::Char(value.chars().next().unwrap_or_default())
    } else {
        Key::Text(value)
    })
}

struct TerminalSession {
    #[cfg(unix)]
    original: libc::termios,
}

impl TerminalSession {
    fn enter() -> Result<Self> {
        #[cfg(unix)]
        {
            // SAFETY: tcgetattr/tcsetattr operate on stdin and initialized termios values.
            unsafe {
                let mut original = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                    return Err(io::Error::last_os_error().into());
                }
                let mut raw = original;
                libc::cfmakeraw(&mut raw);
                // Keep input raw, but preserve normal newline rendering. Without
                // OPOST/ONLCR every `\n` continues at the current column.
                raw.c_oflag |= libc::OPOST | libc::ONLCR;
                if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                    return Err(io::Error::last_os_error().into());
                }
                print!("\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1006h\x1b[?1007h");
                io::stdout().flush()?;
                Ok(Self { original })
            }
        }
        #[cfg(not(unix))]
        {
            print!("\x1b[?1049h\x1b[?25l");
            io::stdout().flush()?;
            Ok(Self {})
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: original was captured from the same stdin descriptor on entry.
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
        print!("\x1b[?1007l\x1b[?1006l\x1b[?1000l\x1b[?25h\x1b[?1049l{RESET}");
        let _ = io::stdout().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_long_paths_from_the_middle() {
        assert_eq!(truncate_middle("/one/two/three/four", 11), "/one/…/four");
        assert_eq!(truncate_middle("short", 20), "short");
    }

    #[test]
    fn removes_terminal_and_bidi_controls_from_untrusted_text() {
        assert_eq!(
            sanitize_terminal("safe\x1b]52;c;secret\x07\u{202e}txt"),
            "safe�]52;c;secret��txt"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recognizes_arrow_modes_and_mouse_wheel() {
        assert!(matches!(parse_escape_sequence(b"[A"), Key::Up));
        assert!(matches!(parse_escape_sequence(b"OB"), Key::Down));
        assert!(matches!(parse_escape_sequence(b"[<64;10;5M"), Key::Up));
        assert!(matches!(parse_escape_sequence(b"[<65;10;5M"), Key::Down));
        assert!(matches!(parse_escape_sequence(b"[<0;10;5M"), Key::Unknown));
    }
}
