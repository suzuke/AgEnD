//! Real SQLite recovery and attribution. No history-based key recovery.
use agend_core::{model::Backend, runtime_records::*};
use agend_daemon::store::SqliteStore;
use agend_testkit::{block_on, tempdir::TempDir};

#[test]
fn uncertain_startup_key_survives_reopen_retention_and_blocks_all_following_keys() {
    let dir = TempDir::new("startup-store").unwrap();
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    let session = "11111111-1111-4111-8111-111111111111";
    block_on(store.add_instance(&Instance {
        id: "claude".into(),
        backend: Backend::Claude,
        program: "native".into(),
        args: vec![],
        working_directory: "/workspace".into(),
        session_id: Some(session.into()),
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    }))
    .unwrap();
    block_on(store.begin_claude_startup("claude", session)).unwrap();
    let startup = block_on(store.claude_startup("claude")).unwrap().unwrap();
    let key = ClaudeStartupKey {
        startup: startup.clone(),
        generation: "holder-1".into(),
        prompt: "trust_no".into(),
        attempt: "attempt-1".into(),
    };
    assert!(block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    drop(store);
    let store = SqliteStore::open(dir.path(), 0).unwrap();
    block_on(store.prune(1000 * 24 * 60 * 60 * 1000)).unwrap();
    assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    let mut next = key.clone();
    next.prompt = "trust_yes".into();
    next.attempt = "attempt-2".into();
    assert!(!block_on(store.reserve_claude_startup_key(next.clone())).unwrap());
    let mut forged = key.clone();
    forged.attempt = "foreign".into();
    block_on(store.finish_claude_startup_key(forged, false)).unwrap();
    assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    // Only a matching definite no-write refusal releases the own intent.
    block_on(store.finish_claude_startup_key(key.clone(), false)).unwrap();
    assert!(block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    block_on(store.finish_claude_startup_key(key.clone(), true)).unwrap();
    assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    next.generation = "foreign-holder".into();
    assert!(!block_on(store.reserve_claude_startup_key(next.clone())).unwrap());
    next.generation = "holder-1".into();
    assert!(block_on(store.reserve_claude_startup_key(next)).unwrap());
    block_on(store.halt_claude_startup("claude", session)).unwrap();
    // New physical starts reset; stale launch completions cannot release keys.
    block_on(store.begin_claude_startup("claude", session)).unwrap();
    let fresh = block_on(store.claude_startup("claude")).unwrap().unwrap();
    assert_ne!(fresh.launch, startup.launch);
    assert!(!block_on(store.reserve_claude_startup_key(key.clone())).unwrap());
    let mut current = key.clone();
    current.startup = fresh;
    assert!(block_on(store.reserve_claude_startup_key(current.clone())).unwrap());
    block_on(store.finish_claude_startup_key(key, false)).unwrap();
    assert!(!block_on(store.reserve_claude_startup_key(current)).unwrap());
    block_on(store.remove_instance("claude")).unwrap();
    assert!(block_on(store.claude_startup("claude")).unwrap().is_none());
}
