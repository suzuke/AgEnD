//! Contract fixtures for the fakes in `crate::fakes`, and [`run_all_fakes`].

use agend_core::model::Backend;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{DriverEventKind, HolderHandle, HolderLaunch, Notification, StoredEvent};

use super::{Report, clock, driver, forge, notifier, runner, runtime, store};
use crate::fakes::{
    FakeBackend, FakeClock, FakeDriver, FakeError, FakeForge, FakeHolders, FakeNotifier,
    FakeRunner, FakeRuntime, FakeStore, FakeStoreFile, ScriptedCommand,
};
use crate::tempdir::TempDir;

/// Name used for the fake side in reports (`contract Forge: fake 10/10 pass`).
pub const FAKE: &str = "fake";

impl forge::ForgeFixture for FakeForge {
    type Forge = FakeForge;
    type Error = FakeError;

    fn forge(&self) -> &FakeForge {
        self
    }

    fn commit_to(&self, branch: &str) -> String {
        self.push(branch)
    }

    fn base_head(&self) -> String {
        FakeForge::base_head(self)
    }

    fn base_contains(&self, commit: &str) -> bool {
        FakeForge::base_contains(self, commit)
    }
}

impl store::StoreFixture for FakeStore {
    type Store = FakeStore;
    type Error = FakeError;
    type Persisted = FakeStoreFile;

    fn store(&self) -> &FakeStore {
        self
    }

    fn insert_workflow(&self, workflow: &Workflow) {
        FakeStore::insert_workflow(self, workflow.clone());
    }

    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        FakeStore::events(self, task_id)
    }

    fn persisted(&self) -> FakeStoreFile {
        self.file()
    }

    fn boot(file: &FakeStoreFile) -> Self {
        FakeStore::open(file)
    }
}

/// A `FakeDriver` with one running instance.
pub struct FakeDriverFixture {
    pub driver: FakeDriver,
}

impl FakeDriverFixture {
    pub const INSTANCE: &'static str = "dev-1";

    pub fn new() -> Self {
        Self {
            driver: FakeDriver::new().with_instance(Self::INSTANCE),
        }
    }
}

impl Default for FakeDriverFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl driver::DriverFixture for FakeDriverFixture {
    type Driver = FakeDriver;
    type Error = FakeError;
    type Persisted = FakeBackend;

    fn driver(&self) -> &FakeDriver {
        &self.driver
    }

    fn instance_id(&self) -> &str {
        Self::INSTANCE
    }

    /// The fake finishes a turn inside `deliver`; waiting longer for a turn
    /// that is not coming (DRV-9) only slows the suite down.
    fn turn_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(200)
    }

    fn persisted(&self) -> FakeBackend {
        self.driver.backend()
    }

    fn boot(backend: &FakeBackend) -> Self {
        Self {
            driver: FakeDriver::connect(backend),
        }
    }

    /// Pushes a whole turn straight into the backend
    /// ([`FakeBackend::push_event`]), not through a driver.
    fn emit_while_down(backend: &FakeBackend) {
        for kind in [
            DriverEventKind::BusyChanged { busy: true },
            DriverEventKind::TurnCompleted {
                summary: Some("finished while the daemon was down".into()),
            },
            DriverEventKind::BusyChanged { busy: false },
        ] {
            backend.push_event(Self::INSTANCE, kind);
        }
    }
}

impl runtime::RuntimeFixture for FakeRuntime {
    type Runtime = FakeRuntime;
    type Error = FakeError;
    type Persisted = FakeHolders;

    fn runtime(&self) -> &FakeRuntime {
        self
    }

    fn launch(&self, instance_id: &str) -> HolderLaunch {
        HolderLaunch {
            instance_id: instance_id.into(),
            backend: Backend::Codex,
            executable: "fake-codex-app-server".into(),
            args: Vec::new(),
            working_directory: "/fake/workspace".into(),
        }
    }

    fn is_running(holders: &FakeHolders, handle: &HolderHandle) -> bool {
        holders.running().contains(handle)
    }

    fn persisted(&self) -> FakeHolders {
        self.holders()
    }

    fn boot(holders: &FakeHolders) -> Self {
        FakeRuntime::on(holders)
    }
}

impl notifier::NotifierFixture for FakeNotifier {
    type Notifier = FakeNotifier;
    type Error = FakeError;

    fn notifier(&self) -> &FakeNotifier {
        self
    }

    fn received(&self) -> Vec<Notification> {
        self.delivered()
    }
}

impl clock::ClockFixture for FakeClock {
    type Clock = FakeClock;

    fn clock(&self) -> &FakeClock {
        self
    }

    fn let_time_pass(&self) {
        self.advance(1);
    }

    fn utc_now_unix_ms(&self) -> u64 {
        // The fake's "now" is whatever the test set; reading it through the
        // trait would count as a read, so peek at the same value.
        self.peek()
    }
}

/// A `FakeRunner` scripted with what `sh` does for the contract commands,
/// in a real temp directory.
pub struct FakeRunnerFixture {
    pub runner: FakeRunner,
    directory: TempDir,
    canonical: String,
}

impl FakeRunnerFixture {
    pub fn new() -> Self {
        let directory = TempDir::new("runner").expect("create runner contract temp dir");
        let canonical = std::fs::canonicalize(directory.path())
            .expect("canonicalize runner contract temp dir")
            .to_string_lossy()
            .into_owned();
        let runner = FakeRunner::new();
        for command in runner::COMMANDS {
            runner.on(
                command.command,
                ScriptedCommand::exits(command.exit_code)
                    .stdout(command.expected_stdout())
                    .stderr(command.expected_stderr())
                    .takes_ms(command.duration_ms),
            );
        }
        runner.on(
            runner::PWD,
            ScriptedCommand::exits(0).stdout(format!("{canonical}\n")),
        );
        Self {
            runner,
            directory,
            canonical,
        }
    }

    pub fn directory(&self) -> &std::path::Path {
        self.directory.path()
    }
}

impl Default for FakeRunnerFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl runner::RunnerFixture for FakeRunnerFixture {
    type Runner = FakeRunner;
    type Error = FakeError;

    fn runner(&self) -> &FakeRunner {
        &self.runner
    }

    fn working_directory(&self) -> &str {
        &self.canonical
    }
}

/// Every contract case as `(contract, rule, case name)`, in trait order.
pub fn case_rules() -> Vec<(&'static str, &'static str, &'static str)> {
    fn tag<F>(
        contract: &'static str,
        cases: Vec<super::Case<F>>,
    ) -> Vec<(&'static str, &'static str, &'static str)> {
        cases.iter().map(|c| (contract, c.rule, c.name)).collect()
    }
    [
        tag("Driver", driver::cases::<FakeDriverFixture>()),
        tag("Forge", forge::cases::<FakeForge>()),
        tag("Store", store::cases::<FakeStore>()),
        tag("Runtime", runtime::cases::<FakeRuntime>()),
        tag("Notifier", notifier::cases::<FakeNotifier>()),
        tag("Clock", clock::cases::<FakeClock>()),
        tag("Runner", runner::cases::<FakeRunnerFixture>()),
    ]
    .concat()
}

/// Runs every contract suite against its fake, in trait order.
pub fn run_all_fakes() -> Vec<Report> {
    vec![
        driver::run(FAKE, FakeDriverFixture::new),
        forge::run(FAKE, FakeForge::new),
        store::run(FAKE, FakeStore::new),
        runtime::run(FAKE, FakeRuntime::new),
        notifier::run(FAKE, FakeNotifier::new),
        clock::run(FAKE, FakeClock::default),
        runner::run(FAKE, FakeRunnerFixture::new),
    ]
}
