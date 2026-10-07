//! App state and key handling. `App::key` is the only way input changes
//! state; `App::tick` pulls events from the source, keeps an open terminal
//! current and reconnects after a disconnect. Navigation is a stack of
//! views, so `←` always returns to the exact screen and selection it came
//! from, including after `t` and `/`.
//!
//! The terminal view (gate 11 B P5, P6) is live: PTY output only marks the
//! screen stale, and the next tick at least [`REFRESH_EVERY`] after the last
//! fetch asks for the holder's screen again, so the last output is always
//! drawn. A terminal that ended is subscribed again every [`RETRY_EVERY`].
//! It is read-only until `i`; while typing, every key goes to the agent
//! except `Ctrl-]` (or `Ctrl-5`, how some terminals report it), which
//! stops. A disconnect, an ended terminal, an error or leaving the view
//! stops typing, and it never comes back by itself.
//!
//! Must NOT: know which `Source` it has, change needs-you state except
//! through the source and the daemon's events, or decide who may type
//! (the daemon does, D17).

pub mod full_terminal;
use agend_core::protocol::terminal::TerminalSize;
use full_terminal::FullView;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::{AttentionAction, error_code};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::i18n::{Language, Text};
use crate::source::{AgentState, Choice, Fleet, Source, SourceError, TerminalEvent};
use crate::ui::Row;
use crate::{agent_detail, attention, finder, home, task_detail, team, terminal};

/// Least time between two screen fetches of a live terminal (P5).
pub const REFRESH_EVERY: Duration = Duration::from_millis(200);
/// How often an ended terminal is subscribed again (P5).
pub const RETRY_EVERY: Duration = Duration::from_secs(1);
/// How often a lost daemon is tried again (T6, P7); `r` tries at once.
pub const RECONNECT_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Goals,
    Agents,
    Pipeline,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Goals, Tab::Agents, Tab::Pipeline];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    Home,
    NeedsYou,
    Team { team: String, tab: Tab },
    Task { task: String },
    Agent { agent: String },
    Terminal { agent: String, screen: String },
}

/// What a selectable row opens. Selections are kept by these stable ids, not
/// by row index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Home,
    /// A needs-you item by `Attention::key`.
    Item(String),
    /// Option `n` of a needs-you ask.
    Choice(String, usize),
    Team(String),
    Task(String),
    /// Stage `n` of a task.
    Stage(String, usize),
    Agent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub screen: Screen,
    pub selected: Option<Target>,
    /// Row index of the selection, used to pick a neighbour when it vanishes.
    index: usize,
    /// The selection was picked automatically (nothing chosen yet), so it
    /// follows the first row as data arrives: the first needs-you item is
    /// selected at launch even if events arrive after the first frame.
    auto: bool,
    /// First visible row.
    pub offset: usize,
    /// A terminal view shows its last rows (the newest output) until the
    /// operator scrolls up; scrolling back to the end follows again.
    follow: bool,
}

impl View {
    fn new(screen: Screen, selected: Option<Target>) -> View {
        View {
            screen,
            auto: selected.is_none(),
            selected,
            index: 0,
            offset: 0,
            follow: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    Connected,
    Disconnected {
        reason: String,
        attempts: u32,
        last_error: Option<String>,
        /// False after a version mismatch: only `r` tries again (P7).
        retry: bool,
    },
}

/// What the open terminal shows (P5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermMode {
    /// The running agent's screen, kept current.
    Live,
    /// A `failed` agent's last screen; no input.
    Stopped,
    /// The terminal ended or its connection broke; subscribing again every
    /// [`RETRY_EVERY`] (the last screen stays).
    Ended,
}

/// The open terminal view's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub full: Option<FullView>,
    pub upgrade_required: bool,
    pub agent: String,
    pub mode: TermMode,
    /// Input mode: keys go to the agent (P6).
    pub typing: bool,
    /// Output arrived after the last fetch.
    stale: bool,
    fetched: Instant,
    retried: Instant,
}

/// The `/` quick jump overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Finder {
    pub query: String,
    pub selected: usize,
}

pub struct App {
    source: Box<dyn Source>,
    pub fleet: Fleet,
    pub lang: Language,
    stack: Vec<View>,
    pub connection: Connection,
    /// Needs-you questions the operator has viewed (`Attention::read_key`):
    /// a follow-up is new again. Viewing only drops the bold; it never
    /// resolves. Protocol 1.6 shares receipts; older peers remain TUI-local.
    pub read: BTreeSet<String>,
    pub finder: Option<Finder>,
    /// Free-text answer being typed: (ask id, text).
    pub input: Option<(String, String)>,
    pub message: Option<String>,
    pub quit: bool,
    /// The open terminal, while the top view is one.
    pub term: Option<Term>,
    /// When the daemon was last tried (reconnects are [`RECONNECT_EVERY`]).
    last_attempt: Instant,
    /// Body height of the last render, for scrolling.
    pub(crate) body_height: usize,
    outer_size: TerminalSize,
}

impl App {
    /// Connects right away; on failure the app starts in the disconnected
    /// state and `tick` keeps trying.
    pub fn new(source: Box<dyn Source>, lang: Language) -> App {
        let mut app = App {
            source,
            fleet: Fleet::default(),
            lang,
            stack: vec![View::new(Screen::Home, None)],
            connection: Connection::Disconnected {
                reason: String::new(),
                attempts: 0,
                last_error: None,
                retry: true,
            },
            read: BTreeSet::new(),
            finder: None,
            input: None,
            message: None,
            quit: false,
            term: None,
            last_attempt: Instant::now(),
            body_height: 20,
            outer_size: TerminalSize {
                rows: 24,
                columns: 80,
            },
        };
        match app.source.connect() {
            Ok(snapshot) => {
                app.read.extend(snapshot.read_keys.iter().cloned());
                app.fleet = Fleet::from_snapshot(snapshot);
                app.connection = Connection::Connected;
                app.pull();
            }
            Err(e) => app.lost(e),
        }
        app.sync();
        app
    }

    pub fn view(&self) -> &View {
        self.stack.last().expect("the stack always has Home")
    }

    fn view_mut(&mut self) -> &mut View {
        self.stack.last_mut().expect("the stack always has Home")
    }

    pub fn screen(&self) -> &Screen {
        &self.view().screen
    }

    /// Rows at the top of the current screen that never scroll: a
    /// terminal's title, which stays visible while its lines follow the end.
    pub fn pinned_rows(&self) -> usize {
        match self.view().screen {
            Screen::Terminal { .. } => 1,
            _ => 0,
        }
    }

    pub fn stack(&self) -> &[View] {
        &self.stack
    }

    pub fn is_connected(&self) -> bool {
        self.connection == Connection::Connected
    }

    /// Pull new events and keep the open terminal current, or try to
    /// reconnect when disconnected (not after a version mismatch). Call it
    /// regularly (the interactive loop does every 100 ms, or 50 ms with a
    /// full terminal open).
    pub fn tick(&mut self) {
        if self.is_connected() {
            self.pull();
            self.pump_terminal();
        } else if matches!(
            self.connection,
            Connection::Disconnected { retry: true, .. }
        ) && self.last_attempt.elapsed() >= RECONNECT_EVERY
        {
            self.reconnect();
        }
        self.sync_terminal();
        self.sync();
    }

    fn pull(&mut self) {
        match self.source.poll() {
            Ok(events) => {
                for event in events {
                    if let agend_core::protocol::client::DaemonEvent::AttentionRead { data } =
                        &event.event
                    {
                        self.read.insert(data.read_key.clone());
                    }
                    self.fleet.apply(event);
                }
            }
            Err(e) => self.lost(e),
        }
    }

    /// A new fleet view; the open terminal is subscribed again by
    /// `sync_terminal` (typing does not come back, P6).
    fn reconnect(&mut self) {
        self.last_attempt = Instant::now();
        match self.source.connect() {
            Ok(snapshot) => {
                self.read.extend(snapshot.read_keys.iter().cloned());
                self.fleet = Fleet::from_snapshot(snapshot);
                // A selection that is gone goes to the first row (P7).
                for view in &mut self.stack {
                    view.index = 0;
                }
                self.connection = Connection::Connected;
                self.message = Some(self.lang.tr(Text::Reconnected).to_owned());
                self.pull();
            }
            Err(e) => {
                if let Connection::Disconnected {
                    attempts,
                    last_error,
                    retry,
                    ..
                } = &mut self.connection
                {
                    *attempts += 1;
                    *last_error = Some(error_text(&e));
                    *retry = !matches!(e, SourceError::Version(_));
                }
            }
        }
    }

    fn lost(&mut self, error: SourceError) {
        let (reason, retry) = match error {
            SourceError::Disconnected(reason) => (reason, true),
            SourceError::Version(message) => (message, false),
            SourceError::Rejected { code, message } => {
                self.message = Some(
                    self.lang
                        .fmt(Text::Rejected, &[&format!("{code}: {message}")]),
                );
                return;
            }
        };
        self.connection = Connection::Disconnected {
            reason,
            attempts: 0,
            last_error: None,
            retry,
        };
        self.last_attempt = Instant::now();
        self.input = None;
        self.finder = None;
        if self.term.take().is_some() {
            self.source.close_terminal();
        }
    }

    /// Rows of the current screen (not of the finder overlay).
    pub fn rows(&self) -> Vec<Row> {
        let view = self.view();
        let ctx = Ctx {
            fleet: &self.fleet,
            lang: self.lang,
            read: &self.read,
            selected: view.selected.as_ref(),
        };
        match &view.screen {
            Screen::Home => home::rows(&ctx),
            Screen::NeedsYou => attention::rows(&ctx),
            Screen::Team { team, tab } => team::rows(&ctx, team, *tab),
            Screen::Task { task } => task_detail::rows(&ctx, task),
            Screen::Agent { agent } => agent_detail::rows(&ctx, agent),
            Screen::Terminal { agent, screen } => {
                terminal::rows(&ctx, agent, screen, self.term.as_ref())
            }
        }
    }

    /// Keep the selection on an existing row (a neighbour if it vanished),
    /// keep it visible, and mark the expanded needs-you item as read.
    fn sync(&mut self) {
        let rows = self.rows();
        let pinned = self.pinned_rows();
        let height = self.body_height.saturating_sub(pinned).max(1);
        let view = self.view_mut();
        let selectable: Vec<usize> = selectable(&rows);
        let found = view
            .selected
            .as_ref()
            .filter(|_| !view.auto)
            .and_then(|t| rows.iter().position(|r| r.target.as_ref() == Some(t)));
        if view.auto {
            view.index = 0;
        }
        let index = match (found, selectable.is_empty()) {
            (Some(i), _) => Some(i),
            (None, true) => None,
            (None, false) => Some(
                selectable
                    .iter()
                    .copied()
                    .find(|&i| i >= view.index)
                    .unwrap_or(*selectable.last().expect("not empty")),
            ),
        };
        view.selected = index.and_then(|i| rows[i].target.clone());
        if let Some(i) = index {
            view.index = i;
            if i < view.offset {
                view.offset = i;
            } else if i >= view.offset + height {
                view.offset = i + 1 - height;
            }
        }
        let max_offset = rows.len().saturating_sub(pinned + height);
        if view.follow && matches!(view.screen, Screen::Terminal { .. }) {
            view.offset = max_offset;
        }
        view.offset = view.offset.min(max_offset);
        let on_needs_you = view.screen == Screen::NeedsYou;
        let selected = view.selected.clone();
        if on_needs_you
            && let Some(Target::Item(key) | Target::Choice(key, _)) = &selected
            && let Some(item) = self.fleet.attention(key)
        {
            let key = item.read_key();
            if !self.read.contains(&key) {
                match self.source.mark_read(&item.data) {
                    Ok(()) => {
                        self.read.insert(key);
                    }
                    Err(error) => self.message = Some(error.to_string()),
                }
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if self.is_connected() {
            self.pull();
            self.pump_terminal();
        }
        if self.full_mode() && self.is_connected() {
            if key.kind != KeyEventKind::Release {
                self.full_key(key);
            }
            return;
        }

        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.term.as_ref().is_some_and(|t| t.typing) && self.is_connected() {
            self.typing_key(key);
            self.sync();
            return;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.finder.is_some() {
            self.finder_key(key.code);
        } else if self.input.is_some() {
            self.input_key(key.code);
        } else if !self.is_connected() {
            match key.code {
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Char('L') => self.lang = self.lang.toggled(),
                KeyCode::Char('r') => {
                    if let Connection::Disconnected { retry, .. } = &mut self.connection {
                        *retry = true;
                    }
                    self.reconnect();
                }
                _ => {}
            }
        } else {
            self.message = None;
            self.screen_key(key.code);
        }
        self.sync_terminal();
        self.sync();
    }

    fn screen_key(&mut self, code: KeyCode) {
        if self.term.as_ref().is_some_and(|term| term.full.is_some()) {
            match code {
                KeyCode::PageUp => {
                    self.scroll_terminal(-5);
                    return;
                }
                KeyCode::PageDown => {
                    self.scroll_terminal(5);
                    return;
                }
                _ => {}
            }
        }
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('L') => self.lang = self.lang.toggled(),
            KeyCode::Char('h') => self.stack.truncate(1),
            KeyCode::Char('!') => self.push(Screen::NeedsYou, None),
            KeyCode::Char('/') => self.finder = Some(Finder::default()),
            KeyCode::Char('t') => self.open_terminal_of_row(),
            KeyCode::Char('a') => self.start_input(),
            KeyCode::Char('i') => self.start_typing(),
            KeyCode::Left | KeyCode::Esc => {
                if self.stack.len() > 1 {
                    self.stack.pop();
                }
            }
            KeyCode::Right | KeyCode::Enter => self.open_selected(),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1),
            KeyCode::PageDown => (0..5).for_each(|_| self.move_by(1)),
            KeyCode::PageUp => (0..5).for_each(|_| self.move_by(-1)),
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(2),
            KeyCode::Char(c @ '1'..='9') => self.digit(c as usize - '1' as usize),
            _ => {}
        }
    }

    fn push(&mut self, screen: Screen, selected: Option<Target>) {
        self.stack.push(View::new(screen, selected));
    }

    fn select(&mut self, target: Option<Target>) {
        let view = self.view_mut();
        view.auto = target.is_none();
        view.selected = target;
    }

    fn move_by(&mut self, delta: i32) {
        let rows = self.rows();
        let pinned = self.pinned_rows();
        let height = self.body_height.saturating_sub(pinned).max(1);
        let selectable = selectable(&rows);
        let view = self.view_mut();
        let current = view
            .selected
            .as_ref()
            .and_then(|t| rows.iter().position(|r| r.target.as_ref() == Some(t)));
        let next = match (current, delta > 0) {
            (Some(c), true) => selectable.iter().copied().find(|&i| i > c),
            (Some(c), false) => selectable.iter().rev().copied().find(|&i| i < c),
            (None, _) => None,
        };
        let max_offset = rows.len().saturating_sub(pinned + height);
        match next {
            Some(i) => {
                view.selected = rows[i].target.clone();
                view.index = i;
                view.auto = false;
                // Also show the lines that belong to the selected row.
                let end = rows[i + 1..]
                    .iter()
                    .position(|r| r.target.is_some())
                    .map_or(rows.len(), |p| i + 1 + p);
                if delta > 0 && end > view.offset + height {
                    view.offset = (end - height).min(i).min(max_offset);
                }
                if i < view.offset {
                    view.offset = i;
                }
            }
            // Past the first or last selectable row: scroll the rest.
            None if delta > 0 => view.offset = (view.offset + 1).min(max_offset),
            None => view.offset = view.offset.saturating_sub(1),
        }
        view.follow = view.offset >= max_offset;
    }

    fn cycle_tab(&mut self, step: usize) {
        if let Screen::Team { tab, .. } = &self.view().screen {
            let i = Tab::ALL.iter().position(|t| t == tab).unwrap_or(0);
            self.set_tab(Tab::ALL[(i + step) % 3]);
        }
    }

    fn set_tab(&mut self, new: Tab) {
        let view = self.view_mut();
        if let Screen::Team { tab, .. } = &mut view.screen
            && *tab != new
        {
            *tab = new;
            view.selected = None;
            view.auto = true;
            view.index = 0;
            view.offset = 0;
        }
    }

    fn digit(&mut self, n: usize) {
        match self.view().screen.clone() {
            Screen::Team { .. } if n < 3 => self.set_tab(Tab::ALL[n]),
            Screen::NeedsYou => {
                if let Some(key) = self.expanded_item() {
                    self.choose(&key, n);
                }
            }
            _ => {}
        }
    }

    /// The needs-you item the selection is on (NeedsYou screen).
    fn expanded_item(&self) -> Option<String> {
        match &self.view().selected {
            Some(Target::Item(key) | Target::Choice(key, _)) => Some(key.clone()),
            _ => None,
        }
    }

    /// Whether `→`/`Enter` does something on `target` right now.
    fn opens(&self, target: &Target) -> bool {
        match target {
            Target::Item(key) if self.view().screen == Screen::NeedsYou => self
                .fleet
                .attention(key)
                .is_some_and(|a| a.waiting() && !a.choices().is_empty()),
            Target::Stage(task, stage) => {
                let current = self.fleet.task(task).and_then(|t| t.current_stage());
                current == Some(*stage) && self.fleet.needs_you_for_task(task).is_some()
            }
            _ => true,
        }
    }

    fn open_selected(&mut self) {
        let Some(target) = self.view().selected.clone() else {
            return;
        };
        if !self.opens(&target) {
            return;
        }
        match target {
            Target::Home => self.stack.truncate(1),
            Target::Item(key) if self.view().screen == Screen::NeedsYou => {
                self.select(Some(Target::Choice(key, 0)));
            }
            Target::Item(key) => self.push(Screen::NeedsYou, Some(Target::Item(key))),
            Target::Choice(key, n) => self.choose(&key, n),
            Target::Team(team) => self.push(
                Screen::Team {
                    team,
                    tab: Tab::Goals,
                },
                None,
            ),
            Target::Task(task) => self.push(Screen::Task { task }, None),
            Target::Stage(task, _) => {
                if let Some(item) = self.fleet.needs_you_for_task(&task) {
                    let key = item.key();
                    self.push(Screen::NeedsYou, Some(Target::Item(key)));
                }
            }
            Target::Agent(agent) => self.push(Screen::Agent { agent }, None),
        }
    }

    /// The help line: only the keys that do something in the current state
    /// (DEMO-01 Amendment 9).
    pub fn help(&self) -> String {
        let view = self.view();
        let row_agent = match &view.screen {
            Screen::Agent { .. } => true,
            _ => view.selected.as_ref().is_some_and(|t| {
                self.rows()
                    .iter()
                    .any(|r| r.target.as_ref() == Some(t) && r.agent.is_some())
            }),
        };
        let opens = view.selected.as_ref().is_some_and(|t| self.opens(t));
        let expanded = self
            .expanded_item()
            .filter(|_| view.screen == Screen::NeedsYou)
            .and_then(|key| self.fleet.attention(&key))
            .filter(|a| a.waiting());
        let waiting_ask = expanded.filter(|a| a.data.ask.is_some());
        let has_options = expanded.is_some_and(|a| !a.choices().is_empty());
        let movement = if view.selected.is_some() {
            Text::KeyMove
        } else {
            Text::KeyScroll
        };
        let keys: Vec<(Text, bool)> = match &view.screen {
            Screen::Home => vec![
                (movement, true),
                (Text::KeyOpen, opens),
                (Text::KeyTerminal, row_agent),
                (Text::KeyNeedsYou, true),
                (Text::KeyFind, true),
                (Text::LangKey, true),
                (Text::KeyQuit, true),
            ],
            Screen::NeedsYou => vec![
                (movement, true),
                (Text::KeyChoose, opens),
                (Text::KeyOption, has_options),
                (Text::KeyAnswer, waiting_ask.is_some()),
                (Text::KeyTerminal, row_agent),
                (Text::KeyBack, true),
                (Text::LangKey, true),
            ],
            Screen::Team { .. } => vec![
                (Text::KeyTabs, true),
                (movement, true),
                (Text::KeyOpen, opens),
                (Text::KeyTerminal, row_agent),
                (Text::KeyBack, true),
                (Text::LangKey, true),
            ],
            Screen::Task { .. } | Screen::Agent { .. } => vec![
                (movement, true),
                (Text::KeyOpen, opens),
                (Text::KeyTerminal, row_agent),
                (Text::KeyBack, true),
                (Text::KeyHome, true),
                (Text::KeyFind, true),
                (Text::LangKey, true),
            ],
            Screen::Terminal { .. } if self.term.as_ref().is_some_and(|t| t.typing) => {
                vec![(Text::KeyStopTyping, true)]
            }
            Screen::Terminal { .. } => vec![
                (Text::KeyScroll, true),
                (
                    Text::KeyType,
                    self.term.as_ref().is_some_and(|t| t.mode == TermMode::Live),
                ),
                (Text::KeyBack, true),
                (Text::KeyHome, true),
                (Text::LangKey, true),
                (Text::KeyQuit, true),
            ],
        };
        keys.into_iter()
            .filter(|(_, works)| *works)
            .map(|(text, _)| self.lang.tr(text))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn choose(&mut self, key: &str, n: usize) {
        let Some(item) = self.fleet.attention(key).filter(|a| a.waiting()) else {
            return;
        };
        let who = self
            .fleet
            .item_agent(item)
            .unwrap_or_else(|| key.to_owned());
        match item.choices().get(n).cloned() {
            Some(Choice::Option(option)) => self.send_answer(key, AskReply::Choice { option }),
            Some(Choice::Action(action)) => self.send_action(key, action, &who),
            None => {}
        }
    }

    /// `resolve_attention`; the item stays until the daemon's
    /// `attention_resolved` (P4).
    fn send_action(&mut self, key: &str, action: AttentionAction, who: &str) {
        if action == AttentionAction::RequestChanges {
            self.input = Some((key.to_owned(), String::new()));
            return;
        }
        match self.source.resolve(key, action) {
            Ok(()) => {
                let label = self.lang.tr(action_text(action));
                self.message = Some(self.lang.fmt(Text::ActionSent, &[label, who]));
                self.pull();
            }
            Err(e) => self.lost(e),
        }
    }

    fn send_answer(&mut self, key: &str, reply: AskReply) {
        let shown = match &reply {
            AskReply::Choice { option } => option.clone(),
            AskReply::Text { text } => text.clone(),
            AskReply::Unknown => String::new(),
        };
        // After answering, select the next item (or the previous one).
        let items: Vec<String> = self.fleet.needs_you().iter().map(|a| a.key()).collect();
        let neighbour = items
            .iter()
            .position(|k| k == key)
            .and_then(|i| {
                items
                    .get(i + 1)
                    .or(i.checked_sub(1).and_then(|p| items.get(p)))
            })
            .cloned();
        match self.source.answer(key, reply) {
            Ok(()) => {
                self.message = Some(self.lang.fmt(Text::AnswerSent, &[key, &shown]));
                self.pull();
                if self.view().screen == Screen::NeedsYou {
                    self.select(neighbour.map(Target::Item));
                }
            }
            Err(e) => self.lost(e),
        }
    }

    fn start_input(&mut self) {
        if self.view().screen != Screen::NeedsYou {
            return;
        }
        if let Some(key) = self.expanded_item()
            && self
                .fleet
                .attention(&key)
                .is_some_and(|a| a.waiting() && a.data.ask.is_some())
        {
            self.input = Some((key, String::new()));
        }
    }

    fn input_key(&mut self, code: KeyCode) {
        let Some((key, text)) = &mut self.input else {
            return;
        };
        match code {
            KeyCode::Esc => self.input = None,
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let (key, text) = (key.clone(), text.clone());
                if !text.trim().is_empty() {
                    self.input = None;
                    if self
                        .fleet
                        .attention(&key)
                        .is_some_and(|a| a.data.ask.is_none())
                    {
                        match self.source.request_changes(&key, text) {
                            Ok(()) => self.pull(),
                            Err(e) => self.lost(e),
                        }
                    } else {
                        self.send_answer(&key, AskReply::Text { text });
                    }
                }
            }
            _ => {}
        }
    }

    /// `t`: open the terminal of the selected row's agent.
    fn open_terminal_of_row(&mut self) {
        let rows = self.rows();
        let agent = match &self.view().screen {
            Screen::Agent { agent } => Some(agent.clone()),
            _ => self.view().selected.as_ref().and_then(|t| {
                rows.iter()
                    .find(|r| r.target.as_ref() == Some(t))
                    .and_then(|r| r.agent.clone())
            }),
        };
        match agent {
            Some(agent) => self.open_terminal(&agent),
            None => self.message = Some(self.lang.tr(Text::NoAgentRow).to_owned()),
        }
    }

    fn open_terminal(&mut self, agent: &str) {
        let mode = self.mode_for(agent);
        match self.new_full_term(agent, mode) {
            Ok(Some(term)) => {
                self.push(
                    Screen::Terminal {
                        agent: agent.into(),
                        screen: String::new(),
                    },
                    None,
                );
                self.term = Some(term);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.message = Some(error_text(&error));
                return;
            }
        }
        match self.source.open_terminal(agent) {
            Ok(screen) if !screen.trim().is_empty() => {
                self.push(
                    Screen::Terminal {
                        agent: agent.to_owned(),
                        screen,
                    },
                    None,
                );
                let mut term = Term::new(agent, mode);
                term.upgrade_required = self.source.legacy_terminal_is_read_only();
                self.term = Some(term);
                return;
            }
            Ok(_) => self.message = Some(self.lang.fmt(Text::NoOutputFor, &[agent])),
            Err(SourceError::Rejected { code, .. }) if code == error_code::NO_TERMINAL => {
                self.message = Some(self.lang.fmt(Text::NoOutputFor, &[agent]));
            }
            Err(e) => self.message = Some(self.lang.fmt(Text::Rejected, &[&error_text(&e)])),
        }
        // Only the terminal connection failed; the view stays where it is.
        self.source.close_terminal();
        if let Some(term) = &self.term {
            // The terminal view below is still open: keep it subscribed.
            let below = term.agent.clone();
            self.term = None;
            self.reopen_terminal(&below);
        }
    }

    /// A stopped agent's terminal shows its last screen; others are live.
    fn mode_for(&self, agent: &str) -> TermMode {
        match self.fleet.agent(agent).map(|a| a.state) {
            Some(AgentState::Failed) => TermMode::Stopped,
            _ => TermMode::Live,
        }
    }

    /// Opens `agent`'s terminal for the terminal view on top (after `←`
    /// back to it, or after a reconnect); on failure it shows the last
    /// screen as ended and keeps trying.
    fn reopen_terminal(&mut self, agent: &str) {
        let mode = self.mode_for(agent);
        let old_data = self
            .term
            .as_ref()
            .and_then(|term| term.full.as_ref())
            .and_then(|full| full.data.clone());
        match self.new_full_term(agent, mode) {
            Ok(Some(mut term)) => {
                term.full.as_mut().unwrap().data = old_data;
                self.term = Some(term);
                return;
            }
            Ok(None) => {}
            Err(_) => {
                // The previous scope was closed even when the new handshake
                // failed. Retire its owner/expanded mode and rate-limit retry.
                let term = self
                    .term
                    .get_or_insert_with(|| Term::new(agent, TermMode::Ended));
                term.typing = false;
                if let Some(full) = &mut term.full {
                    full.revoke();
                }
                term.mode = TermMode::Ended;
                term.retried = Instant::now();
                return;
            }
        }
        let mut term = Term::new(agent, mode);
        term.upgrade_required = self.source.legacy_terminal_is_read_only();
        match self.source.open_terminal(agent) {
            Ok(screen) => self.set_screen(screen),
            Err(_) => term.mode = TermMode::Ended,
        }
        self.term = Some(term);
    }

    fn set_screen(&mut self, new: String) {
        if let Screen::Terminal { screen, .. } = &mut self.view_mut().screen {
            *screen = new;
        }
    }

    /// Keeps the terminal subscription in step with the top view: open for
    /// a terminal view, closed otherwise.
    fn sync_terminal(&mut self) {
        if !self.is_connected() {
            return;
        }
        let top = match &self.view().screen {
            Screen::Terminal { agent, .. } => Some(agent.clone()),
            _ => None,
        };
        let open = self.term.as_ref().map(|t| t.agent.clone());
        match (top, open) {
            (None, Some(_)) => {
                self.term = None;
                self.source.close_terminal();
            }
            (Some(top), open) if open.as_deref() != Some(top.as_str()) => {
                self.reopen_terminal(&top)
            }
            _ => {}
        }
    }

    /// Applies what the terminal delivered and refreshes or resubscribes it
    /// when due (P5).
    fn pump_terminal(&mut self) {
        self.pump_full_terminal();
        if self.term.as_ref().is_some_and(|term| term.full.is_some()) {
            let agent = self.term.as_ref().unwrap().agent.clone();
            let stopped = self.mode_for(&agent) == TermMode::Stopped;
            let term = self.term.as_mut().unwrap();
            if stopped {
                term.typing = false;
                term.full.as_mut().unwrap().revoke();
                term.mode = TermMode::Stopped;
                self.source.close_terminal();
            }
            let retry = (term.mode == TermMode::Ended && term.retried.elapsed() >= RETRY_EVERY)
                || (term.mode == TermMode::Stopped && !stopped);
            if retry {
                self.reopen_terminal(&agent);
            }
            return;
        }
        if self.term.is_none() {
            return;
        }
        for event in self.source.poll_terminal() {
            let Some(term) = self.term.as_mut() else {
                return;
            };
            match event {
                TerminalEvent::Screen(screen) => {
                    term.fetched = Instant::now();
                    let agent = term.agent.clone();
                    let mode = self.mode_for(&agent);
                    if let Some(term) = self.term.as_mut() {
                        term.mode = mode;
                    }
                    self.set_screen(screen);
                }
                TerminalEvent::Output => term.stale = true,
                TerminalEvent::Error { code, message } => {
                    term.typing = false;
                    if code == error_code::NO_TERMINAL {
                        term.mode = TermMode::Ended;
                        term.retried = Instant::now();
                    }
                    self.message = Some(
                        self.lang
                            .fmt(Text::Rejected, &[&format!("{code}: {message}")]),
                    );
                }
                TerminalEvent::Closed(_) => {
                    term.typing = false;
                    term.mode = TermMode::Ended;
                    term.retried = Instant::now();
                }
            }
        }
        // An instance that failed shows its last screen: no typing.
        let agent = self
            .term
            .as_ref()
            .map(|t| t.agent.clone())
            .unwrap_or_default();
        let stopped = self.mode_for(&agent) == TermMode::Stopped;
        if let Some(term) = self.term.as_mut() {
            if stopped && term.mode == TermMode::Live {
                term.mode = TermMode::Stopped;
            }
            if term.mode != TermMode::Live {
                term.typing = false;
            }
        }
        let Some(term) = self.term.as_ref() else {
            return;
        };
        let now = Instant::now();
        let running_again =
            term.mode == TermMode::Stopped && self.mode_for(&term.agent) == TermMode::Live;
        let refresh = match term.mode {
            TermMode::Live => term.stale && now.duration_since(term.fetched) >= REFRESH_EVERY,
            TermMode::Ended => now.duration_since(term.retried) >= RETRY_EVERY,
            TermMode::Stopped => running_again,
        };
        if !refresh {
            return;
        }
        let result = self.source.refresh_terminal();
        let term = self.term.as_mut().expect("checked above");
        term.stale = false;
        term.fetched = now;
        term.retried = now;
        if result.is_err() || running_again {
            // Until a screen arrives (it may take a retry or two).
            term.mode = TermMode::Ended;
            term.typing = false;
        }
    }

    /// `i` in a live terminal: keys go to the agent (P6).
    fn start_typing(&mut self) {
        if let Some(term) = &self.term
            && term.mode != TermMode::Live
        {
            self.message = Some(
                self.lang
                    .tr(if term.mode == TermMode::Stopped {
                        Text::StoppedNoInput
                    } else {
                        Text::EndedNoInput
                    })
                    .into(),
            );
            return;
        }
        if self.term.as_ref().is_some_and(|term| term.full.is_some()) {
            self.begin_full();
            return;
        }
        if self.term.as_ref().is_some_and(|term| term.upgrade_required) {
            self.message = Some(self.lang.tr(Text::FullUpgrade).into());
            return;
        }
        let Some(term) = self.term.as_mut() else {
            return;
        };
        let text = match term.mode {
            TermMode::Live => {
                term.typing = true;
                return;
            }
            TermMode::Stopped => Text::StoppedNoInput,
            TermMode::Ended => Text::EndedNoInput,
        };
        self.message = Some(self.lang.tr(text).to_owned());
    }

    fn typing_key(&mut self, key: KeyEvent) {
        if stops_typing(&key) {
            if let Some(term) = self.term.as_mut() {
                term.typing = false;
            }
            return;
        }
        let bytes = key_bytes(&key);
        if bytes.is_empty() {
            return;
        }
        if self.source.terminal_input(&bytes).is_err()
            && let Some(term) = self.term.as_mut()
        {
            term.typing = false;
            term.mode = TermMode::Ended;
            term.retried = Instant::now();
        }
    }

    pub fn finder_rows(&self) -> Vec<Row> {
        let finder = self.finder.clone().unwrap_or_default();
        finder::rows(&self.fleet, self.lang, &finder)
    }

    fn finder_key(&mut self, code: KeyCode) {
        let Some(finder) = &mut self.finder else {
            return;
        };
        match code {
            KeyCode::Esc | KeyCode::Left => self.finder = None,
            KeyCode::Char(c) => {
                finder.query.push(c);
                finder.selected = 0;
            }
            KeyCode::Backspace => {
                finder.query.pop();
                finder.selected = 0;
            }
            KeyCode::Down => finder.selected += 1,
            KeyCode::Up => finder.selected = finder.selected.saturating_sub(1),
            KeyCode::Enter | KeyCode::Right => {
                let targets: Vec<Target> = self
                    .finder_rows()
                    .into_iter()
                    .filter_map(|r| r.target)
                    .collect();
                let chosen = self
                    .finder
                    .as_ref()
                    .and_then(|f| targets.get(f.selected.min(targets.len().saturating_sub(1))))
                    .cloned();
                let Some(target) = chosen else { return };
                self.finder = None;
                match target {
                    Target::Agent(agent) => self.open_terminal(&agent),
                    Target::Task(task) => self.push(Screen::Task { task }, None),
                    Target::Team(team) => self.push(
                        Screen::Team {
                            team,
                            tab: Tab::Goals,
                        },
                        None,
                    ),
                    _ => {}
                }
            }
            _ => {}
        }
        if let Some(finder) = &mut self.finder {
            let count = finder::rows(&self.fleet, self.lang, finder)
                .iter()
                .filter(|r| r.target.is_some())
                .count();
            finder.selected = finder.selected.min(count.saturating_sub(1));
        }
    }
}

impl Term {
    fn new(agent: &str, mode: TermMode) -> Term {
        let now = Instant::now();
        Term {
            agent: agent.to_owned(),
            mode,
            typing: false,
            full: None,
            upgrade_required: false,
            stale: false,
            fetched: now,
            retried: now,
        }
    }
}

fn action_text(action: AttentionAction) -> Text {
    match action {
        AttentionAction::Retry => Text::ActionRetry,
        AttentionAction::Approve => Text::ActionApprove,
        AttentionAction::RequestChanges => Text::ActionChanges,
        AttentionAction::Acknowledge => Text::ActionAcknowledge,
        AttentionAction::Abandon => Text::ActionAbandon,
        AttentionAction::Unknown => Text::ActionUnknown,
    }
}

/// `Ctrl-]`, or `Ctrl-5` / a raw 0x1D, which some terminals report for it.
fn stops_typing(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    matches!(key.code, KeyCode::Char(']' | '5') if ctrl) || key.code == KeyCode::Char('\u{1d}')
}

/// The bytes a key sends to a PTY: characters as UTF-8, `Enter` `\r`,
/// `Backspace` 0x7f, arrows `ESC [ A`–`D`, `Ctrl-letter` its control code,
/// `Alt-key` `ESC` first. No bracketed paste: a paste is keys.
pub fn key_bytes(key: &KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let mut out: Vec<u8> = match key.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => {
            vec![(c.to_ascii_lowercase() as u8) & 0x1f]
        }
        KeyCode::Char(c) if ctrl => match c {
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![0x1b],
            '\\' | '4' => vec![0x1c],
            '^' | '6' => vec![0x1e],
            '_' | '7' | '/' => vec![0x1f],
            _ => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        _ => Vec::new(),
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

fn error_text(error: &SourceError) -> String {
    match error {
        SourceError::Disconnected(reason) | SourceError::Version(reason) => reason.clone(),
        SourceError::Rejected { code, message } => format!("{code}: {message}"),
    }
}

fn selectable(rows: &[Row]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| r.target.is_some())
        .map(|(i, _)| i)
        .collect()
}

/// What screen modules read to build their rows.
pub struct Ctx<'a> {
    pub fleet: &'a Fleet,
    pub lang: Language,
    pub read: &'a BTreeSet<String>,
    pub selected: Option<&'a Target>,
}

impl Ctx<'_> {
    pub fn tr(&self, text: Text) -> &'static str {
        self.lang.tr(text)
    }

    pub fn fmt(&self, text: Text, args: &[&str]) -> String {
        self.lang.fmt(text, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_become_pty_bytes() {
        let none = KeyModifiers::NONE;
        let cases: [(KeyEvent, &[u8]); 10] = [
            (key(KeyCode::Char('é'), none), "é".as_bytes()),
            (key(KeyCode::Enter, none), b"\r"),
            (key(KeyCode::Backspace, none), b"\x7f"),
            (key(KeyCode::Esc, none), b"\x1b"),
            (key(KeyCode::Up, none), b"\x1b[A"),
            (key(KeyCode::Left, none), b"\x1b[D"),
            (key(KeyCode::Char('c'), KeyModifiers::CONTROL), b"\x03"),
            (key(KeyCode::Char('D'), KeyModifiers::CONTROL), b"\x04"),
            (key(KeyCode::Char('x'), KeyModifiers::ALT), b"\x1bx"),
            (key(KeyCode::F(5), none), b""),
        ];
        for (event, bytes) in cases {
            assert_eq!(key_bytes(&event), bytes, "{event:?}");
        }
    }

    #[test]
    fn only_ctrl_bracket_and_its_aliases_stop_typing() {
        let ctrl = KeyModifiers::CONTROL;
        assert!(stops_typing(&key(KeyCode::Char(']'), ctrl)));
        assert!(stops_typing(&key(KeyCode::Char('5'), ctrl)));
        assert!(stops_typing(&key(
            KeyCode::Char('\u{1d}'),
            KeyModifiers::NONE
        )));
        for other in [
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            key(KeyCode::Char('c'), ctrl),
            key(KeyCode::Char(']'), KeyModifiers::NONE),
        ] {
            assert!(!stops_typing(&other), "{other:?}");
        }
    }
}
