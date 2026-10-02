use agend_testkit::recorder::{self, Scenario};

#[test]
fn repeated_resume_empty_keeps_the_thread_started_notification() {
    let backend = recorder::BACKENDS
        .iter()
        .copied()
        .find(|b| b.name() == "codex")
        .unwrap();
    for run in 0..50 {
        let entries = recorder::run_fake(backend, Scenario::ResumeEmpty).unwrap();
        let count = entries
            .iter()
            .filter(|e| e.str("method") == Some("thread/started"))
            .count();
        assert_eq!(
            count, 1,
            "resume_empty closed before reading thread/started on run {run}"
        );
    }
}
