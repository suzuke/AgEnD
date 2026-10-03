//! Own test process: detect full view/worker descriptor retention.
#![cfg(unix)]
#[path = "common/terminal_parser.rs"]
mod parser;
use agend_core::protocol::terminal::TerminalSize;
use agend_testkit::contract::terminal::Window;
use std::time::{Duration, Instant};
fn fds() -> usize {
    std::fs::read_dir("/dev/fd").unwrap().count()
}
#[test]
fn twenty_full_view_grant_close_cycles_release_socket_descriptors() {
    let fx = parser::Fake::default();
    let before = fds();
    for _ in 0..20 {
        let mut window = Window::open(&fx, None);
        window.acquire(
            "grant",
            TerminalSize {
                rows: 8,
                columns: 28,
            },
        );
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while (fx.daemon.open_connections() != 0 || fds() > before) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fx.daemon.open_connections(), 0);
    assert!(
        fds() <= before,
        "full terminal descriptors {before} -> {}",
        fds()
    );
}
