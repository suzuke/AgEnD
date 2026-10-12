//! Native daemon/holder regression for upgrades of the original executable.
#![cfg(unix)]
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, DAEMON_SOCKET, OperatorCommand, OperatorData,
};
use std::{
    fs,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
const BIN: &str = env!("CARGO_BIN_EXE_agend");

#[test]
fn replacing_original_binary_cannot_redirect_new_holders_or_shims() {
    let mut lab = lab::Lab::with_prefix(Path::new(BIN), "g13-pin");
    let source = lab.root.join("original-agend");
    fs::copy(BIN, &source).unwrap();
    lab.agend = source.clone();
    let home = lab.home(1);
    let mut daemon = lab::Daemon::start(&lab, &home, &[]).unwrap();
    daemon.ready().unwrap();
    let pinned = fs::read_link(home.join("bin/git")).unwrap();
    assert!(
        pinned.starts_with(home.join("runtime-binaries")),
        "{pinned:?}"
    );
    assert_eq!(fs::read(&pinned).unwrap(), fs::read(BIN).unwrap());
    let replacement = lab.root.join("replacement");
    fs::copy("/usr/bin/false", &replacement).unwrap();
    fs::rename(replacement, &source).unwrap();
    assert!(
        !Command::new(&source)
            .arg("--version")
            .status()
            .unwrap()
            .success()
    );
    let response = agend_client::exchange_once(
        &home.join(DAEMON_SOCKET),
        None,
        agend_core::protocol::client::V1_7,
        &ClientRequest::Operator {
            data: OperatorData {
                request_id: "pinned-holder".into(),
                command: OperatorCommand::InstanceAdd {
                    instance_id: "g13-pin-agent".into(),
                    backend: "claude".into(),
                    working_directory: Some(home.display().to_string()),
                    program: Some("/bin/bash".into()),
                    args: vec![
                        "-c".into(),
                        "printf pinned > started; exec sleep 600".into(),
                        "agent".into(),
                    ],
                },
            },
        },
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        !matches!(response, ClientResponse::Error { .. }),
        "{response:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !home.join("started").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(fs::read_to_string(home.join("started")).unwrap(), "pinned");
    let holders = lab.running_holders();
    assert_eq!(holders.len(), 1);
    let command = Command::new("/bin/ps")
        .args(["-p", &holders[0].2.to_string(), "-o", "command="])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&command.stdout).contains(pinned.to_str().unwrap()));
    daemon.interrupt().unwrap();
    assert_eq!(
        lab.running_holders().len(),
        1,
        "holder must survive daemon exit"
    );
    assert!(
        pinned.is_file(),
        "live holder's executable must be retained"
    );
    lab.stop_all_holders();
    assert!(lab.running_holders().is_empty());
    let root = lab.root.clone();
    drop(daemon);
    drop(lab);
    assert!(
        !root.exists(),
        "owned lab and executable cache must be removed"
    );
}
