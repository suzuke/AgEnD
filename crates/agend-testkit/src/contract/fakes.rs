//! Contract fixtures for the fakes in `crate::fakes`, and [`run_all_fakes`].

use agend_core::model::Backend;
use agend_core::pipeline::workflow::Workflow;
use agend_core::traits::{HolderLaunch, Notification, StoredEvent};

use super::{Report, clock, driver, forge, notifier, runner, runtime, store};
use crate::fakes::{
    FakeClock, FakeDriver, FakeError, FakeForge, FakeNotifier, FakeRunner, FakeRuntime, FakeStore,
    ScriptedCommand,
};
use crate::tempdir::TempDir;

/// Name used for the fake side in reports (`contract Forge: fake 8/8 pass`).
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
}

impl store::StoreFixture for FakeStore {
    type Store = FakeStore;
    type Error = FakeError;

    fn store(&self) -> &FakeStore {
        self
    }

    fn insert_workflow(&self, workflow: &Workflow) {
        FakeStore::insert_workflow(self, workflow.clone());
    }

    fn events(&self, task_id: &str) -> Vec<StoredEvent> {
        FakeStore::events(self, task_id)
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

    fn driver(&self) -> &FakeDriver {
        &self.driver
    }

    fn instance_id(&self) -> &str {
        Self::INSTANCE
    }
}

impl runtime::RuntimeFixture for FakeRuntime {
    type Runtime = FakeRuntime;
    type Error = FakeError;

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
                    .stdout(command.stdout)
                    .stderr(command.stderr)
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
