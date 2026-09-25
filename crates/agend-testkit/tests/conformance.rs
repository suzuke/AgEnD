//! Conformance check: each fake agent, driven through the same scenarios as
//! the real CLI, must produce traffic with the same shape as the committed
//! recordings in `transcripts/<backend>/<scenario>.jsonl`. No tokens, no
//! real CLIs. Normalisation rules: `agend_testkit::recorder::shape`.
//!
//! Set `AGEND_CONFORMANCE_DUMP=<dir>` to also write each fake run as a
//! transcript there (for diffing by hand).

#![cfg(unix)]

use std::path::{Path, PathBuf};

use agend_testkit::recorder::{self, Backend, Scenario, shape};
use serde_json::json;

fn transcripts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("transcripts")
}

/// Every backend in `recorder::BACKENDS` is checked (one thread each), so a
/// new backend needs no new test here.
#[test]
fn every_fake_matches_its_real_recordings() {
    let reports: Vec<String> = std::thread::scope(|scope| {
        let runs: Vec<_> = recorder::BACKENDS
            .iter()
            .map(|&backend| scope.spawn(move || check(backend)))
            .collect();
        runs.into_iter()
            .map(|run| run.join().expect("conformance thread"))
            .filter(|report| !report.is_empty())
            .collect()
    });
    assert!(reports.is_empty(), "{}", reports.join("\n"));
}

/// The differences between `backend`'s fake and its recordings, as one
/// report (empty when they match).
fn check(backend: &dyn Backend) -> String {
    let mut report = Vec::new();
    for &scenario in backend.scenarios() {
        if let Err(e) = check_one(backend, scenario, &mut report) {
            report.push(format!("{}/{}: {e}", backend.name(), scenario.name()));
        }
    }
    if report.is_empty() {
        return String::new();
    }
    format!(
        "fake {} differs from the real {} recordings:\n{}",
        backend.fake(),
        backend.program(),
        report.join("\n")
    )
}

fn check_one(
    backend: &dyn Backend,
    scenario: Scenario,
    report: &mut Vec<String>,
) -> Result<(), String> {
    let path = transcripts()
        .join(backend.name())
        .join(format!("{}.jsonl", scenario.name()));
    let fake = recorder::run_fake(backend, scenario)?;
    if let Some(dir) = std::env::var_os("AGEND_CONFORMANCE_DUMP") {
        let out = Path::new(&dir)
            .join(backend.name())
            .join(format!("{}.jsonl", scenario.name()));
        let head = json!({"type": "header", "backend": backend.name(), "scenario": scenario.name(), "fake": true});
        recorder::write_transcript(&out, &head, &fake)?;
    }
    let (header, real) = recorder::read_transcript(&path)?;
    if header["backend"] != backend.name() || header["scenario"] != scenario.name() {
        return Err(format!(
            "{} has a mismatched header {header}",
            path.display()
        ));
    }
    for diff in shape::compare(backend.name(), scenario.name(), &real, &fake) {
        report.push(format!("{}/{}: {diff}", backend.name(), scenario.name()));
    }
    Ok(())
}

/// Mutation check on a real recording: a repeated lifecycle message must
/// be a difference, repeated streaming chunks must not (`shape::COLLAPSED`).
#[test]
fn a_duplicated_turn_completed_is_a_difference_but_extra_deltas_are_not() {
    let path = transcripts().join("codex").join("one_turn.jsonl");
    let (_, real) = recorder::read_transcript(&path).expect("transcript");
    let method = |e: &recorder::Entry| e.msg["method"].as_str().unwrap_or_default().to_owned();
    let dup = |m: &str| {
        let i = real
            .iter()
            .position(|e| method(e) == m)
            .unwrap_or_else(|| panic!("{m} in {}", path.display()));
        let mut out = real.clone();
        out.insert(i, real[i].clone());
        out
    };
    assert_eq!(
        shape::compare("codex", "one_turn", &real, &real),
        Vec::<String>::new()
    );
    assert_eq!(
        shape::compare("codex", "one_turn", &real, &dup("item/agentMessage/delta")),
        Vec::<String>::new()
    );
    let diffs = shape::compare("codex", "one_turn", &real, &dup("turn/completed"));
    assert!(
        diffs.iter().any(|d| d.contains("notify turn/completed")),
        "a second turn/completed went unnoticed: {diffs:?}"
    );
}

#[test]
fn every_backend_has_a_transcript_per_supported_scenario_and_nothing_else() {
    for backend in recorder::BACKENDS {
        let dir = transcripts().join(backend.name());
        let mut found: Vec<String> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| {
                e.expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        let mut expected: Vec<String> = backend
            .scenarios()
            .iter()
            .map(|s| format!("{}.jsonl", s.name()))
            .collect();
        expected.sort();
        assert_eq!(found, expected, "{}", dir.display());
    }
}

#[test]
fn committed_transcripts_pass_the_secret_scan() {
    for backend in recorder::BACKENDS {
        for scenario in backend.scenarios() {
            let path = transcripts()
                .join(backend.name())
                .join(format!("{}.jsonl", scenario.name()));
            let (header, entries) = recorder::read_transcript(&path).expect("transcript");
            let findings = recorder::redact::scan(&header, &entries);
            assert!(findings.is_empty(), "{}: {findings:?}", path.display());
        }
    }
}
