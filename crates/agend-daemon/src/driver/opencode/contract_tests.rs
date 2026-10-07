//! Unmodified DRV suite against the production Driver/Worker, real SQLite,
//! and the native REST producer. Each boot drops worker + DB connection.
use super::{OpenCodeDriver, api::Session, http::Http, worker::Worker};
use crate::store::{Instance, InstanceStatus, SqliteStore};
use agend_core::model::Backend;
use agend_testkit::{
    block_on,
    contract::driver::{self, DriverFixture},
    fake_agent::opencode::Server,
    tempdir::TempDir,
};
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
struct Persisted {
    dir: TempDir,
    port: u16,
    session: String,
}
struct Fixture {
    driver: OpenCodeDriver,
    persisted: Arc<Persisted>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new("opencode-driver-contract").unwrap();
        let server = Server::start(0, Duration::from_millis(30), None).unwrap();
        let session =
            Session::create(Http::new(server.port(), "fixture", "/fixture").unwrap()).unwrap();
        let persisted = Arc::new(Persisted {
            dir,
            port: server.port(),
            session: session.id().into(),
        });
        {
            let store = SqliteStore::open(persisted.dir.path(), 0).unwrap();
            block_on(store.add_instance(&Instance {
                id: "open-contract".into(),
                backend: Backend::Opencode,
                program: "unused".into(),
                args: vec![],
                working_directory: "/fixture".into(),
                session_id: Some(persisted.session.clone()),
                status: InstanceStatus::Running,
                session_started: true,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            }))
            .unwrap();
        }
        Self::boot(&persisted)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
impl DriverFixture for Fixture {
    type Driver = OpenCodeDriver;
    type Error = crate::driver::codex::DriverError;
    type Persisted = Arc<Persisted>;
    fn driver(&self) -> &Self::Driver {
        &self.driver
    }
    fn instance_id(&self) -> &str {
        "open-contract"
    }
    fn persisted(&self) -> Self::Persisted {
        self.persisted.clone()
    }
    fn boot(p: &Self::Persisted) -> Self {
        let store = Arc::new(SqliteStore::open(p.dir.path(), 0).unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            store: store.clone(),
            instance: "open-contract".into(),
            session: Session::resume(
                Http::new(p.port, "fixture", "/fixture").unwrap(),
                &p.session,
            )
            .unwrap(),
            cancelled: cancel.clone(),
            model: None,
            history_before: std::cell::RefCell::new(None),
            reconcile_after: Cell::new(0),
        };
        worker.tick().unwrap(); // Restore backend events before serving a cursor.
        let stop = cancel.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Err(e) = worker.tick() {
                    assert!(stop.load(Ordering::SeqCst), "worker failed: {e}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        Self {
            driver: OpenCodeDriver::new(store),
            persisted: p.clone(),
            cancel,
            thread: Some(thread),
        }
    }
    fn emit_while_down(p: &Self::Persisted) {
        let session = Session::resume(
            Http::new(p.port, "fixture", "/fixture").unwrap(),
            &p.session,
        )
        .unwrap();
        session
            .submit(
                &crate::store::instances::new_session_id().unwrap(),
                "complete while daemon is down",
                None,
            )
            .unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        while session.busy().unwrap() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
#[test]
fn all_driver_contracts_use_native_rest_and_reopened_sqlite() {
    let report = driver::run("OpenCode native REST + SQLite", Fixture::new);
    for result in &report.results {
        assert!(
            result.result.is_ok(),
            "{} {}: {:?}",
            result.rule,
            result.name,
            result.result
        );
    }
    assert_eq!(report.passed(), 10);
}
