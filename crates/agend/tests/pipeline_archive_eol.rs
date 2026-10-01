//! Line ending normalization must not delete physical WIP bytes on cancellation.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;
use agend_core::protocol::client::*;

fn probe(autocrlf: Option<&str>, attribute: Option<&str>) {
    let mut lab = common::Lab::new(&["--hold"]).unwrap();
    if let Some(attribute) = attribute {
        std::fs::write(
            lab.repo().join(".gitattributes"),
            format!("README.md {attribute}\n"),
        )
        .unwrap();
        common::git(&lab.repo(), &["add", ".gitattributes"]).unwrap();
        common::git(&lab.repo(), &["commit", "-m", "Line ending attributes"]).unwrap();
    }
    lab.boot(None).unwrap();
    let task = lab.create("g10h", "demo", "physical CRLF WIP").unwrap();
    let wt = lab.home.join("worktrees").join(&task);
    if let Some(value) = autocrlf {
        let out = std::process::Command::new(lab.home.join("bin/git"))
            .current_dir(&wt)
            .args(["config", "--worktree", "core.autocrlf", value])
            .env("AGEND_HOME", &lab.home)
            .env("AGEND_INSTANCE", "g10-hold")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let bytes = b"UNIQUE_PHYSICAL_CRLF_WIP\r\nsecond line\r\n";
    std::fs::write(wt.join("README.md"), bytes).unwrap();
    let index = common::git(
        &wt,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .unwrap();
    let original_index = std::fs::read(&index).unwrap();
    let error = lab
        .operator(OperatorCommand::TaskCancel {
            task_id: task.clone(),
            reason: None,
        })
        .unwrap_err();
    assert!(error.contains("content conversion"), "{error}");
    lab.wait_stage(&task, "failed").unwrap();
    assert!(wt.exists());
    assert_eq!(std::fs::read(wt.join("README.md")).unwrap(), bytes);
    assert_eq!(std::fs::read(index).unwrap(), original_index);
    assert!(
        std::fs::read_dir(lab.home.join("archive"))
            .unwrap()
            .all(|e| e
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "patch"))
    );
}
#[test]
fn autocrlf_input_retains_physical_crlf_and_index() {
    probe(Some("input"), None);
}
#[test]
fn autocrlf_true_retains_physical_crlf_and_index() {
    probe(Some("true"), None);
}
#[test]
fn text_attribute_retains_physical_crlf_and_index() {
    probe(None, Some("text"));
}
#[test]
fn text_auto_attribute_retains_physical_crlf_and_index() {
    probe(None, Some("text=auto"));
}
#[test]
fn eol_attribute_retains_physical_crlf_and_index() {
    probe(None, Some("eol=lf"));
}
#[test]
fn legacy_crlf_attribute_retains_physical_crlf_and_index() {
    probe(None, Some("crlf=input"));
}
