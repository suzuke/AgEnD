//! Gate 10 across real processes, git, SQLite, shim and checks sandbox.
#[path = "../../agend-daemon/tests/common/pipeline_process.rs"]
mod common;

#[test]
fn happy_pipeline_merges_once() {
    common::happy().unwrap();
}
#[test]
fn failed_checks_return_to_the_holder() {
    common::rework("--fail-checks-once").unwrap();
}
#[test]
fn review_changes_return_to_the_holder() {
    common::rework("--changes-once").unwrap();
}
#[test]
fn completion_preserves_uncommitted_work() {
    common::rework("--leave-wip").unwrap();
}
#[test]
fn main_advance_rechecks_and_keeps_same_patch_approvals() {
    common::main_advanced().unwrap();
}
#[test]
fn four_boots_recover_an_intent_before_main_moves() {
    common::four_boots("after-merge-intent").unwrap();
}
#[test]
fn four_boots_find_the_merge_after_losing_the_intent() {
    common::four_boots("after-main-moved").unwrap();
}
#[test]
fn checks_cannot_write_the_canonical_repo() {
    common::sandbox_escape().unwrap();
}
#[test]
fn shim_guards_the_binding_and_cancel_unbinds_it() {
    common::hooks_and_cancel().unwrap();
}
#[test]
fn real_commands_persist_dialogues_and_reminders_across_restart() {
    common::commands_and_dialogue().unwrap();
}
#[test]
fn a_busy_holder_queues_and_a_new_role_resumes_review() {
    common::queue_and_role_join().unwrap();
}
#[test]
fn missing_sandbox_fails_closed_and_retry_reprobes_it() {
    common::sandbox_retry().unwrap();
}
#[test]
fn a_dirty_main_blocks_merge_and_cannot_be_cancelled() {
    common::merge_blocked().unwrap();
}
#[test]
fn malformed_snapshots_fail_one_task_and_acknowledgment_persists() {
    common::malformed_restore().unwrap();
}
#[test]
fn a_fresh_home_cannot_recover_the_previous_home_task() {
    common::fresh_home_negative().unwrap();
}
