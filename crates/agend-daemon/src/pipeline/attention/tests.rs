//! Controlled interleavings over the real Fleet and pipeline refresh.
use super::super::*;
use crate::fleet::{Fleet, failed_attention_id};
use crate::store::{Instance, InstanceStatus};
use crate::supervisor::failed_item;
use agend_core::model::Backend;
use agend_testkit::fakes::{FakeClock, FakeDriver, FakePipelineExecutor, FakeStore};
use std::sync::atomic::{AtomicUsize, Ordering};

struct RacingView {
    fleet: Fleet,
    snapshots: AtomicUsize,
    replacement: Option<AttentionRequiredData>,
}
impl PipelineView for RacingView {
    fn view(&self) -> FleetView {
        let snapshot = self.fleet.view();
        // Refresh has captured the failure list. The operator resolves it
        // before its delayed enrichment is applied.
        if self.snapshots.fetch_add(1, Ordering::SeqCst) == 2 {
            assert!(
                self.fleet
                    .resolve(&failed_attention_id("retry-race"), AttentionAction::Retry)
                    .is_some()
            );
            if let Some(new) = &self.replacement {
                self.fleet.raise(new.clone());
            }
        }
        snapshot
    }
    fn set_teams(&self, teams: Vec<TeamView>) {
        self.fleet.set_teams(teams);
    }
    fn set_instance(&self, instance: InstanceView, summary: String) {
        self.fleet.set_instance(instance, summary);
    }
    fn sync_tasks(&self, tasks: Vec<TaskView>) {
        self.fleet.sync_tasks(tasks);
    }
    fn dismiss(&self, id: &str) {
        self.fleet.dismiss(id);
    }
    fn raise(&self, item: AttentionRequiredData) {
        self.fleet.raise(item);
    }
    fn upsert_attention(&self, item: AttentionRequiredData) {
        self.fleet.upsert_attention(item);
    }
    fn replace_attention_if(
        &self,
        expected: &AttentionRequiredData,
        item: AttentionRequiredData,
    ) -> bool {
        self.fleet.replace_attention_if(expected, item)
    }
    fn attention(&self, id: &str) -> Option<AttentionRequiredData> {
        self.fleet.attention(id)
    }
    fn resolve(&self, id: &str, action: AttentionAction) -> Option<AttentionRequiredData> {
        self.fleet.resolve(id, action)
    }
    fn publish(&self, event: DaemonEvent) -> u64 {
        self.fleet.publish(event)
    }
}
fn failure(reason: &str, since: u64) -> AttentionRequiredData {
    failed_item(
        &Instance {
            id: "retry-race".into(),
            backend: Backend::Claude,
            program: "/bin/bash".into(),
            args: Vec::new(),
            working_directory: "/tmp".into(),
            session_id: Some("session".into()),
            status: InstanceStatus::Failed,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        },
        reason,
        since,
    )
}
async fn refresh_with(replacement: Option<AttentionRequiredData>) -> Arc<RacingView> {
    let view = Arc::new(RacingView {
        fleet: Fleet::new(1000),
        snapshots: AtomicUsize::new(0),
        replacement,
    });
    view.fleet.raise(failure("old failure", 42));
    let store = Arc::new(FakeStore::new());
    let executor = FakePipelineExecutor::new(store.clone());
    let (tx, _rx) = mpsc::unbounded_channel();
    let engine = Engine {
        home: PathBuf::from("/unused"),
        store,
        fleet: view.clone(),
        codex: FakeDriver::new(),
        git: Some(executor.clone()),
        executor,
        clock: FakeClock::new(1000),
        tx,
        running: BTreeSet::new(),
        timers: BTreeSet::new(),
        checks: Arc::new(Semaphore::new(1)),
        last_day: 0,
    };
    engine.refresh().await.unwrap();
    assert_eq!(
        view.snapshots.load(Ordering::SeqCst),
        3,
        "the interleaving must run at the failure snapshot"
    );
    view
}
#[tokio::test]
async fn refresh_cannot_resurrect_a_failure_resolved_by_retry() {
    let view = refresh_with(None).await;
    assert_eq!(
        view.fleet.attention(&failed_attention_id("retry-race")),
        None,
        "stale refresh resurrected a resolved failure"
    );
    let events = view.fleet.subscribe(None).unwrap().backlog;
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.event, DaemonEvent::AttentionRequired { .. }))
            .count(),
        1,
        "retry must not republish the old failure"
    );
    assert!(
        matches!(events.last().unwrap().event, DaemonEvent::AttentionResolved { ref data } if data.action == AttentionAction::Retry)
    );
}
#[tokio::test]
async fn refresh_cannot_overwrite_a_new_failure_with_the_old_snapshot() {
    let new = failure("new failure after retry", 99);
    let view = refresh_with(Some(new.clone())).await;
    assert_eq!(
        view.fleet.attention(&failed_attention_id("retry-race")),
        Some(new),
        "stale refresh overwrote a fresh failure"
    );
    assert_eq!(
        view.fleet.subscribe(None).unwrap().backlog.len(),
        3,
        "only original failure, retry and new failure should be published"
    );
}

#[test]
fn matching_failure_enrichment_updates_and_publishes_once() {
    let fleet = Fleet::new(1000);
    let original = failure("still failed", 42);
    fleet.raise(original.clone());
    let mut enriched = original.clone();
    enriched.unblocks = Some(1);
    assert!(fleet.replace_attention_if(&original, enriched.clone()));
    assert_eq!(
        fleet.attention(&failed_attention_id("retry-race")),
        Some(enriched.clone())
    );
    assert!(fleet.replace_attention_if(&enriched, enriched.clone()));
    assert!(!fleet.replace_attention_if(&original, original.clone()));
    let mut other_id = enriched.clone();
    other_id.attention_id = Some("different-id".into());
    assert!(!fleet.replace_attention_if(&enriched, other_id));
    assert_eq!(fleet.subscribe(None).unwrap().backlog.len(), 2);
    assert!(
        fleet
            .resolve(&failed_attention_id("retry-race"), AttentionAction::Retry)
            .is_some()
    );
    assert!(!fleet.replace_attention_if(&enriched, enriched.clone()));
    assert_eq!(fleet.attention(&failed_attention_id("retry-race")), None);
    assert_eq!(fleet.subscribe(None).unwrap().backlog.len(), 3);
}
