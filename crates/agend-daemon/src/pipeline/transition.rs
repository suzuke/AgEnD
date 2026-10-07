//! Return side effects only after the atomic store commit has succeeded.
use super::*;
use agend_core::pipeline::state::{TransitionError, step};
use agend_core::traits::Clock;

pub(crate) struct WallClock;
impl Clock for WallClock {
    fn now_unix_ms(&self) -> u64 {
        log::now_unix_ms()
    }
}

pub(super) async fn advance<S: PipelineStore, C: Clock>(
    store: &S,
    clock: &C,
    loaded: &Loaded,
    event: PipelineEvent,
    detail: Option<String>,
    event_id: String,
) -> Result<(Task, PipelineState, u64, Vec<PipelineAction>), Refusal>
where
    S::Error: std::fmt::Display,
{
    let confirmation = match &event {
        PipelineEvent::WorkCompleted { .. } => Some(format!("dispatch:{}", ticket(&loaded.state))),
        PipelineEvent::ApprovalGranted { reviewer, .. }
        | PipelineEvent::ChangesRequested { reviewer, .. }
            if loaded.state.current_stage().is_some_and(|s| {
                matches!(
                    s.stage,
                    Stage::Approval {
                        by: Approver::Role(_),
                        ..
                    }
                )
            }) =>
        {
            Some(format!("dispatch:{}/{reviewer}", ticket(&loaded.state)))
        }
        _ => None,
    };
    let (next, actions) = step(&loaded.state, event.clone()).map_err(|e| {
        let code = match e {
            TransitionError::MergeInFlight => "merge_in_flight",
            _ => error_code::STALE_RESULT,
        };
        (code.into(), format!("{e}; now {}", ticket(&loaded.state)))
    })?;
    let now = clock.now_unix_ms();
    let mut task = loaded.task.clone();
    task.status = match next.status() {
        PipelineStatus::Pending => TaskStatus::Open,
        PipelineStatus::Running => TaskStatus::Running,
        PipelineStatus::Done => TaskStatus::Done,
        PipelineStatus::Failed => TaskStatus::Failed,
        PipelineStatus::Cancelled => TaskStatus::Cancelled,
    };
    task.merge_commit = next.merge_commit().map(str::to_owned);
    let entered = if (next.stage_index(), next.attempt())
        != (loaded.state.stage_index(), loaded.state.attempt())
    {
        now
    } else {
        loaded.progress.data.stage_entered_at_unix_ms
    };
    let progress = TaskProgress {
        pipeline: serde_json::to_string(&next.snapshot()).map_err(db)?,
        stage_entered_at_unix_ms: entered,
        merge_intent: loaded.progress.data.merge_intent.clone(),
        block_reason: None,
    };
    let record = StoredEvent {
        id: event_id,
        occurred_at_unix_ms: now,
        kind: "pipeline".into(),
        detail: format!(
            "{event:?}{}",
            detail.map_or(String::new(), |d| format!("\n{d}"))
        ),
    };
    if !matches!(
        store
            .advance_pipeline(
                &task,
                loaded.version,
                &progress,
                &record,
                confirmation.as_deref()
            )
            .await
            .map_err(db)?,
        CasResult::Written { .. }
    ) {
        log::line(&format!(
            "error: {}: pipeline CAS conflict; event discarded",
            task.id
        ));
        return Err(invalid("pipeline CAS conflict; event discarded"));
    }
    Ok((task, next, entered, actions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::traits::Store;
    use agend_testkit::{
        block_on,
        fakes::{FakeClock, FakeStore},
    };
    fn setup() -> (FakeStore, FakeClock, Loaded) {
        let store = FakeStore::new();
        let task = Task::new("t-1", "Research", "general", "research", 1);
        block_on(store.create_task(&task)).unwrap();
        let state = PipelineState::new("t-1", validate(Workflow::builtin_research()).unwrap());
        let progress = Progress {
            attention_revision: 0,
            data: TaskProgress {
                pipeline: serde_json::to_string(&state.snapshot()).unwrap(),
                stage_entered_at_unix_ms: 123,
                merge_intent: None,
                block_reason: None,
            },
            block_reason: None,
            attention_reason: None,
            acknowledged: false,
        };
        (
            store,
            FakeClock::new(1000),
            Loaded {
                version: 1,
                task,
                state,
                progress,
            },
        )
    }
    #[test]
    fn rejected_events_and_store_failures_never_release_actions_or_write_state() {
        let (store, clock, loaded) = setup();
        let before = store.calls().len();
        assert!(
            block_on(advance(
                &store,
                &clock,
                &loaded,
                PipelineEvent::CommandFinished {
                    stage_id: "checks".into(),
                    attempt: 99,
                    head: None,
                    exit_code: Some(0)
                },
                None,
                "stale".into()
            ))
            .is_err()
        );
        assert_eq!(
            store.calls().len(),
            before,
            "core rejection must not touch storage"
        );
        store.fail_next("advance_task", "disk full");
        assert!(
            block_on(advance(
                &store,
                &clock,
                &loaded,
                PipelineEvent::Start,
                None,
                "failed".into()
            ))
            .is_err()
        );
        assert_eq!(
            block_on(store.load_task("t-1")).unwrap().unwrap().version,
            1
        );
        assert!(store.events("t-1").is_empty());
        assert!(block_on(store.load_task_progress("t-1")).unwrap().is_none());
        let (_, state, entered, actions) = block_on(advance(
            &store,
            &clock,
            &loaded,
            PipelineEvent::Start,
            None,
            "ok".into(),
        ))
        .unwrap();
        assert_eq!(entered, 1000);
        assert_eq!(state.attempt(), 1);
        assert!(!actions.is_empty());
        assert_eq!(store.events("t-1").len(), 1);
        assert!(
            block_on(advance(
                &store,
                &clock,
                &loaded,
                PipelineEvent::Start,
                None,
                "conflict".into()
            ))
            .unwrap_err()
            .1
            .contains("CAS conflict")
        );
        assert_eq!(
            store.events("t-1").len(),
            1,
            "CAS conflict cannot append or retry"
        );
    }
}
