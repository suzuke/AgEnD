//! Observe configured external executables without changing running holders.
use crate::{handlers::Context, log};
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
            let home = ctx.runtime.home();
            if home.join("canary-scope.json").symlink_metadata().is_ok() {
                return;
            }
            match crate::notifier::config::read(home) {
                Ok(config) if config.backend_version_checks == Some(false) => return,
                Ok(_) => (),
                Err(e) => {
                    log::line(&format!("Backend version monitor disabled: {e}"));
                    return;
                }
            }
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            loop {
                tokio::select! { _=stopped.changed()=>return, _=tick.tick()=>() }
                let instances = match ctx.store.instances().await {
                    Ok(rows) => rows,
                    Err(e) => {
                        log::line(&format!("Backend version instances unavailable: {e}"));
                        continue;
                    }
                };
                for instance in instances {
                    if *stopped.borrow() {
                        return;
                    }
                    if let Err(e) = check(&ctx, &instance.id, &stopped).await {
                        log::line(&format!("Backend version observation failed: {e}"));
                    }
                }
            }
        });
        Self { stop, task }
    }
    #[cfg(test)]
    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
    pub(crate) async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}
async fn check(
    ctx: &Context,
    id: &str,
    stopped: &watch::Receiver<bool>,
) -> Result<(), crate::store::StoreError> {
    let Some(instance) = ctx.store.instance(id).await? else {
        return Ok(());
    };
    // Managed admission owns artifact verification; never treat it as an
    // external executable merely because a disk probe was unavailable.
    if let Some(launch) = ctx.store.managed_launch(id).await?
        && launch.configured_program == instance.program
    {
        return Ok(());
    }
    if *stopped.borrow() {
        return Ok(());
    }
    let Some(ticket) = ctx
        .store
        .begin_system_version_check(id, log::now_unix_ms())
        .await?
    else {
        return Ok(());
    };
    let Some(backend) = agend_core::model::Backend::parse(&ticket.backend) else {
        return Ok(());
    };
    if *stopped.borrow() {
        return Ok(());
    }
    // Await the bounded child probe through shutdown; dropping a blocking
    // task would not stop its child process.
    let result = match ctx
        .runtime
        .observe_backend_version(id, backend, &ticket.program, &ticket.working_directory)
        .await
    {
        Ok(Some(value)) => Ok(value),
        Ok(None) => return Ok(()),
        Err(e) => {
            let mut message = e.to_string();
            if message.len() > 512 {
                let mut end = 512;
                while !message.is_char_boundary(end) {
                    end -= 1
                }
                message.truncate(end);
            }
            Err(message)
        }
    };
    ctx.store
        .finish_system_version_check(&ticket, log::now_unix_ms(), result)
        .await?;
    Ok(())
}
