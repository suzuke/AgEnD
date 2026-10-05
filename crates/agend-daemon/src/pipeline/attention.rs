use super::*;
use agend_core::protocol::ask::ContextRecap;

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
    pub(super) fn clear_task_attention(
        &self,
        task: &str,
        next: &PipelineState,
        human_resolution: Option<(String, AttentionAction)>,
    ) {
        // A notification or partial approval can leave the same human ticket pending.
        let pending = (next.status() == PipelineStatus::Running
            && matches!(
                next.current_stage().map(|s| &s.stage),
                Some(Stage::Approval {
                    by: Approver::Human,
                    ..
                })
            ))
        .then(|| format!("approval:{}", ticket(next)));
        for a in self.fleet.view().attention {
            if a.task_id.as_deref() == Some(task)
                && a.ask.is_none()
                && let Some(id) = a.attention_id
            {
                if id.starts_with(crate::handlers::claude_attention::PREFIX) {
                    continue;
                }
                if pending.as_deref() == Some(id.as_str()) {
                    continue;
                }
                match &human_resolution {
                    Some((resolved_id, action)) if resolved_id == &id => {
                        self.fleet.resolve(&id, *action);
                    }
                    _ => self.fleet.dismiss(&id),
                }
            }
        }
    }
    pub(super) async fn no_role(
        &self,
        task: &Task,
        state: &PipelineState,
        role: &str,
    ) -> Result<(), Refusal> {
        let id = format!("no-role:{}/{role}", task.team_id);
        let entered = self
            .store
            .progress(&task.id)
            .await
            .map_err(db)?
            .map_or(self.clock.now_unix_ms(), |p| {
                p.data.stage_entered_at_unix_ms
            });
        self.fleet.raise(item(
            &id,
            &format!("{} needs role {role} in {}", task.id, task.team_id),
            Some(task),
            entered,
            Vec::new(),
            None,
        ));
        let _ = state;
        Ok(())
    }
    pub(super) async fn resolve(&mut self, data: ResolveAttentionData) -> Reply {
        let item = self.fleet.attention(&data.attention_id).ok_or_else(|| {
            (
                error_code::UNKNOWN_ATTENTION.into(),
                format!("unknown attention {}", data.attention_id),
            )
        })?;
        if !item.actions.contains(&data.action) {
            return Err((
                error_code::UNKNOWN_ATTENTION.into(),
                "action is not available for this item".into(),
            ));
        }
        let task = item
            .task_id
            .ok_or_else(|| invalid("not a task attention"))?;
        match data.action {
            AttentionAction::Approve | AttentionAction::RequestChanges => {
                let loaded = self.load(&task).await?;
                if data.attention_id != format!("approval:{}", ticket(&loaded.state)) {
                    return Err((
                        error_code::UNKNOWN_ATTENTION.into(),
                        "approval attempt is no longer current".into(),
                    ));
                }
                let stage = loaded
                    .state
                    .current_stage()
                    .ok_or_else(|| invalid("missing approval stage"))?;
                let Stage::Approval {
                    by: Approver::Human,
                    bind_head,
                    ..
                } = stage.stage
                else {
                    return Err(invalid("not a human approval"));
                };
                let head = if bind_head {
                    loaded.state.current_head().map(str::to_owned)
                } else {
                    None
                };
                let stage_id = stage.id.clone();
                let attempt = loaded.state.attempt();
                let event = if data.action == AttentionAction::Approve {
                    PipelineEvent::ApprovalGranted {
                        stage_id,
                        attempt,
                        reviewer: "operator".into(),
                        head,
                        selected_child: None,
                    }
                } else {
                    let reason = data
                        .note
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| invalid("request_changes requires a nonempty note"))?;
                    PipelineEvent::ChangesRequested {
                        stage_id,
                        attempt,
                        reviewer: "operator".into(),
                        head,
                        reason,
                    }
                };
                self.event(&task, event).await?;
            }
            AttentionAction::Acknowledge => {
                let p = self
                    .store
                    .progress(&task)
                    .await
                    .map_err(db)?
                    .ok_or_else(|| invalid("missing task progress"))?;
                self.store
                    .task_note(&task, p.block_reason, p.attention_reason, true)
                    .await
                    .map_err(db)?;
            }
            AttentionAction::Retry => {
                self.note(&task, None).await?;
                let loaded = self.load(&task).await?;
                for action in outstanding_actions(&loaded.state) {
                    self.action(
                        &loaded.task,
                        &loaded.state,
                        loaded.progress.data.stage_entered_at_unix_ms,
                        action,
                    )
                    .await?;
                }
            }
            AttentionAction::Abandon | AttentionAction::Unknown => {
                return Err(invalid("action is not a pipeline action"));
            }
        }
        self.fleet.resolve(&data.attention_id, data.action);
        Ok(CommandResult::Accepted)
    }
    pub(super) async fn refresh(&self) -> Result<(), Refusal> {
        let teams = self.store.teams().await.map_err(db)?;
        self.fleet.set_teams(
            teams
                .iter()
                .map(|t| TeamView {
                    team_id: t.id.clone(),
                })
                .collect(),
        );
        let mut expected = BTreeMap::new();
        let mut views = Vec::new();
        let members = self.store.members().await.map_err(db)?;
        for mut instance in self.fleet.view().instances {
            if let Some(member) = members.iter().find(|m| m.id == instance.instance_id) {
                instance.team_id = member.team.clone();
                self.fleet.set_instance(instance, "team updated".into());
            }
        }
        for task in self.store.tasks().await.map_err(db)? {
            let loaded = self.load(&task.id).await;
            let state = loaded.as_ref().ok().map(|l| &l.state);
            let progress = self.store.progress(&task.id).await.map_err(db)?;
            let bindings = self.store.bindings().await.map_err(db)?;
            let events = self.store.load_events(&task.id).await.map_err(db)?;
            let view = TaskView {
                task_id: task.id.clone(),
                title: task.title.clone(),
                team_id: task.team_id.clone(),
                status: task.status.as_str().into(),
                assignee: task.assignee.clone(),
                stages: state.map_or(Vec::new(), |s| {
                    s.workflow().stages.iter().map(|s| s.id.clone()).collect()
                }),
                current_stage: state.and_then(|s| s.current_stage().map(|s| s.id.clone())),
                pipeline: Some(TaskPipelineView {
                    repo: teams
                        .iter()
                        .find(|t| t.id == task.team_id)
                        .and_then(|t| t.repo.clone()),
                    stage_kinds: state.map_or(Vec::new(), |s| {
                        s.workflow()
                            .stages
                            .iter()
                            .map(|s| s.stage.kind().as_str().into())
                            .collect()
                    }),
                    stage_agents: state.map_or(Vec::new(), |s| {
                        s.workflow()
                            .stages
                            .iter()
                            .enumerate()
                            .map(|(index, stage)| {
                                if index == s.stage_index()
                                    && matches!(
                                        stage.stage,
                                        Stage::Approval {
                                            by: Approver::Role(_),
                                            ..
                                        }
                                    )
                                {
                                    bindings
                                        .iter()
                                        .find(|b| b.task == task.id && b.kind == "review")
                                        .map(|b| b.instance.clone())
                                } else if matches!(stage.stage, Stage::Work { .. }) {
                                    events
                                        .iter()
                                        .rev()
                                        .find_map(|event| {
                                            let identity = event.id.strip_prefix("assigned:")?;
                                            let mut parts = identity.split('/');
                                            (parts.next() == Some(task.id.as_str())
                                                && parts.next() == Some(stage.id.as_str()))
                                            .then(|| event.detail.clone())
                                        })
                                        .or_else(|| {
                                            (index == s.stage_index())
                                                .then(|| task.assignee.clone())
                                                .flatten()
                                        })
                                } else {
                                    s.approvals()
                                        .iter()
                                        .find(|a| a.stage_id == stage.id)
                                        .and_then(|a| a.reviewers.first().cloned())
                                }
                            })
                            .collect()
                    }),
                    block_reason: progress.as_ref().and_then(|p| p.block_reason.clone()),
                    archive_paths: events
                        .into_iter()
                        .filter(|e| e.kind == "wip_archived")
                        .map(|e| e.detail)
                        .collect(),
                }),
            };
            views.push(view);
            let entered = progress
                .as_ref()
                .map_or(0, |p| p.data.stage_entered_at_unix_ms);
            if task.status == TaskStatus::Failed
                && !progress.as_ref().is_some_and(|p| p.acknowledged)
            {
                let id = format!("task-failed:{}", task.id);
                expected.insert(
                    id.clone(),
                    item(
                        &id,
                        &format!("{} failed", task.id),
                        Some(&task),
                        entered,
                        vec![AttentionAction::Acknowledge],
                        None,
                    ),
                );
            }
            if let Some(progress) = progress
                && let Some(reason) = progress.attention_reason
            {
                let (kind, reason) = reason.split_once(':').unwrap_or(("task-failed", &reason));
                let id = format!("{kind}:{}", task.id);
                expected.insert(
                    id.clone(),
                    item(
                        &id,
                        reason,
                        Some(&task),
                        entered,
                        vec![AttentionAction::Retry],
                        None,
                    ),
                );
            }
            if let Some(state) = state
                && state.status() == PipelineStatus::Running
            {
                match state.current_stage().map(|s| &s.stage) {
                    Some(Stage::Approval {
                        by: Approver::Human,
                        ..
                    }) => {
                        let id = format!("approval:{}", ticket(state));
                        let head = state.current_head().unwrap_or("result");
                        let asking = format!(
                            "Approve {} at {}",
                            state.branch().unwrap_or("result"),
                            head.chars().take(7).collect::<String>()
                        );
                        expected.insert(
                            id.clone(),
                            item(
                                &id,
                                &asking,
                                Some(&task),
                                entered,
                                vec![AttentionAction::Approve, AttentionAction::RequestChanges],
                                Some(ContextRecap {
                                    goal: task.title.clone(),
                                    decisions: {
                                        let product = super::context::work_product(state);
                                        if product.is_empty() {
                                            Vec::new()
                                        } else {
                                            vec![product]
                                        }
                                    },
                                    asking: asking.clone(),
                                    next: if state
                                        .workflow()
                                        .stages
                                        .get(state.stage_index() + 1)
                                        .is_some_and(|s| matches!(s.stage, Stage::Work { .. }))
                                    {
                                        "implement the approved work product"
                                    } else if state.workflow().requires_repo() {
                                        "merge into main"
                                    } else {
                                        "complete task"
                                    }
                                    .into(),
                                }),
                            ),
                        );
                    }
                    Some(Stage::Work { role, .. })
                    | Some(Stage::Approval {
                        by: Approver::Role(role),
                        ..
                    }) => {
                        let candidates = members
                            .iter()
                            .filter(|m| m.team == task.team_id && m.role == *role)
                            .collect::<Vec<_>>();
                        let review = matches!(
                            state.current_stage().map(|s| &s.stage),
                            Some(Stage::Approval { .. })
                        );
                        if candidates.is_empty()
                            || (review
                                && candidates
                                    .iter()
                                    .all(|m| Some(&m.id) == task.assignee.as_ref()))
                        {
                            let id = format!("no-role:{}/{role}", task.team_id);
                            let a = expected.entry(id.clone()).or_insert_with(|| {
                                item(
                                    &id,
                                    &format!("{} needs role {role}", task.team_id),
                                    None,
                                    entered,
                                    Vec::new(),
                                    None,
                                )
                            });
                            a.unblocks = Some(a.unblocks.unwrap_or(0) + 1);
                            a.waiting_since_unix_ms =
                                Some(a.waiting_since_unix_ms.unwrap_or(entered).min(entered));
                        }
                    }
                    _ => {}
                }
            }
        }
        self.refresh_asks(&mut expected).await?;
        let before = self.fleet.view();
        self.fleet.sync_tasks(views);
        for old in before.attention {
            if let Some(id) = old.attention_id
                && !id.starts_with("instance-failed:")
                && !id.starts_with(crate::handlers::claude_attention::PREFIX)
                && !expected.contains_key(&id)
            {
                self.fleet.dismiss(&id);
            }
        }
        for (_, new) in expected {
            self.fleet.upsert_attention(new);
        }
        // Failed agents now report the tasks that wait for them.
        for failed in self.fleet.view().attention.into_iter().filter(|a| {
            a.attention_id
                .as_ref()
                .is_some_and(|id| id.starts_with("instance-failed:"))
        }) {
            if let Some(instance) = failed.instance_id.as_deref() {
                let mut enriched = failed.clone();
                enriched.unblocks = Some(u32::from(
                    self.store.held_task(instance).await.map_err(db)?.is_some(),
                ));
                // Retry or a fresh failure may have changed this item while
                // the store lookup was pending. Never recreate that snapshot.
                self.fleet.replace_attention_if(&failed, enriched);
            }
        }
        Ok(())
    }
}
fn item(
    id: &str,
    reason: &str,
    task: Option<&Task>,
    entered: u64,
    actions: Vec<AttentionAction>,
    recap: Option<agend_core::protocol::ask::ContextRecap>,
) -> AttentionRequiredData {
    AttentionRequiredData {
        reason: reason.into(),
        task_id: task.map(|t| t.id.clone()),
        ask: None,
        recap,
        attention_id: Some(id.into()),
        unblocks: Some(u32::from(task.is_some())),
        waiting_since_unix_ms: Some(entered),
        if_ignored: Some(task.map_or("tasks wait for this role".into(), |t| {
            format!("{} waits for you", t.id)
        })),
        actions,
        instance_id: task.and_then(|t| t.assignee.clone()),
    }
}

#[cfg(test)]
mod tests;
