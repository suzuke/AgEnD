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
    fn base_head(&self) -> String {
        self.0.base_head()
    }
}

#[test]
fn forge_contract_catches_a_merge_that_ignores_the_head() {
    let report = forge::run("broken", || MergesStaleHeads(FakeForge::new()));
    assert_eq!(
        failing_cases(&report),
        [
            "merge_with_stale_head_echoes_actual_head",
            "stale_merge_changes_nothing"
        ],
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

/// Merges the current head, then claims the head changed (a check-after-merge
/// race in a real forge): the refusal is a lie, the base moved.
struct MergesThenRefuses(FakeForge);

impl Forge for MergesThenRefuses {
    type Error = FakeError;
    async fn submit(&self, change: &Submission) -> Result<SubmittedChange, FakeError> {
        self.0.submit(change).await
    }
    async fn head(&self, branch: &str) -> Result<String, FakeError> {
        self.0.head(branch).await
    }
    async fn merge_if_head_is(&self, request: &MergeRequest) -> Result<MergeResult, FakeError> {
        let current = self.0.head(&request.branch).await?;
        let merged = self
            .0
            .merge_if_head_is(&MergeRequest {
                branch: request.branch.clone(),
                expected_head: current.clone(),
            })
            .await?;
        if current == request.expected_head {
            Ok(merged)
        } else {
            Ok(MergeResult::HeadChanged {
                actual_head: current,
            })
        }
    }
}

impl forge::ForgeFixture for MergesThenRefuses {
    type Forge = Self;
    type Error = FakeError;
    fn forge(&self) -> &Self {
        self
    }
    fn commit_to(&self, branch: &str) -> String {
        self.0.push(branch)
    }
    fn base_head(&self) -> String {
        self.0.base_head()
    }
}

#[test]
fn forge_contract_catches_a_refused_merge_that_merged_anyway() {
    let report = forge::run("broken", || MergesThenRefuses(FakeForge::new()));
    assert_eq!(
        failing_cases(&report),
        ["stale_merge_changes_nothing"],
        "{report}"
    );
}

/// Versions toggle 1 -> 2 -> 1 (ABA): every single write looks newer.
struct TogglingVersions {
    inner: FakeStore,
}

fn toggled(version: u64) -> u64 {
    if version % 2 == 1 { 1 } else { 2 }
}

impl Store for TogglingVersions {
    type Error = FakeError;
    async fn load_task(&self, id: &str) -> Result<Option<VersionedTask>, FakeError> {
        Ok(self.inner.load_task(id).await?.map(|v| VersionedTask {
            version: toggled(v.version),
            task: v.task,
        }))
    }
    async fn create_task(&self, task: &Task) -> Result<(), FakeError> {
        self.inner.create_task(task).await
    }
    async fn compare_and_swap_task(
        &self,
        task: &Task,
        expected: u64,
    ) -> Result<CasResult, FakeError> {
        let Some(current) = self.inner.load_task(&task.id).await? else {
            return self.inner.compare_and_swap_task(task, expected).await;
        };
        if toggled(current.version) != expected {
            return Ok(CasResult::Conflict {
                current_version: Some(toggled(current.version)),
            });
        }
        Ok(
            match self
                .inner
                .compare_and_swap_task(task, current.version)
                .await?
            {
                CasResult::Written { new_version } => CasResult::Written {
                    new_version: toggled(new_version),
                },
                CasResult::Conflict { current_version } => CasResult::Conflict {
                    current_version: current_version.map(toggled),
                },
            },
        )
    }
    async fn load_workflow(&self, id: &str, version: u64) -> Result<Option<Workflow>, FakeError> {
        self.inner.load_workflow(id, version).await
    }
    async fn append_event(&self, id: &str, event: &StoredEvent) -> Result<(), FakeError> {
        self.inner.append_event(id, event).await
    }
}

impl store::StoreFixture for TogglingVersions {
    type Store = Self;
    type Error = FakeError;
    fn store(&self) -> &Self {
        self
    }
    fn insert_workflow(&self, workflow: &Workflow) {
        self.inner.insert_workflow(workflow.clone());
    }
    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        self.inner.events(task_id)
    }
}

#[test]
fn store_contract_catches_versions_that_go_back() {
    let report = store::run("broken", || TogglingVersions {
        inner: FakeStore::new(),
    });
    assert_eq!(
        failing_cases(&report),
        ["cas_with_current_version_writes_a_newer_version"],
        "{report}"
    );
}

/// Reports the timeout only after the command would have finished (a runner
/// that checks elapsed time after `wait()`).
struct TimesOutLate(FakeRunnerFixture);

impl Runner for TimesOutLate {
    type Error = FakeError;
    async fn run(&self, c: &str, wd: &str, timeout_ms: u64) -> Result<CommandOutput, FakeError> {
        let output = self.0.runner.run(c, wd, timeout_ms).await?;
        if output.timed_out {
            std::thread::sleep(std::time::Duration::from_millis(
                runner::TIMES_OUT.duration_ms,
            ));
        }
        Ok(output)
    }
}

impl runner::RunnerFixture for TimesOutLate {
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
fn runner_contract_catches_a_timeout_reported_late() {
    let report = runner::run("broken", || TimesOutLate(FakeRunnerFixture::new()));
    assert_eq!(
        failing_cases(&report),
        ["timeout_reports_timed_out_without_exit_code"],
        "{report}"
    );
}

/// Reports the timeout on time but never kills the command: it really runs
/// the command with `sh` and leaves it running. The child is waited for (not
/// signalled) when the wrapper drops.
struct LeavesChildRunning {
    fixture: FakeRunnerFixture,
    children: std::sync::Mutex<Vec<std::process::Child>>,
}

impl Runner for LeavesChildRunning {
    type Error = FakeError;
    async fn run(&self, c: &str, wd: &str, timeout_ms: u64) -> Result<CommandOutput, FakeError> {
        let output = self.fixture.runner.run(c, wd, timeout_ms).await?;
        if output.timed_out {
            let child = std::process::Command::new("sh")
                .args(["-c", c])
                .current_dir(wd)
                .spawn()
                .expect("spawn sh");
            self.children.lock().unwrap().push(child);
        }
        Ok(output)
    }
}

impl Drop for LeavesChildRunning {
    fn drop(&mut self) {
        for mut child in self.children.lock().unwrap().drain(..) {
            let _ = child.wait();
        }
    }
}

impl runner::RunnerFixture for LeavesChildRunning {
    type Runner = Self;
    type Error = FakeError;
    fn runner(&self) -> &Self {
        self
    }
    fn working_directory(&self) -> &str {
        runner::RunnerFixture::working_directory(&self.fixture)
    }
}

#[test]
fn runner_contract_catches_a_timeout_that_leaves_the_child_running() {
    let report = runner::run("broken", || LeavesChildRunning {
        fixture: FakeRunnerFixture::new(),
        children: std::sync::Mutex::new(Vec::new()),
    });
    assert_eq!(
        failing_cases(&report),
        ["timeout_reports_timed_out_without_exit_code"],
        "{report}"
    );
}
