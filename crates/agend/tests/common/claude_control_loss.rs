//! Lose the real holder's Esc completion after the real PTY consumes it.
use super::*;
use agend_core::protocol::{
    holder::{HolderRequest, HolderResponse},
    terminal::TerminalControlOperation,
};
use std::{
    collections::BTreeSet,
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

struct LostKeyReply {
    socket: PathBuf,
    upstream: PathBuf,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<UnixStream>>>,
    dropped: Arc<AtomicUsize>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl LostKeyReply {
    fn start(home: &Path) -> Self {
        let socket = agend_daemon::runtime::files::socket_path(home, "claude");
        let upstream = socket.with_extension("native");
        fs::rename(&socket, &upstream).unwrap();
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicUsize::new(0));
        let stopped = stop.clone();
        let active = sockets.clone();
        let target = upstream.clone();
        let losses = dropped.clone();
        let thread = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let client = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(e) => panic!("holder proxy accept: {e}"),
                };
                client.set_nonblocking(false).unwrap();
                let holder = UnixStream::connect(&target).unwrap();
                active
                    .lock()
                    .unwrap()
                    .extend([client.try_clone().unwrap(), holder.try_clone().unwrap()]);
                let lost = losses.clone();
                connections.push(std::thread::spawn(move || {
                    let ids = Arc::new(Mutex::new(BTreeSet::new()));
                    let sent_ids = ids.clone();
                    let mut from_client = BufReader::new(client.try_clone().unwrap());
                    let mut to_holder = holder.try_clone().unwrap();
                    let outgoing = std::thread::spawn(move || {
                        loop {
                            let mut line = String::new();
                            match from_client.read_line(&mut line) {
                                Ok(0) => break,
                                Err(_) => break,
                                _ => (),
                            }
                            if let Ok(HolderRequest::TerminalControl { data }) =
                                serde_json::from_str(&line)
                                && matches!(
                                    data.operation,
                                    TerminalControlOperation::DaemonKey { .. }
                                )
                            {
                                sent_ids.lock().unwrap().insert(data.request_id);
                            }
                            if to_holder.write_all(line.as_bytes()).is_err() {
                                break;
                            }
                        }
                        let _ = to_holder.shutdown(Shutdown::Both);
                    });
                    let mut incoming = BufReader::new(holder);
                    let mut client = client;
                    loop {
                        let mut line = String::new();
                        match incoming.read_line(&mut line) {
                            Ok(0) => break,
                            Err(_) => break,
                            _ => (),
                        }
                        if let Ok(HolderResponse::TerminalControl { data }) =
                            serde_json::from_str(&line)
                            && ids.lock().unwrap().contains(&data.request_id)
                        {
                            lost.fetch_add(1, Ordering::SeqCst);
                            continue;
                        }
                        if client.write_all(line.as_bytes()).is_err() {
                            break;
                        }
                    }
                    let _ = client.shutdown(Shutdown::Both);
                    outgoing.join().unwrap();
                }));
            }
            for stream in active.lock().unwrap().iter() {
                let _ = stream.shutdown(Shutdown::Both);
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            socket,
            upstream,
            stop,
            sockets,
            dropped,
            thread: Some(thread),
        }
    }
}
impl Drop for LostKeyReply {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for stream in self.sockets.lock().unwrap().iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
        fs::remove_file(&self.socket).unwrap();
        fs::rename(&self.upstream, &self.socket).unwrap();
    }
}

#[test]
fn a_lost_native_esc_reply_keeps_unknown_and_never_replays_key_or_content_across_boots() {
    let mut f = Fixture::with_script(
        0,
        None,
        "/bin/bash",
        r#"stty raw min 1 time 0 -echo; printf 'native ready\r\n'; while IFS= read -r -n 1 byte; do printf '%d\n' "'$byte" >> "$AGEND_HOME/keys.log"; done"#,
    );
    let message = id();
    block_on(f.store().claim_message(
        &NewMessage {
            id: message.clone(),
            from_instance: "operator".into(),
            to_instance: "claude".into(),
            task_id: None,
            body: "Native lost Esc completion".into(),
            level: BusyLevel::Interrupt,
        },
        0,
    ))
    .unwrap();
    f.start();
    f.stop();
    let proxy = LostKeyReply::start(&f.home);
    let mut delivery = None;
    for boot in 1..=4 {
        f.start();
        f.hook("UserPromptSubmit", json!({"prompt":"native busy turn"}));
        let mut channel = Channel::new(&f.home);
        if boot == 1 {
            let until = Instant::now() + Duration::from_secs(8);
            while proxy.dropped.load(Ordering::SeqCst) == 0 {
                assert!(
                    Instant::now() < until,
                    "no real holder key completion intercepted"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            channel
                .output
                .recv_timeout(Duration::from_millis(2500))
                .is_err(),
            "unknown key completion sent content"
        );
        channel.close();
        if boot == 1 {
            f.kill9();
        } else {
            f.stop();
        }
        assert_eq!(
            proxy.dropped.load(Ordering::SeqCst),
            1,
            "key replayed at boot {boot}"
        );
        assert_eq!(fs::read_to_string(f.home.join("keys.log")).unwrap(), "27\n");
        let store = f.store();
        let row = block_on(store.message(&message)).unwrap().unwrap();
        let metadata = block_on(store.claude_delivery(&message)).unwrap().unwrap();
        assert!(metadata.outcome_unknown(&row));
        let attempt = metadata.attempt.unwrap();
        if let Some(expected) = &delivery {
            assert_eq!(&attempt.delivery_id, expected);
        }
        delivery = Some(attempt.delivery_id);
        assert!(attempt.sent_at_unix_ms.is_none());
        assert!(attempt.confirmed_at_unix_ms.is_none());
    }
    drop(proxy);
}
