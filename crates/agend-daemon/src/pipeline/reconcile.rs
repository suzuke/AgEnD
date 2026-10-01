use super::*;
use agend_core::traits::StoredEvent;

impl Engine {
    pub(super) async fn boot(&mut self) -> Result<(), Refusal> {
        if let Err(reason) = crate::checks::readiness(&self.home).await {
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
            if matches!(task.status, TaskStatus::Running | TaskStatus::Open) {
                let loaded = self.load(&task.id).await?;
                if loaded.state.status() == PipelineStatus::Pending {
                    self.commit_transition(&loaded, PipelineEvent::Start, None)
                        .await?;
                    continue;
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
            }
        }
        for (_, _, due) in self.store.reminders().await.map_err(db)? {
            let tx = self.tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(
                    due.saturating_sub(log::now_unix_ms()),
                ))
                .await;
                let _ = tx.send(Input::Wake);
            });
        }
        self.refresh().await?;
        Ok(())
    }
    async fn fail_restore(&self, id: &str, reason: &str) -> Result<(), Refusal> {
        let row = self
            .store
            .load_task(id)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("task disappeared"))?;
        let mut task = row.task;
        task.status = TaskStatus::Failed;
        let event = StoredEvent {
            id: format!("restore-failed:{}", log::now_unix_ms()),
            occurred_at_unix_ms: log::now_unix_ms(),
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
                stage_entered_at_unix_ms: log::now_unix_ms(),
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
        Ok(())
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
                if let Err((_, e)) = self.release(&b, task.status == TaskStatus::Done).await {
                    log::line(&format!("{}: cleanup failed: {e}", b.task));
                }
                continue;
            }
            if let Some(repo) = self.team(&task.team_id).await?.repo
                && let Some(git) = self.git.as_ref()
            {
                if let Ok(loaded) = self.load(&task.id).await {
                    if b.kind == "review" && b.ticket != ticket(&loaded.state) {
                        self.release(&b, false).await?;
                        continue;
                    }
                    if loaded.state.merge_in_flight() {
                        let forge = crate::forge::local::LocalForge {
                            repo: PathBuf::from(&repo),
                            git: git.clone(),
                            store: self.store.clone(),
                            expected_main: None,
                        };
                        if forge
                            .find_merge(&task.id, loaded.state.current_head().unwrap_or_default())
                            .await
                            .map_err(invalid)?
                            .is_some()
                        {
                            // The merge proof is sufficient even if cleanup already removed its worktree.
                            continue;
                        }
                    }
                }
                if let Err(e) = crate::bindings::ensure(
                    &self.store,
                    git,
                    &self.home,
                    &self.exe,
                    Path::new(&repo),
                    &b,
                )
                .await
                {
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
        for member in &members {
            if bindings.iter().any(|b| b.instance == member.id) {
                continue;
            }
            let repo = self.team(&member.team).await?.repo;
            crate::bindings::snapshot(&self.home, &member.id, repo, None).map_err(invalid)?;
        }
        let dir = self.home.join("bindings");
        if let Ok(files) = std::fs::read_dir(&dir) {
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|e| e == "json")
                    && path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| !members.iter().any(|m| m.id == s))
                {
                    std::fs::remove_file(path).map_err(db)?;
                }
            }
        }
        let Some(git) = self.git.clone() else {
            return Ok(());
        };
        let repos = self
            .store
            .teams()
            .await
            .map_err(db)?
            .into_iter()
            .filter_map(|t| t.repo)
            .collect::<BTreeSet<_>>();
        for repo in repos {
            let repo = Path::new(&repo);
            let list = git
                .run(repo, &["worktree", "list", "--porcelain"])
                .await
                .map_err(invalid)?;
            for block in list.split("\n\n") {
                let Some(path) = block.lines().find_map(|l| l.strip_prefix("worktree ")) else {
                    continue;
                };
                let wt = Path::new(path);
                let checks = wt.parent() == Some(self.home.join("checks").as_path());
                let managed = wt.parent() == Some(self.home.join("worktrees").as_path())
                    && wt.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        let id = n.strip_suffix("-review").unwrap_or(n);
                        tasks.iter().any(|t| t.id == id)
                            || id.strip_prefix("t-").is_some_and(|id| {
                                !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
                            })
                    });
                let active = bindings.iter().any(|b| b.worktree == path);
                let running = wt.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    self.running
                        .iter()
                        .any(|ticket| n.starts_with(&format!("{}-", ticket.replace('/', "-"))))
                });
                if (checks && !running) || (managed && !active) {
                    let branch = block
                        .lines()
                        .find_map(|l| l.strip_prefix("branch refs/heads/"));
                    let task = branch
                        .and_then(agend_core::model::task_id_of_branch)
                        .unwrap_or("orphan");
                    crate::bindings::archive(&git, &self.home, repo, wt, task, branch, false)
                        .await
                        .map_err(invalid)?;
                    git.run(repo, &["worktree", "remove", "--force", path])
                        .await
                        .map_err(invalid)?;
                }
            }
            let branches = git
                .run(
                    repo,
                    &[
                        "for-each-ref",
                        "--format=%(refname:short)",
                        "refs/heads/agend/",
                    ],
                )
                .await
                .map_err(invalid)?;
            for branch in branches.lines() {
                let Some(id) = agend_core::model::task_id_of_branch(branch) else {
                    continue;
                };
                let task = tasks.iter().find(|t| t.id == id);
                if task.is_none()
                    || task.is_some_and(|t| {
                        matches!(
                            t.status,
                            TaskStatus::Done
                                | TaskStatus::Failed
                                | TaskStatus::Cancelled
                                | TaskStatus::Superseded
                        )
                    })
                {
                    crate::bindings::archive(
                        &git,
                        &self.home,
                        repo,
                        Path::new("/nonexistent-agend-worktree"),
                        id,
                        Some(branch),
                        task.is_some_and(|t| t.status == TaskStatus::Done),
                    )
                    .await
                    .map_err(invalid)?;
                    git.run(repo, &["branch", "-D", branch])
                        .await
                        .map_err(invalid)?;
                }
            }
        }
        let checks = self.home.join("checks");
        if let Ok(entries) = std::fs::read_dir(&checks) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name
                    .to_str()
                    .is_some_and(|n| n.starts_with("t-") && n.ends_with(".tmp"))
                    && !name.to_str().is_some_and(|n| {
                        self.running
                            .iter()
                            .any(|ticket| n.starts_with(&format!("{}-", ticket.replace('/', "-"))))
                    })
                {
                    let _ = std::fs::remove_dir_all(entry.path());
                }
            }
        }
        Ok(())
    }
}
