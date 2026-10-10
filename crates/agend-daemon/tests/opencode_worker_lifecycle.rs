//! Real worker threads must finish before a version switch can stop the holder.
use agend_daemon::{
    driver::opencode::runtime::{Notice, Runtime, Sink},
    store::SqliteStore,
};
use agend_testkit::tempdir::TempDir;
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

fn blocked_failure() -> (Sink, mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (arrived, arrival) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let wait = Mutex::new(wait);
    let sink = Arc::new(move |notice| {
        assert!(matches!(notice, Notice::Failed(_)));
        arrived.send(()).unwrap();
        // Real worker startup has no holder in this isolated HOME. Hold its
        // native failure callback open to distinguish cancellation from exit.
        let _ = wait.lock().unwrap().recv_timeout(Duration::from_secs(10));
    });
    (sink, arrival, release)
}

#[test]
fn cancelled_and_replaced_generations_remain_live_until_their_threads_exit() {
    let root = TempDir::new("opencode-worker-lifecycle").unwrap();
    let store = Arc::new(SqliteStore::open(root.path(), 0).unwrap());
    let runtime = Runtime::new(store);
    assert!(runtime.workers_stopped("worker"));
    let (first, arrived, release_first) = blocked_failure();
    runtime.start("worker", first);
    arrived.recv_timeout(Duration::from_secs(5)).unwrap();
    runtime.disconnect("worker");
    assert!(!runtime.workers_stopped("worker"));

    let (second, arrived, release_second) = blocked_failure();
    runtime.start("worker", second);
    arrived.recv_timeout(Duration::from_secs(5)).unwrap();
    runtime.disconnect("worker");
    release_second.send(()).unwrap();
    // The first cancelled generation remains blocked even after its replacement
    // is allowed to finish. Repeated disconnects cannot discard that proof.
    for _ in 0..10 {
        runtime.disconnect("worker");
        assert!(!runtime.workers_stopped("worker"));
        std::thread::sleep(Duration::from_millis(5));
    }
    release_first.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !runtime.workers_stopped("worker") {
        assert!(Instant::now() < deadline, "workers did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(runtime.workers_stopped("unrelated"));
}
