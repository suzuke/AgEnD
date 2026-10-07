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
        self.cleanup_terminal_mode(task, false).await
    }

    pub(super) async fn cleanup_terminal_action(&self, task: &Task) -> Result<(), Refusal> {
        self.cleanup_terminal_mode(task, true).await
    }

    async fn cleanup_terminal_mode(
        &self,
        task: &Task,
        report_local_failure: bool,
    ) -> Result<(), Refusal> {
        if !matches!(
            task.status,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Superseded
        ) {
            return Ok(());
        }
        let deferred = self
            .store
            .progress(&task.id)
            .await
            .map_err(db)?
            .and_then(|p| p.attention_reason)
            .is_some_and(|reason| reason.starts_with("cleanup-remote:"));
        if !deferred
            && let Some(repo) = self.team(&task.team_id).await?.repo
            && let Err(reason) = self
                .executor
                .cleanup_remote(&repo, &task.id, task.merge_commit.is_some())
                .await
        {
            // Keep remote ownership and expose Retry; local WIP archival
            // and holder release can still finish independently.
            self.note(&task.id, Some(format!("cleanup-remote:{reason}")))
                .await?;
        }
        for b in self.store.bindings().await.map_err(db)? {
            if b.task == task.id
                && let Err(refusal) = self.release(&b, task.merge_commit.is_some()).await
            {
                // Immediate terminal actions must preserve their error contract;
                // only background cleanup may defer a local archival failure.
                if report_local_failure {
                    return Err(refusal);
                }
                let (_, reason) = refusal;
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
