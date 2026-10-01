//! In-memory durable records for the serialized pipeline, including its outboxes.
use super::*;
use agend_core::{model::DeliveryState, pipeline::ports::PipelineStore, runtime_records::*};
#[derive(Debug, Default)]
pub(super) struct Data {
    teams: BTreeMap<String, Team>,
    members: BTreeMap<String, Member>,
    instances: BTreeMap<String, Instance>,
    bindings: BTreeMap<String, BindingRow>,
    notes: BTreeMap<String, (Option<String>, Option<String>, bool)>,
    pub(super) messages: BTreeMap<String, Message>,
    asks: BTreeMap<String, AskRow>,
    answers: Vec<(u64, String, String, Option<String>, String, bool)>,
    reminders: Vec<(u64, String, u64)>,
    next: u64,
}
impl FakeStore {
    pub fn insert_instance(&self, instance: Instance, member: Member) {
        let mut d = lock(&self.data);
        d.pipeline.members.insert(member.id.clone(), member);
        d.pipeline.instances.insert(instance.id.clone(), instance);
    }
}
impl PipelineStore for FakeStore {
    async fn advance_pipeline(
        &self,
        task: &Task,
        version: u64,
        progress: &TaskProgress,
        event: &StoredEvent,
        confirmation: Option<&str>,
    ) -> Result<CasResult, FakeError> {
        self.advance_with_receipt(task, version, progress, event, confirmation)
    }

    async fn tasks(&self) -> Result<Vec<Task>, FakeError> {
        Ok(lock(&self.data)
            .tasks
            .values()
            .map(|r| r.task.clone())
            .collect())
    }
    async fn instances(&self) -> Result<Vec<Instance>, FakeError> {
        Ok(lock(&self.data)
            .pipeline
            .instances
            .values()
            .cloned()
            .collect())
    }
    async fn instance(&self, id: &str) -> Result<Option<Instance>, FakeError> {
        Ok(lock(&self.data).pipeline.instances.get(id).cloned())
    }
    async fn teams(&self) -> Result<Vec<Team>, FakeError> {
        Ok(lock(&self.data).pipeline.teams.values().cloned().collect())
    }
    async fn members(&self) -> Result<Vec<Member>, FakeError> {
        Ok(lock(&self.data)
            .pipeline
            .members
            .values()
            .cloned()
            .collect())
    }
    async fn add_team(&self, t: &Team) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        if d.pipeline.teams.contains_key(&t.id) {
            return Err(FakeError::new("add_team", "duplicate team"));
        }
        d.pipeline.teams.insert(t.id.clone(), t.clone());
        Ok(())
    }
    async fn join_team(&self, team: &str, instance: &str, role: &str) -> Result<(), FakeError> {
        if self.held_task(instance).await?.is_some() {
            return Err(FakeError::new("join_team", "instance holds task"));
        }
        let mut d = lock(&self.data);
        if !d.pipeline.teams.contains_key(team) {
            return Err(FakeError::new("join_team", "unknown team"));
        }
        let m = d
            .pipeline
            .members
            .get_mut(instance)
            .ok_or_else(|| FakeError::new("join_team", "unknown instance"))?;
        m.team = team.into();
        m.role = role.into();
        Ok(())
    }
    async fn set_team_workflow(&self, team: &str, workflow: &str) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        let t = d
            .pipeline
            .teams
            .get_mut(team)
            .ok_or_else(|| FakeError::new("set_team_workflow", "unknown team"))?;
        t.default_workflow = workflow.into();
        Ok(())
    }
    async fn progress(&self, id: &str) -> Result<Option<Progress>, FakeError> {
        let d = lock(&self.data);
        Ok(d.progress.get(id).map(|data| {
            let note = d.pipeline.notes.get(id).cloned().unwrap_or_default();
            Progress {
                data: data.clone(),
                block_reason: data.block_reason.clone(),
                attention_reason: note.1,
                acknowledged: note.2,
            }
        }))
    }
    async fn task_note(
        &self,
        id: &str,
        block: Option<String>,
        attention: Option<String>,
        ack: bool,
    ) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        if let Some(p) = d.progress.get_mut(id) {
            p.block_reason = block.clone();
        }
        d.pipeline.notes.insert(id.into(), (block, attention, ack));
        Ok(())
    }
    async fn bindings(&self) -> Result<Vec<BindingRow>, FakeError> {
        Ok(lock(&self.data)
            .pipeline
            .bindings
            .values()
            .cloned()
            .collect())
    }
    async fn put_binding(&self, b: &BindingRow) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        if let Some(old) = d.pipeline.bindings.get(&b.instance)
            && (old.task != b.task || old.kind != b.kind || old.worktree != b.worktree)
        {
            return Err(FakeError::new("put_binding", "instance already bound"));
        }
        d.pipeline.bindings.insert(b.instance.clone(), b.clone());
        Ok(())
    }
    async fn delete_binding(&self, id: &str) -> Result<(), FakeError> {
        lock(&self.data).pipeline.bindings.remove(id);
        Ok(())
    }
    async fn latest_workflow(&self, id: &str) -> Result<Option<Workflow>, FakeError> {
        Ok(lock(&self.data)
            .workflows
            .iter()
            .rev()
            .find(|((w, _), _)| w == id)
            .map(|(_, w)| w.clone()))
    }
    async fn save_workflow(&self, w: &Workflow) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        let key = (w.id.clone(), w.version);
        if d.workflows.contains_key(&key) {
            return Err(FakeError::new(
                "save_workflow",
                "duplicate workflow version",
            ));
        }
        d.workflows.insert(key, w.clone());
        Ok(())
    }
    async fn workflow_ids(&self) -> Result<Vec<String>, FakeError> {
        Ok(lock(&self.data)
            .workflows
            .keys()
            .map(|(id, _)| id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    async fn held_task(&self, id: &str) -> Result<Option<String>, FakeError> {
        let d = lock(&self.data);
        Ok(d.tasks
            .values()
            .find(|t| {
                t.task.assignee.as_deref() == Some(id)
                    && !matches!(
                        t.task.status,
                        agend_core::pipeline::task::TaskStatus::Done
                            | agend_core::pipeline::task::TaskStatus::Failed
                            | agend_core::pipeline::task::TaskStatus::Cancelled
                            | agend_core::pipeline::task::TaskStatus::Superseded
                    )
            })
            .map(|t| t.task.id.clone())
            .or_else(|| d.pipeline.bindings.get(id).map(|b| b.task.clone())))
    }
    async fn create_pipeline_task(
        &self,
        task: &Task,
        pipeline: &str,
        now: u64,
    ) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        if d.tasks.contains_key(&task.id) {
            return Err(FakeError::new("create_pipeline_task", "duplicate task"));
        }
        d.tasks.insert(
            task.id.clone(),
            VersionedTask {
                task: task.clone(),
                version: 1,
            },
        );
        d.progress.insert(
            task.id.clone(),
            TaskProgress {
                pipeline: pipeline.into(),
                stage_entered_at_unix_ms: now,
                merge_intent: None,
                block_reason: None,
            },
        );
        Ok(())
    }
    async fn load_events(&self, id: &str) -> Result<Vec<StoredEvent>, FakeError> {
        Ok(self.events(id))
    }
    async fn message(&self, id: &str) -> Result<Option<Message>, FakeError> {
        Ok(lock(&self.data).pipeline.messages.get(id).cloned())
    }
    async fn claim_message(&self, new: &NewMessage, now: u64) -> Result<Claim, FakeError> {
        let mut d = lock(&self.data);
        if let Some(m) = d.pipeline.messages.get(&new.id) {
            return Ok(if m.same_as(new) {
                Claim::Existing(m.clone())
            } else {
                Claim::Different(m.clone())
            });
        }
        d.pipeline.next += 1;
        let m = Message {
            seq: d.pipeline.next as i64,
            id: new.id.clone(),
            from_instance: new.from_instance.clone(),
            to_instance: new.to_instance.clone(),
            task_id: new.task_id.clone(),
            body: new.body.clone(),
            level: new.level,
            state: DeliveryState::Queued,
            turn_id: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            attempted_at_unix_ms: None,
        };
        d.pipeline.messages.insert(m.id.clone(), m.clone());
        Ok(Claim::Inserted(m))
    }
    async fn advance_message(
        &self,
        id: &str,
        next: DeliveryState,
        turn: Option<String>,
        now: u64,
    ) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        let Some(m) = d.pipeline.messages.get_mut(id) else {
            return Ok(());
        };
        if !m.state.can_transition_to(next) {
            return Ok(());
        }
        m.state = next;
        if turn.is_some() {
            m.turn_id = turn;
        }
        m.updated_at_unix_ms = now;
        Ok(())
    }
    async fn asks(&self) -> Result<Vec<AskRow>, FakeError> {
        Ok(lock(&self.data).pipeline.asks.values().cloned().collect())
    }
    async fn save_ask(&self, row: &AskRow) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        let count = d
            .pipeline
            .asks
            .get(&row.thread.ask_id)
            .map_or(0, |r| r.thread.entries.len());
        for (index, entry) in row.thread.entries.iter().enumerate().skip(count) {
            if matches!(entry, agend_core::protocol::ask::AskEntry::Answer { .. }) {
                d.pipeline.next += 1;
                let seq = d.pipeline.next;
                d.pipeline.answers.push((
                    seq,
                    format!("ask:{}/{}", row.thread.ask_id, index + 1),
                    row.instance.clone(),
                    row.thread.task_id.clone(),
                    serde_json::to_string(entry).unwrap(),
                    false,
                ));
            }
        }
        d.pipeline
            .asks
            .insert(row.thread.ask_id.clone(), row.clone());
        Ok(())
    }
    async fn pending_answers(&self) -> Result<Vec<PendingAnswer>, FakeError> {
        Ok(lock(&self.data)
            .pipeline
            .answers
            .iter()
            .filter(|a| !a.5)
            .map(|a| (a.0, a.1.clone(), a.2.clone(), a.3.clone(), a.4.clone()))
            .collect())
    }
    async fn mark_answer_sent(&self, seq: u64) -> Result<(), FakeError> {
        if let Some(a) = lock(&self.data)
            .pipeline
            .answers
            .iter_mut()
            .find(|a| a.0 == seq)
        {
            a.5 = true;
        }
        Ok(())
    }
    async fn reminders(&self) -> Result<Vec<(u64, String, u64)>, FakeError> {
        Ok(lock(&self.data).pipeline.reminders.clone())
    }
    async fn add_reminder(&self, task: &str, due: u64) -> Result<(), FakeError> {
        let mut d = lock(&self.data);
        d.pipeline.next += 1;
        let seq = d.pipeline.next;
        d.pipeline.reminders.push((seq, task.into(), due));
        Ok(())
    }
    async fn delete_reminder(&self, seq: u64) -> Result<(), FakeError> {
        lock(&self.data).pipeline.reminders.retain(|r| r.0 != seq);
        Ok(())
    }
}
