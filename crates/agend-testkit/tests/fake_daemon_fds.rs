//! The fake daemon lets go of a closed connection's descriptors at once,
//! also when it had subscribed to events (round-2 verifier: 100 closed
//! subscribers kept 100 descriptors until the next event). Its own test
//! binary, so no other test opens descriptors meanwhile.
#![cfg(unix)]

use std::time::{Duration, Instant};

use agend_core::protocol::client::{ClientRequest, SubscribeEventsData};
use agend_testkit::fake_daemon::{FakeDaemon, ProbeClient};

fn open_fds() -> usize {
    std::fs::read_dir("/dev/fd").map_or(0, |d| d.count())
}

#[test]
fn closed_subscribers_release_their_descriptors_without_a_new_event() {
    let daemon = FakeDaemon::start().unwrap();
    let before = open_fds();
    for _ in 0..100 {
        let (mut c, _) = ProbeClient::hello(daemon.socket_path(), None).unwrap();
        c.send(&ClientRequest::SubscribeEvents {
            data: SubscribeEventsData {
                after_event_id: None,
            },
        })
        .unwrap();
        // The subscription is in place once a request after it is answered.
        c.request(&ClientRequest::GetFleet {
            data: agend_core::protocol::client::RequestIdData {
                request_id: "r".into(),
            },
        })
        .unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while (daemon.open_connections() > 0 || open_fds() > before + 5) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(daemon.open_connections(), 0);
    let after = open_fds();
    assert!(after <= before + 5, "descriptors {before} -> {after}");
}
