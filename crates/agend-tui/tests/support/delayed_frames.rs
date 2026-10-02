//! Delay only real ClientSource events; cells and identities are never invented.
use super::{Permit, acquire, key, parser, wait};
use agend_core::protocol::ask::AskReply;
use agend_core::protocol::client::*;
use agend_core::protocol::terminal::TerminalViewport;
use agend_tui::{
    App,
    i18n::Language,
    source::{
        FullTerminalEvent, Snapshot, Source, SourceError, TerminalEvent, client::ClientSource,
    },
};
use ratatui::crossterm::event::KeyCode;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Schedule {
    hold: bool,
    held: Vec<ClientTerminalFrameData>,
    replay: Vec<FullTerminalEvent>,
}
struct DelayedSource {
    real: ClientSource,
    schedule: Arc<Mutex<Schedule>>,
}
impl Source for DelayedSource {
    fn legacy_terminal_is_read_only(&self) -> bool {
        self.real.legacy_terminal_is_read_only()
    }
    fn connect(&mut self) -> Result<Snapshot, SourceError> {
        self.real.connect()
    }
    fn poll(&mut self) -> Result<Vec<EventData>, SourceError> {
        self.real.poll()
    }
    fn answer(&mut self, id: &str, reply: AskReply) -> Result<(), SourceError> {
        self.real.answer(id, reply)
    }
    fn resolve(&mut self, id: &str, action: AttentionAction) -> Result<(), SourceError> {
        self.real.resolve(id, action)
    }
    fn request_changes(&mut self, id: &str, note: String) -> Result<(), SourceError> {
        self.real.request_changes(id, note)
    }
    fn open_terminal(&mut self, id: &str) -> Result<String, SourceError> {
        self.real.open_terminal(id)
    }
    fn refresh_terminal(&mut self) -> Result<(), SourceError> {
        self.real.refresh_terminal()
    }
    fn poll_terminal(&mut self) -> Vec<TerminalEvent> {
        self.real.poll_terminal()
    }
    fn terminal_input(&mut self, bytes: &[u8]) -> Result<(), SourceError> {
        self.real.terminal_input(bytes)
    }
    fn close_terminal(&mut self) {
        self.real.close_terminal();
    }
    fn open_full_terminal(
        &mut self,
        instance: &str,
        id: String,
        viewport: TerminalViewport,
    ) -> Result<bool, SourceError> {
        self.real.open_full_terminal(instance, id, viewport)
    }
    fn terminal_control(&mut self, data: ClientTerminalControlData) -> Result<(), SourceError> {
        self.real.terminal_control(data)
    }
    fn terminal_viewport(&mut self, data: TerminalViewportData) -> Result<(), SourceError> {
        self.real.terminal_viewport(data)
    }
    fn poll_full_terminal(&mut self) -> Vec<FullTerminalEvent> {
        let events = self.real.poll_full_terminal();
        let mut state = self.schedule.lock().unwrap();
        let mut output = Vec::new();
        for event in events {
            match event {
                FullTerminalEvent::Frame(data) if state.hold => state.held.push(*data),
                other => output.push(other),
            }
        }
        output.append(&mut state.replay);
        output
    }
}
fn open(fake: &parser::Fake) -> (App, Arc<Mutex<Schedule>>) {
    let schedule = Arc::new(Mutex::new(Schedule::default()));
    let source = DelayedSource {
        real: ClientSource::new(fake.daemon.socket_path(), None),
        schedule: schedule.clone(),
    };
    let mut app = App::new(Box::new(source), Language::En);
    app.resize(80, 24);
    key(&mut app, KeyCode::Char('/'));
    for c in parser::ID.chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    wait(&mut app, |a| {
        a.term
            .as_ref()
            .and_then(|t| t.full.as_ref())
            .is_some_and(|f| f.ready)
    });
    (app, schedule)
}
fn data(app: &App) -> &ClientTerminalFrameData {
    app.term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
}
fn text(frame: &ClientTerminalFrameData) -> String {
    frame
        .frame
        .cells
        .iter()
        .flat_map(|r| r.iter())
        .map(|c| c.text.as_str())
        .collect()
}
fn replay(app: &mut App, schedule: &Arc<Mutex<Schedule>>, frame: ClientTerminalFrameData) {
    schedule
        .lock()
        .unwrap()
        .replay
        .push(FullTerminalEvent::Frame(Box::new(frame)));
    app.tick();
}
#[test]
fn delayed_real_revision_cannot_replace_the_newer_display() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let (mut app, schedule) = open(&fake);
    acquire(&mut app);
    let old = data(&app).clone();
    fake.parser.feed(b"\x1b[2J\x1b[HNEW-REVISION\r\n");
    wait(&mut app, |a| {
        data(a).frame.revision > old.frame.revision && text(data(a)).contains("NEW-REVISION")
    });
    let current = data(&app).clone();
    assert_eq!(old.request_id, current.request_id);
    assert_eq!(old.view_id, current.view_id);
    assert_eq!(old.frame.generation, current.frame.generation);
    assert!(!text(&old).contains("NEW-REVISION"));
    replay(&mut app, &schedule, old);
    assert_eq!(
        data(&app),
        &current,
        "late real frame rolled the display back"
    );
    assert!(app.term.as_ref().unwrap().typing);
    let rendered = agend_tui::render_to_string(&mut app, 80, 24);
    assert!(rendered.contains("NEW-REVISION"), "{rendered}");
}
#[test]
fn reversed_real_viewport_replies_keep_the_latest_selection_at_equal_revision() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    fake.parser.output();
    let (mut app, schedule) = open(&fake);
    acquire(&mut app);
    let live = data(&app).clone();
    assert!(live.frame.live_top > 10);
    schedule.lock().unwrap().hold = true;
    app.scroll_terminal(-5);
    wait(&mut app, |_| {
        schedule
            .lock()
            .unwrap()
            .held
            .iter()
            .any(|f| f.request_id != live.request_id)
    });
    let first = schedule
        .lock()
        .unwrap()
        .held
        .iter()
        .find(|f| f.request_id != live.request_id)
        .unwrap()
        .clone();
    assert_ne!(first.request_id, live.request_id);
    app.scroll_terminal(-5);
    wait(&mut app, |_| {
        schedule
            .lock()
            .unwrap()
            .held
            .iter()
            .any(|f| f.request_id != first.request_id && f.request_id != live.request_id)
    });
    let second = schedule
        .lock()
        .unwrap()
        .held
        .iter()
        .find(|f| f.request_id != first.request_id && f.request_id != live.request_id)
        .unwrap()
        .clone();
    assert_ne!(first.request_id, second.request_id);
    assert_eq!(first.view_id, second.view_id);
    assert_eq!(first.frame.generation, second.frame.generation);
    assert_eq!(first.frame.revision, second.frame.revision);
    assert!(second.frame.viewport_top < first.frame.viewport_top);
    assert_ne!(first.frame.cells, second.frame.cells);
    replay(&mut app, &schedule, second.clone());
    assert_eq!(data(&app), &second);
    replay(&mut app, &schedule, first);
    assert_eq!(
        data(&app),
        &second,
        "late query changed the selected history"
    );
    let rendered = agend_tui::render_to_string(&mut app, 80, 24);
    assert!(rendered.contains("number-"), "{rendered}");
}
#[test]
fn a_real_old_generation_view_cannot_revoke_the_reopened_controller() {
    let _permit = Permit::new();
    let fake = parser::Fake::default();
    let (mut app, schedule) = open(&fake);
    acquire(&mut app);
    for _ in 0..20 {
        fake.parser.feed(b"old output\r\n");
    }
    wait(&mut app, |a| text(data(a)).contains("old output"));
    let old = data(&app).clone();
    fake.parser.restart();
    wait(&mut app, |a| {
        a.term
            .as_ref()
            .and_then(|t| t.full.as_ref())
            .is_some_and(|f| {
                f.ready
                    && f.data
                        .as_ref()
                        .is_some_and(|d| d.frame.generation != old.frame.generation)
            })
    });
    acquire(&mut app);
    let current = data(&app).clone();
    assert_eq!(
        old.request_id, current.request_id,
        "exercise reused App request names after reopen"
    );
    assert_ne!(old.view_id, current.view_id);
    assert_ne!(old.frame.generation, current.frame.generation);
    assert!(
        old.frame.revision > current.frame.revision,
        "generation order must not be inferred from revision"
    );
    replay(&mut app, &schedule, old);
    assert_eq!(data(&app), &current);
    assert!(
        app.term.as_ref().unwrap().typing,
        "old view revoked the current owner"
    );
    key(&mut app, KeyCode::Char('z'));
    wait(&mut app, |_| fake.parser.received_bytes() == b"z");
}
