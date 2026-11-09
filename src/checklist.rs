//! The install checklist: a pure model of the lines and their ticks, its
//! ratatui rendering, and the terminal session that runs it: raw mode, an
//! inline viewport on stderr, and the restore on every way out.

use crate::audit::Assessment;
use crate::catalog::Catalog;
use crate::exit::Exit;
use crate::manifest::Verdict;
use crate::ops::use_color;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::flag;
use signal_hook::SigId;
use std::io::{IsTerminal, Stderr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const KEYS_LINE: &str = "j/k move  space tick  enter review  q cancel";
const HEADER_TITLE: &str = "Choose what to install";
const HEADER_AUDITED: &str = "Ticked: what the audit of this repository recommends.";
const HEADER_UNAUDITED: &str = "Not in a git repository: nothing is pre-ticked.";
const REPO_LINE: &str = "also set up this repository";
const EXPERIMENT_STATE: &str = "experiment: not ticked by default";
const INSTALLED_STATE: &str = "installed";
const UNPUBLISHED_STATE: &str = "not published yet";
const NOTHING_TICKED: &str = "nothing ticked";
const CURSOR: &str = "> ";
const NO_CURSOR: &str = "  ";
const MIN_ROWS: u16 = 8;
const MIN_COLUMNS: u16 = 40;
const SIGNAL_POLL: Duration = Duration::from_millis(50);
/// Printed before the part listing when the terminal is too small to open in.
pub const TOO_SMALL: &str =
    "terminal too small for the checklist (needs 8 rows, 40 columns): enlarge it or name the parts";
const BOX_WIDTH: usize = 4;
const STATE_GAP: usize = 2;

/// A key the checklist understands; the terminal layer maps its events here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
    ControlC,
    Char(char),
}

/// What the caller does after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Stay,
    Confirm { parts: Vec<String>, repo: bool },
    Cancel,
    Interrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Open,
    Installed,
    Unpublished,
}

#[derive(Clone, Debug)]
struct PartLine {
    name: String,
    summary: String,
    availability: Availability,
    experimental: bool,
    needs: Vec<String>,
    enhances: Vec<String>,
    /// Audit's reason, when it recommends the part.
    reason: Option<String>,
    ticked: bool,
}

impl PartLine {
    fn is_toggleable(&self) -> bool {
        self.availability == Availability::Open
    }
}

/// The checklist's state: one line per catalog part, then the repository line
/// when there is a repository.
#[derive(Clone, Debug)]
pub struct Checklist {
    parts: Vec<PartLine>,
    repo: Option<bool>,
    cursor: usize,
    notice: Option<String>,
}

impl Checklist {
    /// Lines in catalog order, pre-ticked from `assessment`; `installed` are
    /// the receipt's part names.
    pub fn new(
        catalog: &Catalog,
        assessment: Option<&Assessment<'_>>,
        installed: &[String],
    ) -> Checklist {
        let parts = catalog
            .parts
            .iter()
            .map(|part| {
                let availability = if installed.iter().any(|name| name == part.name()) {
                    Availability::Installed
                } else if !part.is_published() {
                    Availability::Unpublished
                } else {
                    Availability::Open
                };
                let reason = assessment.and_then(|found| {
                    found
                        .rows
                        .iter()
                        .find(|row| row.part == part.name() && row.verdict == Verdict::Recommend)
                        .map(|row| row.reason.clone())
                        .filter(|reason| !reason.is_empty())
                });
                PartLine {
                    name: part.name().to_string(),
                    summary: part.summary().to_string(),
                    availability,
                    experimental: part.is_experimental(),
                    needs: part.needs().to_vec(),
                    enhances: part.enhances().to_vec(),
                    reason,
                    ticked: false,
                }
            })
            .collect();
        let mut checklist = Checklist {
            parts,
            repo: assessment.map(|_| true),
            cursor: 0,
            notice: None,
        };
        if let Some(assessment) = assessment {
            for name in &assessment.install {
                if let Some(index) = checklist.index_of(name) {
                    if !checklist.parts[index].experimental {
                        checklist.tick(index);
                    }
                }
            }
        }
        checklist
    }

    /// Apply one key.
    pub fn on_key(&mut self, key: Key) -> Action {
        self.notice = None;
        match key {
            Key::Up | Key::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                Action::Stay
            }
            Key::Down | Key::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.last_index());
                Action::Stay
            }
            Key::Char(' ') => {
                self.toggle_cursor();
                Action::Stay
            }
            Key::Enter => self.confirm(),
            Key::Escape | Key::Char('q') => Action::Cancel,
            Key::ControlC => Action::Interrupt,
            Key::Char(_) => Action::Stay,
        }
    }

    fn last_index(&self) -> usize {
        (self.parts.len() + usize::from(self.repo.is_some())).saturating_sub(1)
    }

    fn index_of(&self, name: &str) -> Option<usize> {
        self.parts.iter().position(|line| line.name == name)
    }

    /// The first ticked part that needs the part at `index`.
    fn needed_by(&self, index: usize) -> Option<&str> {
        let name = &self.parts[index].name;
        self.parts
            .iter()
            .find(|line| line.ticked && line.needs.contains(name))
            .map(|line| line.name.as_str())
    }

    /// Tick a toggleable part and, through its needs, what it needs.
    fn tick(&mut self, index: usize) {
        if !self.parts[index].is_toggleable() || self.parts[index].ticked {
            return;
        }
        self.parts[index].ticked = true;
        for need in self.parts[index].needs.clone() {
            if let Some(needed) = self.index_of(&need) {
                self.tick(needed);
            }
        }
    }

    fn toggle_cursor(&mut self) {
        let Some(line) = self.parts.get(self.cursor) else {
            self.repo = self.repo.map(|ticked| !ticked);
            return;
        };
        match line.availability {
            Availability::Installed => {
                self.notice = Some(format!("{} is {INSTALLED_STATE}", line.name));
            }
            Availability::Unpublished => {
                self.notice = Some(format!("{} is {UNPUBLISHED_STATE}", line.name));
            }
            Availability::Open if line.ticked => {
                if let Some(owner) = self.needed_by(self.cursor) {
                    self.notice = Some(format!("needed by {owner}: untick {owner} first"));
                } else {
                    self.parts[self.cursor].ticked = false;
                }
            }
            Availability::Open => self.tick(self.cursor),
        }
    }

    fn confirm(&mut self) -> Action {
        let parts: Vec<String> = self
            .parts
            .iter()
            .filter(|line| line.ticked)
            .map(|line| line.name.clone())
            .collect();
        if parts.is_empty() {
            self.notice = Some(NOTHING_TICKED.to_string());
            return Action::Stay;
        }
        Action::Confirm {
            parts,
            repo: self.repo.unwrap_or(false),
        }
    }
}

/// One checklist entry as the words around its box.
struct Entry {
    mark: &'static str,
    name: String,
    summary: String,
    state: String,
    selected: bool,
    tone: Tone,
}

#[derive(Clone, Copy)]
enum Tone {
    Ticked,
    Plain,
    Dim,
    Experiment,
}

impl Tone {
    fn style(self, colour: bool) -> Style {
        if !colour {
            return Style::default();
        }
        match self {
            Tone::Ticked => Style::default().fg(Color::Green),
            Tone::Plain => Style::default(),
            Tone::Dim => Style::default().fg(Color::DarkGray),
            Tone::Experiment => Style::default().fg(Color::Yellow),
        }
    }
}

fn entries(model: &Checklist) -> Vec<Entry> {
    let mut entries: Vec<Entry> = model
        .parts
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let mut states: Vec<String> = Vec::new();
            match line.availability {
                Availability::Installed => states.push(INSTALLED_STATE.to_string()),
                Availability::Unpublished => states.push(UNPUBLISHED_STATE.to_string()),
                Availability::Open => states.extend(line.reason.clone()),
            }
            if line.experimental {
                states.push(EXPERIMENT_STATE.to_string());
            }
            if let Some(owner) = model.needed_by(index) {
                states.push(format!("needed by {owner}"));
            }
            for partner in &line.enhances {
                states.push(format!("works with {partner}"));
            }
            let (mark, tone) = match (line.availability, line.ticked) {
                (Availability::Open, true) => ("[x]", Tone::Ticked),
                (Availability::Open, false) if line.experimental => ("[ ]", Tone::Experiment),
                (Availability::Open, false) => ("[ ]", Tone::Plain),
                _ => ("[-]", Tone::Dim),
            };
            Entry {
                mark,
                name: line.name.clone(),
                summary: line.summary.clone(),
                state: states.join("; "),
                selected: index == model.cursor,
                tone,
            }
        })
        .collect();
    if let Some(ticked) = model.repo {
        entries.push(Entry {
            mark: if ticked { "[x]" } else { "[ ]" },
            name: REPO_LINE.to_string(),
            summary: String::new(),
            state: String::new(),
            selected: model.cursor == model.parts.len(),
            tone: if ticked { Tone::Ticked } else { Tone::Plain },
        });
    }
    entries
}

/// Greedy word wrap: the first row holds `first` columns, the others `rest`.
/// A word longer than its row is split.
fn wrap_words(text: &str, first: usize, rest: usize) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        loop {
            let width = if rows.is_empty() { first } else { rest }.max(1);
            let used = current.chars().count();
            let length = word.chars().count();
            if used == 0 && length <= width {
                current = word;
                break;
            }
            if used == 0 {
                rows.push(word.chars().take(width).collect());
                word = word.chars().skip(width).collect();
            } else if used + 1 + length <= width {
                current.push(' ');
                current.push_str(&word);
                break;
            } else {
                rows.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() || rows.is_empty() {
        rows.push(current);
    }
    rows
}

/// The display rows of one entry, each exactly `width` columns wide.
fn entry_rows(entry: &Entry, name_width: usize, width: usize, colour: bool) -> Vec<Line<'static>> {
    let cursor = if entry.selected { CURSOR } else { NO_CURSOR };
    let indent = cursor.len() + BOX_WIDTH;
    let head = format!("{cursor}{} ", entry.mark);
    let mut texts: Vec<String> = Vec::new();
    if entry.summary.is_empty() {
        texts.push(format!("{head}{}", entry.name));
    } else {
        let name = format!("{:<name_width$}", entry.name);
        let first_len = head.chars().count() + name.chars().count() + 1;
        let summary_len = entry.summary.chars().count();
        let state_len = entry.state.chars().count();
        if state_len > 0 && first_len + summary_len + STATE_GAP + state_len <= width {
            let gap = width - first_len - summary_len - state_len;
            texts.push(format!(
                "{head}{name} {}{}{}",
                entry.summary,
                " ".repeat(gap),
                entry.state
            ));
        } else {
            let room = width.saturating_sub(indent);
            let summary = wrap_words(&entry.summary, width.saturating_sub(first_len), room);
            for (row, text) in summary.into_iter().enumerate() {
                if row == 0 {
                    texts.push(format!("{head}{name} {text}"));
                } else {
                    texts.push(format!("{}{text}", " ".repeat(indent)));
                }
            }
            if state_len > 0 {
                for text in wrap_words(&entry.state, room, room) {
                    texts.push(format!("{}{text}", " ".repeat(indent)));
                }
            }
        }
    }
    let mut style = entry.tone.style(colour);
    if entry.selected {
        style = style.add_modifier(Modifier::REVERSED);
    }
    texts
        .into_iter()
        .map(|text| {
            let padded = format!("{text:<width$}");
            let fitted: String = padded.chars().take(width).collect();
            Line::from(Span::styled(fitted, style))
        })
        .collect()
}

/// The rows to show in `body` rows of room, scrolled so the cursor entry is
/// visible.
fn visible_rows(
    rows: &[Vec<Line<'static>>],
    cursor_entry: usize,
    body: usize,
) -> Vec<Line<'static>> {
    let start: usize = rows[..cursor_entry].iter().map(Vec::len).sum();
    let end = start + rows[cursor_entry].len();
    let offset = end.saturating_sub(body).min(start);
    rows.iter()
        .flatten()
        .skip(offset)
        .take(body)
        .cloned()
        .collect()
}

fn body_rows(model: &Checklist, width: usize, colour: bool) -> Vec<Vec<Line<'static>>> {
    let all = entries(model);
    let name_width = all
        .iter()
        .filter(|entry| !entry.summary.is_empty())
        .map(|entry| entry.name.chars().count())
        .max()
        .unwrap_or(0);
    all.iter()
        .map(|entry| entry_rows(entry, name_width, width, colour))
        .collect()
}

fn audited_text(model: &Checklist) -> &'static str {
    if model.repo.is_some() {
        HEADER_AUDITED
    } else {
        HEADER_UNAUDITED
    }
}

/// How many rows the whole checklist wants at `width` columns: the header,
/// every entry and the footer.
fn wanted_height(model: &Checklist, width: usize) -> usize {
    let header = 1 + wrap_words(audited_text(model), width, width).len();
    let body: usize = body_rows(model, width, false).iter().map(Vec::len).sum();
    header + body + 1
}

/// Draw the checklist into `frame`; `colour` adds named ANSI colours, never
/// meaning: every state is also a word or a glyph.
pub fn draw(frame: &mut Frame<'_>, model: &Checklist, colour: bool) {
    let area = frame.area();
    render(area, frame.buffer_mut(), model, colour);
}

fn render(area: Rect, buffer: &mut Buffer, model: &Checklist, colour: bool) {
    let width = usize::from(area.width);
    Clear.render(area, buffer);
    let audited = audited_text(model);
    let mut header: Vec<Line<'static>> = vec![Line::from(Span::styled(
        HEADER_TITLE.to_string(),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    header.extend(
        wrap_words(audited, width, width)
            .into_iter()
            .map(Line::from),
    );
    let header_rows = u16::try_from(header.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    Paragraph::new(header).render(
        Rect {
            height: header_rows,
            ..area
        },
        buffer,
    );

    if area.height > header_rows {
        let footer = Rect {
            y: area.y + area.height - 1,
            height: 1,
            ..area
        };
        let footer_text = model.notice.as_deref().unwrap_or(KEYS_LINE);
        Paragraph::new(Line::from(footer_text.to_string())).render(footer, buffer);
    }

    let body_height = area.height.saturating_sub(header_rows + 1);
    if body_height == 0 {
        return;
    }
    let rows = body_rows(model, width, colour);
    let shown = visible_rows(&rows, model.cursor, usize::from(body_height));
    let body = Rect {
        y: area.y + header_rows,
        height: body_height,
        ..area
    };
    Paragraph::new(shown).render(body, buffer);
}

/// What opening the checklist came to, short of an exit.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// The terminal cannot host it (the cursor read failed): show the listing.
    Unavailable,
    /// Under 8 rows or 40 columns: say so, then show the listing.
    TooSmall,
    Confirmed {
        parts: Vec<String>,
        repo: bool,
    },
    Cancelled,
}

/// Whether the checklist may open at all: stdin, stdout and stderr are
/// terminals and `TERM` is not `dumb`.
pub fn terminal_available() -> bool {
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal()
        && std::env::var("TERM").map_or(true, |term| term != "dumb")
}

/// SIGTERM and SIGHUP as flags the event loop polls. Dropping it unregisters
/// the flag actions and arms the default action, so a signal after the
/// checklist (at `Proceed?`, during the install) ends grh as it would have.
#[derive(Debug)]
struct Signals {
    terminated: Arc<AtomicBool>,
    hung_up: Arc<AtomicBool>,
    default_action: Arc<AtomicBool>,
    flag_ids: Vec<SigId>,
}

impl Signals {
    fn register() -> Result<Signals, Exit> {
        let terminated = Arc::new(AtomicBool::new(false));
        let hung_up = Arc::new(AtomicBool::new(false));
        let default_action = Arc::new(AtomicBool::new(false));
        let mut flag_ids = Vec::new();
        for (signal, seen) in [(SIGTERM, &terminated), (SIGHUP, &hung_up)] {
            // Registered first, so it runs first: a no-op until `default_action` is set.
            flag::register_conditional_default(signal, Arc::clone(&default_action))
                .map_err(|e| signal_error(&e))?;
            flag_ids.push(flag::register(signal, Arc::clone(seen)).map_err(|e| signal_error(&e))?);
        }
        Ok(Signals {
            terminated,
            hung_up,
            default_action,
            flag_ids,
        })
    }

    fn pending(&self) -> Option<Exit> {
        if self.hung_up.load(Ordering::Relaxed) {
            Some(Exit::HungUp)
        } else if self.terminated.load(Ordering::Relaxed) {
            Some(Exit::Terminated)
        } else {
            None
        }
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        self.default_action.store(true, Ordering::Relaxed);
        for id in self.flag_ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

fn signal_error(error: &std::io::Error) -> Exit {
    Exit::Failed(format!("cannot watch for signals: {error}"))
}

/// Puts the terminal back: raw mode off, cursor shown. Runs on every way out.
#[derive(Debug)]
struct RestoreGuard {
    /// True while the checklist holds the terminal; the panic hook restores only then.
    holding: Arc<AtomicBool>,
}

fn restore_terminal() {
    let _ = disable_raw_mode(); // best effort: a failed restore, EIO after a hang-up, leaves nothing to do
    let _ = crossterm::execute!(std::io::stderr(), crossterm::cursor::Show); // best effort, as above
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        self.holding.store(false, Ordering::Relaxed);
        restore_terminal();
    }
}

fn key_of(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(Key::ControlC),
        KeyCode::Char(character) => Some(Key::Char(character)),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Escape),
        _ => None,
    }
}

fn terminal_error(error: &std::io::Error) -> Exit {
    Exit::Failed(format!("the terminal failed: {error}"))
}

/// Open the checklist on the terminal and run it until the person decides.
/// Call it only when `terminal_available`; it draws on stderr.
pub fn open(model: Checklist) -> Result<Opened, Exit> {
    let Ok((columns, rows)) = crossterm::terminal::size() else {
        return Ok(Opened::Unavailable);
    };
    if rows < MIN_ROWS || columns < MIN_COLUMNS {
        return Ok(Opened::TooSmall);
    }
    // Declared first, so dropped last: after the terminal is back.
    let signals = Signals::register()?;
    let holding = Arc::new(AtomicBool::new(true));
    let previous_hook = std::panic::take_hook();
    let hook_holding = Arc::clone(&holding);
    std::panic::set_hook(Box::new(move |info| {
        if hook_holding.load(Ordering::Relaxed) {
            restore_terminal();
        }
        previous_hook(info);
    }));
    let guard = RestoreGuard { holding };
    enable_raw_mode().map_err(|e| terminal_error(&e))?;
    let height = u16::try_from(wanted_height(&model, usize::from(columns)))
        .unwrap_or(u16::MAX)
        .clamp(MIN_ROWS, rows);
    let Ok(mut terminal) = Terminal::with_options(
        CrosstermBackend::new(std::io::stderr()),
        TerminalOptions {
            viewport: Viewport::Inline(height),
        },
    ) else {
        return Ok(Opened::Unavailable);
    };
    let outcome = event_loop(&mut terminal, model, &signals);
    if !matches!(outcome, Err(Exit::HungUp | Exit::Terminated)) {
        drain_pending_events();
    }
    // The terminal may be gone after a hang-up; the clear is best effort then.
    let cleared = terminal.clear();
    drop(terminal);
    drop(guard);
    let opened = outcome?;
    cleared.map_err(|e| terminal_error(&e))?;
    Ok(opened)
}

/// Read and drop the keys already waiting, so a held or doubled enter cannot
/// answer `Proceed?`.
fn drain_pending_events() {
    while event::poll(Duration::ZERO).unwrap_or(false) {
        if event::read().is_err() {
            break;
        }
    }
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stderr>>,
    mut model: Checklist,
    signals: &Signals,
) -> Result<Opened, Exit> {
    let colour = use_color(true);
    loop {
        if let Some(exit) = signals.pending() {
            return Err(exit);
        }
        let drawn = terminal.draw(|frame| draw(frame, &model, colour));
        if let Err(error) = drawn {
            return Err(signals.pending().unwrap_or_else(|| terminal_error(&error)));
        }
        loop {
            if let Some(exit) = signals.pending() {
                return Err(exit);
            }
            match event::poll(SIGNAL_POLL) {
                Ok(false) => continue,
                Ok(true) => {}
                Err(error) => {
                    return Err(signals.pending().unwrap_or_else(|| terminal_error(&error)));
                }
            }
            let event = match event::read() {
                Ok(event) => event,
                Err(error) => {
                    return Err(signals.pending().unwrap_or_else(|| terminal_error(&error)));
                }
            };
            match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    let Some(key) = key_of(key.code, key.modifiers) else {
                        continue;
                    };
                    match model.on_key(key) {
                        Action::Stay => {}
                        Action::Confirm { parts, repo } => {
                            return Ok(Opened::Confirmed { parts, repo });
                        }
                        Action::Cancel => return Ok(Opened::Cancelled),
                        Action::Interrupt => return Err(Exit::Interrupted),
                    }
                    break;
                }
                Event::Resize(_, _) => break,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const CATALOG_HEAD: &str = r#"
skills_repo = "o/r"
skills_version = "1"
skills_sha256 = "0"
foreign_markers = ["other:begin"]
order = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]
"#;

    fn part(name: &str, summary: &str, extra: &str) -> String {
        format!(
            "[[part]]\n[part.extension]\nname = \"{name}\"\nversion = \"1.0.0\"\nharness = \">=0.4\"\nsummary = \"{summary}\"\n{extra}\n"
        )
    }

    fn catalog() -> Catalog {
        let source = format!(
            "{CATALOG_HEAD}{}{}{}{}{}{}",
            part("alpha", "the first thing a person runs", "needs = []"),
            part(
                "beta",
                "a second thing that wants the first",
                "needs = [\"alpha\"]"
            ),
            part("gamma", "already on this machine", "needs = []"),
            part("delta", "not out yet", "needs = []\npublished = false"),
            part(
                "epsilon",
                "a risky try-out",
                "needs = []\nexperimental = true"
            ),
            part(
                "zeta",
                "pairs well with the first",
                "needs = []\nenhances = [\"alpha\"]"
            ),
        );
        Catalog::parse(&source).unwrap_or_else(|e| panic!("fixture catalog: {e:?}"))
    }

    fn model_in(cat: &Catalog, install: &[&str], installed: &[&str]) -> Checklist {
        let install: Vec<&str> = install
            .iter()
            .map(|name| cat.part(name).unwrap().name())
            .collect();
        let found = Assessment {
            root: std::path::PathBuf::from("."),
            languages: Default::default(),
            rows: Vec::new(),
            hooks: Vec::new(),
            warnings: Vec::new(),
            adr_folder: None,
            ci: Vec::new(),
            signals: Default::default(),
            install,
            experimental: cat.experimental_names(),
        };
        let installed: Vec<String> = installed.iter().map(|s| s.to_string()).collect();
        Checklist::new(cat, Some(&found), &installed)
    }

    #[test]
    fn a_recommended_line_shows_the_audit_reason() {
        let cat = catalog();
        let found = Assessment {
            root: std::path::PathBuf::from("."),
            languages: Default::default(),
            rows: vec![crate::audit::Row {
                part: cat.part("alpha").unwrap().name(),
                verdict: Verdict::Recommend,
                reason: "a git repository".to_string(),
            }],
            hooks: Vec::new(),
            warnings: Vec::new(),
            adr_folder: None,
            ci: Vec::new(),
            signals: Default::default(),
            install: vec![cat.part("alpha").unwrap().name()],
            experimental: cat.experimental_names(),
        };
        let checklist = Checklist::new(&cat, Some(&found), &[]);
        let rows = screen(&checklist, 100, 24, false);
        let alpha = rows.iter().find(|r| r.contains("alpha")).unwrap();
        assert!(
            alpha.contains("[x]") && alpha.contains("a git repository"),
            "{rows:#?}"
        );
    }

    fn model(install: &[&str], installed: &[&str]) -> Checklist {
        model_in(&catalog(), install, installed)
    }

    fn ticked(model: &Checklist) -> Vec<&str> {
        model
            .parts
            .iter()
            .filter(|line| line.ticked)
            .map(|line| line.name.as_str())
            .collect()
    }

    fn move_to(model: &mut Checklist, name: &str) {
        model.cursor = model.index_of(name).unwrap();
    }

    #[test]
    fn pre_ticks_follow_the_install_list_and_never_an_experiment() {
        let checklist = model(&["zeta", "epsilon"], &[]);
        assert_eq!(ticked(&checklist), vec!["zeta"]);
        assert_eq!(checklist.repo, Some(true));
    }

    #[test]
    fn outside_a_repository_nothing_is_ticked_and_no_repo_line() {
        let cat = catalog();
        let checklist = Checklist::new(&cat, None, &[]);
        assert!(ticked(&checklist).is_empty());
        assert_eq!(checklist.repo, None);
        assert_eq!(checklist.last_index(), checklist.parts.len() - 1);
        let rows = screen(&checklist, 80, 24, false);
        assert!(rows.iter().any(|r| r.contains(HEADER_UNAUDITED)));
        assert!(!rows.iter().any(|r| r.contains(REPO_LINE)));
    }

    #[test]
    fn space_on_installed_and_unpublished_lines_changes_nothing() {
        let mut checklist = model(&[], &["gamma"]);
        move_to(&mut checklist, "gamma");
        assert_eq!(checklist.on_key(Key::Char(' ')), Action::Stay);
        assert!(ticked(&checklist).is_empty());
        assert_eq!(checklist.notice.as_deref(), Some("gamma is installed"));
        move_to(&mut checklist, "delta");
        checklist.on_key(Key::Char(' '));
        assert!(ticked(&checklist).is_empty());
        assert_eq!(
            checklist.notice.as_deref(),
            Some("delta is not published yet")
        );
        checklist.on_key(Key::Char('j'));
        assert_eq!(checklist.notice, None);
    }

    #[test]
    fn ticking_a_part_ticks_its_needs_and_a_needed_part_cannot_be_unticked() {
        let mut checklist = model(&[], &[]);
        move_to(&mut checklist, "beta");
        checklist.on_key(Key::Char(' '));
        assert_eq!(ticked(&checklist), vec!["alpha", "beta"]);
        move_to(&mut checklist, "alpha");
        checklist.on_key(Key::Char(' '));
        assert_eq!(ticked(&checklist), vec!["alpha", "beta"]);
        assert_eq!(
            checklist.notice.as_deref(),
            Some("needed by beta: untick beta first")
        );
        move_to(&mut checklist, "beta");
        checklist.on_key(Key::Char(' '));
        move_to(&mut checklist, "alpha");
        checklist.on_key(Key::Char(' '));
        assert!(ticked(&checklist).is_empty());
    }

    #[test]
    fn enter_with_nothing_ticked_stays_with_a_notice() {
        let mut checklist = model(&[], &[]);
        assert_eq!(checklist.on_key(Key::Enter), Action::Stay);
        assert_eq!(checklist.notice.as_deref(), Some("nothing ticked"));
    }

    #[test]
    fn enter_confirms_the_ticked_parts_in_catalog_order_with_the_repo_flag() {
        let mut checklist = model(&["zeta", "beta"], &[]);
        let expected: Vec<String> = ["alpha", "beta", "zeta"].map(String::from).to_vec();
        assert_eq!(
            checklist.on_key(Key::Enter),
            Action::Confirm {
                parts: expected.clone(),
                repo: true
            }
        );
        checklist.cursor = checklist.last_index();
        checklist.on_key(Key::Char(' '));
        assert_eq!(
            checklist.on_key(Key::Enter),
            Action::Confirm {
                parts: expected,
                repo: false
            }
        );
    }

    #[test]
    fn cancel_and_interrupt_keys() {
        let mut checklist = model(&[], &[]);
        assert_eq!(checklist.on_key(Key::Char('q')), Action::Cancel);
        assert_eq!(checklist.on_key(Key::Escape), Action::Cancel);
        assert_eq!(checklist.on_key(Key::ControlC), Action::Interrupt);
    }

    #[test]
    fn the_cursor_stays_inside_the_list() {
        let mut checklist = model(&[], &[]);
        checklist.on_key(Key::Up);
        assert_eq!(checklist.cursor, 0);
        for _ in 0..20 {
            checklist.on_key(Key::Char('j'));
        }
        assert_eq!(checklist.cursor, checklist.last_index());
        checklist.on_key(Key::Char('k'));
        assert_eq!(checklist.cursor, checklist.last_index() - 1);
    }

    fn screen(checklist: &Checklist, width: u16, height: u16, colour: bool) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, checklist, colour))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn the_experiment_text_is_on_the_experimental_row_at_80_columns() {
        let rows = screen(&model(&["alpha"], &[]), 80, 24, true);
        let row = rows.iter().find(|r| r.contains("epsilon")).unwrap();
        assert!(row.contains(EXPERIMENT_STATE), "{row:?}");
        assert!(rows.iter().any(|r| r.contains(KEYS_LINE)));
    }

    #[test]
    fn at_60_columns_the_state_moves_to_an_indented_second_line() {
        let rows = screen(&model(&["alpha"], &[]), 60, 24, false);
        assert!(rows.iter().all(|r| r.chars().count() <= 60));
        let at = rows.iter().position(|r| r.contains("epsilon")).unwrap();
        assert!(!rows[at].contains(EXPERIMENT_STATE), "{:?}", rows[at]);
        assert!(rows[at + 1].starts_with("      "), "{:?}", rows[at + 1]);
        assert!(
            rows[at + 1].contains(EXPERIMENT_STATE),
            "{:?}",
            rows[at + 1]
        );
    }

    #[test]
    fn at_80x10_the_cursor_row_is_visible_after_moving_to_the_last_line() {
        let mut checklist = model(&["alpha"], &[]);
        for _ in 0..20 {
            checklist.on_key(Key::Down);
        }
        let rows = screen(&checklist, 80, 10, false);
        assert_eq!(rows.len(), 10);
        assert!(
            rows.iter()
                .any(|r| r.starts_with("> ") && r.contains(REPO_LINE)),
            "{rows:#?}"
        );
        assert!(rows[9].contains(KEYS_LINE));
    }

    #[test]
    fn without_colour_the_selection_is_reversed_and_every_state_has_a_word() {
        let checklist = model(&["alpha"], &["gamma"]);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw(frame, &checklist, false))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let selected = (0..24u16)
            .find(|&y| buffer[(0, y)].symbol() == ">")
            .unwrap();
        assert!(buffer[(0, selected)].modifier.contains(Modifier::REVERSED));
        assert!(buffer[(79, selected)].modifier.contains(Modifier::REVERSED));
        for y in 0..24u16 {
            for x in 0..80u16 {
                assert_eq!(buffer[(x, y)].fg, Color::Reset, "colour at {x},{y}");
            }
        }
        let rows = screen(&checklist, 80, 24, false);
        for word in [
            INSTALLED_STATE,
            UNPUBLISHED_STATE,
            EXPERIMENT_STATE,
            "works with alpha",
            "[x]",
            "[ ]",
            "[-]",
        ] {
            assert!(rows.iter().any(|r| r.contains(word)), "missing {word}");
        }
    }

    #[test]
    fn needed_by_shows_on_the_needed_row() {
        let mut checklist = model(&[], &[]);
        move_to(&mut checklist, "beta");
        checklist.on_key(Key::Char(' '));
        let rows = screen(&checklist, 80, 24, false);
        let row = rows.iter().find(|r| r.contains("alpha")).unwrap();
        assert!(row.contains("needed by beta"), "{row:?}");
    }
}
