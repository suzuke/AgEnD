use super::*;
use agend_core::pipeline::state::WorkProduct;
use agend_core::pipeline::task::TaskOperation;
use agend_core::pipeline::workflow::WorkOutput;
use agend_core::protocol::ask::{AskEntry, AskThread};
use agend_core::runtime_records::AskRow;

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
    pub(super) async fn agent(&mut self, caller: Option<&str>, command: AgentCommand) -> Reply {
        let caller = caller.ok_or_else(|| {
            (
                error_code::FORBIDDEN.into(),
                "this command requires an agent".into(),
            )
        })?;
        if self.store.instance(caller).await.map_err(db)?.is_none() {
            return Err((
                error_code::UNKNOWN_INSTANCE.into(),
                format!("unknown instance {caller}"),
            ));
        }
        match command {
            AgentCommand::TaskCreate {
                title,
                role,
                team_id,
                workflow_id,
            } => {
                let member = self
                    .store
                    .members()
                    .await
                    .map_err(db)?
                    .into_iter()
                    .find(|m| m.id == caller)
                    .ok_or_else(|| invalid("unknown instance"))?;
                self.create_task(title, role, team_id.unwrap_or(member.team), workflow_id)
                    .await
            }
            AgentCommand::Status => {
                let Some(task) = self.store.held_task(caller).await.map_err(db)? else {
                    let instance = self
                        .store
                        .instance(caller)
                        .await
                        .map_err(db)?
                        .ok_or_else(|| invalid("unknown instance"))?;
                    return Ok(CommandResult::Status {
                        data: StatusData {
                            task_id: None,
                            instance_id: Some(caller.into()),
                            summary: format!(
                                "{caller} ({}): no task\nnext: agend inbox | agend send <name> \"<message>\"",
                                instance.backend.as_str()
                            ),
                            identity: None,
                        },
                    });
                };
                let loaded = self.load(&task).await?;
                let stage = loaded
                    .state
                    .current_stage()
                    .ok_or_else(|| invalid("task has no active stage"))?;
                Ok(CommandResult::Status {
                    data: StatusData {
                        task_id: Some(task.clone()),
                        instance_id: Some(caller.into()),
                        summary: format!(
                            "{task} · {} (attempt {}) · ticket {}{}",
                            stage.id,
                            loaded.state.attempt(),
                            ticket(&loaded.state),
                            loaded
                                .progress
                                .block_reason
                                .map_or(String::new(), |r| format!("\nblocked: {r}"))
                        ),
                        identity: Some(ResultIdentity {
                            stage_id: stage.id.clone(),
                            attempt: loaded.state.attempt(),
                        }),
                    },
                })
            }
            AgentCommand::Done { task_id, identity } => {
                let loaded = self.owned_result(caller, &task_id, identity, false).await?;
                let repo = self
                    .team(&loaded.task.team_id)
                    .await?
                    .repo
                    .ok_or_else(|| invalid("done requires a repository; use agend result"))?;
                let binding = self
                    .store
                    .bindings()
                    .await
                    .map_err(db)?
                    .into_iter()
                    .find(|b| b.instance == caller && b.task == task_id && b.kind == "work")
                    .ok_or_else(|| invalid("no work binding"))?;
                let branch = binding.branch.ok_or_else(|| invalid("no branch"))?;
                let git = self
                    .git
                    .as_ref()
                    .ok_or_else(|| invalid("git unavailable"))?;
                let head = git
                    .run(&repo, &["rev-parse", &format!("refs/heads/{branch}")])
                    .await
                    .map_err(invalid)?;
                if git.ancestor(&repo, &head, "main").await.map_err(invalid)? {
                    return Err(invalid(format!(
                        "nothing to merge: {branch} has no commits beyond main; commit your work, then agend done {}",
                        ticket(&loaded.state)
                    )));
                }
                let patch_id = git.patch_id(&repo, &head).await.map_err(invalid)?;
                let stage_id = loaded
                    .state
                    .current_stage()
                    .ok_or_else(|| invalid("no stage"))?
                    .id
                    .clone();
                self.event(
                    &task_id,
                    PipelineEvent::WorkCompleted {
                        stage_id,
                        attempt: loaded.state.attempt(),
                        product: WorkProduct::Branch {
                            branch,
                            head,
                            patch_id,
                        },
                    },
                )
                .await?;
                Ok(CommandResult::Accepted)
            }
            AgentCommand::Result {
                task_id,
                summary,
                output,
                identity,
            } => {
                let loaded = self.owned_result(caller, &task_id, identity, false).await?;
                let stage = loaded
                    .state
                    .current_stage()
                    .ok_or_else(|| invalid("no stage"))?;
                let product = if matches!(
                    stage.stage,
                    Stage::Work {
                        output: WorkOutput::Plan,
                        ..
                    }
                ) {
                    WorkProduct::Plan {
                        items: summary.lines().map(str::to_owned).collect(),
                    }
                } else {
                    WorkProduct::Result { summary, output }
                };
                self.event(
                    &task_id,
                    PipelineEvent::WorkCompleted {
                        stage_id: stage.id.clone(),
                        attempt: loaded.state.attempt(),
                        product,
                    },
                )
                .await?;
                Ok(CommandResult::Accepted)
            }
            AgentCommand::ReviewApprove { task_id, identity } => {
                self.review(caller, &task_id, identity, None).await
            }
            AgentCommand::ReviewChanges {
                task_id,
                identity,
                summary,
            } => {
                if summary.trim().is_empty() {
                    return Err(invalid("review changes requires a reason"));
                }
                self.review(caller, &task_id, identity, Some(summary)).await
            }
            AgentCommand::Block { task_id, reason } => {
                self.block(caller, &task_id, Some(reason)).await
            }
            AgentCommand::Unblock { task_id } => self.block(caller, &task_id, None).await,
            AgentCommand::Remind {
                task_id,
                delay_seconds,
            } => {
                self.owned(caller, &task_id).await?;
                let due = delay_seconds
                    .checked_mul(1000)
                    .and_then(|ms| self.clock.now_unix_ms().checked_add(ms))
                    .filter(|v| *v <= i64::MAX as u64)
                    .ok_or_else(|| invalid("reminder delay is too large"))?;
                self.store.add_reminder(&task_id, due).await.map_err(db)?;
                let tx = self.tx.clone();
                let delay = due.saturating_sub(self.clock.now_unix_ms());
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    let _ = tx.send(Input::Wake);
                });
                Ok(CommandResult::Accepted)
            }
            AgentCommand::Ask { question, options } => {
                if question.trim().is_empty() {
                    return Err(invalid("ask question is empty"));
                }
                let task = self.store.held_task(caller).await.map_err(db)?;
                let id = format!("A-{}", self.executor.new_id().map_err(db)?);
                let row = AskRow {
                    instance: caller.into(),
                    created: self.clock.now_unix_ms(),
                    thread: AskThread {
                        ask_id: id.clone(),
                        task_id: task,
                        entries: vec![AskEntry::Question {
                            from: caller.into(),
                            text: question,
                            options,
                        }],
                    },
                };
                self.store.save_ask(&row).await.map_err(db)?;
                Ok(CommandResult::AskCreated {
                    data: AskCreatedData { ask_id: id },
                })
            }
            AgentCommand::AskFollowUp {
                ask_id,
                question,
                options,
            } => {
                let mut row = self.owned_ask(caller, &ask_id).await?;
                if question.trim().is_empty()
                    || !matches!(row.thread.entries.last(), Some(AskEntry::Answer { .. }))
                {
                    return Err(invalid(
                        "follow-up requires an answer and a nonempty question",
                    ));
                }
                row.thread.entries.push(AskEntry::FollowUp {
                    from: caller.into(),
                    text: question,
                    options,
                });
                self.store.save_ask(&row).await.map_err(db)?;
                self.fleet
                    .publish(DaemonEvent::AskUpdated { data: row.thread });
                Ok(CommandResult::Accepted)
            }
            AgentCommand::AskResolve { ask_id, summary } => {
                let mut row = self.owned_ask(caller, &ask_id).await?;
                if summary.trim().is_empty() {
                    return Err(invalid("resolution requires a summary"));
                }
                row.thread.entries.push(AskEntry::Resolution {
                    from: caller.into(),
                    summary,
                });
                self.store.save_ask(&row).await.map_err(db)?;
                self.fleet
                    .publish(DaemonEvent::AskUpdated { data: row.thread });
                Ok(CommandResult::Accepted)
            }
            _ => Err(invalid("unsupported pipeline command")),
        }
    }
    async fn owned(&self, caller: &str, task: &str) -> Result<Loaded, Refusal> {
        let loaded = self.load(task).await?;
        if loaded.task.assignee.as_deref() != Some(caller) {
            return Err((
                error_code::FORBIDDEN.into(),
                format!("{caller} does not hold {task}"),
            ));
        }
        Ok(loaded)
    }
    async fn owned_result(
        &self,
        caller: &str,
        task: &str,
        identity: Option<ResultIdentity>,
        review: bool,
    ) -> Result<Loaded, Refusal> {
        let loaded = self.load(task).await?;
        if matches!(
            loaded.task.status,
            TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Superseded
        ) {
            return Err((
                error_code::STALE_RESULT.into(),
                "task is closed; ticket is no longer current".into(),
            ));
        }
        let expected = loaded.state.current_stage().map(|s| ResultIdentity {
            stage_id: s.id.clone(),
            attempt: loaded.state.attempt(),
        });
        if identity.is_none()
            || identity != expected
            || loaded.state.status() != PipelineStatus::Running
        {
            return Err((
                error_code::STALE_RESULT.into(),
                format!(
                    "ticket is no longer current (now {})",
                    ticket(&loaded.state)
                ),
            ));
        }
        if review {
            if !self.store.bindings().await.map_err(db)?.iter().any(|b| {
                b.instance == caller
                    && b.task == task
                    && b.kind == "review"
                    && b.ticket == ticket(&loaded.state)
            }) {
                return Err((
                    error_code::FORBIDDEN.into(),
                    "only this attempt's assigned reviewer can report review".into(),
                ));
            }
        } else if loaded.task.assignee.as_deref() != Some(caller) {
            return Err((
                error_code::FORBIDDEN.into(),
                "only the task holder can report work".into(),
            ));
        }
        Ok(loaded)
    }
    async fn review(
        &mut self,
        caller: &str,
        task: &str,
        identity: Option<ResultIdentity>,
        reason: Option<String>,
    ) -> Reply {
        let loaded = self.owned_result(caller, task, identity, true).await?;
        let binding = self
            .store
            .bindings()
            .await
            .map_err(db)?
            .into_iter()
            .find(|b| b.instance == caller && b.task == task && b.kind == "review")
            .ok_or_else(|| invalid("review binding missing"))?;
        let stage_id = loaded
            .state
            .current_stage()
            .ok_or_else(|| invalid("no stage"))?
            .id
            .clone();
        let attempt = loaded.state.attempt();
        let head = if loaded.state.current_stage().is_some_and(|s| {
            matches!(
                s.stage,
                Stage::Approval {
                    bind_head: true,
                    ..
                }
            )
        }) {
            binding.head.clone()
        } else {
            None
        };
        let event = match reason {
            None => PipelineEvent::ApprovalGranted {
                stage_id,
                attempt,
                head,
                reviewer: caller.into(),
                selected_child: None,
            },
            Some(reason) => PipelineEvent::ChangesRequested {
                stage_id,
                attempt,
                head,
                reviewer: caller.into(),
                reason,
            },
        };
        self.event(task, event).await?;
        if self.store.bindings().await.map_err(db)?.contains(&binding) {
            self.release(&binding, false).await?;
        }
        Ok(CommandResult::Accepted)
    }
    async fn block(&mut self, caller: &str, id: &str, reason: Option<String>) -> Reply {
        let loaded = self.owned(caller, id).await?;
        if loaded.state.merge_in_flight() {
            return Err((
                "merge_in_flight".into(),
                "merging task cannot be blocked".into(),
            ));
        }
        if reason.as_ref().is_some_and(|r| r.trim().is_empty()) {
            return Err(invalid("block requires a reason"));
        }
        let operation = if reason.is_some() {
            TaskOperation::Block
        } else {
            TaskOperation::Unblock
        };
        let task = loaded.task.apply(operation).map_err(db)?;
        let mut progress = loaded.progress.data.clone();
        progress.block_reason = reason.clone();
        let event = StoredEvent {
            id: self.executor.new_id().map_err(db)?,
            occurred_at_unix_ms: self.clock.now_unix_ms(),
            kind: "block".into(),
            detail: reason.clone().unwrap_or_else(|| "unblocked".into()),
        };
        if !matches!(
            self.store
                .advance_task(&task, loaded.version, &progress, &event)
                .await
                .map_err(db)?,
            CasResult::Written { .. }
        ) {
            return Err(invalid("block CAS conflict"));
        }
        if reason.is_none() {
            self.timers.remove(&ticket(&loaded.state));
            for action in outstanding_actions(&loaded.state) {
                self.action(
                    &task,
                    &loaded.state,
                    progress.stage_entered_at_unix_ms,
                    action,
                )
                .await?;
            }
        }
        Ok(CommandResult::Accepted)
    }
    async fn create_task(
        &mut self,
        title: String,
        role: String,
        team_id: String,
        workflow_id: Option<String>,
    ) -> Reply {
        if title.trim().is_empty() || role.trim().is_empty() {
            return Err(invalid("task requires title and role"));
        }
        let team = self.team(&team_id).await?;
        let id = workflow_id.unwrap_or(team.default_workflow);
        let workflow = self
            .store
            .latest_workflow(&id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid(format!("unknown workflow {id}")))?;
        let first = workflow
            .stages
            .first()
            .ok_or_else(|| invalid("workflow has no stages"))?;
        if !matches!(&first.stage,Stage::Work {role:r,..} if r==&role) {
            return Err(invalid(format!(
                "--role must match workflow's first work role: {:?}",
                first.stage
            )));
        }
        if workflow.requires_repo() && team.repo.is_none() {
            return Err(invalid("workflow requires a repo; this team has none"));
        }
        if workflow.requires_repo() {
            let git = self
                .git
                .as_ref()
                .ok_or_else(|| invalid("real git not found"))?;
            let out = git
                .run(team.repo.as_deref().unwrap_or_default(), &["--version"])
                .await
                .map_err(invalid)?;
            if !agend_core::setup::parse_git_version(&out)
                .is_some_and(agend_core::setup::git_is_new_enough)
            {
                return Err(invalid("git 2.38 or newer is required"));
            }
        }
        let number = self.store.tasks().await.map_err(db)?.len() + 1;
        let task_id = format!("t-{number}");
        let task = Task::new(
            &task_id,
            title,
            team_id,
            workflow.id.clone(),
            workflow.version,
        )
        .set_requires_repo(workflow.requires_repo());
        let state = PipelineState::new(&task_id, validate(workflow).map_err(invalid)?);
        self.store
            .create_pipeline_task(
                &task,
                &serde_json::to_string(&state.snapshot()).map_err(db)?,
                self.clock.now_unix_ms(),
            )
            .await
            .map_err(db)?;
        self.event(&task_id, PipelineEvent::Start).await?;
        Ok(CommandResult::TaskCreated {
            data: TaskCreatedData { task_id },
        })
    }
    pub(super) async fn operator(&mut self, command: OperatorCommand) -> Reply {
        match command {
            OperatorCommand::TaskCreate {
                title,
                role,
                team_id,
                workflow_id,
            } => self.create_task(title, role, team_id, workflow_id).await,
            OperatorCommand::TaskCancel { task_id, reason } => {
                self.event(
                    &task_id,
                    PipelineEvent::Cancel {
                        reason: reason.unwrap_or_else(|| "cancelled by operator".into()),
                    },
                )
                .await
            }
            OperatorCommand::TeamAdd {
                team_id,
                repo,
                workflow_id,
            } => {
                agend_core::runtime_records::validate_id(&team_id).map_err(invalid)?;
                let id = workflow_id
                    .unwrap_or_else(|| if repo.is_some() { "code" } else { "research" }.into());
                if self.store.latest_workflow(&id).await.map_err(db)?.is_none() {
                    return Err(invalid("unknown workflow"));
                }
                let repo = repo
                    .map(|r| self.executor.canonical_repo(&r))
                    .transpose()
                    .map_err(db)?;
                if let Some(repo) = &repo {
                    self.git
                        .as_ref()
                        .ok_or_else(|| invalid("git unavailable"))?
                        .run(repo, &["rev-parse", "--verify", "main"])
                        .await
                        .map_err(invalid)?;
                }
                self.store
                    .add_team(&Team {
                        id: team_id,
                        repo,
                        default_workflow: id,
                    })
                    .await
                    .map_err(db)?;
                Ok(CommandResult::Accepted)
            }
            OperatorCommand::TeamList => Ok(CommandResult::Text {
                text: self
                    .store
                    .teams()
                    .await
                    .map_err(db)?
                    .iter()
                    .map(|t| {
                        format!(
                            "{}\t{}\t{}",
                            t.id,
                            t.default_workflow,
                            t.repo.as_deref().unwrap_or("no repo")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
            OperatorCommand::TeamJoin {
                team_id,
                instance_id,
                role,
            } => {
                self.team(&team_id).await?;
                agend_core::runtime_records::validate_id(&role).map_err(invalid)?;
                self.store
                    .join_team(&team_id, &instance_id, &role)
                    .await
                    .map_err(db)?;
                Ok(CommandResult::Accepted)
            }
            OperatorCommand::TeamSetWorkflow {
                team_id,
                workflow_id,
            } => {
                self.team(&team_id).await?;
                let w = self
                    .store
                    .latest_workflow(&workflow_id)
                    .await
                    .map_err(db)?
                    .ok_or_else(|| invalid("unknown workflow"))?;
                validate(w).map_err(invalid)?;
                self.store
                    .set_team_workflow(&team_id, &workflow_id)
                    .await
                    .map_err(db)?;
                Ok(CommandResult::Accepted)
            }
            OperatorCommand::WorkflowList => Ok(CommandResult::Text {
                text: self.store.workflow_ids().await.map_err(db)?.join("\n"),
            }),
            OperatorCommand::WorkflowShow { workflow_id } => {
                let workflow = self
                    .store
                    .latest_workflow(&workflow_id)
                    .await
                    .map_err(db)?
                    .ok_or_else(|| invalid("unknown workflow"))?;
                Ok(CommandResult::Text {
                    text: toml::to_string(&workflow).map_err(db)?,
                })
            }
            OperatorCommand::WorkflowCheck { toml } => {
                validate(toml::from_str(&toml).map_err(db)?).map_err(invalid)?;
                Ok(CommandResult::Text {
                    text: "workflow: valid".into(),
                })
            }
            OperatorCommand::WorkflowApply { toml } => {
                let mut workflow: Workflow = toml::from_str(&toml).map_err(db)?;
                if ["code", "research", "planned", "epic"].contains(&workflow.id.as_str()) {
                    return Err(invalid("built-in workflows are read-only"));
                }
                agend_core::runtime_records::validate_id(&workflow.id).map_err(invalid)?;
                workflow.version = self
                    .store
                    .latest_workflow(&workflow.id)
                    .await
                    .map_err(db)?
                    .map_or(1, |w| w.version + 1);
                validate(workflow.clone()).map_err(invalid)?;
                self.store.save_workflow(&workflow).await.map_err(db)?;
                Ok(CommandResult::Text {
                    text: format!("saved {} v{}", workflow.id, workflow.version),
                })
            }
            _ => Err(invalid("unsupported pipeline operator command")),
        }
    }
    async fn owned_ask(&self, caller: &str, id: &str) -> Result<AskRow, Refusal> {
        let row = self
            .store
            .asks()
            .await
            .map_err(db)?
            .into_iter()
            .find(|r| r.thread.ask_id == id && !r.thread.is_resolved())
            .ok_or_else(|| (error_code::UNKNOWN_ASK.into(), "no open ask".into()))?;
        if row.instance != caller {
            return Err((
                error_code::FORBIDDEN.into(),
                "only the asking agent can continue this ask".into(),
            ));
        }
        Ok(row)
    }
    pub(super) async fn answer(&mut self, data: AnswerAskData) -> Reply {
        let mut row = self
            .store
            .asks()
            .await
            .map_err(db)?
            .into_iter()
            .find(|r| r.thread.ask_id == data.ask_id)
            .ok_or_else(|| (error_code::UNKNOWN_ASK.into(), "unknown ask".into()))?;
        if !row.thread.accepts(&data.reply) {
            return Err((
                error_code::UNKNOWN_ASK.into(),
                "ask has no pending question accepting this answer".into(),
            ));
        }
        row.thread.entries.push(AskEntry::Answer {
            from: "operator".into(),
            source: data.source,
            reply: data.reply,
        });
        self.store.save_ask(&row).await.map_err(db)?;
        self.deliver_answers().await?;
        self.fleet
            .publish(DaemonEvent::AskUpdated { data: row.thread });
        Ok(CommandResult::Accepted)
    }
    pub(super) async fn deliver_answers(&self) -> Result<(), Refusal> {
        for (seq, id, instance, task, body) in self.store.pending_answers().await.map_err(db)? {
            self.deliver(&instance, &id, task, &body).await?;
            self.store.mark_answer_sent(seq).await.map_err(db)?;
        }
        Ok(())
    }
    pub(super) async fn refresh_asks(
        &self,
        expected: &mut BTreeMap<String, AttentionRequiredData>,
    ) -> Result<(), Refusal> {
        for row in self.store.asks().await.map_err(db)? {
            if row.thread.is_resolved() {
                continue;
            }
            if let Some(AskEntry::Question { text, .. } | AskEntry::FollowUp { text, .. }) =
                row.thread.entries.last()
            {
                let id = row.thread.ask_id.clone();
                expected.insert(
                    id.clone(),
                    AttentionRequiredData {
                        reason: text.clone(),
                        task_id: row.thread.task_id.clone(),
                        ask: Some(row.thread),
                        recap: None,
                        attention_id: Some(id),
                        unblocks: Some(1),
                        waiting_since_unix_ms: Some(row.created),
                        if_ignored: Some("agent waits for your answer".into()),
                        actions: Vec::new(),
                        instance_id: Some(row.instance),
                    },
                );
            }
        }
        Ok(())
    }
    pub(super) async fn send_reminders(&self) -> Result<(), Refusal> {
        for (seq, task, due) in self.store.reminders().await.map_err(db)? {
            if due > self.clock.now_unix_ms() {
                continue;
            }
            let row = self.store.load_task(&task).await.map_err(db)?;
            if let Some(row) = row
                && let Some(holder) = row.task.assignee
            {
                self.deliver(
                    &holder,
                    &format!("remind:{task}/{seq}"),
                    Some(task.clone()),
                    &format!("Reminder for {task}: {}", row.task.title),
                )
                .await?;
            }
            self.store.delete_reminder(seq).await.map_err(db)?;
        }
        Ok(())
    }
}
