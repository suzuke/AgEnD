//! Dashboard navigation state. Hit regions are rebuilt from the painted cells.
use super::*;
use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Nav {
    Home,
    Attention,
    Team(String),
    Agent(String),
}
#[derive(Debug, Clone)]
pub(crate) enum Hit {
    Nav(Nav),
    Row(Target),
    Previous,
    Next,
}
#[derive(Default)]
pub(crate) struct ShellState {
    pub focused: bool,
    pub selected: Option<Nav>,
    pub collapsed: BTreeSet<String>,
    pub hits: Vec<(Rect, Hit)>,
    pub detail_offset: usize,
    pub detail_key: Option<String>,
    pub detail_selection: Option<Target>,
    pub detail_size: (u16, u16),
    pub detail_choice_row: Option<usize>,
    pub painted_size: (u16, u16),
    pub painted_rows: Vec<Row>,
    pub painted_nav: Vec<Nav>,
}

impl App {
    pub(crate) fn shell_visible(&self) -> bool {
        self.outer_size.columns >= 110
            && self.outer_size.rows >= 20
            && self.is_connected()
            && self.finder.is_none()
            && !matches!(self.screen(), Screen::Terminal { .. })
    }

    pub(crate) fn nav_entries(&self) -> Vec<Nav> {
        let mut entries = vec![Nav::Home, Nav::Attention];
        for team in &self.fleet.catalog.teams {
            entries.push(Nav::Team(team.clone()));
            if !self.shell.collapsed.contains(team) {
                entries.extend(self.fleet.agents_of(team).map(|a| Nav::Agent(a.id.clone())));
            }
        }
        entries.extend(
            self.fleet
                .catalog
                .agents
                .iter()
                .filter(|a| !self.fleet.catalog.teams.contains(&a.team_id))
                .map(|a| Nav::Agent(a.id.clone())),
        );
        entries
    }

    pub(crate) fn current_nav(&self) -> Nav {
        match self.screen() {
            Screen::Home => Nav::Home,
            Screen::NeedsYou => Nav::Attention,
            Screen::Team { team, .. } => Nav::Team(team.clone()),
            Screen::Agent { agent } | Screen::Terminal { agent, .. } => Nav::Agent(agent.clone()),
            Screen::Task { task } => self
                .fleet
                .task(task)
                .map_or(Nav::Home, |t| Nav::Team(t.team_id.clone())),
        }
    }

    fn activate_nav(&mut self, nav: Nav) {
        match nav {
            Nav::Home => self.stack.truncate(1),
            Nav::Attention if !matches!(self.screen(), Screen::NeedsYou) => {
                self.push(Screen::NeedsYou, None)
            }
            Nav::Attention => {}
            Nav::Team(team) => self.push(
                Screen::Team {
                    team,
                    tab: Tab::Goals,
                },
                None,
            ),
            Nav::Agent(agent) => self.push(Screen::Agent { agent }, None),
        }
        self.shell.focused = false;
    }

    pub(crate) fn adjacent_attention(&mut self, delta: isize) {
        if self.screen() != &Screen::NeedsYou {
            return;
        }
        let items: Vec<_> = self.fleet.needs_you().iter().map(|a| a.key()).collect();
        if items.is_empty() {
            return;
        }
        let key = self.expanded_item();
        let index = items
            .iter()
            .position(|k| Some(k) == key.as_ref())
            .unwrap_or(0);
        let next = index.saturating_add_signed(delta).min(items.len() - 1);
        self.select(Some(Target::Item(items[next].clone())));
        self.shell.detail_offset = 0;
    }

    pub(crate) fn shell_key(&mut self, code: KeyCode) -> bool {
        if !self.shell_visible() {
            return false;
        }
        if matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            self.shell.focused = !self.shell.focused;
            if self.shell.focused {
                self.shell.selected = Some(self.current_nav());
            }
            return true;
        }
        if !self.shell.focused {
            if self.screen() == &Screen::NeedsYou
                && matches!(code, KeyCode::PageUp | KeyCode::PageDown)
            {
                self.shell.detail_offset = self
                    .shell
                    .detail_offset
                    .saturating_add_signed(if code == KeyCode::PageDown { 5 } else { -5 });
                return true;
            }
            return false;
        }
        let entries = self.nav_entries();
        let index = entries
            .iter()
            .position(|n| Some(n) == self.shell.selected.as_ref())
            .unwrap_or(0);
        let nav = entries[index].clone();
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.shell.selected = Some(entries[index.saturating_sub(1)].clone())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.shell.selected = Some(entries[(index + 1).min(entries.len() - 1)].clone())
            }
            KeyCode::Enter => self.activate_nav(nav),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                if let Nav::Team(team) = nav {
                    if code == KeyCode::Left
                        || (code == KeyCode::Char(' ') && !self.shell.collapsed.contains(&team))
                    {
                        self.shell.collapsed.insert(team);
                    } else {
                        self.shell.collapsed.remove(&team);
                    }
                } else if code == KeyCode::Right {
                    self.activate_nav(nav);
                }
            }
            KeyCode::Esc => self.shell.focused = false,
            KeyCode::Char('t') => {
                if let Nav::Agent(agent) = nav {
                    self.activate_nav(Nav::Agent(agent));
                    self.open_terminal_of_row();
                }
            }
            KeyCode::Char('h' | '!' | '/' | 'L' | 'q') => {
                self.shell.focused = false;
                return false;
            }
            _ => {}
        }
        true
    }

    pub(crate) fn shell_mouse(&mut self, event: MouseEvent) -> bool {
        if !self.shell_visible() {
            return false;
        }
        if self.input.is_some()
            || self.shell.painted_size != (self.outer_size.columns, self.outer_size.rows)
            || self.shell.painted_rows != self.rows()
            || self.shell.painted_nav != self.nav_entries()
        {
            return true;
        }
        let hit = self
            .shell
            .hits
            .iter()
            .find(|(r, _)| r.contains((event.column, event.row).into()))
            .map(|(_, h)| h.clone());
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => match hit {
                Some(Hit::Nav(nav)) => {
                    self.shell.selected = Some(nav.clone());
                    self.shell.focused = true;
                    if let Nav::Team(team) = &nav {
                        if event.column < 4 {
                            if !self.shell.collapsed.remove(team) {
                                self.shell.collapsed.insert(team.clone());
                            }
                        } else {
                            self.activate_nav(nav);
                        }
                    } else {
                        self.activate_nav(nav);
                    }
                }
                Some(Hit::Row(target)) => {
                    self.shell.focused = false;
                    let open =
                        !matches!(target, Target::Item(_)) || self.screen() != &Screen::NeedsYou;
                    self.select(Some(target));
                    if open {
                        self.open_selected();
                    }
                }
                Some(Hit::Previous) => self.adjacent_attention(-1),
                Some(Hit::Next) => self.adjacent_attention(1),
                None => {}
            },
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let down = event.kind == MouseEventKind::ScrollDown;
                if event.column < 26 {
                    self.shell.focused = true;
                    self.shell_key(if down { KeyCode::Down } else { KeyCode::Up });
                } else if self.screen() == &Screen::NeedsYou && event.column >= 55 {
                    self.shell.detail_offset = self
                        .shell
                        .detail_offset
                        .saturating_add_signed(if down { 3 } else { -3 });
                } else {
                    self.shell.focused = false;
                    if self.screen() == &Screen::NeedsYou {
                        self.adjacent_attention(if down { 1 } else { -1 });
                    } else {
                        self.move_by(if down { 1 } else { -1 });
                    }
                }
            }
            _ => {}
        }
        self.sync_terminal();
        self.sync();
        true
    }
}
