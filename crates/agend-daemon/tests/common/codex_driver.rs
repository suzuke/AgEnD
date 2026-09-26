//! The codex driver against the fake app-server (gate 7 P5–P8), shared by
//! `tests/codex_driver.rs` and `examples/codex_demo.rs` (`#[path]`), so the
//! demo prints what the tests check. Needs the gate 6 lab module as
//! `crate::lab` (homes under `/tmp/g7-<pid>-<n>`).
//!
//! - [`Backend`]: one codex instance in the DB of a home, and the fake
//!   app-server (`agend_testkit::fake_agent::codex::Server`) listening on its
//!   socket, in this process: a real unix socket, real WebSocket. It is the
//!   backend, so it outlives every driver (daemon boots).
//! - [`Fixture`]: the DRV contract fixture (a boot = a new `SqliteStore` and
//!   `CodexDriver` over the same home and socket).
//! - Sections for the demo and the tests; each returns the lines to print.
//!
//! Safety: no process is signalled here; boot children are this binary,
//! waited with a deadline and killed (`Child::kill`) only if they overrun.
//!
//! Must NOT: run the real `codex`.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agend_core::model::{Backend as Kind, DeliveryState};
use agend_core::policy::busy::BusyLevel;
use agend_core::traits::{AgentMessage, Driver, DriverEvent, DriverEventKind};
use agend_daemon::delivery::render;
use agend_daemon::driver::codex::{CodexDriver, CodexSink, DriverError, launch};
use agend_daemon::runtime::files;
use agend_daemon::store::messages::state_text;
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::block_on;
use agend_testkit::contract::driver::DriverFixture;
use agend_testkit::fake_agent::codex::{Probe, Server};
use serde_json::{Value, json};

fn ensure(ok: bool, what: impl FnOnce() -> String) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what()) }
}

fn quiet_sink() -> CodexSink {
    Arc::new(|_| {})
}

/// Adds a codex instance `id` to `home`'s DB (no daemon may run), with its
/// working directory `home/workspace/<id>`.
pub fn add_codex(home: &Path, id: &str, session: Option<&str>) -> Result<Instance, String> {
    fs::create_dir_all(files::holders_dir(home)).map_err(|e| e.to_string())?;
    let workdir = home.join("workspace").join(id);
    fs::create_dir_all(&workdir).map_err(|e| e.to_string())?;
    let instance = Instance {
        id: id.into(),
        backend: Kind::Codex,
        program: "fake-codex".into(),
        args: Vec::new(),
        working_directory: workdir.display().to_string(),
        session_id: session.map(str::to_owned),
        status: InstanceStatus::Running,
        session_started: true,
        agent_pid: None,
        legacy_no_thread: false,
    };
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.add_instance(&instance)).map_err(|e| format!("add {id}: {e}"))?;
    Ok(instance)
}

/// The thread of `id` from the DB (no daemon may run).
pub fn thread_of(home: &Path, id: &str) -> Result<String, String> {
    let store = SqliteStore::open(home, 0).map_err(|e| format!("open store: {e}"))?;
    block_on(store.instance(id))
        .map_err(|e| e.to_string())?
        .and_then(|i| i.session_id)
        .ok_or_else(|| format!("{id} has no thread"))
}

/// A message as the contract builds them.
pub fn message(id: &str, body: &str) -> AgentMessage {
    AgentMessage {
        id: id.into(),
        from: "operator".into(),
        task_id: Some("T-g7".into()),
        body: body.into(),
    }
}

/// One codex instance and its fake app-server.
pub struct Backend {
    pub home: PathBuf,
    pub id: String,
    pub listen: PathBuf,
    server: Server,
}

impl Backend {
    /// `turn`: how long a fake turn runs.
    pub fn new(home: &Path, id: &str, turn: Duration) -> Result<Arc<Backend>, String> {
        add_codex(home, id, None)?;
        let listen = launch::socket_path(home, id);
        let server =
            Server::bind(&listen, turn, None).map_err(|e| format!("fake app-server: {e}"))?;
        Ok(Arc::new(Backend {
            home: home.to_path_buf(),
            id: id.into(),
            listen,
            server,
        }))
    }

    /// Another home whose instance `id` reaches this same app-server (the
    /// negative check: a new `AGEND_HOME` each boot).
    pub fn other_home(&self, home: &Path) -> Result<(), String> {
        add_codex(home, &self.id, None)?;
        std::os::unix::fs::symlink(
            self.server.bound_path(),
            launch::socket_path(home, &self.id),
        )
        .map_err(|e| e.to_string())
    }

    pub fn probe(&self) -> Result<Probe, String> {
        let mut probe = Probe::connect(&self.listen)?;
        probe.call(
            "initialize",
            json!({"clientInfo": {"name": "g7-test", "title": null, "version": "0"}}),
        )?;
        Ok(probe)
    }

    /// Every turn of `thread`, through a separate client.
    pub fn turns(&self, thread: &str) -> Result<Vec<Value>, String> {
        let mut probe = self.probe()?;
        let page = probe.call(
            "thread/turns/list",
            json!({"threadId": thread, "cursor": null, "limit": 1000}),
        )?;
        Ok(page["data"].as_array().cloned().unwrap_or_default())
    }

    /// Waits until `thread` has `completed` ended turns and none running.
    pub fn wait_turns(&self, thread: &str, completed: usize) -> Result<Vec<Value>, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let turns = self.turns(thread)?;
            let running = turns.iter().any(|t| t["status"] == "inProgress");
            if !running && turns.len() >= completed {
                return Ok(turns);
            }
            if Instant::now() > deadline {
                return Err(format!(
                    "{thread}: {} turns, running={running}",
                    turns.len()
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The agent finishes a turn on its own (as if a human typed in the
    /// TUI), with no driver connected.
    pub fn agent_turn(&self, thread: &str, text: &str) -> Result<(), String> {
        let before = self.turns(thread)?.len();
        let mut probe = self.probe()?;
        probe.call(
            "thread/resume",
            json!({"threadId": thread, "excludeTurns": true}),
        )?;
        probe.call(
            "turn/start",
            json!({"threadId": thread, "input": [{"type": "text", "text": text, "text_elements": []}]}),
        )?;
        self.wait_turns(thread, before + 1).map(|_| ())
    }
}

/// A daemon boot over a backend: a new store and driver, connected.
pub struct Fixture {
    pub backend: Arc<Backend>,
    pub store: Arc<SqliteStore>,
    pub driver: CodexDriver,
    /// What `connect` logged.
    pub connected: Vec<String>,
}

impl Fixture {
    pub fn boot(backend: &Arc<Backend>) -> Result<Fixture, String> {
        let store =
            Arc::new(SqliteStore::open(&backend.home, 0).map_err(|e| format!("open store: {e}"))?);
        let driver = CodexDriver::new(&backend.home, Arc::clone(&store), quiet_sink());
        let connected = block_on(driver.connect(&backend.id, 1))?.unwrap_or_default();
        Ok(Fixture {
            backend: Arc::clone(backend),
            store,
            driver,
            connected,
        })
    }

    pub fn deliver(&self, id: &str, body: &str, level: BusyLevel) -> Result<DeliveryState, String> {
        block_on(
            self.driver
                .deliver(&self.backend.id, &message(id, body), level),
        )
        .map(|r| r.state)
        .map_err(|e| e.to_string())
    }

    pub fn events(&self, after: Option<&str>) -> Result<Vec<DriverEvent>, String> {
        block_on(self.driver.events(&self.backend.id, after)).map_err(|e| e.to_string())
    }

    pub fn state(&self, id: &str) -> Result<String, String> {
        let row = block_on(self.store.message(id)).map_err(|e| e.to_string())?;
        Ok(row.map_or("absent".into(), |r| state_text(r.state).into()))
    }

    pub fn thread(&self) -> Result<String, String> {
        block_on(self.store.instance(&self.backend.id))
            .map_err(|e| e.to_string())?
            .and_then(|i| i.session_id)
            .ok_or_else(|| "no thread".into())
    }

    /// Reads events until the thread is idle with at least `turns` ended
    /// turns; returns them.
    pub fn settle(&self, turns: usize) -> Result<Vec<DriverEvent>, String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let all = self.events(None)?;
            let done = completed(&all);
            let idle = matches!(
                all.last().map(|e| &e.kind),
                Some(DriverEventKind::BusyChanged { busy: false })
            );
            if idle && done >= turns {
                return Ok(all);
            }
            if Instant::now() > deadline {
                return Err(format!("not settled at {turns} turns: {all:?}"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl DriverFixture for Fixture {
    type Driver = CodexDriver;
    type Error = DriverError;
    type Persisted = Arc<Backend>;

    fn driver(&self) -> &CodexDriver {
        &self.driver
    }

    fn instance_id(&self) -> &str {
        &self.backend.id
    }

    fn turn_timeout(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn persisted(&self) -> Arc<Backend> {
        Arc::clone(&self.backend)
    }

    fn boot(persisted: &Arc<Backend>) -> Self {
        Fixture::boot(persisted).expect("boot")
    }

    fn emit_while_down(persisted: &Arc<Backend>) {
        let thread = thread_of(&persisted.home, &persisted.id).expect("thread");
        persisted
            .agent_turn(&thread, "typed in the TUI while the daemon was down")
            .expect("agent turn");
    }
}

pub fn completed(events: &[DriverEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e.kind, DriverEventKind::TurnCompleted { .. }))
        .count()
}

/// The turn a message was confirmed in, from the events.
pub fn turn_of(events: &[DriverEvent], id: &str) -> Option<String> {
    events.iter().find_map(|e| match &e.kind {
        DriverEventKind::MessageConfirmed { message_id } if message_id == id => {
            e.cursor.split_once(':').map(|(t, _)| t.to_owned())
        }
        _ => None,
    })
}

/// How the turn ended (`completed`, `interrupted`), from the events.
pub fn status_of(events: &[DriverEvent], turn: &str) -> Option<String> {
    events.iter().find_map(|e| match &e.kind {
        DriverEventKind::TurnCompleted { summary } if e.cursor.starts_with(&format!("{turn}:")) => {
            summary.clone()
        }
        _ => None,
    })
}

fn short(id: &str) -> &str {
    id.get(id.len().saturating_sub(4)..).unwrap_or(id)
}

// ---- sections ----

/// `== busy` (P6): one message to the idle agent, then, three times, a long
/// turn A and one message at each level. Queue: a new turn after A; steer:
/// inside A; interrupt: A `interrupted`, the message in a new turn.
pub fn busy(lab: &crate::lab::Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(20);
    let id = format!("g7-{tag}b");
    let backend = Backend::new(&home, &id, Duration::from_millis(1500))?;
    let fx = Fixture::boot(&backend)?;
    let mut out = Vec::new();
    ensure(
        fx.deliver("m-idle", "hello", BusyLevel::Queue)? == DeliveryState::Sent,
        || "idle delivery not sent".into(),
    )?;
    let all = fx.settle(1)?;
    let idle_turn = turn_of(&all, "m-idle").ok_or("m-idle not confirmed")?;
    out.push(format!(
        "m-idle (queue, agent idle): turn/start → {} (turn …{})",
        fx.state("m-idle")?,
        short(&idle_turn)
    ));
    let mut turns = 1;
    for (level, name, msg) in [
        (BusyLevel::Queue, "queue", "m-q"),
        (BusyLevel::Steer, "steer", "m-s"),
        (BusyLevel::Interrupt, "interrupt", "m-i"),
    ] {
        let long = format!("m-long-{name}");
        fx.deliver(&long, "a long task", BusyLevel::Queue)?;
        let a = fx.events(None)?;
        let a = a
            .iter()
            .rev()
            .find(|e| matches!(e.kind, DriverEventKind::BusyChanged { busy: true }))
            .and_then(|e| e.cursor.split_once(':').map(|(t, _)| t.to_owned()))
            .ok_or("no running turn A")?;
        ensure(fx.driver.busy(&id) == Some(true), || {
            "the driver sees A idle".into()
        })?;
        let state = fx.deliver(msg, &format!("{name} this"), level)?;
        ensure(state == DeliveryState::Sent, || format!("{msg}: {state:?}"))?;
        turns += if level == BusyLevel::Steer { 1 } else { 2 };
        let all = fx.settle(turns)?;
        let t = turn_of(&all, msg).ok_or_else(|| format!("{msg} not confirmed: {all:?}"))?;
        let a_status = status_of(&all, &a).unwrap_or_default();
        let (method, where_) = match level {
            BusyLevel::Queue => {
                ensure(t != a && a_status == "completed", || {
                    format!("{msg} in turn {t}, A {a} {a_status}")
                })?;
                ("thread/queue/add", "a new turn after A")
            }
            BusyLevel::Steer => {
                ensure(t == a && a_status == "completed", || {
                    format!("{msg} in turn {t}, A {a} {a_status}")
                })?;
                ("turn/steer", "inside A")
            }
            BusyLevel::Interrupt => {
                ensure(t != a && a_status == "interrupted", || {
                    format!("{msg} in turn {t}, A {a} {a_status}")
                })?;
                ("turn/interrupt, turn/start", "a new turn")
            }
        };
        out.push(format!(
            "{msg} ({name}, agent busy in A …{}): {method} → sent → {} (turn …{}: {where_}; A {a_status})",
            short(&a),
            fx.state(msg)?,
            short(&t)
        ));
    }
    out.push(format!(
        "m-long-interrupt stays {}: A was interrupted before its user message (P5: never confirmed = stays sent)",
        fx.state("m-long-interrupt")?
    ));
    for msg in ["m-idle", "m-q", "m-s", "m-i"] {
        ensure(fx.state(msg)? == "confirmed", || {
            format!("{msg} not confirmed")
        })?;
    }
    out.push("all four: queued → sent → confirmed".into());
    Ok(out)
}

/// `== idempotent` (P5, DRV-9): the same id again, before and after a
/// restart, sends nothing; the same id with other content is refused; two
/// deliveries of one id at once insert it once.
pub fn idempotent(lab: &crate::lab::Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(21);
    let id = format!("g7-{tag}i");
    let backend = Backend::new(&home, &id, Duration::from_millis(100))?;
    let mut out = Vec::new();
    {
        let fx = Fixture::boot(&backend)?;
        fx.deliver("m-7", "do it once", BusyLevel::Queue)?;
        fx.settle(1)?;
        let again = fx.deliver("m-7", "do it once", BusyLevel::Queue)?;
        std::thread::sleep(Duration::from_secs(1));
        let turns = completed(&fx.settle(1)?);
        ensure(turns == 1, || format!("{turns} turns after m-7 twice"))?;
        out.push(format!(
            "m-7 again: {} (already confirmed; not sent again); turns: {turns}",
            state_text(again)
        ));
        let other = fx.deliver("m-7", "something else", BusyLevel::Queue);
        let refused = other.expect_err("other content must be refused");
        ensure(refused.starts_with("invalid_request"), || refused.clone())?;
        out.push(format!("m-7 with other content: {refused}"));
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| fx.deliver("m-8", "at once", BusyLevel::Queue));
            let b = s.spawn(|| fx.deliver("m-8", "at once", BusyLevel::Queue));
            (a.join().unwrap(), b.join().unwrap())
        });
        a?;
        b?;
        let rows = block_on(fx.store.messages_to(&id)).map_err(|e| e.to_string())?;
        let m8 = rows.iter().filter(|r| r.id == "m-8").count();
        ensure(m8 == 1, || format!("m-8 stored {m8} times"))?;
        let turns = completed(&fx.settle(2)?);
        ensure(turns == 2, || {
            format!("{turns} turns after m-8 twice at once")
        })?;
        out.push(format!(
            "m-8 twice at the same moment: stored once; turns: {turns}"
        ));
    }
    // A daemon restart: a new store and driver.
    let fx = Fixture::boot(&backend)?;
    let again = fx.deliver("m-7", "do it once", BusyLevel::Queue)?;
    std::thread::sleep(Duration::from_secs(1));
    let turns = completed(&fx.settle(2)?);
    ensure(turns == 2, || {
        format!("{turns} turns after m-7 after a restart")
    })?;
    out.push(format!(
        "after a daemon restart, m-7 again: {} (not sent again); fake app-server turns: {turns}",
        state_text(again)
    ));
    Ok(out)
}

/// `== crash-window` (P5): the send happened, `sent` was never written (a
/// crash). Reconnecting finds one message in the thread history (`sent`,
/// then `confirmed`) and one in the thread queue (`sent`); neither is sent
/// again.
pub fn crash_window(lab: &crate::lab::Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(22);
    let id = format!("g7-{tag}c");
    let backend = Backend::new(&home, &id, Duration::from_millis(1500))?;
    let thread = {
        let fx = Fixture::boot(&backend)?;
        fx.thread()?
    };
    // What a daemon killed right after its RPCs left behind: the rows are
    // `queued`, the RPCs reached codex.
    let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
    for (m, body) in [
        ("m-7", "sent before the crash"),
        ("m-8", "queued before the crash"),
    ] {
        let new = agend_daemon::store::NewMessage {
            id: m.into(),
            from_instance: "operator".into(),
            to_instance: id.clone(),
            task_id: Some("T-g7".into()),
            body: body.into(),
            level: BusyLevel::Queue,
        };
        block_on(store.claim_message(&new, 1)).map_err(|e| e.to_string())?;
        block_on(store.mark_message_attempted(m, 2)).map_err(|e| e.to_string())?;
    }
    drop(store);
    let text = |body: &str| render("operator", Some("T-g7"), body);
    let input = |body: &str| json!([{"type": "text", "text": text(body), "text_elements": []}]);
    let mut probe = backend.probe()?;
    probe.call(
        "thread/resume",
        json!({"threadId": thread, "excludeTurns": true}),
    )?;
    probe.call(
        "turn/start",
        json!({"threadId": thread, "input": input("sent before the crash"), "clientUserMessageId": "m-7"}),
    )?;
    backend.wait_turns(&thread, 1)?;
    // Someone else's long turn (a human in the TUI), so m-8 waits in the
    // queue.
    probe.call(
        "turn/start",
        json!({"threadId": thread, "input": [{"type": "text", "text": "a human's turn", "text_elements": []}]}),
    )?;
    probe.call(
        "thread/queue/add",
        json!({"threadId": thread, "input": input("queued before the crash"), "clientUserMessageId": "m-8"}),
    )?;
    drop(probe);
    let fx = Fixture::boot(&backend)?;
    let m7 = fx.state("m-7")?;
    let m8 = fx.state("m-8")?;
    ensure(m7 == "confirmed" && m8 == "sent", || {
        format!("m-7 {m7}, m-8 {m8}")
    })?;
    let all = fx.settle(3)?;
    ensure(completed(&all) == 3, || format!("turns: {all:?}"))?;
    let m8 = fx.state("m-8")?;
    Ok(vec![
        "rows m-7, m-8 left queued; codex got m-7 (turn/start) and m-8 (thread/queue/add)".into(),
        format!(
            "reconnect: m-7 found in thread history → sent → {m7}; m-8 found in thread queue → sent"
        ),
        format!(
            "after the queue ran: m-8 {m8}; fake app-server turns: {} (m-7, a human's, m-8: nothing sent twice)",
            completed(&all)
        ),
    ])
}

/// `== reply-lost` (P5): `turn/start` reached codex but its reply never came
/// back (a timeout, a broken link, a stop mid-send): the row is `queued` with
/// `attempted_at` set. On the next connect its turn is running and its user
/// message is not in the history yet; the driver waits for the thread to be
/// idle, finds it, and confirms it: one user message, one turn.
pub fn reply_lost(lab: &crate::lab::Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(24);
    let id = format!("g7-{tag}l");
    let backend = Backend::new(&home, &id, Duration::from_millis(1500))?;
    let thread = Fixture::boot(&backend)?.thread()?;
    let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
    let new = agend_daemon::store::NewMessage {
        id: "m-9".into(),
        from_instance: "operator".into(),
        to_instance: id.clone(),
        task_id: Some("T-g7".into()),
        body: "the reply got lost".into(),
        level: BusyLevel::Queue,
    };
    block_on(store.claim_message(&new, 1)).map_err(|e| e.to_string())?;
    block_on(store.mark_message_attempted("m-9", 2)).map_err(|e| e.to_string())?;
    drop(store);
    let text = render("operator", Some("T-g7"), "the reply got lost");
    let mut probe = backend.probe()?;
    probe.call(
        "thread/resume",
        json!({"threadId": thread, "excludeTurns": true}),
    )?;
    probe.call(
        "turn/start",
        json!({"threadId": thread, "input": [{"type": "text", "text": text, "text_elements": []}],
               "clientUserMessageId": "m-9"}),
    )?;
    drop(probe);
    let fx = Fixture::boot(&backend)?;
    let right_after = fx.state("m-9")?;
    let all = fx.settle(1)?;
    let turns = backend.turns(&thread)?;
    let users: usize = turns
        .iter()
        .flat_map(|t| t["items"].as_array().cloned().unwrap_or_default())
        .filter(|i| i["type"] == "userMessage")
        .count();
    let state = fx.state("m-9")?;
    ensure(
        completed(&all) == 1 && users == 1 && state == "confirmed",
        || {
            format!(
                "turns {}, user messages {users}, m-9 {state}",
                completed(&all)
            )
        },
    )?;
    Ok(vec![
        "m-9: queued, attempted; codex got it (turn/start) but the reply was lost".into(),
        format!(
            "reconnect while its turn runs (no user message yet): m-9 still {right_after}, not sent again"
        ),
        format!("the turn ended: m-9 {state}; turns: 1, user messages: {users} (no duplicate)"),
    ])
}

/// `== approval` (P4): codex asks to run a command; the driver answers
/// `decline` and the turn ends normally.
pub fn approval(lab: &crate::lab::Lab, tag: &str) -> Result<Vec<String>, String> {
    let home = lab.home(23);
    let id = format!("g7-{tag}a");
    let backend = Backend::new(&home, &id, Duration::from_millis(100))?;
    let fx = Fixture::boot(&backend)?;
    fx.deliver("m-run", "run: echo hi", BusyLevel::Queue)?;
    fx.settle(1)?;
    let turns = backend.turns(&fx.thread()?)?;
    let items = turns[0]["items"].as_array().cloned().unwrap_or_default();
    let command = items
        .iter()
        .find(|i| i["type"] == "commandExecution")
        .ok_or("no command item")?;
    let reply = items
        .iter()
        .rev()
        .find(|i| i["type"] == "agentMessage")
        .and_then(|i| i["text"].as_str())
        .unwrap_or_default();
    ensure(command["status"] == "declined", || format!("{command}"))?;
    ensure(reply.contains("decline"), || reply.to_owned())?;
    Ok(vec![format!(
        "m-run asked to run {}: declined by the driver; the turn {} (agent: {reply:?}); m-run {}",
        command["command"],
        turns[0]["status"].as_str().unwrap_or_default(),
        fx.state("m-run")?
    )])
}

// ---- four boots in four processes (DRV-6, DRV-9) ----

pub const BOOT_ENV: &str = "G7_DRIVER_BOOT";

fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(&format!("{key}=")))
}

/// One boot (in a child process): `AGEND_HOME`, `G7_ID`, `G7_CURSOR` (the
/// previous boot's last cursor), `G7_OLD` (an older one), `G7_SERVER`
/// (the app-server's pid, for the line).
pub fn boot_main(n: usize) -> Result<String, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let home = PathBuf::from(env("AGEND_HOME").ok_or("AGEND_HOME")?);
    let id = env("G7_ID").ok_or("G7_ID")?;
    let cursor = env("G7_CURSOR");
    let before_mq = {
        let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
        block_on(store.message("m-q"))
            .map_err(|e| e.to_string())?
            .map(|r| state_text(r.state).to_owned())
    };
    let store = Arc::new(SqliteStore::open(&home, 0).map_err(|e| format!("open store: {e}"))?);
    let driver = CodexDriver::new(&home, Arc::clone(&store), quiet_sink());
    let backend_id = id.clone();
    let connected = block_on(driver.connect(&id, 1))?.unwrap_or_default();
    let events = |after: Option<&str>| {
        block_on(driver.events(&backend_id, after)).map_err(|e| e.to_string())
    };
    let deliver = |m: &str, body: &str, level| {
        block_on(driver.deliver(&backend_id, &message(m, body), level))
            .map(|r| r.state)
            .map_err(|e| e.to_string())
    };
    let state = |m: &str| -> Result<String, String> {
        Ok(block_on(store.message(m))
            .map_err(|e| e.to_string())?
            .map_or("absent".into(), |r| state_text(r.state).to_owned()))
    };
    let thread = block_on(store.instance(&id))
        .map_err(|e| e.to_string())?
        .and_then(|i| i.session_id)
        .ok_or("no thread")?;
    let backfill = events(cursor.as_deref())?;
    let all = events(None)?;
    if let Some(old) = env("G7_OLD") {
        let at = all
            .iter()
            .position(|e| e.cursor == old)
            .ok_or_else(|| format!("the older cursor {old} is gone"))?;
        let got = events(Some(&old))?;
        ensure(got == all[at + 1..], || {
            format!("backfill after the older cursor {old}: {got:?}")
        })?;
    }
    let mut notes = Vec::new();
    let redeliver = |ids: &[&str]| -> Result<usize, String> {
        let turns = completed(&events(None)?);
        for m in ids {
            deliver(m, &format!("work {m}"), BusyLevel::Queue)?;
        }
        std::thread::sleep(Duration::from_secs(2));
        Ok(completed(&events(None)?) - turns)
    };
    match n {
        1 => {
            deliver("m-1", "work m-1", BusyLevel::Queue)?;
            let q = deliver("m-q", "work m-q", BusyLevel::Queue)?;
            ensure(driver.busy(&id) == Some(true), || {
                "m-1's turn not running".into()
            })?;
            notes.push(format!(
                "m-1 sent (turn/start, a 5 s turn); m-q {} (thread/queue/add)",
                state_text(q)
            ));
        }
        2 => notes.push("idle: backfill only".into()),
        3 => {
            ensure(completed(&backfill) >= 2, || {
                format!("the turns run while down are not in the backfill: {backfill:?}")
            })?;
            let mq = state("m-q")?;
            ensure(mq == "confirmed", || format!("m-q is {mq}"))?;
            if before_mq.as_deref() == Some("sent") {
                notes.push("m-q confirmed while daemon was down".into());
            } else {
                notes.push(format!("m-q {mq} (was {before_mq:?})"));
            }
            let extra = redeliver(&["m-1", "m-q"])?;
            ensure(extra == 0, || {
                format!("m-1, m-q again made {extra} turn(s)")
            })?;
            notes.push("m-1, m-q again: no new turn".into());
            let turns = completed(&events(None)?);
            deliver("m-3", "work m-3", BusyLevel::Queue)?;
            let deadline = Instant::now() + Duration::from_secs(20);
            while completed(&events(None)?) == turns || state("m-3")? != "confirmed" {
                ensure(Instant::now() < deadline, || "m-3 never confirmed".into())?;
                std::thread::sleep(Duration::from_millis(50));
            }
            notes.push("m-3 confirmed".into());
        }
        _ => {
            ensure(completed(&backfill) >= 1, || {
                format!("the TUI turn while down is not in the backfill: {backfill:?}")
            })?;
            for m in ["m-1", "m-q", "m-3"] {
                let s = state(m)?;
                ensure(s == "confirmed", || format!("{m} is {s}"))?;
            }
            let extra = redeliver(&["m-1", "m-q", "m-3"])?;
            ensure(extra == 0, || format!("resending made {extra} turn(s)"))?;
            notes.push("m-1, m-q, m-3 confirmed; all three again: no new turn; ok".into());
        }
    }
    let last = events(None)?
        .last()
        .map(|e| e.cursor.clone())
        .or(cursor.clone())
        .unwrap_or_default();
    let how = connected.join("; ").replace(&thread, "<T>");
    Ok(format!(
        "boot {n} daemon pid={} app-server pid={} thread={thread} backfilled={} from={} cursor={last} | {how} | {}",
        std::process::id(),
        env("G7_SERVER").unwrap_or_default(),
        backfill.len(),
        cursor.as_deref().unwrap_or("-"),
        notes.join("; ")
    ))
}

/// Runs boot `n` as `command` (this binary, re-executed) with `env`;
/// returns its `@` line.
pub fn run_boot(
    mut command: Command,
    n: usize,
    env: &[(String, String)],
) -> Result<String, String> {
    command
        .env(BOOT_ENV, n.to_string())
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("boot {n} ran over 90 s"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    let mut err = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut out).unwrap_or(0);
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut err).unwrap_or(0);
    let line = out
        .lines()
        .find_map(|l| l.strip_prefix('@'))
        .map(str::to_owned);
    match (status.success(), line) {
        (true, Some(line)) => Ok(line),
        _ => Err(format!("boot {n} failed: {status}\n{out}\n{err}")),
    }
}

/// `== restart`: four daemon boots in four processes (work, idle, work,
/// check) against one fake app-server (DRV-6, DRV-9). `home_of(n)` is the
/// home of boot `n` (the same one, or, for the negative check, a new one
/// each boot, which must fail at boot 2); `command()` re-executes this
/// binary as a boot.
pub fn restart(
    lab: &crate::lab::Lab,
    tag: &str,
    new_home_each_boot: bool,
    command: &dyn Fn() -> Command,
) -> Result<Vec<String>, String> {
    let base = if new_home_each_boot { 40 } else { 30 };
    let id = format!("g7-{tag}r");
    let first = lab.home(base);
    let backend = Backend::new(&first, &id, Duration::from_millis(5000))?;
    let home_of = |n: usize| -> Result<PathBuf, String> {
        if n == 1 || !new_home_each_boot {
            return Ok(first.clone());
        }
        let home = lab.home(base + n);
        backend.other_home(&home)?;
        Ok(home)
    };
    let mut lines: Vec<String> = Vec::new();
    let mut cursors: Vec<String> = Vec::new();
    for n in 1..=4 {
        let home = home_of(n)?;
        let mut env = vec![
            ("AGEND_HOME".to_owned(), home.display().to_string()),
            ("G7_ID".to_owned(), id.clone()),
            ("G7_SERVER".to_owned(), std::process::id().to_string()),
        ];
        if let Some(c) = cursors.last() {
            env.push(("G7_CURSOR".into(), c.clone()));
        }
        if cursors.len() >= 2 {
            env.push(("G7_OLD".into(), cursors[cursors.len() - 2].clone()));
        }
        let line = run_boot(command(), n, &env)?;
        let thread = field(&line, "thread").unwrap_or_default().to_owned();
        if let Some(first_line) = lines.first() {
            let before = field(first_line, "thread").unwrap_or_default();
            ensure(thread == before, || {
                format!("boot {n} failed: thread {thread}, not boot 1's {before}")
            })?;
        }
        cursors.push(field(&line, "cursor").unwrap_or_default().to_owned());
        lines.push(line);
        // While down.
        match n {
            2 => {
                backend.wait_turns(&thread, 2)?;
            }
            3 => backend.agent_turn(&thread, "typed in the TUI while the daemon was down")?,
            _ => {}
        }
    }
    let pids: std::collections::BTreeSet<&str> =
        lines.iter().filter_map(|l| field(l, "pid")).collect();
    ensure(pids.len() == 4, || format!("daemon pids: {pids:?}"))?;
    let thread = field(&lines[0], "thread").unwrap_or_default().to_owned();
    let mut out: Vec<String> = lines.iter().map(|l| l.replace(&thread, "<T>")).collect();
    out.push(format!(
        "4 boots, 4 daemon pids, one app-server (pid {}), one thread <T>={thread}",
        std::process::id()
    ));
    Ok(out)
}
