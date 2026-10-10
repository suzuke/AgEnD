//! Deterministic App mailbox tests; frames come from the real holder parser.
//! This source spy does not exercise the native event loop or socket scheduling.
use super::*;
use crate::source::{Snapshot, TerminalEvent};
use agend_core::protocol::client::{
    ClientTerminalControlAck, EventData, TerminalControlChangedData,
};
use agend_holder::screen::{ReplySink, Screen as HolderScreen};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Probe {
    events: Vec<FullTerminalEvent>,
    calls: Vec<&'static str>,
    fits: Vec<Option<agend_core::protocol::terminal::TerminalSize>>,
}
struct Spy(Rc<RefCell<Probe>>);
impl Spy {
    fn record(&self, name: &'static str) {
        self.0.borrow_mut().calls.push(name);
    }
}
impl Source for Spy {
    fn connect(&mut self) -> Result<Snapshot, SourceError> {
        self.record("connect");
        Ok(Snapshot::default())
    }
    fn poll(&mut self) -> Result<Vec<EventData>, SourceError> {
        self.record("fleet");
        Ok(Vec::new())
    }
    fn poll_full_terminal(&mut self) -> Vec<FullTerminalEvent> {
        self.record("mailbox");
        std::mem::take(&mut self.0.borrow_mut().events)
    }
    fn terminal_control(&mut self, _: ClientTerminalControlData) -> Result<(), SourceError> {
        self.record("control");
        Ok(())
    }
    fn terminal_viewport(&mut self, data: TerminalViewportData) -> Result<(), SourceError> {
        self.record("viewport");
        self.0.borrow_mut().fits.push(data.fit_size);
        Ok(())
    }
    fn answer(&mut self, _: &str, _: AskReply) -> Result<(), SourceError> {
        self.record("answer");
        Ok(())
    }
    fn resolve(&mut self, _: &str, _: AttentionAction) -> Result<(), SourceError> {
        self.record("resolve");
        Ok(())
    }
    fn open_terminal(&mut self, _: &str) -> Result<String, SourceError> {
        self.record("open");
        Ok(String::new())
    }
    fn refresh_terminal(&mut self) -> Result<(), SourceError> {
        self.record("refresh");
        Ok(())
    }
    fn poll_terminal(&mut self) -> Vec<TerminalEvent> {
        self.record("legacy_poll");
        Vec::new()
    }
    fn terminal_input(&mut self, _: &[u8]) -> Result<(), SourceError> {
        self.record("input");
        Ok(())
    }
    fn close_terminal(&mut self) {
        self.record("close");
    }
}

fn setup() -> (App, Rc<RefCell<Probe>>, HolderScreen) {
    let probe = Rc::new(RefCell::new(Probe::default()));
    let mut app = App::new(Box::new(Spy(probe.clone())), Language::En);
    app.resize(20, 4);
    let mut term = Term::new("mailbox-agent", TermMode::Live);
    term.full = Some(FullView::new(1000));
    // Existing mailbox cases start after the initial size request.
    term.full.as_mut().unwrap().fit_requested = Some(TerminalSize {
        rows: 2,
        columns: 20,
    });
    app.term = Some(term);
    app.stack.push(View::new(
        Screen::Terminal {
            agent: "mailbox-agent".into(),
            screen: String::new(),
        },
        None,
    ));
    probe.borrow_mut().calls.clear();
    let mut screen = HolderScreen::new(3, 20, ReplySink::default());
    screen.process("READY 繁中".as_bytes());
    (app, probe, screen)
}
fn produced(screen: &HolderScreen) -> ClientTerminalFrameData {
    ClientTerminalFrameData {
        request_id: "tui-view".into(),
        instance_id: "mailbox-agent".into(),
        view_id: "producer-view".into(),
        frame: screen
            .frame(TerminalViewport { top: None, rows: 3 })
            .unwrap(),
    }
}
fn deliver(app: &mut App, probe: &Rc<RefCell<Probe>>, frame: ClientTerminalFrameData) {
    probe
        .borrow_mut()
        .events
        .push(FullTerminalEvent::Frame(Box::new(frame)));
    assert!(app.pump_full_terminal_ready());
}

#[test]
fn ready_frame_updates_without_tick_or_fleet_poll() {
    let (mut app, probe, mut screen) = setup();
    deliver(&mut app, &probe, produced(&screen));
    let old_revision = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .data
        .as_ref()
        .unwrap()
        .frame
        .revision;
    screen.process(b"\r\nNEW-FRAME");
    let expected = produced(&screen);
    assert!(expected.frame.revision > old_revision);
    deliver(&mut app, &probe, expected.clone());
    assert_eq!(
        app.term
            .as_ref()
            .unwrap()
            .full
            .as_ref()
            .unwrap()
            .data
            .as_ref(),
        Some(&expected)
    );
    assert_eq!(probe.borrow().calls, ["mailbox", "mailbox"]);
}

#[test]
fn empty_ready_polls_do_not_run_resize_viewport_or_fleet_maintenance() {
    for expanded in [false, true] {
        let (mut app, probe, screen) = setup();
        deliver(&mut app, &probe, produced(&screen));
        let full = app.term.as_mut().unwrap().full.as_mut().unwrap();
        full.expanded = expanded;
        full.owner = Some("existing-owner".into());
        // Deliberately leave maintenance due without invoking resize(): a fast
        // empty poll must not perform the original tick's maintenance work.
        if expanded {
            app.outer_size.columns = 21;
        } else {
            full.viewport = TerminalViewport {
                top: Some(0),
                rows: 1,
            };
        }
        let before = app.term.clone();
        probe.borrow_mut().calls.clear();
        for _ in 0..5 {
            assert!(!app.pump_full_terminal_ready());
        }
        assert_eq!(app.term, before);
        assert_eq!(probe.borrow().calls, ["mailbox"; 5]);
        // The original maintenance entry point still acts on the same state.
        app.pump_full_terminal();
        assert_eq!(
            probe.borrow().calls.last(),
            Some(&if expanded { "control" } else { "viewport" })
        );
    }
}

#[test]
fn control_loss_followed_by_late_frame_and_grant_never_restores_input() {
    let (mut app, probe, mut screen) = setup();
    let old = produced(&screen);
    deliver(&mut app, &probe, old.clone());
    let term = app.term.as_mut().unwrap();
    term.typing = true;
    let full = term.full.as_mut().unwrap();
    full.expanded = true;
    full.owner = Some("retired-owner".into());
    probe
        .borrow_mut()
        .events
        .push(FullTerminalEvent::ControlChanged(
            TerminalControlChangedData {
                instance_id: old.instance_id.clone(),
                view_id: old.view_id.clone(),
                generation: old.frame.generation.clone(),
                control: TerminalControlState::ReadOnly,
                reason: "producer ownership changed".into(),
            },
        ));
    assert!(app.pump_full_terminal_ready());
    assert!(!app.term.as_ref().unwrap().typing);
    screen.process(b"\r\nAFTER-LOSS");
    let mut current = produced(&screen);
    current.request_id = app
        .term
        .as_ref()
        .unwrap()
        .full
        .as_ref()
        .unwrap()
        .selection
        .clone();
    deliver(&mut app, &probe, current.clone());
    probe.borrow_mut().events.extend([
        FullTerminalEvent::Frame(Box::new(old.clone())),
        FullTerminalEvent::ControlAck(Box::new(ClientTerminalControlAck {
            request_id: "retired-grant".into(),
            instance_id: old.instance_id,
            view_id: old.view_id,
            generation: old.frame.generation.clone(),
            control: TerminalControlState::Controlled {
                attach_id: "retired-owner".into(),
            },
            frame: Some(old.frame),
        })),
    ]);
    assert!(app.pump_full_terminal_ready());
    let term = app.term.as_ref().unwrap();
    let full = term.full.as_ref().unwrap();
    assert!(!term.typing);
    assert!(full.owner.is_none());
    assert!(full.pending.is_none());
    assert_eq!(full.data.as_ref(), Some(&current));
    assert!(
        probe
            .borrow()
            .calls
            .iter()
            .all(|call| matches!(*call, "mailbox" | "viewport"))
    );
}

#[test]
fn readonly_fit_is_sent_once_per_outer_size_without_acquiring_control() {
    let (mut app, probe, screen) = setup();
    app.term
        .as_mut()
        .unwrap()
        .full
        .as_mut()
        .unwrap()
        .fit_requested = None;
    probe
        .borrow_mut()
        .events
        .push(FullTerminalEvent::Frame(Box::new(produced(&screen))));
    app.pump_full_terminal_ready();
    app.pump_full_terminal();
    assert_eq!(
        probe.borrow().fits,
        vec![Some(TerminalSize {
            rows: 2,
            columns: 20
        })]
    );
    app.pump_full_terminal();
    assert_eq!(probe.borrow().fits.len(), 1);
    app.resize(40, 10);
    assert_eq!(
        probe.borrow().fits.last(),
        Some(&Some(TerminalSize {
            rows: 8,
            columns: 40
        }))
    );
    assert!(!probe.borrow().calls.contains(&"control"));
    assert!(!app.term.as_ref().unwrap().typing);
}
