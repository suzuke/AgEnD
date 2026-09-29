//! The fake daemon and the CLP proxy stop even when the process has no
//! descriptor left (round-3 verifier: under `ulimit -n 256` their drop
//! woke the accept thread with a `connect`, which failed, and the join
//! hung forever). Its own test binary: it lowers this process's limit.
#![cfg(unix)]

use std::fs::File;
use std::sync::mpsc;
use std::time::Duration;

use agend_testkit::contract::client::proxy::{Options, Proxy};
use agend_testkit::fake_daemon::FakeDaemon;

#[test]
fn drop_stops_the_accept_threads_with_no_descriptor_left() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit/setrlimit on this process with a valid struct.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    let saved = limit;
    limit.rlim_cur = limit.rlim_cur.min(128);
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };

    let daemon = FakeDaemon::start().unwrap();
    let proxy = Proxy::start(
        daemon.socket_path().to_path_buf(),
        Options::rewrite(std::sync::Arc::new(|_, _, line| vec![line])),
    )
    .unwrap();
    let mut hog = Vec::new();
    while let Ok(f) = File::open("/dev/null") {
        hog.push(f);
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        drop(proxy);
        drop(daemon);
        let _ = tx.send(());
    });
    let stopped = rx.recv_timeout(Duration::from_secs(5));
    drop(hog);
    unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &saved) };
    assert!(stopped.is_ok(), "drop hung with no descriptor left");
}
