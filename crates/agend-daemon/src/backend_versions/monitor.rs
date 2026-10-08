//! Daily discovery for the managed fleet. Never launches, installs or switches.
use crate::{handlers::Context, log};
use agend_core::model::Backend;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct Monitor {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Monitor {
    pub(crate) fn start(ctx: Arc<Context>) -> Self {
        let (stop, mut stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            match crate::notifier::config::read(ctx.runtime.home()) {
                Ok(config) if config.registry_checks == Some(false) => return,
                Ok(_) => (),
                Err(error) => {
                    log::line(&format!("Backend registry monitor disabled: {error}"));
                    return;
                }
            }
            // A canary checks a fixed candidate and must not discover unrelated
            // releases or make extra external requests during its budget.
            if ctx
                .runtime
                .home()
                .join("canary-scope.json")
                .symlink_metadata()
                .is_ok()
            {
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
                    for instance in ctx.fleet.view().instances {
                        if instance.backend != backend.as_str() {
                            continue;
                        }
                        match ctx.store.managed_launch(&instance.instance_id).await {
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
                    if let Err(e) = check(
                        &ctx.store,
                        backend,
                        log::now_unix_ms(),
                        super::registry::latest,
                    )
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
        let _ = self.task.await;
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
}
