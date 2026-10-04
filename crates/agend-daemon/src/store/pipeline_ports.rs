//! Core pipeline port implemented by the SQLite adapter.
use super::*;
use agend_core::pipeline::workflow::Workflow;
use agend_core::runtime_records::*;
use agend_core::traits::{StoredEvent, TaskProgress};

impl agend_core::pipeline::ports::PipelineStore for SqliteStore {
    async fn advance_pipeline(
        &self,
        task: &Task,
        version: u64,
        progress: &TaskProgress,
        event: &StoredEvent,
        confirmation: Option<&str>,
    ) -> Result<CasResult, StoreError> {
        self.advance_with_receipt(task, version, progress, event, confirmation, true)
            .await
    }

    async fn advance_message(
        &self,
        id: &str,
        next: agend_core::model::DeliveryState,
        turn_id: Option<String>,
        now: u64,
    ) -> Result<(), StoreError> {
        let message_id = id.to_owned();
        let has_turn = turn_id.is_some();
        let observed = self
            .call(move |conn| {
                use agend_core::model::DeliveryState::*;
                if super::claude::get(conn, &message_id)?.is_none() {
                    return Ok(false);
                }
                let message = super::messages::get(conn, &message_id)?
                    .ok_or_else(|| StoreError::Invalid("missing Claude dispatch".into()))?;
                // The Driver/helper already owns the transaction. The pipeline
                // may observe that state (or a receipt overtaken by ACK), but
                // cannot invent Sent/Confirmed or record a generic backend turn.
                let recorded = next == message.state
                    || matches!(
                        (next, message.state),
                        (Queued, Sent | Confirmed | Failed) | (Sent, Confirmed | Failed)
                    );
                if has_turn || !recorded {
                    return Err(StoreError::Invalid(
                        "Claude dispatch receipt is not recorded by its Driver/helper".into(),
                    ));
                }
                Ok(true)
            })
            .await?;
        if observed {
            return Ok(());
        }
        SqliteStore::advance_message(self, id, next, turn_id, now).await
    }
    async fn teams(&self) -> Result<Vec<Team>, StoreError> {
        SqliteStore::teams(self).await
    }
    async fn add_team(&self, team: &Team) -> Result<(), StoreError> {
        SqliteStore::add_team(self, team).await
    }
    async fn members(&self) -> Result<Vec<Member>, StoreError> {
        SqliteStore::members(self).await
    }
    async fn join_team(&self, team: &str, instance: &str, role: &str) -> Result<(), StoreError> {
        SqliteStore::join_team(self, team, instance, role).await
    }
    async fn set_team_workflow(&self, team: &str, workflow: &str) -> Result<(), StoreError> {
        SqliteStore::set_team_workflow(self, team, workflow).await
    }
    async fn progress(&self, task: &str) -> Result<Option<Progress>, StoreError> {
        SqliteStore::progress(self, task).await
    }
    async fn task_note(
        &self,
        task: &str,
        block: Option<String>,
        attention: Option<String>,
        ack: bool,
    ) -> Result<(), StoreError> {
        SqliteStore::task_note(self, task, block, attention, ack).await
    }
    async fn bindings(&self) -> Result<Vec<BindingRow>, StoreError> {
        SqliteStore::bindings(self).await
    }
    async fn put_binding(&self, b: &BindingRow) -> Result<(), StoreError> {
        SqliteStore::put_binding(self, b).await
    }
    async fn delete_binding(&self, instance: &str) -> Result<(), StoreError> {
        SqliteStore::delete_binding(self, instance).await
    }
    async fn latest_workflow(
        &self,
        id: &str,
    ) -> Result<Option<agend_core::pipeline::workflow::Workflow>, StoreError> {
        SqliteStore::latest_workflow(self, id).await
    }
    async fn workflow_ids(&self) -> Result<Vec<String>, StoreError> {
        SqliteStore::workflow_ids(self).await
    }
    async fn held_task(&self, instance: &str) -> Result<Option<String>, StoreError> {
        SqliteStore::held_task(self, instance).await
    }
    async fn asks(&self) -> Result<Vec<AskRow>, StoreError> {
        SqliteStore::asks(self).await
    }
    async fn save_ask(&self, row: &AskRow) -> Result<(), StoreError> {
        SqliteStore::save_ask(self, row).await
    }
    async fn pending_answers(&self) -> Result<Vec<PendingAnswer>, StoreError> {
        SqliteStore::pending_answers(self).await
    }
    async fn mark_answer_sent(&self, seq: u64) -> Result<(), StoreError> {
        SqliteStore::mark_answer_sent(self, seq).await
    }
    async fn reminders(&self) -> Result<Vec<(u64, String, u64)>, StoreError> {
        SqliteStore::reminders(self).await
    }
    async fn add_reminder(&self, task: &str, due: u64) -> Result<(), StoreError> {
        SqliteStore::add_reminder(self, task, due).await
    }
    async fn delete_reminder(&self, seq: u64) -> Result<(), StoreError> {
        SqliteStore::delete_reminder(self, seq).await
    }
    async fn create_pipeline_task(
        &self,
        task: &agend_core::pipeline::task::Task,
        pipeline: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        SqliteStore::create_pipeline_task(self, task, pipeline, now).await
    }
    async fn tasks(&self) -> Result<Vec<Task>, StoreError> {
        SqliteStore::tasks(self).await
    }
    async fn instance(&self, id: &str) -> Result<Option<Instance>, StoreError> {
        SqliteStore::instance(self, id).await
    }
    async fn instances(&self) -> Result<Vec<Instance>, StoreError> {
        SqliteStore::instances(self).await
    }
    async fn message(&self, id: &str) -> Result<Option<Message>, StoreError> {
        SqliteStore::message(self, id).await
    }
    async fn claim_message(
        &self,
        message: &NewMessage,
        now_unix_ms: u64,
    ) -> Result<Claim, StoreError> {
        SqliteStore::claim_message(self, message, now_unix_ms).await
    }
    async fn load_events(&self, task_id: &str) -> Result<Vec<StoredEvent>, StoreError> {
        SqliteStore::load_events(self, task_id).await
    }
    async fn save_workflow(&self, workflow: &Workflow) -> Result<(), StoreError> {
        SqliteStore::save_workflow(self, workflow).await
    }
}
