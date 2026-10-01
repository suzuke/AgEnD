use super::*;
use crate::forge::local::{LocalError, LocalForge};
use agend_core::model::{Backend, DeliveryState};
use agend_core::policy::{
    assign::{self, *},
    busy::BusyLevel,
};
use agend_core::traits::{AgentMessage, Driver, Forge, MergeRequest, MergeResult, Submission};

impl Engine {
    pub(super) async fn action(
        &mut self,
        task: &Task,
        state: &PipelineState,
        entered: u64,
        action: PipelineAction,
    ) -> Result<(), Refusal> {
        match action {
            PipelineAction::ScheduleTimeout {
                stage_id,
                attempt,
                timeout_ms,
                ..
            } => {
                let kind = state.current_stage().map(|s| s.stage.kind());
                if matches!(
                    kind,
                    Some(
                        agend_core::pipeline::stage::StageKind::Command
                            | agend_core::pipeline::stage::StageKind::Merge
                    )
                ) {
                    return Ok(());
                }
                let key = format!("{}/{stage_id}/{attempt}", task.id);
                if self.timers.insert(key) {
                    let tx = self.tx.clone();
                    let task = task.id.clone();
                    let delay = entered
                        .saturating_add(timeout_ms)
                        .saturating_sub(log::now_unix_ms());
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        let _ = tx.send(Input::Event(
                            task,
                            PipelineEvent::StageTimedOut { stage_id, attempt },
                        ));
                    });
                }
            }
            PipelineAction::AssignWork { role, .. } | PipelineAction::ReturnToWork { role, .. } => {
                self.assign(task, state, &role, false).await?;
            }
            PipelineAction::RequestApproval { .. } => {
                if let Some(Stage::Approval {
                    by: Approver::Role(role),
                    ..
                }) = state.current_stage().map(|s| &s.stage)
                {
                    self.assign(task, state, role, true).await?;
                }
            }
            PipelineAction::Submit {
                stage_id, attempt, ..
            } => {
                let repo = self
                    .team(&task.team_id)
                    .await?
                    .repo
                    .ok_or_else(|| invalid("submit requires repo"))?;
                let forge = LocalForge {
                    repo: repo.into(),
                    git: self.git.clone().ok_or_else(|| invalid("git unavailable"))?,
                    store: self.store.clone(),
                    expected_main: None,
                };
                let submission = Submission {
                    task_id: task.id.clone(),
                    branch: state.branch().unwrap_or_default().into(),
                    title: task.title.clone(),
                    body: String::new(),
                };
                let event = match forge.submit(&submission).await {
                    Ok(c) => PipelineEvent::Submitted {
                        stage_id,
                        attempt,
                        change_id: c.id,
                    },
                    Err(e) => PipelineEvent::StageFailed {
                        stage_id,
                        attempt,
                        reason: e.to_string(),
                    },
                };
                let _ = self.tx.send(Input::Event(task.id.clone(), event));
            }
            PipelineAction::RunCommand {
                stage_id,
                attempt,
                command,
                head,
                timeout_ms,
                ..
            } => {
                let key = format!("{}/{stage_id}/{attempt}", task.id);
                if !self.running.insert(key.clone()) {
                    return Ok(());
                }
                let team = self.team(&task.team_id).await?;
                let (Some(repo), Some(git), Some(head_sha)) =
                    (team.repo, self.git.clone(), head.clone())
                else {
                    self.running.remove(&key);
                    return Err(invalid("checks require repo, head and git"));
                };
                let (tx, home, task, checks) = (
                    self.tx.clone(),
                    self.home.clone(),
                    task.id.clone(),
                    self.checks.clone(),
                );
                tokio::spawn(async move {
                    let Ok(_permit) = checks.acquire().await else {
                        return;
                    };
                    let result = crate::checks::run(
                        &git,
                        &home,
                        Path::new(&repo),
                        &key,
                        &head_sha,
                        &command,
                        timeout_ms,
                    )
                    .await;
                    let _ = tx.send(Input::Check {
                        task,
                        stage: stage_id,
                        attempt,
                        head,
                        result,
                    });
                });
            }
            PipelineAction::Merge {
                stage_id,
                attempt,
                head,
            } => {
                let repo = self
                    .team(&task.team_id)
                    .await?
                    .repo
                    .ok_or_else(|| invalid("merge requires repo"))?;
                let git = self.git.clone().ok_or_else(|| invalid("git unavailable"))?;
                let forge = LocalForge {
                    repo: repo.clone().into(),
                    git: git.clone(),
                    store: self.store.clone(),
                    expected_main: Some(
                        git.run(Path::new(&repo), &["rev-parse", "main"])
                            .await
                            .map_err(invalid)?,
                    ),
                };
                if forge
                    .find_merge(&task.id, &head)
                    .await
                    .map_err(invalid)?
                    .is_none()
                {
                    let main = git
                        .run(Path::new(&repo), &["rev-parse", "main"])
                        .await
                        .map_err(invalid)?;
                    if !git
                        .ancestor(Path::new(&repo), &main, &head)
                        .await
                        .map_err(invalid)?
                    {
                        let binding = self
                            .store
                            .bindings()
                            .await
                            .map_err(db)?
                            .into_iter()
                            .find(|b| b.task == task.id && b.kind == "work")
                            .ok_or_else(|| invalid("missing work binding for rebase"))?;
                        let wt = Path::new(&binding.worktree);
                        let clean = git
                            .run(wt, &["status", "--porcelain"])
                            .await
                            .map_err(invalid)?
                            .is_empty();
                        let conflict = !clean
                            || git
                                .run(wt, &["rebase", "--no-autostash", "main"])
                                .await
                                .is_err();
                        if conflict && clean {
                            let _ = git.run(wt, &["rebase", "--abort"]).await;
                        }
                        let rebased = git.run(wt, &["rev-parse", "HEAD"]).await.map_err(invalid)?;
                        let patch_id = git
                            .patch_id(Path::new(&repo), &rebased)
                            .await
                            .map_err(invalid)?;
                        log::line(&format!(
                            "{}: main advanced; rebased, {}",
                            task.id,
                            if !conflict && state.patch_id() == Some(&patch_id) {
                                "approvals kept, checks run again"
                            } else {
                                "back to work"
                            }
                        ));
                        let _ = self.tx.send(Input::Event(
                            task.id.clone(),
                            PipelineEvent::MainAdvanced {
                                rebased_head: rebased,
                                patch_id,
                                conflict,
                            },
                        ));
                        let _ = self.tx.send(Input::Event(
                            task.id.clone(),
                            PipelineEvent::MergeFailed {
                                stage_id,
                                attempt,
                                head,
                                reason: "main advanced".into(),
                            },
                        ));
                        return Ok(());
                    }
                }
                let event = match forge
                    .merge_if_head_is(&MergeRequest {
                        branch: state.branch().unwrap_or_default().into(),
                        expected_head: head.clone(),
                    })
                    .await
                {
                    Ok(MergeResult::Merged { merge_commit }) => PipelineEvent::MergeCompleted {
                        stage_id,
                        attempt,
                        head,
                        merge_commit,
                    },
                    Ok(MergeResult::HeadChanged { actual_head }) => {
                        let patch_id = git
                            .patch_id(Path::new(&repo), &actual_head)
                            .await
                            .map_err(invalid)?;
                        let _ = self.tx.send(Input::Event(
                            task.id.clone(),
                            PipelineEvent::CommitCreated {
                                head: actual_head,
                                patch_id,
                            },
                        ));
                        PipelineEvent::MergeFailed {
                            stage_id,
                            attempt,
                            head,
                            reason: "branch head moved".into(),
                        }
                    }
                    Err(LocalError::Blocked(reason)) => {
                        self.note(&task.id, Some(format!("merge-blocked:{reason}")))
                            .await?;
                        return Ok(());
                    }
                    Err(e) => PipelineEvent::MergeFailed {
                        stage_id,
                        attempt,
                        head,
                        reason: e.to_string(),
                    },
                };
                let _ = self.tx.send(Input::Event(task.id.clone(), event));
            }
            PipelineAction::TaskDone { .. }
            | PipelineAction::TaskCancelled { .. }
            | PipelineAction::TaskFailed { .. } => {
                for b in self
                    .store
                    .bindings()
                    .await
                    .map_err(db)?
                    .into_iter()
                    .filter(|b| b.task == task.id)
                {
                    self.release(&b, state.status() == PipelineStatus::Done)
                        .await?;
                }
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
            PipelineAction::NotifyTimeout { stage_id } => {
                log::line(&format!("{}: {stage_id} timed out (notification)", task.id))
            }
            _ => return Err(invalid("unsupported pipeline action")),
        }
        Ok(())
    }
    async fn assign(
        &mut self,
        task: &Task,
        state: &PipelineState,
        role: &str,
        review: bool,
    ) -> Result<(), Refusal> {
        let ticket = ticket(state);
        let bindings = self.store.bindings().await.map_err(db)?;
        let existing_reviewer = bindings
            .iter()
            .find(|b| review && b.task == task.id && b.kind == "review" && b.ticket == ticket)
            .map(|b| b.instance.clone());
        if !review
            && let Some(holder) = &task.assignee
            && self
                .store
                .instance(holder)
                .await
                .map_err(db)?
                .is_some_and(|i| i.status != crate::store::InstanceStatus::Running)
        {
            return Ok(());
        }
        let members = self.store.members().await.map_err(db)?;
        let tasks = self.store.tasks().await.map_err(db)?;
        let instances = self.store.instances().await.map_err(db)?;
        let candidates = instances
            .iter()
            .filter(|i| i.status == crate::store::InstanceStatus::Running)
            .filter_map(|i| {
                let m = members
                    .iter()
                    .find(|m| m.id == i.id && m.team == task.team_id)?;
                let held = tasks
                    .iter()
                    .find(|t| {
                        t.assignee.as_deref() == Some(&i.id)
                            && !matches!(
                                t.status,
                                TaskStatus::Done
                                    | TaskStatus::Failed
                                    | TaskStatus::Cancelled
                                    | TaskStatus::Superseded
                            )
                    })
                    .map(|t| t.id.clone())
                    .or_else(|| {
                        bindings
                            .iter()
                            .find(|b| b.instance == i.id)
                            .map(|b| b.task.clone())
                    });
                Some(Candidate {
                    instance_id: i.id.clone(),
                    team_id: m.team.clone(),
                    role: m.role.clone(),
                    backend: i.backend,
                    held_task: held,
                    ephemeral: false,
                    usage_available: !state.approval_reviewers().contains(&i.id),
                })
            })
            .collect::<Vec<_>>();
        let capacity = members
            .iter()
            .filter(|m| m.team == task.team_id && m.role == role)
            .count();
        let holder_backend = instances
            .iter()
            .find(|i| Some(&i.id) == task.assignee.as_ref())
            .map_or(Backend::Claude, |i| i.backend);
        let purpose = if review {
            Purpose::Review {
                task_holder: task.assignee.clone().unwrap_or_default(),
                task_holder_backend: holder_backend,
            }
        } else if let Some(holder) = &task.assignee {
            Purpose::Rework {
                task_id: task.id.clone(),
                task_holder: holder.clone(),
                branch: state.branch().map(str::to_owned),
                review_comments: Vec::new(),
            }
        } else {
            Purpose::NewTask
        };
        let request = AssignmentRequest {
            team_id: task.team_id.clone(),
            role: role.into(),
            allowed_backends: Backend::ALL.to_vec(),
            available_backends: Vec::new(),
            role_capacity: (capacity > 0).then_some(RoleCapacity {
                minimum_instances: 0,
                maximum_instances: capacity,
                current_instances: capacity,
            }),
            purpose,
        };
        let instance = match existing_reviewer
            .map(|instance_id| AssignmentDecision::Assigned { instance_id })
            .unwrap_or_else(|| assign::choose(&request, &candidates))
        {
            AssignmentDecision::Assigned { instance_id } => instance_id,
            AssignmentDecision::AskForRole { .. }
            | AssignmentDecision::Queue {
                reason: QueueReason::NoEligibleReviewer,
            } => {
                self.no_role(task, state, role).await?;
                return Ok(());
            }
            _ => {
                log::line(&format!(
                    "{}: queued (no free {role} in {})",
                    task.id, task.team_id
                ));
                return Ok(());
            }
        };
        if !review && task.assignee.is_none() {
            let row = self
                .store
                .load_task(&task.id)
                .await
                .map_err(db)?
                .ok_or_else(|| invalid("task disappeared"))?;
            let mut assigned = row.task;
            assigned.assignee = Some(instance.clone());
            let p = self
                .store
                .progress(&task.id)
                .await
                .map_err(db)?
                .ok_or_else(|| invalid("pipeline missing"))?;
            let event = StoredEvent {
                id: format!("assigned:{ticket}"),
                occurred_at_unix_ms: log::now_unix_ms(),
                kind: "assigned".into(),
                detail: instance.clone(),
            };
            if !matches!(
                self.store
                    .advance_task(&assigned, row.version, &p.data, &event)
                    .await
                    .map_err(db)?,
                CasResult::Written { .. }
            ) {
                return Err(invalid("assignment CAS conflict"));
            }
        }
        let team = self.team(&task.team_id).await?;
        let mut worktree = self
            .home
            .join("workspace")
            .join(&instance)
            .display()
            .to_string();
        if let Some(repo) = team.repo {
            let branch = if review {
                None
            } else {
                Some(state.branch().map(str::to_owned).unwrap_or_else(|| {
                    agend_core::model::work_branch(&task.id, &slug(&task.title))
                }))
            };
            worktree = self
                .home
                .join("worktrees")
                .join(if review {
                    format!("{}-review", task.id)
                } else {
                    task.id.clone()
                })
                .display()
                .to_string();
            let b = BindingRow {
                instance: instance.clone(),
                task: task.id.clone(),
                kind: if review { "review" } else { "work" }.into(),
                worktree: worktree.clone(),
                branch,
                head: if review {
                    state.current_head().map(str::to_owned)
                } else {
                    None
                },
                ticket: ticket.clone(),
                status: "pending".into(),
            };
            self.store.put_binding(&b).await.map_err(db)?;
            let git = self
                .git
                .as_ref()
                .ok_or_else(|| invalid("real git unavailable"))?;
            crate::bindings::ensure(
                &self.store,
                git,
                &self.home,
                &self.exe,
                Path::new(&repo),
                &b,
            )
            .await
            .map_err(invalid)?;
        } else if review {
            let b = BindingRow {
                instance: instance.clone(),
                task: task.id.clone(),
                kind: "review".into(),
                worktree: worktree.clone(),
                branch: None,
                head: None,
                ticket: ticket.clone(),
                status: "ready".into(),
            };
            self.store.put_binding(&b).await.map_err(db)?;
        }
        let instructions = state
            .current_stage()
            .map(|s| match &s.stage {
                Stage::Work { instructions, .. } => instructions.as_str(),
                _ => "Review the committed changes",
            })
            .unwrap_or_default();
        let body = format!(
            "dispatch {ticket}\nkind: {}\ntitle: {}\nworktree: {worktree}\n{instructions}\nreason: {}\nnext: agend {} {ticket}",
            if review { "review" } else { "work" },
            task.title,
            state.pending_work_reason().unwrap_or("initial assignment"),
            if review {
                "review approve"
            } else if state.workflow().requires_repo() {
                "done"
            } else {
                "result"
            }
        );
        self.deliver(
            &instance,
            &format!("dispatch:{ticket}"),
            Some(task.id.clone()),
            &body,
        )
        .await?;
        log::line(&format!("{}: assigned {ticket} to {instance}", task.id));
        Ok(())
    }
    pub(super) async fn deliver(
        &self,
        to: &str,
        id: &str,
        task: Option<String>,
        body: &str,
    ) -> Result<(), Refusal> {
        let member = self
            .store
            .members()
            .await
            .map_err(db)?
            .into_iter()
            .find(|m| m.id == to)
            .ok_or_else(|| invalid("unknown instance"))?;
        if member.delivery == "inbox" {
            let new = crate::store::NewMessage {
                id: id.into(),
                from_instance: "daemon".into(),
                to_instance: to.into(),
                task_id: task,
                body: body.into(),
                level: BusyLevel::Queue,
            };
            if matches!(
                self.store
                    .claim_message(&new, log::now_unix_ms())
                    .await
                    .map_err(db)?,
                crate::store::Claim::Different(_)
            ) {
                return Err(invalid("message id already has different content"));
            }
            self.store
                .advance_message(id, DeliveryState::Sent, None, log::now_unix_ms())
                .await
                .map_err(db)?;
        } else {
            self.codex
                .deliver(
                    to,
                    &AgentMessage {
                        id: id.into(),
                        from: "daemon".into(),
                        task_id: task,
                        body: body.into(),
                    },
                    BusyLevel::Queue,
                )
                .await
                .map_err(db)?;
        }
        Ok(())
    }
    pub(super) async fn release(&self, b: &BindingRow, merged: bool) -> Result<(), Refusal> {
        let team = self
            .store
            .load_task(&b.task)
            .await
            .map_err(db)?
            .ok_or_else(|| invalid("binding task missing"))?
            .task
            .team_id;
        if let Some(repo) = self.team(&team).await?.repo {
            let patch = crate::bindings::release(
                &self.store,
                self.git
                    .as_ref()
                    .ok_or_else(|| invalid("git unavailable"))?,
                &self.home,
                &self.exe,
                Path::new(&repo),
                b,
                merged,
            )
            .await
            .map_err(invalid)?;
            if let Some(patch) = patch {
                self.store
                    .append_event(
                        &b.task,
                        &StoredEvent {
                            id: format!("wip:{patch}"),
                            occurred_at_unix_ms: log::now_unix_ms(),
                            kind: "wip_archived".into(),
                            detail: patch.clone(),
                        },
                    )
                    .await
                    .map_err(db)?;
                log::line(&format!("{}: WIP saved {patch}", b.task));
            }
        } else {
            self.store.delete_binding(&b.instance).await.map_err(db)?;
        }
        Ok(())
    }
    pub(super) async fn wake(&mut self) -> Result<(), Refusal> {
        for task in self.store.tasks().await.map_err(db)? {
            if matches!(task.status, TaskStatus::Running | TaskStatus::Open) {
                let loaded = self.load(&task.id).await?;
                if loaded.progress.attention_reason.is_some() {
                    continue;
                }
                for action in outstanding_actions(&loaded.state) {
                    if matches!(
                        action,
                        PipelineAction::AssignWork { .. }
                            | PipelineAction::ReturnToWork { .. }
                            | PipelineAction::RequestApproval { .. }
                    ) {
                        if let Some(id) = task.assignee.as_ref()
                            && self
                                .store
                                .message(&format!("dispatch:{}", ticket(&loaded.state)))
                                .await
                                .map_err(db)?
                                .is_some()
                            && matches!(
                                action,
                                PipelineAction::AssignWork { .. }
                                    | PipelineAction::ReturnToWork { .. }
                            )
                        {
                            let _ = id;
                            continue;
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
        }
        self.send_reminders().await?;
        Ok(())
    }
}
fn slug(title: &str) -> String {
    let slug = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-').chars().take(40).collect::<String>();
    if slug.is_empty() { "task".into() } else { slug }
}
