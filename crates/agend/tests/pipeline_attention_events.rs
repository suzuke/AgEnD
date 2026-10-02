//! Approval events and debug watch through real daemon, Git, SQLite and CLI producers.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;
use agend_daemon::store::SqliteStore;
use agend_testkit::block_on;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Watch {
    child: Child,
    path: PathBuf,
}
impl Watch {
    fn start(lab: &common::Lab) -> Self {
        let path = lab.home.join("watch.log");
        let file = std::fs::File::create(&path).unwrap();
        let child = Command::new(&lab.agend)
            .args(["debug", "watch"])
            .env("AGEND_HOME", &lab.home)
            .env_remove("AGEND_INSTANCE")
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap();
        let watch = Self { child, path };
        wait_until(|| watch.text().contains("fleet:"));
        watch
    }
    fn text(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap()
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "condition did not become true within 30 seconds"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn lab() -> common::Lab {
    let mut lab = common::Lab::new(&[]).unwrap();
    {
        let store = SqliteStore::open(&lab.home, 0).unwrap();
        let mut workflow = common::workflow("attention-events", "test -f hello.txt");
        workflow
            .stages
            .iter_mut()
            .find(|s| s.id == "approve")
            .unwrap()
            .timeout_ms = Some(2_000);
        block_on(store.save_workflow(&workflow)).unwrap();
    }
    lab.boot(None).unwrap();
    lab
}
fn resolve(lab: &common::Lab, id: &str, action: AttentionAction, note: Option<&str>) {
    let reply = lab
        .request(
            None,
            ClientRequest::ResolveAttention {
                data: ResolveAttentionData {
                    request_id: "human-event".into(),
                    attention_id: id.into(),
                    action,
                    note: note.map(str::to_owned),
                },
            },
        )
        .unwrap();
    assert!(
        matches!(reply, ClientResponse::CommandResult { .. }),
        "{reply:?}"
    );
}
fn assert_events(text: &str, id: &str, action: &str) {
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with(&format!("attention_required {id} ")))
            .count(),
        1,
        "one approval must not disappear and reappear: {text}"
    );
    assert_eq!(
        text.lines()
            .filter(|l| *l == format!("attention_resolved {id} {action}"))
            .count(),
        1,
        "human action must be published once, after commit: {text}"
    );
    assert!(
        !text.contains(&format!("attention_resolved {id} unknown")),
        "{text}"
    );
}
#[test]
fn approval_timeout_preserves_one_item_and_approve_reports_its_action_and_stages() {
    let lab = lab();
    let watch = Watch::start(&lab);
    let task = lab
        .create("g10", "attention-events", "approval events")
        .unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    let id = format!("approval:{task}/approve/1");
    let before = lab
        .fleet()
        .unwrap()
        .attention
        .into_iter()
        .find(|a| a.attention_id.as_deref() == Some(&id))
        .unwrap();
    wait_until(|| {
        lab.logs()
            .contains(&format!("{task}: approve timed out (notification)"))
    });
    // A serialized command is a barrier after the real timeout input and its refresh.
    lab.agent("g10-dev", AgentCommand::Status).unwrap();
    let after = lab
        .fleet()
        .unwrap()
        .attention
        .into_iter()
        .find(|a| a.attention_id.as_deref() == Some(&id))
        .unwrap();
    assert_eq!(
        after, before,
        "notification must not change the pending approval"
    );
    resolve(&lab, &id, AttentionAction::Approve, None);
    lab.wait_stage(&task, "done").unwrap();
    wait_until(|| {
        watch
            .text()
            .contains(&format!("task_changed {task}: {task}: done"))
    });
    let text = watch.text();
    eprintln!("watch:\n{text}\ndaemon:\n{}", lab.logs());
    assert_events(&text, &id, "approve");
    let positions = ["work", "submit", "checks", "review", "approve", "merge"].map(|stage| {
        text.find(&format!("(stage: {stage})"))
            .unwrap_or_else(|| panic!("missing stage {stage}: {text}"))
    });
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
    let log = common::git(&lab.repo(), &["log", "--format=%B", "main"]).unwrap();
    assert_eq!(log.matches(&format!("Agend-Task: {task}")).count(), 1);
}
#[test]
fn request_changes_reports_its_action_and_only_the_new_attempt_is_raised() {
    let lab = lab();
    let watch = Watch::start(&lab);
    let task = lab
        .create("g10", "attention-events", "changes events")
        .unwrap();
    lab.wait_stage(&task, "approve").unwrap();
    let old = format!("approval:{task}/approve/1");
    resolve(
        &lab,
        &old,
        AttentionAction::RequestChanges,
        Some("Please revise the work"),
    );
    let new = format!("approval:{task}/approve/2");
    wait_until(|| {
        lab.fleet()
            .unwrap()
            .attention
            .iter()
            .any(|a| a.attention_id.as_deref() == Some(&new))
    });
    let view = lab.fleet().unwrap();
    assert_eq!(
        view.attention
            .iter()
            .filter(|a| a.task_id.as_deref() == Some(&task))
            .count(),
        1
    );
    resolve(&lab, &new, AttentionAction::Approve, None);
    lab.wait_stage(&task, "done").unwrap();
    wait_until(|| {
        watch
            .text()
            .contains(&format!("task_changed {task}: {task}: done"))
    });
    let text = watch.text();
    eprintln!("watch:\n{text}\ndaemon:\n{}", lab.logs());
    assert_events(&text, &old, "request_changes");
    assert_events(&text, &new, "approve");
    let log = common::git(&lab.repo(), &["log", "--format=%B", "main"]).unwrap();
    assert_eq!(log.matches(&format!("Agend-Task: {task}")).count(), 1);
}

#[test]
fn legacy_note_failure_cannot_split_human_decision_from_its_action_or_merge() {
    for action in [AttentionAction::Approve, AttentionAction::RequestChanges] {
        let mut lab = lab();
        let watch = Watch::start(&lab);
        let task = lab
            .create("g10", "attention-events", "atomic human decision")
            .unwrap();
        lab.wait_stage(&task, "approve").unwrap();
        let id = format!("approval:{task}/approve/1");
        wait_until(|| watch.text().contains(&format!("attention_required {id} ")));
        let fleets = watch
            .text()
            .lines()
            .filter(|l| l.starts_with("fleet:"))
            .count();
        lab.stop(false);
        let connection = common::database(&lab).unwrap();
        // Reject only the obsolete secondary note write, not the pipeline CAS.
        connection.execute_batch("CREATE TRIGGER reject_secondary_note BEFORE UPDATE OF failure_acknowledged ON tasks BEGIN SELECT RAISE(ABORT, 'secondary note rejected'); END;").unwrap();
        drop(connection);
        lab.boot(None).unwrap();
        wait_until(|| {
            watch
                .text()
                .lines()
                .filter(|l| l.starts_with("fleet:"))
                .count()
                > fleets
        });
        resolve(&lab, &id, action, Some("please revise"));
        if action == AttentionAction::RequestChanges {
            let new = format!("approval:{task}/approve/2");
            wait_until(|| {
                lab.fleet()
                    .unwrap()
                    .attention
                    .iter()
                    .any(|a| a.attention_id.as_deref() == Some(&new))
            });
            resolve(&lab, &new, AttentionAction::Approve, None);
        }
        lab.wait_stage(&task, "done").unwrap();
        wait_until(|| {
            watch
                .text()
                .contains(&format!("task_changed {task}: {task}: done"))
        });
        let text = watch.text();
        assert_eq!(
            text.matches(&format!("attention_resolved {id} {}", action.as_str()))
                .count(),
            1,
            "{text}"
        );
        assert!(
            !text.contains(&format!("attention_resolved {id} unknown")),
            "{text}"
        );
        let log = common::git(&lab.repo(), &["log", "--format=%B", "main"]).unwrap();
        assert_eq!(log.matches(&format!("Agend-Task: {task}")).count(), 1);
        lab.stop(false);
        let connection = common::database(&lab).unwrap();
        let event = match action {
            AttentionAction::Approve => "ApprovalGranted",
            _ => "ChangesRequested",
        };
        let events: i64 = connection
            .query_row(
                "SELECT count(*) FROM task_events WHERE task_id=?1 AND detail LIKE ?2",
                rusqlite::params![task, format!("%{event}%operator%")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(events, 1, "the human decision must be durable exactly once");
        let note: Option<String> = connection
            .query_row(
                "SELECT attention_reason FROM tasks WHERE id=?1",
                [&task],
                |r| r.get(0),
            )
            .unwrap();
        assert!(note.is_none());
    }
}
