//! A human requests changes in the actual TUI against the real pipeline.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_tui::{App, i18n::Language, source::client::ClientSource};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
#[test]
fn human_changes_collect_a_note_and_return_to_work() {
    let mut lab = common::Lab::new(&[]).unwrap();
    lab.boot(None).unwrap();
    let task = lab.create("g10", "demo", "TUI changes").unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    let mut app = App::new(
        Box::new(ClientSource::new(&lab.home.join("run/daemon.sock"), None)),
        Language::En,
    );
    for key in [KeyCode::Char('!'), KeyCode::Enter, KeyCode::Char('2')] {
        app.key(KeyEvent::from(key));
    }
    assert!(
        app.input.is_some(),
        "request changes did not ask for a reason"
    );
    app.key(KeyEvent::from(KeyCode::Enter));
    assert!(app.input.is_some(), "empty reason was sent");
    for c in "Please revise hello.txt".chars() {
        app.key(KeyEvent::from(KeyCode::Char(c)));
    }
    app.key(KeyEvent::from(KeyCode::Enter));
    assert!(app.input.is_none());
    common::wait_until(&lab, || {
        Ok(lab
            .fleet()?
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&format!("approval:{task}/approve/2"))))
    })
    .unwrap();
    app.tick();
    drop(app);
    lab.approve(&task).unwrap();
    lab.wait_stage(&task, "done").unwrap();
}
