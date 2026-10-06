//! Actual daemon, holder/parser and PTY producer; no Claude executions.
use super::*;
use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};
use agend_testkit::fake_daemon::ProbeClient;
use std::sync::atomic::Ordering;

fn recorded(width: u16) -> Vec<&'static str> {
    // include_str is relative to this module, not the integration root.
    if width == 100 {
        vec![
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-100x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-selected-yes-100x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-100x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-0.txt"
            ),
        ]
    } else {
        vec![
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-140x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-workspace-trust-selected-yes-140x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-140x24.txt"
            ),
            include_str!(
                "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-140x24-0.txt"
            ),
        ]
    }
}
fn producer(width: u16, mutate: impl Fn(&mut Vec<String>), stuck: bool) -> Fixture {
    producer_with_ready(width, mutate, stuck, None)
}
fn producer_with_ready(
    width: u16,
    mutate: impl Fn(&mut Vec<String>),
    stuck: bool,
    ready: Option<&str>,
) -> Fixture {
    let f = Fixture::with_script(
        1,
        None,
        "/bin/sh",
        "exec python3 -u \"$AGEND_HOME/startup.py\"",
    );
    let own = f.home.canonicalize().unwrap().display().to_string();
    let mut source = recorded(width);
    if let Some(ready) = ready {
        source[3] = ready;
    }
    let mut frames = source
        .into_iter()
        .map(|s| {
            s.trim_end()
                .replace("<rec>/h1/workspace/g12-startup-capture", &own)
        })
        .collect::<Vec<_>>();
    mutate(&mut frames);
    fs::write(
        f.home.join("frames.json"),
        serde_json::to_vec(&frames).unwrap(),
    )
    .unwrap();
    let script = format!(
        r#"import os,sys,time,json,tty
tty.setraw(0)
home=os.environ['AGEND_HOME']
frames=json.load(open(home+'/frames.json'))
sys.stdout.write('UNKNOWN STARTUP\r\n');sys.stdout.flush()
while not os.path.exists(home+'/show'): time.sleep(0.02)
index=0
def show():
    sys.stdout.write('\x1b[2J\x1b[H'+frames[index].replace('\n','\r\n'));sys.stdout.flush()
show()
while True:
    count=3 if index==0 else 1
    value=os.read(0,count)
    if not value: break
    with open(home+'/keys.log','a') as log:
        log.write(json.dumps(list(value))+'\n');log.flush()
    if {stuck}: continue
    expected=b'\x1b[B' if index==0 else b'\r'
    if value==expected and index<3:
        index+=1;show()
"#,
        stuck = if stuck { "True" } else { "False" }
    );
    fs::write(f.home.join("startup.py"), script).unwrap();
    f
}
fn keys(f: &Fixture) -> Vec<u8> {
    fs::read_to_string(f.home.join("keys.log"))
        .unwrap_or_default()
        .lines()
        .flat_map(|l| serde_json::from_str::<Vec<u8>>(l).unwrap())
        .collect()
}
fn wait_keys(f: &Fixture, expected: &[u8]) {
    let until = Instant::now() + Duration::from_secs(10);
    while keys(f) != expected {
        assert!(
            Instant::now() < until,
            "keys {:?}, expected {expected:?}",
            keys(f)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn owner(f: &Fixture, width: u16) -> ProbeClient {
    let (mut client, _) = ProbeClient::hello(&f.home.join(DAEMON_SOCKET), None).unwrap();
    client
        .send(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: id(),
                instance_id: "claude".into(),
                viewport: TerminalViewport { top: None, rows: 1 },
            },
        })
        .unwrap();
    let Some(ClientResponse::TerminalFrame { data }) =
        client.recv_within(Duration::from_secs(5)).unwrap()
    else {
        panic!("missing frame")
    };
    assert_eq!(
        data.frame.size,
        TerminalSize {
            rows: 24,
            columns: 100
        },
        "initial size before Spawn"
    );
    let request = id();
    client
        .send(&ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: request.clone(),
                instance_id: "claude".into(),
                view_id: data.view_id,
                generation: data.frame.generation,
                operation: ClientTerminalOperation::Acquire {
                    size: TerminalSize {
                        rows: 24,
                        columns: width,
                    },
                },
            },
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < until);
        if let Some(ClientResponse::TerminalControlAck { data }) =
            client.recv_within(Duration::from_secs(1)).unwrap()
            && data.request_id == request
        {
            assert!(matches!(
                data.control,
                TerminalControlState::Controlled { .. }
            ));
            break;
        }
    }
    client
}
#[test]
fn startup_native_menus_use_three_keys_at_both_recorded_widths_then_require_session_start() {
    for width in [100, 140] {
        let mut f = producer(width, |_| {}, false);
        f.start();
        if width == 140 {
            drop(owner(&f, width));
            std::thread::sleep(Duration::from_millis(150));
        }
        fs::write(f.home.join("show"), b"").unwrap();
        wait_keys(&f, b"\x1b[B\r\r");
        std::thread::sleep(Duration::from_millis(5200));
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty(),
            "UI without SessionStart is not idle"
        );
        f.hook("SessionStart", json!({"source":"startup"}));
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty(),
            "five-second idle window"
        );
        std::thread::sleep(Duration::from_millis(5300));
        assert_eq!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .len(),
            1
        );
        assert_eq!(keys(&f), b"\x1b[B\r\r");
        f.stop();
        assert!(
            block_on(f.store().claude_startup("claude"))
                .unwrap()
                .unwrap()
                .halted
        );
    }
}
#[test]
fn startup_captured_ready_hint_waits_for_completion_and_stable_idle() {
    captured_ready_waits_for_stable_idle(include_str!(
        "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-3.txt"
    ));
}
#[test]
fn startup_captured_how_does_hint_waits_for_completion_and_stable_idle() {
    captured_ready_waits_for_stable_idle(include_str!(
        "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-4.txt"
    ));
}
fn captured_ready_waits_for_stable_idle(ready: &'static str) {
    captured_ready_waits_for_stable_idle_at_width(ready, 100);
}
fn captured_ready_waits_for_stable_idle_at_width(ready: &str, width: u16) {
    let mut f = producer_with_ready(width, |_| {}, false, Some(ready));
    f.start();
    if width == 140 {
        drop(owner(&f, width));
        std::thread::sleep(Duration::from_millis(150));
    }
    f.hook("SessionStart", json!({"source":"startup"}));
    std::thread::sleep(Duration::from_millis(5200));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    fs::write(f.home.join("show"), b"").unwrap();
    wait_keys(&f, b"\x1b[B\r\r");
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty()
    );
    std::thread::sleep(Duration::from_millis(5300));
    assert_eq!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .len(),
        1
    );
    assert_eq!(keys(&f), b"\x1b[B\r\r", "Ready must not send another key");
    f.stop();
    let state = block_on(f.store().claude_startup("claude"))
        .unwrap()
        .unwrap();
    assert!(state.halted, "captured Ready ends automatic startup");
}

#[test]
fn startup_variable_ready_suggestions_replay_actual_v5_and_both_widths() {
    for ready in [
        include_str!(
            "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-v5-typecheck.txt"
        ),
        include_str!(
            "../../../agend-core/tests/fixtures/screens/claude-2.1.284-main-100x24-v5-lint.txt"
        ),
    ] {
        captured_ready_waits_for_stable_idle_at_width(ready, 100);
    }
    let wide = recorded(140)[3].replace(
        "write a test for <filepath>",
        "another unrecorded suggestion",
    );
    captured_ready_waits_for_stable_idle_at_width(&wide, 140);
}

#[test]
fn startup_variable_ready_rejects_unknown_footer_and_split_hint_without_idle_or_more_keys() {
    for variant in 0..2 {
        let mut f = producer(
            100,
            |frames| {
                frames[3] = frames[3].replace("edit <filepath> to...", "fix typecheck errors");
                if variant == 0 {
                    frames[3] = frames[3].replace("bypass permissions", "confirm permissions");
                } else {
                    frames[3] = frames[3].replace("fix typecheck errors", "fix typecheck\nerrors");
                }
            },
            false,
        );
        f.start();
        f.hook("SessionStart", json!({"source":"startup"}));
        fs::write(f.home.join("show"), b"").unwrap();
        wait_keys(&f, b"\x1b[B\r\r");
        std::thread::sleep(Duration::from_millis(5500));
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty()
        );
        assert_eq!(keys(&f), b"\x1b[B\r\r");
        f.stop();
        assert!(
            !block_on(f.store().claude_startup("claude"))
                .unwrap()
                .unwrap()
                .halted
        );
    }
}
#[test]
fn startup_unknown_conflicting_or_foreign_frames_never_authorize_a_key() {
    for variant in 0..3 {
        let mut f = producer(
            100,
            |frames| match variant {
                0 => {
                    frames[0] = frames[0]
                        .replace("Yes, I trust this folder", "Yes, I trust this other folder")
                }
                1 => frames[0].push_str("\n❯ 2. Exit"),
                _ => frames[0] = frames[0].replace("Accessing workspace:", "Foreign workspace:"),
            },
            false,
        );
        f.start();
        f.hook("SessionStart", json!({"source":"startup"}));
        fs::write(f.home.join("show"), b"").unwrap();
        std::thread::sleep(Duration::from_millis(6200));
        assert!(keys(&f).is_empty());
        assert!(
            f.rpc(ClaudeOperation::Poll {
                session_id: SESSION.into()
            })
            .messages
            .is_empty()
        );
        f.stop();
    }
}
#[test]
fn startup_operator_owner_blocks_keys_and_busy_hook_ends_startup() {
    let mut f = producer(100, |_| {}, false);
    f.start();
    let control = owner(&f, 100);
    fs::write(f.home.join("show"), b"").unwrap();
    std::thread::sleep(Duration::from_millis(2400));
    assert!(
        keys(&f).is_empty(),
        "daemon must not steal operator control"
    );
    f.hook(
        "UserPromptSubmit",
        json!({"prompt":"operator begins working"}),
    );
    drop(control);
    std::thread::sleep(Duration::from_millis(1600));
    assert!(keys(&f).is_empty(), "working is not startup");
    f.stop();
}
#[test]
fn startup_lost_native_key_completion_never_replays_across_four_daemon_boots() {
    let mut f = producer(100, |_| {}, true);
    f.start();
    f.stop();
    let proxy = claude_control_loss::LostKeyReply::start(&f.home);
    let mut launch = None;
    for boot in 1..=4 {
        f.start();
        if boot == 1 {
            fs::write(f.home.join("show"), b"").unwrap();
            wait_keys(&f, b"\x1b[B");
            let end = Instant::now() + Duration::from_secs(5);
            while proxy.dropped.load(Ordering::SeqCst) == 0 {
                assert!(Instant::now() < end);
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        std::thread::sleep(Duration::from_millis(2300));
        assert_eq!(keys(&f), b"\x1b[B");
        f.stop();
        let store = f.store();
        let state = block_on(store.claude_startup("claude")).unwrap().unwrap();
        if let Some(previous) = &launch {
            assert_eq!(previous, &state.launch)
        } else {
            launch = Some(state.launch.clone());
        }
        assert!(!state.halted);
        drop(store);
    }
    assert_eq!(proxy.dropped.load(Ordering::SeqCst), 1);
    drop(proxy);
}

// Fresh verifier counterexamples: keep the actual producer/PTY/daemon path.
#[test]
fn verifier_ready_with_extra_unknown_content_never_becomes_initial_idle() {
    let mut f = producer(
        100,
        |frames| frames[3].push_str("\n❯ Unknown startup confirmation"),
        false,
    );
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    fs::write(f.home.join("show"), b"").unwrap();
    wait_keys(&f, b"\x1b[B\r\r");
    std::thread::sleep(Duration::from_millis(5500));
    assert!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .is_empty(),
        "unknown content cannot authorize initial idle"
    );
    f.stop();
}

#[test]
fn verifier_ready_after_post_five_second_unknown_requires_new_stability_window() {
    let mut f = producer(100, |_| {}, false);
    let path = f.home.join("startup.py");
    let script = fs::read_to_string(&path).unwrap();
    let script = script.replace(
        "while True:\n    count=",
        r#"import threading
def transition():
    while index != 3: time.sleep(0.02)
    time.sleep(5.8)
    sys.stdout.write('\x1b[2J\x1b[HUNKNOWN STARTUP CONFIRMATION\r\n');sys.stdout.flush()
    time.sleep(1.0)
    show()
    open(home+'/ready-restored','w').write('ready')
threading.Thread(target=transition,daemon=True).start()
while True:
    count="#,
    );
    fs::write(path, script).unwrap();
    f.start();
    f.hook("SessionStart", json!({"source":"startup"}));
    fs::write(f.home.join("show"), b"").unwrap();
    wait_keys(&f, b"\x1b[B\r\r");
    let until = Instant::now() + Duration::from_secs(12);
    while !f.home.join("ready-restored").exists() {
        assert!(
            Instant::now() < until,
            "native producer did not restore ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    let reply = f.rpc(ClaudeOperation::Poll {
        session_id: SESSION.into(),
    });
    assert!(
        reply.messages.is_empty(),
        "restored Ready has been stable only 200ms; native delivery was reserved: {:?}",
        reply.messages
    );
    std::thread::sleep(Duration::from_millis(5300));
    assert_eq!(
        f.rpc(ClaudeOperation::Poll {
            session_id: SESSION.into()
        })
        .messages
        .len(),
        1,
        "restored known Ready eventually satisfies a fresh stability window"
    );
    f.stop();
}
