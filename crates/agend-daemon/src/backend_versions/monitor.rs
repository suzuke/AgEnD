//! Daily discovery for the managed fleet. Never launches, installs or switches.
use crate::{handlers::Context, log};
use agend_core::model::Backend;
use std::{path::PathBuf, sync::Arc, time::Duration};
type Query = Arc<
    dyn Fn(Backend) -> Result<agend_core::setup::backend::PublishedBackend, String> + Send + Sync,
>;
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct Monitor {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Monitor {
    pub(crate) fn start(ctx: Arc<Context>) -> Self {
        Self::start_with(
            ctx.runtime.home().to_owned(),
            ctx.store.clone(),
            ctx.fleet.clone(),
            Arc::new(super::registry::latest),
        )
    }
    #[cfg(test)]
    pub(crate) fn start_at(ctx: Arc<Context>, origin: String) -> Self {
        Self::start_with(
            ctx.runtime.home().to_owned(),
            ctx.store.clone(),
            ctx.fleet.clone(),
            Arc::new(move |backend| {
                super::registry::fetch(backend, &origin, Duration::from_secs(5))
            }),
        )
    }
    fn start_with(
        home: PathBuf,
        store: Arc<crate::store::SqliteStore>,
        fleet: Arc<crate::fleet::Fleet>,
        query: Query,
    ) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            match crate::notifier::config::read(&home) {
                Ok(config) if config.registry_checks == Some(false) => return,
                Ok(_) => (),
                Err(error) => {
                    log::line(&format!("Backend registry monitor disabled: {error}"));
                    return;
                }
            }
            // A canary checks a fixed candidate and must not discover unrelated
            // releases or make extra external requests during its budget.
            if home.join("canary-scope.json").symlink_metadata().is_ok() {
                return;
            }
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = stopped.changed() => return,
                    _ = tick.tick() => {}
                }
                for backend in Backend::ALL {
                    if *stopped.borrow() {
                        return;
                    }
                    let mut managed = false;
                    for instance in fleet.view().instances {
                        if instance.backend != backend.as_str() {
                            continue;
                        }
                        match store.managed_launch(&instance.instance_id).await {
                            Ok(Some(launch))
                                if instance.program.as_deref()
                                    == Some(launch.configured_program.as_str())
                                    && launch.artifact.backend == backend.as_str() =>
                            {
                                managed = true;
                                break;
                            }
                            Ok(_) => (),
                            Err(e) => {
                                log::line(&format!("Backend monitor identity unavailable: {e}"))
                            }
                        }
                    }
                    if !managed {
                        continue;
                    }
                    if *stopped.borrow() {
                        return;
                    }
                    let query = query.clone();
                    if let Err(e) = check(&store, backend, log::now_unix_ms(), move |backend| {
                        query(backend)
                    })
                    .await
                    {
                        log::line(&format!("Backend registry check failed: {e}"));
                    }
                }
            }
        });
        Self { stop, task }
    }
    pub(crate) async fn stop(self) {
        let _ = self.stop.send(true);
        log::line("Backend registry monitor stop requested");
        let _ = self.task.await;
        log::line("Backend registry monitor stopped");
    }
}

async fn check<F>(
    store: &crate::store::SqliteStore,
    backend: Backend,
    now: u64,
    query: F,
) -> Result<bool, crate::store::StoreError>
where
    F: FnOnce(Backend) -> Result<agend_core::setup::backend::PublishedBackend, String>
        + Send
        + 'static,
{
    let Some(row) = store.begin_registry_check(backend, now).await? else {
        return Ok(false);
    };
    // Await bounded I/O through shutdown; abandoning a blocking handle does not stop it.
    let result = tokio::task::spawn_blocking(move || query(backend))
        .await
        .unwrap_or_else(|_| Err("backend registry worker failed".into()));
    store
        .finish_registry_check(backend, row.attempt, log::now_unix_ms(), result)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn scheduled_worker_reserves_before_query_and_reopen_does_not_repeat_it() {
        let dir = TempDir::new("registry-worker").unwrap();
        let home = dir.path().join("home");
        let store = Arc::new(crate::store::SqliteStore::open(&home, 100).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let inspect = store.clone();
        assert!(
            check(&store, Backend::Codex, 100, move |_| {
                count.fetch_add(1, Ordering::SeqCst);
                let recorded =
                    agend_testkit::block_on(inspect.registry_observation(Backend::Codex))
                        .unwrap()
                        .unwrap();
                assert_eq!(recorded.attempt, 1);
                assert!(recorded.completed_ms.is_none());
                Err("test registry unavailable".into())
            })
            .await
            .unwrap()
        );
        drop(store);
        let store = crate::store::SqliteStore::open(&home, 101).unwrap();
        assert!(
            !check(&store, Backend::Codex, 101, |_| panic!(
                "must not query before tomorrow"
            ))
            .await
            .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            store
                .registry_observation(Backend::Codex)
                .await
                .unwrap()
                .unwrap()
                .error
                .as_deref(),
            Some("test registry unavailable")
        );
    }
    #[tokio::test]
    async fn native_http_shutdown_waits_for_publication_and_never_starts_the_next_backend() {
        use agend_core::{
            protocol::client::{AgentState, InstanceView},
            runtime_records::{Instance, InstanceStatus},
            setup::backend::ImportedBackend,
            traits::HolderLaunch,
        };
        use std::io::{BufRead, BufReader, Write};
        let dir = TempDir::new("registry-stop").unwrap();
        let store = Arc::new(crate::store::SqliteStore::open(dir.path(), 100).unwrap());
        let fleet = Arc::new(crate::fleet::Fleet::new(100));
        for backend in [Backend::Claude, Backend::Codex] {
            let instance = Instance {
                id: backend.as_str().into(),
                backend,
                program: format!("/managed/{}", backend.as_str()),
                args: vec![],
                working_directory: "/workspace".into(),
                session_id: None,
                status: InstanceStatus::Failed,
                session_started: false,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            };
            store.add_instance(&instance).await.unwrap();
            store
                .prepare_managed_launch(
                    &instance,
                    &HolderLaunch {
                        instance_id: instance.id.clone(),
                        backend,
                        executable: instance.program.clone(),
                        args: vec![],
                        working_directory: instance.working_directory.clone(),
                    },
                    ImportedBackend {
                        format: 1,
                        backend: backend.as_str().into(),
                        version: "1.0".into(),
                        sha256: "a".repeat(64),
                        bytes: 1,
                    },
                    None,
                )
                .await
                .unwrap();
            fleet.set_instance(
                InstanceView {
                    program: Some(instance.program),
                    instance_id: instance.id,
                    team_id: "general".into(),
                    backend: backend.as_str().into(),
                    state: AgentState::Idle,
                    working_directory: Some(instance.working_directory),
                },
                "fixture".into(),
            );
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (started, arrived) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(request.starts_with("GET /@anthropic-ai%2fclaude-code/latest "));
            let body = include_bytes!("../../tests/fixtures/backend_registry/claude.json");
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            let middle = body.len() / 2;
            socket.write_all(&body[..middle]).unwrap();
            started.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap();
            socket.write_all(&body[middle..]).unwrap();
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let monitor = Monitor::start_with(
            dir.path().to_owned(),
            store.clone(),
            fleet,
            Arc::new(move |backend| {
                count.fetch_add(1, Ordering::SeqCst);
                super::super::registry::fetch(backend, &origin, Duration::from_secs(2))
            }),
        );
        tokio::time::timeout(Duration::from_secs(3), arrived)
            .await
            .unwrap()
            .unwrap();
        let mut stopping = tokio::spawn(monitor.stop());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut stopping)
                .await
                .is_err(),
            "stop abandoned active HTTP"
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(3), stopping)
            .await
            .unwrap()
            .unwrap();
        server.join().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let row = store
            .registry_observation(Backend::Claude)
            .await
            .unwrap()
            .unwrap();
        assert!(row.completed_ms.is_some() && row.latest.is_some());
        assert!(
            store
                .registry_observation(Backend::Codex)
                .await
                .unwrap()
                .is_none()
        );
        drop(store);
        let reopened = crate::store::SqliteStore::open(dir.path(), 200).unwrap();
        assert!(
            reopened
                .registry_observation(Backend::Claude)
                .await
                .unwrap()
                .unwrap()
                .latest
                .is_some()
        );
    }
}
