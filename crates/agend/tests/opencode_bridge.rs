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
    let store = SqliteStore::open(&home, 0).unwrap();
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
    lab.stop_all_holders();
    wait(&home, || {
        TcpStream::connect(("127.0.0.1", port))
            .is_err()
            .then_some(())
    });
    drop(store);
}
