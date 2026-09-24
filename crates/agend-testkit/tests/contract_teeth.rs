//! Each contract suite catches a fake that drifted: a wrapper that breaks
//! one rule fails the case named for that rule, and only that kind of case.

use agend_core::model::DeliveryState;
use agend_core::pipeline::task::Task;
use agend_core::pipeline::workflow::Workflow;
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::*;
use agend_testkit::contract::fakes::{FakeDriverFixture, FakeRunnerFixture};
use agend_testkit::contract::{Report, clock, driver, forge, notifier, runner, runtime, store};
use agend_testkit::fakes::{FakeClock, FakeError, FakeForge, FakeNotifier, FakeRuntime, FakeStore};

fn failing_cases(report: &Report) -> Vec<&'static str> {
    report
        .results
        .iter()
        .filter(|(_, r)| r.is_err())
        .map(|(name, _)| *name)
        .collect()
}

/// Merges whatever head it is given.
struct MergesStaleHeads(FakeForge);

impl Forge for MergesStaleHeads {
    type Error = FakeError;
    async fn submit(&self, change: &Submission) -> Result<SubmittedChange, FakeError> {
        self.0.submit(change).await
    }
    async fn head(&self, branch: &str) -> Result<String, FakeError> {
        self.0.head(branch).await
    }
    async fn merge_if_head_is(&self, request: &MergeRequest) -> Result<MergeResult, FakeError> {
        let current = self.0.head(&request.branch).await?;
        let honest = MergeRequest {
            branch: request.branch.clone(),
            expected_head: current,
        };
        self.0.merge_if_head_is(&honest).await
    }
}

impl forge::ForgeFixture for MergesStaleHeads {
    type Forge = Self;
    type Error = FakeError;
    fn forge(&self) -> &Self {
        self
    }
    fn commit_to(&self, branch: &str) -> String {
        self.0.push(branch)
    }
}

#[test]
fn forge_contract_catches_a_merge_that_ignores_the_head() {
    let report = forge::run("broken", || MergesStaleHeads(FakeForge::new()));
    assert_eq!(
        failing_cases(&report),
        ["merge_with_stale_head_echoes_actual_head"],
        "{report}"
    );
    assert!(
        report
            .to_string()
            .contains("FAIL forge.merge_with_stale_head_echoes_actual_head: expected HeadChanged")
    );
}

/// Writes regardless of the expected version.
struct LastWriterWins(FakeStore);

impl Store for LastWriterWins {
    type Error = FakeError;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, FakeError> {
        self.0.load_task(id).await
    }
    async fn create_task(&self, task: &Task) -> Result<(), FakeError> {
        self.0.create_task(task).await
    }
    async fn compare_and_swap_task(&self, task: &Task, _: u64) -> Result<CasResult, FakeError> {
        match self.0.load_task(&task.id).await? {
            Some(current) => self.0.compare_and_swap_task(task, current.version).await,
            None => self.0.compare_and_swap_task(task, 0).await,
        }
    }
    async fn load_workflow(&self, id: &str, version: u64) -> Result<Option<Workflow>, FakeError> {
        self.0.load_workflow(id, version).await
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), FakeError> {
        self.0.append_event(id, event).await
    }
}

impl store::StoreFixture for LastWriterWins {
    type Store = Self;
    type Error = FakeError;
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.0.insert_workflow(workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        self.0.events(task_id)
    }
}

#[test]
fn store_contract_catches_a_write_that_ignores_the_version() {
    let report = store::run("broken", || LastWriterWins(FakeStore::new()));
    assert_eq!(
        failing_cases(&report),
        ["cas_with_stale_version_conflicts_and_changes_nothing"],
        "{report}"
    );
}

/// Replays every event no matter the cursor.
struct IgnoresCursor(FakeDriverFixture);

impl Driver for IgnoresCursor {
    type Error = FakeError;
    async fn deliver(
        &self,
        id: &str,
        message: &AgentMessage,
        mode: BusyLevel,
    ) -> Result<DeliveryReceipt, FakeError> {
        self.0.driver.deliver(id, message, mode).await
    }
    async fn events(&self, id: &str, _: Option<&str>) -> Result<Vec<DriverEvent>, FakeError> {
        self.0.driver.events(id, None).await
    }
}

impl driver::DriverFixture for IgnoresCursor {
    type Driver = Self;
    type Error = FakeError;
    fn driver(&self) -> &Self {
        self
    }
    fn instance_id(&self) -> &str {
        FakeDriverFixture::INSTANCE
    }
}

#[test]
fn driver_contract_catches_events_that_ignore_the_cursor() {
    let report = driver::run("broken", || IgnoresCursor(FakeDriverFixture::new()));
    assert_eq!(
        failing_cases(&report),
        ["events_after_a_cursor_are_only_newer_ones"],
        "{report}"
    );
}

#[test]
fn driver_contract_catches_a_delivery_left_queued() {
    let report = driver::run("broken", || {
        let fixture = FakeDriverFixture::new();
        for _ in 0..8 {
            fixture.driver.next_receipt(DeliveryReceipt {
                backend_message_id: None,
                state: DeliveryState::Queued,
            });
        }
        fixture
    });
    assert!(
        failing_cases(&report).contains(&"delivery_to_idle_instance_is_sent"),
        "{report}"
    );
}

/// Forgets every holder it started.
struct ForgetfulRuntime(FakeRuntime);

impl Runtime for ForgetfulRuntime {
    type Error = FakeError;
    async fn start_holder(&self, launch: &HolderLaunch) -> Result<HolderHandle, FakeError> {
        self.0.start_holder(launch).await
    }
    async fn stop_holder(&self, id: &str) -> Result<(), FakeError> {
        self.0.stop_holder(id).await
    }
    async fn recover_holders(&self) -> Result<Vec<HolderHandle>, FakeError> {
        Ok(Vec::new())
    }
}

impl runtime::RuntimeFixture for ForgetfulRuntime {
    type Runtime = Self;
    type Error = FakeError;
    fn runtime(&self) -> &Self {
        self
    }
    fn launch(&self, id: &str) -> HolderLaunch {
        runtime::RuntimeFixture::launch(&self.0, id)
    }
}

#[test]
fn runtime_contract_catches_a_recovery_that_loses_holders() {
    let report = runtime::run("broken", || ForgetfulRuntime(FakeRuntime::new()));
    assert_eq!(
        failing_cases(&report),
        [
            "recovery_lists_running_holders",
            "stopped_holder_is_not_recovered"
        ],
        "{report}"
    );
}

/// Truncates bodies to 10 bytes (v1 truncated pushes; delivery.md).
struct Truncates(FakeNotifier);

impl Notifier for Truncates {
    type Error = FakeError;
    async fn notify(&self, n: &Notification) -> Result<(), FakeError> {
        let mut short = n.clone();
        short.body.truncate(10);
        self.0.notify(&short).await
    }
}

impl notifier::NotifierFixture for Truncates {
    type Notifier = Self;
    type Error = FakeError;
    fn notifier(&self) -> &Self {
        self
    }
    fn received(&self) -> Vec<Notification> {
        self.0.delivered()
    }
}

#[test]
fn notifier_contract_catches_truncated_bodies() {
    let report = notifier::run("broken", || Truncates(FakeNotifier::new()));
    assert_eq!(report.passed(), 0, "{report}");
}

/// Reports seconds instead of milliseconds.
struct Seconds(FakeClock);

impl Clock for Seconds {
    fn now_unix_ms(&self) -> u64 {
        self.0.now_unix_ms() / 1_000
    }
}

impl clock::ClockFixture for Seconds {
    type Clock = Self;
    fn clock(&self) -> &Self {
        self
    }
    fn let_time_pass(&self) {
        self.0.advance(1);
    }
}

#[test]
fn clock_contract_catches_seconds() {
    let report = clock::run("broken", || Seconds(FakeClock::default()));
    assert_eq!(
        failing_cases(&report),
        ["reads_unix_milliseconds"],
        "{report}"
    );
}

/// Reports a timeout as exit code 124 (what `timeout(1)` does).
struct TimeoutExitCode(FakeRunnerFixture);

impl Runner for TimeoutExitCode {
    type Error = FakeError;
    async fn run(&self, c: &str, wd: &str, timeout_ms: u64) -> Result<CommandOutput, FakeError> {
        let mut output = self.0.runner.run(c, wd, timeout_ms).await?;
        if output.timed_out {
            output.exit_code = Some(124);
        }
        Ok(output)
    }
}

impl runner::RunnerFixture for TimeoutExitCode {
    type Runner = Self;
    type Error = FakeError;
    fn runner(&self) -> &Self {
        self
    }
    fn working_directory(&self) -> &str {
        runner::RunnerFixture::working_directory(&self.0)
    }
}

#[test]
fn runner_contract_catches_a_timeout_with_an_exit_code() {
    let report = runner::run("broken", || TimeoutExitCode(FakeRunnerFixture::new()));
    assert_eq!(
        failing_cases(&report),
        ["timeout_reports_timed_out_without_exit_code"],
        "{report}"
    );
}
