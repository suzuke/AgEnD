//! Retry terminal cleanup before allocating capacity to another task.
use super::*;

impl<S, D, E, C, V> Engine<S, D, E, C, V>
where
    S: PipelineStore + Send + 'static,
    S::Error: std::fmt::Display,
    D: Driver + Send + 'static,
    D::Error: std::fmt::Display,
    E: PipelineExecutor,
    C: Clock + Send + Sync + 'static,
    V: PipelineView,
{
    pub(super) async fn cleanup_terminal(&self, task: &Task) -> Result<(), Refusal> {
        if !matches!(
            task.status,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Superseded
        ) {
            return Ok(());
        }
        for b in self.store.bindings().await.map_err(db)? {
            if b.task == task.id
                && let Err((_, reason)) = self.release(&b, task.status == TaskStatus::Done).await
            {
                // Retain the durable binding until WIP and unbinding succeed.
                // A later wake retries without preventing unrelated dispatch.
                log::line(&format!("{}: cleanup deferred: {reason}", task.id));
            }
        }
        if self
            .store
            .bindings()
            .await
            .map_err(db)?
            .iter()
            .any(|b| b.task == task.id)
        {
            return Ok(());
        }
        let row = self
            .store
            .load_task(&task.id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("terminal task disappeared"))?;
        if row.task.assignee.is_none() {
            return Ok(());
        }
        let mut free = row.task;
        free.assignee = None;
        if !matches!(
            self.store
                .compare_and_swap_task(&free, row.version)
                .await
                .map_err(db)?,
            CasResult::Written { .. }
        ) {
            return Err(invalid("terminal cleanup CAS conflict"));
        }
        Ok(())
    }
}
