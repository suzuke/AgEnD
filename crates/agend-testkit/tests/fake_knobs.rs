//! The test knobs every fake promises (README "假實作：共同規則"): `fail_next`
//! fails exactly the next call of that method, `calls()` records every call
//! in order (failed ones included), plus `FakeForge::merges`. Each test runs
//! every trait method of one fake, so dropping a knob from any method fails.

use std::fmt::Debug;

use agend_core::model::Backend;
use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::*;
use agend_testkit::block_on;
use agend_testkit::fakes::{
    DriverCall, FakeDriver, FakeError, FakeForge, FakeNotifier, FakeRunner, FakeRuntime, FakeStore,
    ForgeCall, RunnerCall, RuntimeCall, ScriptedCommand, StoreCall,
};

fn message(operation: &str) -> String {
    format!("scripted {operation}")
}

/// `result` is the scripted failure for `operation`.
fn scripted<T: Debug>(operation: &str, result: Result<T, FakeError>) {
    match result {
        Err(error) => assert_eq!(
            (error.operation, error.message),
            (operation, message(operation)),
            "wrong error for {operation}"
        ),
        Ok(value) => panic!("fail_next({operation:?}) was ignored: got Ok({value:?})"),
    }
}

/// `result` is not a scripted failure (the failure was used up).
fn unscripted<T: Debug>(operation: &str, result: Result<T, FakeError>) -> T {
    result.unwrap_or_else(|e| panic!("{operation} after its scripted failure: {e}"))
}

#[test]
fn forge_fails_each_method_once_records_calls_and_merges() {
    let forge = FakeForge::new();
    let branch = "agend/T-1/fix";
    let head = forge.push(branch);
    let change = Submission {
        task_id: "T-1".into(),
        branch: branch.into(),
        title: "fix".into(),
        body: String::new(),
    };
    let merge = MergeRequest {
        branch: branch.into(),
        expected_head: head.clone(),
    };
    for operation in ["submit", "head", "merge_if_head_is"] {
        forge.fail_next(operation, &message(operation));
    }
    scripted("submit", block_on(forge.submit(&change)));
    scripted("head", block_on(forge.head(branch)));
    scripted("merge_if_head_is", block_on(forge.merge_if_head_is(&merge)));
    assert!(forge.merges().is_empty(), "a failed merge was recorded");

    unscripted("submit", block_on(forge.submit(&change)));
    assert_eq!(unscripted("head", block_on(forge.head(branch))), head);
    let merged = unscripted("merge_if_head_is", block_on(forge.merge_if_head_is(&merge)));
    let MergeResult::Merged { merge_commit } = merged else {
        panic!("expected Merged, got {merged:?}");
    };
    forge.push(branch);
    let refused = unscripted("merge_if_head_is", block_on(forge.merge_if_head_is(&merge)));
    assert!(matches!(refused, MergeResult::HeadChanged { .. }));

    assert_eq!(forge.merges(), [(branch.to_owned(), merge_commit.clone())]);
    assert_eq!(forge.base_head(), merge_commit);
    assert_eq!(
        forge.calls(),
        [
            ForgeCall::Submit(change.clone()),
            ForgeCall::Head {
                branch: branch.into()
            },
            ForgeCall::MergeIfHeadIs(merge.clone()),
            ForgeCall::Submit(change),
            ForgeCall::Head {
                branch: branch.into()
            },
            ForgeCall::MergeIfHeadIs(merge.clone()),
            ForgeCall::MergeIfHeadIs(merge),
        ]
    );
}

#[test]
fn driver_fails_each_method_once_and_records_calls() {
    let driver = FakeDriver::new().with_instance("dev-1");
    let sent = AgentMessage {
        id: "m-1".into(),
        from: "operator".into(),
        task_id: None,
        body: "hi".into(),
    };
    for operation in ["deliver", "events"] {
        driver.fail_next(operation, &message(operation));
    }
    scripted(
        "deliver",
        block_on(driver.deliver("dev-1", &sent, BusyLevel::Queue)),
    );
    scripted("events", block_on(driver.events("dev-1", None)));
    unscripted(
        "deliver",
        block_on(driver.deliver("dev-1", &sent, BusyLevel::Queue)),
    );
    let events = unscripted(
        "events",
        block_on(driver.events("dev-1", Some("0000000001"))),
    );
    assert!(!events.is_empty());

    let deliver = DriverCall::Deliver {
        instance_id: "dev-1".into(),
        message: sent,
        mode: BusyLevel::Queue,
    };
    assert_eq!(
        driver.calls(),
        [
            deliver.clone(),
            DriverCall::Events {
                instance_id: "dev-1".into(),
                after_cursor: None
            },
            deliver,
            DriverCall::Events {
                instance_id: "dev-1".into(),
                after_cursor: Some("0000000001".into())
            },
        ]
    );
}

#[test]
fn store_fails_each_method_once_and_records_calls() {
    let store = FakeStore::new();
    let task = Task::new("T-1", "fix", "general", "code", 1);
    let workflow = Workflow::builtin_code();
    store.insert_workflow(workflow.clone());
    let event = StoredEvent {
        id: "e-1".into(),
        occurred_at_unix_ms: 1,
        kind: "created".into(),
        detail: String::new(),
    };
    let operations = [
        "create_task",
        "load_task",
        "compare_and_swap_task",
        "load_workflow",
        "append_event",
    ];
    for operation in operations {
        store.fail_next(operation, &message(operation));
    }
    scripted("create_task", block_on(store.create_task(&task)));
    scripted("load_task", block_on(store.load_task("T-1")));
    scripted(
        "compare_and_swap_task",
        block_on(store.compare_and_swap_task(&task, 1)),
    );
    scripted(
        "load_workflow",
        block_on(store.load_workflow(&workflow.id, workflow.version)),
    );
    scripted("append_event", block_on(store.append_event("T-1", &event)));

    unscripted("create_task", block_on(store.create_task(&task)));
    assert!(unscripted("load_task", block_on(store.load_task("T-1"))).is_some());
    let written = unscripted(
        "compare_and_swap_task",
        block_on(store.compare_and_swap_task(&task, 1)),
    );
    assert_eq!(written, CasResult::Written { new_version: 2 });
    assert!(
        unscripted(
            "load_workflow",
            block_on(store.load_workflow(&workflow.id, workflow.version))
        )
        .is_some()
    );
    unscripted("append_event", block_on(store.append_event("T-1", &event)));
    assert_eq!(store.events("T-1"), std::slice::from_ref(&event));

    let once = [
        StoreCall::CreateTask(task.clone()),
        StoreCall::LoadTask {
            task_id: "T-1".into(),
        },
        StoreCall::CompareAndSwapTask {
            task: task.clone(),
            expected_version: 1,
        },
        StoreCall::LoadWorkflow {
            workflow_id: workflow.id.clone(),
            version: workflow.version,
        },
        StoreCall::AppendEvent {
            task_id: "T-1".into(),
            event,
        },
    ];
    assert_eq!(store.calls(), [once.clone(), once].concat());
}

#[test]
fn runtime_fails_each_method_once_and_records_calls() {
    let runtime = FakeRuntime::new();
    let launch = HolderLaunch {
        instance_id: "dev-1".into(),
        backend: Backend::Codex,
        executable: "codex".into(),
        args: Vec::new(),
        working_directory: "/w".into(),
    };
    for operation in ["start_holder", "recover_holders", "stop_holder"] {
        runtime.fail_next(operation, &message(operation));
    }
    scripted("start_holder", block_on(runtime.start_holder(&launch)));
    scripted("recover_holders", block_on(runtime.recover_holders()));
    scripted("stop_holder", block_on(runtime.stop_holder("dev-1")));
    assert!(
        runtime.running().is_empty(),
        "a failed start started a holder"
    );

    unscripted("start_holder", block_on(runtime.start_holder(&launch)));
    assert_eq!(
        unscripted("recover_holders", block_on(runtime.recover_holders())).len(),
        1
    );
    unscripted("stop_holder", block_on(runtime.stop_holder("dev-1")));

    let once = [
        RuntimeCall::Start(launch),
        RuntimeCall::Recover,
        RuntimeCall::Stop {
            instance_id: "dev-1".into(),
        },
    ];
    assert_eq!(runtime.calls(), [once.clone(), once].concat());
}

#[test]
fn runner_fails_once_and_records_calls() {
    let runner = FakeRunner::new();
    runner.on("make", ScriptedCommand::exits(0));
    runner.fail_next("run", &message("run"));
    scripted("run", block_on(runner.run("make", "/w", 1_000)));
    let output = unscripted("run", block_on(runner.run("make", "/w", 1_000)));
    assert_eq!(output.exit_code, Some(0));
    let call = RunnerCall {
        command: "make".into(),
        working_directory: "/w".into(),
        timeout_ms: 1_000,
    };
    assert_eq!(runner.calls(), [call.clone(), call]);
}

#[test]
fn notifier_fails_once_and_records_calls() {
    let notifier = FakeNotifier::new();
    let notification = Notification {
        severity: NotificationSeverity::Attention,
        title: "t".into(),
        body: "b".into(),
        task_id: None,
    };
    notifier.fail_next("notify", &message("notify"));
    scripted("notify", block_on(notifier.notify(&notification)));
    unscripted("notify", block_on(notifier.notify(&notification)));
    assert_eq!(notifier.delivered(), std::slice::from_ref(&notification));
    assert_eq!(notifier.calls(), [notification.clone(), notification]);
}
