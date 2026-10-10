//! Full native daemon/holder/wrapper/REST/ask path, without model execution.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::{
    model::{Backend, DeliveryState},
    policy::busy::BusyLevel,
    protocol::{ask::*, client::*},
};
use agend_daemon::store::{Instance, InstanceStatus, NewMessage, SqliteStore};
use agend_testkit::{block_on, fake_daemon::ProbeClient};
use std::{
    fs,
    net::TcpStream,
    path::Path,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");
fn request(home: &Path, caller: Option<&str>, request: ClientRequest) -> ClientResponse {
    let (mut client, _) = ProbeClient::hello(&home.join(DAEMON_SOCKET), caller).unwrap();
    client.request(&request).unwrap()
}
fn fleet(home: &Path) -> FleetView {
    let ClientResponse::Fleet { data } = request(
        home,
        None,
        ClientRequest::GetFleet {
            data: RequestIdData {
                request_id: "fleet".into(),
            },
        },
    ) else {
        panic!("fleet");
    };
    data.fleet
}
fn wait<T>(home: &Path, mut probe: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(25);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        if Instant::now() >= end {
            for file in fs::read_dir(home.join("logs"))
                .into_iter()
                .flatten()
                .flatten()
            {
                if file.file_name().to_string_lossy().starts_with("daemon-") {
                    eprintln!("{}", fs::read_to_string(file.path()).unwrap_or_default());
                }
            }
            panic!("native OpenCode condition timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
fn daemon_and_holder_restarts_preserve_session_permission_and_one_delivery() {
    run(false);
}
#[test]
fn killed_holder_is_swept_before_resuming_the_original_session() {
    run(true);
}

struct Cleanup<'a> {
    lab: &'a lab::Lab,
    home: std::path::PathBuf,
    pgid: u32,
    program: String,
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        self.lab.stop_all_holders();
        if agend_daemon::runtime::files::running(&self.home, "open")
            .ok()
            .flatten()
            .is_none()
        {
            let layout =
                agend_daemon::driver::opencode::launch::Layout::new(&self.home, "open").unwrap();
            let _ = layout.sweep(self.pgid, &self.program);
        }
    }
}
fn run(crash: bool) {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12open");
    let home = lab.home(0);
    let fake = Path::new(BIN).parent().unwrap().join("fake-opencode-cli");
    assert!(
        fake.is_file(),
        "build agend-testkit --bin fake-opencode-cli first"
    );
    let workspace = home.join("workspace/open");
    fs::create_dir_all(&workspace).unwrap();
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.add_instance(&Instance {
            id: "open".into(),
            backend: Backend::Opencode,
            program: fake.display().to_string(),
            args: vec![],
            working_directory: workspace.display().to_string(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        block_on(
            store.claim_message(
                &NewMessage {
                    id: "native-first".into(),
                    from_instance: "operator".into(),
                    to_instance: "open".into(),
                    task_id: None,
                    body: "run: echo permission".into(),
                    level: BusyLevel::Queue,
                },
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            ),
        )
        .unwrap();
    }
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let ask = wait(&home, || {
        fleet(&home).attention.into_iter().find_map(|a| a.ask)
    });
    let original_go = fs::read_to_string(home.join("opencode/open/go")).unwrap();
    let original_holder = agend_daemon::runtime::files::running(&home, "open")
        .unwrap()
        .unwrap();
    let answer = || ClientRequest::AnswerAsk {
        data: AnswerAskData {
            request_id: "reply".into(),
            ask_id: ask.ask_id.clone(),
            source: AnswerSource::Cli,
            reply: AskReply::Choice {
                option: "Reject".into(),
            },
        },
    };
    let ClientResponse::Error { data } = request(&home, Some("open"), answer()) else {
        panic!("agent replied to permission");
    };
    assert_eq!(data.code, error_code::FORBIDDEN);
    daemon.interrupt().unwrap();
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    wait(&home, || {
        fleet(&home)
            .attention
            .into_iter()
            .find(|a| a.attention_id.as_deref() == Some(&ask.ask_id))
    });
    assert_eq!(
        agend_daemon::runtime::files::running(&home, "open").unwrap(),
        Some(original_holder)
    );
    assert!(matches!(
        request(&home, None, answer()),
        ClientResponse::CommandResult { .. }
    ));
    wait(&home, || fleet(&home).attention.is_empty().then_some(()));
    assert!(matches!(
        request(&home, None, answer()),
        ClientResponse::Error { .. }
    ));
    daemon.interrupt().unwrap();
    let pgid = {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.instance("open"))
            .unwrap()
            .unwrap()
            .agent_pid
            .unwrap()
    };
    let _cleanup = Cleanup {
        lab: &lab,
        home: home.clone(),
        pgid,
        program: fake.display().to_string(),
    };
    if crash {
        assert_eq!(
            agend_daemon::runtime::files::running(&home, "open").unwrap(),
            Some(original_holder)
        );
        assert!(original_holder > 1);
        // SAFETY: this lab's live holder lock identifies the child to kill.
        assert_eq!(
            unsafe { libc::kill(original_holder as i32, libc::SIGKILL) },
            0
        );
    } else {
        agend_daemon::runtime::shutdown_holder(&home, "open").unwrap();
    }
    wait(&home, || {
        agend_daemon::runtime::files::running(&home, "open")
            .unwrap()
            .is_none()
            .then_some(())
    });
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    wait(&home, || {
        fs::read_to_string(home.join("opencode/open/go"))
            .ok()
            .filter(|g| g != &original_go)
    });
    let new_go = fs::read_to_string(home.join("opencode/open/go")).unwrap();
    assert_eq!(original_go.lines().next(), new_go.lines().next());
    wait(&home, || {
        fleet(&home)
            .instances
            .iter()
            .any(|i| i.instance_id == "open" && i.state == AgentState::Idle)
            .then_some(())
    });
    daemon.interrupt().unwrap();
    let store = std::sync::Arc::new(SqliteStore::open(&home, 0).unwrap());
    let row = block_on(store.message("native-first")).unwrap().unwrap();
    assert_eq!(row.state, DeliveryState::Confirmed);
    let instance = block_on(store.instance("open")).unwrap().unwrap();
    assert_eq!(instance.session_id.as_deref(), new_go.lines().next());
    let port = new_go
        .lines()
        .nth(1)
        .unwrap()
        .strip_prefix("http://127.0.0.1:")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let layout = agend_daemon::driver::opencode::launch::Layout::new(&home, "open").unwrap();
    let api = agend_daemon::driver::opencode::api::Session::resume(
        agend_daemon::driver::opencode::http::Http::new(
            port,
            &layout.password().unwrap(),
            &workspace.display().to_string(),
        )
        .unwrap(),
        instance.session_id.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        agend_daemon::driver::opencode::history::users(api.id(), &api.history().unwrap())
            .unwrap()
            .len(),
        1
    );
    let driver = agend_daemon::driver::opencode::OpenCodeDriver::new(store.clone());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(runtime.block_on(driver.session_idle("open")).unwrap());
    let original_session = instance.session_id.as_deref().unwrap();
    block_on(store.set_session_id("open", "ses_foreign")).unwrap();
    assert!(runtime.block_on(driver.session_idle("open")).is_err());
    block_on(store.set_session_id("open", original_session)).unwrap();
    api.submit("idle-probe", "run: echo idle-probe", None)
        .unwrap();
    wait(&home, || api.busy().unwrap().then_some(()));
    assert!(!runtime.block_on(driver.session_idle("open")).unwrap());
    api.abort().unwrap();
    wait(&home, || (!api.busy().unwrap()).then_some(()));
    assert!(runtime.block_on(driver.session_idle("open")).unwrap());
    lab.stop_all_holders();
    wait(&home, || {
        TcpStream::connect(("127.0.0.1", port))
            .is_err()
            .then_some(())
    });
    assert!(runtime.block_on(driver.session_idle("open")).is_err());
    drop(driver);
    drop(store);
}

#[test]
fn unknown_delivery_survives_restart_and_only_operator_can_abandon_it() {
    let lab = lab::Lab::with_prefix(Path::new(BIN), "g12unknown");
    let home = lab.home(0);
    let fake = Path::new(BIN).parent().unwrap().join("fake-opencode-cli");
    let workspace = home.join("workspace/open");
    fs::create_dir_all(&workspace).unwrap();
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.add_instance(&Instance {
            id: "open".into(),
            backend: Backend::Opencode,
            program: fake.display().to_string(),
            args: vec![],
            working_directory: workspace.display().to_string(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
    }
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let go = wait(&home, || {
        fs::read_to_string(home.join("opencode/open/go")).ok()
    });
    let session = go.lines().next().unwrap().to_owned();
    daemon.interrupt().unwrap();
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        block_on(store.claim_message(
            &NewMessage {
                id: "unknown-native".into(),
                from_instance: "operator".into(),
                to_instance: "open".into(),
                task_id: None,
                body: "must never be replayed".into(),
                level: BusyLevel::Queue,
            },
            1,
        ))
        .unwrap();
        assert!(
            block_on(store.begin_opencode_attempt(
                "unknown-native",
                "open",
                &session,
                &agend_daemon::driver::opencode::history::message_id("unknown-native"),
                1
            ))
            .unwrap()
        );
    }
    let attention = "opencode-delivery:unknown-native";
    for boot in 0..2 {
        let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
        daemon.ready().unwrap();
        let item = wait(&home, || {
            fleet(&home)
                .attention
                .into_iter()
                .find(|a| a.attention_id.as_deref() == Some(attention))
        });
        assert_eq!(item.actions, vec![AttentionAction::Abandon]);
        let resolve = || ClientRequest::ResolveAttention {
            data: ResolveAttentionData {
                request_id: "resolve-open".into(),
                attention_id: attention.into(),
                action: AttentionAction::Abandon,
                note: Some("native operator ends unknown delivery".into()),
            },
        };
        assert!(
            matches!(request(&home,Some("open"),resolve()),ClientResponse::Error {data} if data.code==error_code::FORBIDDEN)
        );
        if boot == 1 {
            assert!(matches!(
                request(&home, None, resolve()),
                ClientResponse::CommandResult { .. }
            ));
            wait(&home, || {
                (!fleet(&home)
                    .attention
                    .iter()
                    .any(|a| a.attention_id.as_deref() == Some(attention)))
                .then_some(())
            });
        }
        daemon.interrupt().unwrap();
    }
    {
        let store = SqliteStore::open(&home, 0).unwrap();
        assert_eq!(
            block_on(store.message("unknown-native"))
                .unwrap()
                .unwrap()
                .state,
            DeliveryState::Failed
        );
        assert!(
            block_on(store.confirm_opencode_attempt(
                "unknown-native",
                &session,
                &agend_daemon::driver::opencode::history::message_id("unknown-native"),
                10
            ))
            .unwrap()
            .is_none()
        );
    }
    let layout = agend_daemon::driver::opencode::launch::Layout::new(&home, "open").unwrap();
    let holder = agend_daemon::runtime::files::running(&home, "open")
        .unwrap()
        .unwrap();
    let (port, _) = layout.endpoint(holder).unwrap();
    let http = agend_daemon::driver::opencode::http::Http::new(
        port,
        &layout.password().unwrap(),
        workspace.to_str().unwrap(),
    )
    .unwrap();
    let native = agend_daemon::driver::opencode::api::Session::resume(http, &session).unwrap();
    assert!(
        native.history().unwrap().as_array().unwrap().is_empty(),
        "unknown attempt must never reach backend"
    );
    lab.stop_all_holders();
}
