//! App state and key handling. `App::key` is the only way input changes
//! state; `App::tick` pulls events from the source and reconnects after a
//! disconnect. Navigation is a stack of views, so `←` always returns to the
//! exact screen and selection it came from, including after `t` and `/`.
//!
//! Must NOT: know which `Source` it has, or change needs-you state except by
//! sending an answer through the source.

use std::collections::BTreeSet;

use agend_core::protocol::ask::AskReply;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::i18n::{Language, Text};
use crate::source::{Fleet, Source, SourceError};
use crate::ui::Row;
use crate::{agent_detail, attention, finder, home, task_detail, team, terminal};

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
}

impl View {
    fn new(screen: Screen, selected: Option<Target>) -> View {
        View {
            screen,
            auto: selected.is_none(),
            selected,
            index: 0,
            offset: 0,
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
    },
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
    /// Needs-you items the operator has viewed. Viewing only drops the bold;
    /// it never resolves (protocol v1 has no read state: TUI-local).
    pub read: BTreeSet<String>,
    pub finder: Option<Finder>,
    /// Free-text answer being typed: (ask id, text).
    pub input: Option<(String, String)>,
    pub message: Option<String>,
    pub quit: bool,
    /// Body height of the last render, for scrolling.
    pub(crate) body_height: usize,
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
            },
            read: BTreeSet::new(),
            finder: None,
            input: None,
            message: None,
            quit: false,
            body_height: 20,
        };
        match app.source.connect() {
            Ok(catalog) => {
                app.fleet = Fleet::new(catalog);
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

    pub fn stack(&self) -> &[View] {
        &self.stack
    }

    pub fn is_connected(&self) -> bool {
        self.connection == Connection::Connected
    }

    /// Pull new events, or try to reconnect when disconnected. Call it
    /// regularly (the interactive loop does every 500 ms).
    pub fn tick(&mut self) {
        if self.is_connected() {
            self.pull();
        } else {
            self.reconnect();
        }
        self.sync();
    }

    fn pull(&mut self) {
        match self.source.poll() {
            Ok(events) => {
                for event in events {
                    self.fleet.apply(event);
                }
            }
            Err(e) => self.lost(e),
        }
    }

    fn reconnect(&mut self) {
        match self.source.connect() {
            Ok(catalog) => {
                self.fleet = Fleet::new(catalog);
                self.connection = Connection::Connected;
                self.message = Some(self.lang.tr(Text::Reconnected).to_owned());
                self.pull();
            }
            Err(e) => {
                if let Connection::Disconnected {
                    attempts,
                    last_error,
                    ..
                } = &mut self.connection
                {
                    *attempts += 1;
                    *last_error = Some(error_text(&e));
                }
            }
        }
    }

    fn lost(&mut self, error: SourceError) {
        match error {
            SourceError::Disconnected(reason) => {
                self.connection = Connection::Disconnected {
                    reason,
                    attempts: 0,
                    last_error: None,
                };
                self.input = None;
                self.finder = None;
            }
            SourceError::Rejected { code, message } => {
                self.message = Some(
                    self.lang
                        .fmt(Text::Rejected, &[&format!("{code}: {message}")]),
                );
            }
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
            Screen::Terminal { agent, screen } => terminal::rows(&ctx, agent, screen),
        }
    }

    /// Keep the selection on an existing row (a neighbour if it vanished),
    /// keep it visible, and mark the expanded needs-you item as read.
    fn sync(&mut self) {
        let rows = self.rows();
        let height = self.body_height.max(1);
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
        view.offset = view.offset.min(rows.len().saturating_sub(height));
        if view.screen == Screen::NeedsYou
            && let Some(Target::Item(key) | Target::Choice(key, _)) = view.selected.clone()
        {
            self.read.insert(key);
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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
                KeyCode::Char('r') => self.reconnect(),
                _ => {}
            }
        } else {
            self.message = None;
            self.screen_key(key.code);
        }
        self.sync();
    }

    fn screen_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('L') => self.lang = self.lang.toggled(),
            KeyCode::Char('h') => self.stack.truncate(1),
            KeyCode::Char('!') => self.push(Screen::NeedsYou, None),
            KeyCode::Char('/') => self.finder = Some(Finder::default()),
            KeyCode::Char('t') => self.open_terminal_of_row(),
            KeyCode::Char('a') => self.start_input(),
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
        let height = self.body_height.max(1);
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
        let max_offset = rows.len().saturating_sub(height);
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
                .is_some_and(|a| a.waiting() && !a.question().1.is_empty()),
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
        let waiting_ask = self
            .expanded_item()
            .filter(|_| view.screen == Screen::NeedsYou)
            .and_then(|key| self.fleet.attention(&key))
            .filter(|a| a.waiting() && a.data.ask.is_some());
        let has_options = waiting_ask.is_some_and(|a| !a.question().1.is_empty());
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
            Screen::Terminal { .. } => vec![
                (Text::KeyScroll, true),
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
        let Some(option) = self
            .fleet
            .attention(key)
            .filter(|a| a.waiting())
            .and_then(|a| a.question().1.get(n).cloned())
        else {
            return;
        };
        self.send_answer(key, AskReply::Choice { option });
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
                    self.send_answer(&key, AskReply::Text { text });
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
        match self.source.terminal(agent) {
            Ok(screen) if screen.trim().is_empty() => {
                self.message = Some(self.lang.fmt(Text::NoOutputFor, &[agent]));
            }
            Ok(screen) => self.push(
                Screen::Terminal {
                    agent: agent.to_owned(),
                    screen,
                },
                None,
            ),
            Err(e) => self.lost(e),
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

fn error_text(error: &SourceError) -> String {
    match error {
        SourceError::Disconnected(reason) => reason.clone(),
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
