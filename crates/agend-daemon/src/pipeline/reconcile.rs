use super::*;
use agend_core::traits::StoredEvent;

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
    pub(super) async fn boot(&mut self) -> Result<(), Refusal> {
        if let Err(reason) = self.executor.readiness().await {
            log::line(&format!("checks unavailable: {reason}"));
        }
        for workflow in [
            Workflow::builtin_code(),
            Workflow::builtin_research(),
            Workflow::builtin_planned(),
            Workflow::builtin_epic(),
        ] {
            if self
                .store
                .latest_workflow(&workflow.id)
                .await
                .map_err(db)?
                .is_none()
            {
                self.store.save_workflow(&workflow).await.map_err(db)?;
            }
        }
        for task in self.store.tasks().await.map_err(db)? {
            if !matches!(
                task.status,
                TaskStatus::Done
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
                    | TaskStatus::Superseded
            ) && let Err((_, reason)) = self.load(&task.id).await
            {
                self.fail_restore(&task.id, &reason).await?;
            }
        }
        self.reconcile_bindings().await?;
        self.deliver_answers().await?;
        for task in self.store.tasks().await.map_err(db)? {
            if !matches!(task.status, TaskStatus::Running | TaskStatus::Open) {
                continue;
            }
            // Missing git disables repo execution without disabling the fleet.
            if self.git.is_none() && self.team(&task.team_id).await?.repo.is_some() {
                log::line(&format!(
                    "{}: repo execution unavailable; waiting for git",
                    task.id
                ));
                continue;
            }
            let result = async {
                let loaded = self.load(&task.id).await?;
                if loaded.state.status() == PipelineStatus::Pending {
                    return self
                        .commit_transition(&loaded, PipelineEvent::Start, None)
                        .await;
                }
                for action in outstanding_actions(&loaded.state) {
                    if matches!(action, PipelineAction::RunCommand { .. }) {
                        log::line(&format!(
                            "{}: re-running checks {} after restart",
                            task.id,
                            ticket(&loaded.state)
                        ));
                    }
                    self.action(
                        &task,
                        &loaded.state,
                        loaded.progress.data.stage_entered_at_unix_ms,
                        action,
                    )
                    .await?;
                }
                Ok(())
            }
            .await;
            if let Err((_, reason)) = result
                && self
                    .store
                    .load_task(&task.id)
                    .await
                    .map_err(db)?
                    .is_some_and(|r| r.task.status != TaskStatus::Failed)
            {
                self.fail_restore(&task.id, &format!("boot execution failed: {reason}"))
                    .await?;
            }
        }
        for (_, _, due) in self.store.reminders().await.map_err(db)? {
            let tx = self.tx.clone();
            let delay = due.saturating_sub(self.clock.now_unix_ms());
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                let _ = tx.send(Input::Wake);
            });
        }
        self.refresh().await?;
        Ok(())
    }
    pub(super) async fn fail_restore(&self, id: &str, reason: &str) -> Result<(), Refusal> {
        let row = self
            .store
            .load_task(id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("task disappeared"))?;
        let mut task = row.task;
        task.status = TaskStatus::Failed;
        let event = StoredEvent {
            id: format!("restore-failed:{}", self.executor.new_id().map_err(db)?),
            occurred_at_unix_ms: self.clock.now_unix_ms(),
            kind: "restore_failed".into(),
            detail: reason.into(),
        };
        let progress = self
            .store
            .progress(id)
            .await
            .map_err(db)?
            .map(|p| p.data)
            .unwrap_or(TaskProgress {
                pipeline: "{}".into(),
                stage_entered_at_unix_ms: self.clock.now_unix_ms(),
                merge_intent: None,
                block_reason: None,
            });
        if !matches!(
            self.store
                .advance_task(&task, row.version, &progress, &event)
                .await
                .map_err(db)?,
            CasResult::Written { .. }
        ) {
            return Err(invalid("restore failure CAS conflict"));
        }
        log::line(&format!("{id}: snapshot restore failed: {reason}"));
        self.cleanup_terminal(&task).await
    }
    pub(super) async fn reconcile_bindings(&mut self) -> Result<(), Refusal> {
        let tasks = self.store.tasks().await.map_err(db)?;
        for b in self.store.bindings().await.map_err(db)? {
            let Some(task) = tasks.iter().find(|t| t.id == b.task) else {
                continue;
            };
            if matches!(
                task.status,
                TaskStatus::Done
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
                    | TaskStatus::Superseded
            ) {
                if let Err((_, e)) = self.release(&b, task.merge_commit.is_some()).await {
                    log::line(&format!("{}: cleanup failed: {e}", b.task));
                }
                continue;
            }
            if let Some(repo) = self.team(&task.team_id).await?.repo
                && let Some(git) = self.git.as_ref()
            {
                if let Ok(loaded) = self.load(&task.id).await {
                    if b.kind == "review"
                        && (b.ticket != ticket(&loaded.state)
                            || loaded.state.approval_reviewers().contains(&b.instance))
                    {
                        if let Err((_, reason)) = self.release(&b, false).await {
                            self.fail_restore(
                                &b.task,
                                &format!("binding cleanup failed: {reason}"),
                            )
                            .await?;
                        }
                        continue;
                    }
                    if b.kind == "review" && b.head.is_none() {
                        continue;
                    }
                    if loaded.state.merge_in_flight() {
                        match git
                            .find_merge(
                                &repo,
                                &task.id,
                                loaded.state.current_head().unwrap_or_default(),
                            )
                            .await
                        {
                            Ok(Some(_)) => {
                                // Proven merge survives worktree cleanup.
                                continue;
                            }
                            Ok(None) => {}
                            Err(reason) => {
                                self.fail_restore(
                                    &b.task,
                                    &format!("merge recovery failed: {reason}"),
                                )
                                .await?;
                                continue;
                            }
                        }
                    }
                }
                if let Err(e) = git.ensure(&repo, &b).await {
                    self.fail_restore(&b.task, &e).await?;
                    if let Err((_, e)) = self.release(&b, false).await {
                        log::line(&format!("{}: cleanup failed: {e}", b.task));
                    }
                }
            }
        }
        let members = self.store.members().await.map_err(db)?;
        let bindings = self.store.bindings().await.map_err(db)?;
        for task in &tasks {
            if matches!(
                task.status,
                TaskStatus::Done
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
                    | TaskStatus::Superseded
            ) && task.assignee.is_some()
                && !bindings.iter().any(|b| b.task == task.id)
            {
                let row = self
                    .store
                    .load_task(&task.id)
                    .await
                    .map_err(db)?
                    .ok_or_else(|| invalid("task disappeared"))?;
                let mut free = row.task;
                free.assignee = None;
                if !matches!(
                    self.store
                        .compare_and_swap_task(&free, row.version)
                        .await
                        .map_err(db)?,
                    CasResult::Written { .. }
                ) {
                    return Err(invalid("cleanup CAS conflict"));
                }
            }
        }
        let teams = self.store.teams().await.map_err(db)?;
        self.executor
            .projections(&members, &teams, &bindings)
            .await
            .map_err(invalid)?;
        self.executor
            .orphans(
                &tasks,
                &teams,
                &bindings,
                &self.running.iter().cloned().collect::<Vec<_>>(),
            )
            .await
            .map_err(invalid)?;
        Ok(())
    }
}
