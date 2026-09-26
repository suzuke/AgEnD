//! One long-lived connection per codex instance, on its own std thread
//! (gate 7 P1, like the holder links of gate 6 H7). The thread owns the
//! connection: it is the only one that sends to this instance, so a message
//! is never sent twice by two callers, and it reads every notification.
//!
//! - Handshake ([`Worker::open`], on the caller's thread): `initialize`,
//!   then `thread/start` or `thread/resume {excludeTurns: true}` (without a
//!   resume only coarse status arrives, spike S2).
//! - On (re)connect ([`Worker::catch_up`]; the first time before the
//!   driver's `connect` returns): reconcile, then send every `queued`
//!   message (seq order). Reconcile (P5): a `queued` message found in the thread history
//!   becomes `sent` then `confirmed`, one found in `thread/queue/list`
//!   becomes `sent`; a `sent` one found in the history becomes `confirmed`.
//!   Only what is found nowhere is sent again.
//! - Notifications: `thread/status/changed` sets busy (not debounced, P6);
//!   `turn/started` / `turn/completed` the running turn; `item/completed`
//!   of a user message confirms it. Approval requests are answered
//!   `decline` (P4: gate 7 has no one to ask).
//! - The connection ends: reconnect every 100 ms (with `thread/resume`) for
//!   [`launch::READY_WITHIN`]; then the app-server counts as dead
//!   ([`CodexEvent::Gone`]) and the thread ends.
//!
//! Must NOT: send a message that is not `queued`, or type anything into a
//! PTY.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::model::DeliveryState;
use serde_json::{Value, json};

use super::history::{UserItem, match_items, text_of, user_items};
use super::launch;
use super::rpc::{Conn, Incoming, RpcError};
use super::send::{self, Rpc};
use crate::log;
use crate::store::{Message, SqliteStore, StoreError, messages};

/// What a link reports to the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexEvent {
    /// The app-server of `id` could not be reached for 20 s: a death.
    Gone { id: String, generation: u64 },
}

pub type CodexSink = Arc<dyn Fn(CodexEvent) + Send + Sync>;

/// Page size of `thread/turns/list`.
const TURNS_PAGE: u64 = 100;
/// Longest wait for one RPC answer (other than the handshake's).
const CALL_WITHIN: Duration = Duration::from_secs(30);
const RECONNECT_EVERY: Duration = Duration::from_millis(100);

pub(crate) enum Command {
    Flush(Sender<()>),
    Turns(Sender<Result<Vec<Value>, String>>),
}

/// Shared with the driver.
#[derive(Default)]
pub(crate) struct Shared {
    pub connected: AtomicBool,
    pub busy: AtomicBool,
    stopping: AtomicBool,
}

/// The driver's handle on a link thread.
pub(crate) struct Link {
    commands: Option<Sender<Command>>,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// An answer the link thread will give.
pub(crate) struct Pending<T>(Option<Receiver<T>>);

impl Pending<()> {
    /// Waits at most `within`; false when the link could not do it.
    pub fn wait(self, within: Duration) -> bool {
        self.0.is_some_and(|rx| rx.recv_timeout(within).is_ok())
    }
}

impl Pending<Result<Vec<Value>, String>> {
    pub fn wait(self, within: Duration) -> Result<Vec<Value>, String> {
        let rx = self.0.ok_or("the link has ended")?;
        rx.recv_timeout(within)
            .map_err(|_| "no answer from the link".to_owned())?
    }
}

impl Link {
    /// Asks the link to send every `queued` message.
    pub fn flush_request(&self) -> Pending<()> {
        let (tx, rx) = mpsc::channel();
        let sent = self
            .commands
            .as_ref()
            .is_some_and(|c| c.send(Command::Flush(tx)).is_ok());
        Pending(sent.then_some(rx))
    }

    /// Asks for every turn of the thread (`thread/turns/list`, all pages).
    pub fn turns_request(&self) -> Pending<Result<Vec<Value>, String>> {
        let (tx, rx) = mpsc::channel();
        let sent = self
            .commands
            .as_ref()
            .is_some_and(|c| c.send(Command::Turns(tx)).is_ok());
        Pending(sent.then_some(rx))
    }

    pub fn close(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.commands.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Confirms message `row` (turn `turn`): `queued` → `sent` → `confirmed`,
/// `sent` → `confirmed`. Returns the state it had when it changed.
pub(crate) fn confirm(
    store: &SqliteStore,
    row: &str,
    turn: &str,
) -> Result<Option<DeliveryState>, StoreError> {
    let (row, turn) = (row.to_owned(), turn.to_owned());
    let now = log::now_unix_ms();
    store.call_blocking(move |conn| {
        let Some(found) = messages::get(conn, &row)? else {
            return Ok(None);
        };
        match found.state {
            DeliveryState::Queued => {
                messages::advance(conn, &row, DeliveryState::Sent, Some(&turn), now)?;
            }
            DeliveryState::Sent => {}
            _ => return Ok(None),
        }
        messages::advance(conn, &row, DeliveryState::Confirmed, None, now)?;
        Ok(Some(found.state))
    })
}

pub(crate) fn messages_to(store: &SqliteStore, id: &str) -> Result<Vec<Message>, StoreError> {
    let id = id.to_owned();
    store.call_blocking(move |conn| messages::to_instance(conn, &id))
}

fn advance(
    store: &SqliteStore,
    row: &str,
    next: DeliveryState,
    turn: Option<&str>,
) -> Result<Option<Message>, StoreError> {
    let (row, turn) = (row.to_owned(), turn.map(str::to_owned));
    let now = log::now_unix_ms();
    store.call_blocking(move |conn| messages::advance(conn, &row, next, turn.as_deref(), now))
}

pub(crate) struct Worker {
    pub id: String,
    pub generation: u64,
    pub thread: String,
    listen: PathBuf,
    conn: Conn,
    store: Arc<SqliteStore>,
    busy: bool,
    active: Option<String>,
    shared: Arc<Shared>,
}

impl Worker {
    /// Connects and says `initialize` (the thread is set later).
    pub fn open(
        id: &str,
        generation: u64,
        listen: PathBuf,
        store: Arc<SqliteStore>,
    ) -> Result<Worker, RpcError> {
        let conn = Conn::open(&listen).map_err(|e| RpcError::Transport(e.to_string()))?;
        let mut worker = Worker {
            id: id.to_owned(),
            generation,
            thread: String::new(),
            listen,
            conn,
            store,
            busy: false,
            active: None,
            shared: Arc::new(Shared::default()),
        };
        worker.initialize()?;
        Ok(worker)
    }

    fn initialize(&mut self) -> Result<(), RpcError> {
        self.call_within(
            "initialize",
            json!({"clientInfo": {"name": "agend", "title": null, "version": env!("CARGO_PKG_VERSION")},
                   "capabilities": {"experimentalApi": true}}),
            Duration::from_secs(10),
        )?;
        self.conn.send(&json!({"method": "initialized"}))
    }

    /// `thread/start` in `cwd` (approval `never`, sandbox
    /// `danger-full-access`, P4); returns the thread id.
    pub fn start_thread(&mut self, cwd: &str) -> Result<String, RpcError> {
        let result = self.call_within(
            "thread/start",
            json!({"cwd": cwd, "approvalPolicy": "never", "sandbox": "danger-full-access"}),
            launch::THREAD_WITHIN,
        )?;
        let thread = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| RpcError::Transport(format!("thread/start without an id: {result}")))?;
        self.thread = thread.to_owned();
        Ok(self.thread.clone())
    }

    /// `thread/resume {threadId, excludeTurns: true}`.
    pub fn resume(&mut self, thread: &str) -> Result<(), RpcError> {
        self.thread = thread.to_owned();
        self.call_within(
            "thread/resume",
            json!({"threadId": thread, "excludeTurns": true}),
            launch::THREAD_WITHIN,
        )?;
        Ok(())
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// Starts the link thread; `sink` hears when the app-server is gone.
    pub fn spawn(self, sink: CodexSink) -> std::io::Result<Link> {
        let shared = Arc::clone(&self.shared);
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name(format!("codex-link-{}", self.id))
            .spawn(move || self.run(&rx, &sink))?;
        Ok(Link {
            commands: Some(tx),
            shared,
            thread: Some(thread),
        })
    }

    fn stopping(&self) -> bool {
        self.shared.stopping.load(Ordering::SeqCst)
    }

    fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        self.shared.busy.store(busy, Ordering::SeqCst);
    }

    /// After a (re)connect: reconcile, then send what is still `queued`.
    pub fn catch_up(&mut self) -> Result<(), RpcError> {
        self.reconcile()?;
        self.flush()
    }

    /// The link thread; [`Worker::catch_up`] ran on the first connection.
    fn run(mut self, commands: &Receiver<Command>, sink: &CodexSink) {
        loop {
            self.shared.connected.store(true, Ordering::SeqCst);
            if self.pump(commands) {
                return;
            }
            self.shared.connected.store(false, Ordering::SeqCst);
            if self.stopping() {
                return;
            }
            if !self.reconnect(commands) {
                if !self.stopping() {
                    log::line(&format!(
                        "{}: app-server is gone (no connection for {} s)",
                        self.id,
                        launch::READY_WITHIN.as_secs()
                    ));
                    sink(CodexEvent::Gone {
                        id: self.id.clone(),
                        generation: self.generation,
                    });
                }
                return;
            }
        }
    }

    /// Serves commands and reads until the connection ends (false) or the
    /// link is closed (true).
    fn pump(&mut self, commands: &Receiver<Command>) -> bool {
        loop {
            if self.stopping() {
                return true;
            }
            loop {
                match commands.try_recv() {
                    Ok(Command::Flush(reply)) => {
                        let flushed = self.flush();
                        let _ = reply.send(());
                        if flushed.is_err() {
                            return false;
                        }
                    }
                    Ok(Command::Turns(reply)) => {
                        let turns = self.turns_all();
                        let broken =
                            matches!(turns, Err(RpcError::Transport(_) | RpcError::Timeout(_)));
                        let _ = reply.send(turns.map_err(|e| e.to_string()));
                        if broken {
                            return false;
                        }
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return true,
                }
            }
            match self.conn.read_message() {
                Ok(Some(message)) => self.dispatch(message),
                Ok(None) => {}
                Err(_) => return false,
            }
        }
    }

    /// Tries to connect again for [`launch::READY_WITHIN`]; answers commands
    /// meanwhile without sending anything.
    fn reconnect(&mut self, commands: &Receiver<Command>) -> bool {
        let deadline = Instant::now() + launch::READY_WITHIN;
        while Instant::now() < deadline {
            match commands.recv_timeout(RECONNECT_EVERY) {
                Ok(Command::Flush(reply)) => {
                    let _ = reply.send(());
                }
                Ok(Command::Turns(reply)) => {
                    let _ = reply.send(Err("the app-server is not connected".into()));
                }
                Err(RecvTimeoutError::Disconnected) => return false,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.stopping() {
                return false;
            }
            let Ok(conn) = Conn::open(&self.listen) else {
                continue;
            };
            self.conn = conn;
            let thread = self.thread.clone();
            if self.initialize().is_ok() && self.resume(&thread).is_ok() {
                log::line(&format!(
                    "{}: app-server connected again, thread {thread} resumed",
                    self.id
                ));
                if self.catch_up().is_ok() {
                    return true;
                }
            }
        }
        false
    }

    fn call_within(
        &mut self,
        method: &str,
        params: Value,
        within: Duration,
    ) -> Result<Value, RpcError> {
        let id = self.conn.request(method, params)?;
        let deadline = Instant::now() + within;
        loop {
            match self.conn.read_message()? {
                Some(Incoming::Response { id: got, result }) if got == id => return result,
                Some(other) => self.dispatch(other),
                None if Instant::now() >= deadline => {
                    return Err(RpcError::Timeout(format!(
                        "{method} ({} s)",
                        within.as_secs()
                    )));
                }
                None if self.stopping() => {
                    return Err(RpcError::Transport("the link is closing".into()));
                }
                None => {}
            }
        }
    }

    fn ours(&self, params: &Value) -> bool {
        params["threadId"] == self.thread.as_str()
    }

    fn dispatch(&mut self, message: Incoming) {
        match message {
            Incoming::Notification { method, params } => self.notification(&method, &params),
            Incoming::Request { id, method, params } => self.server_request(id, &method, &params),
            Incoming::Response { .. } => {}
        }
    }

    fn notification(&mut self, method: &str, params: &Value) {
        if !self.ours(params) {
            return;
        }
        match method {
            "thread/status/changed" => {
                let busy = params["status"]["type"] == "active";
                self.set_busy(busy);
            }
            "turn/started" => {
                self.active = params["turn"]["id"].as_str().map(str::to_owned);
                self.set_busy(true);
            }
            "turn/completed" => {
                if self.active.as_deref() == params["turn"]["id"].as_str() {
                    self.active = None;
                    self.set_busy(false);
                }
            }
            "item/completed" => {
                let turn = params["turnId"].as_str().unwrap_or_default();
                if let Some(item) = UserItem::from_item(turn, &params["item"]) {
                    self.confirm_live(&item);
                }
            }
            _ => {}
        }
    }

    /// A user message just completed: confirms the `queued`/`sent` row it is.
    fn confirm_live(&mut self, item: &UserItem) {
        let rows = match messages_to(&self.store, &self.id) {
            Ok(rows) => rows,
            Err(e) => return log::line(&format!("{}: cannot read messages: {e}", self.id)),
        };
        let open: Vec<Message> = rows
            .into_iter()
            .filter(|r| matches!(r.state, DeliveryState::Queued | DeliveryState::Sent))
            .collect();
        if let Some(Some(row)) = match_items(std::slice::from_ref(item), &open).pop() {
            self.confirm_logged(&row, &item.turn_id, "");
        }
    }

    fn confirm_logged(&self, row: &str, turn: &str, how: &str) {
        match confirm(&self.store, row, turn) {
            Ok(Some(_)) => log::line(&format!("{}: {row} confirmed (turn {turn}){how}", self.id)),
            Ok(None) => {}
            Err(e) => log::line(&format!("{}: cannot confirm {row}: {e}", self.id)),
        }
    }

    fn server_request(&mut self, id: Value, method: &str, params: &Value) {
        let reply = if method.ends_with("requestApproval") {
            let what = params["command"]
                .as_str()
                .or(params["reason"].as_str())
                .unwrap_or(method);
            log::line(&format!(
                "{}: approval declined (gate 7 has no handler): {what}",
                self.id
            ));
            json!({"id": id, "result": {"decision": "decline"}})
        } else {
            json!({"id": id, "error": {"code": -32601, "message": format!("agend does not handle {method}")}})
        };
        let _ = self.conn.send(&reply);
    }

    /// Every turn, oldest first (all pages; U5: the paging is not verified).
    fn turns_all(&mut self) -> Result<Vec<Value>, RpcError> {
        let mut turns = Vec::new();
        let mut cursor = Value::Null;
        loop {
            let page = self.call_within(
                "thread/turns/list",
                json!({"threadId": self.thread, "cursor": cursor, "limit": TURNS_PAGE}),
                CALL_WITHIN,
            )?;
            turns.extend(page["data"].as_array().cloned().unwrap_or_default());
            match page["nextCursor"].as_str() {
                Some(next) => cursor = json!(next),
                None => return Ok(turns),
            }
        }
    }

    /// See the module doc (P5 crash reconciliation).
    fn reconcile(&mut self) -> Result<(), RpcError> {
        let turns = self.turns_all()?;
        if let Some(last) = turns.last().filter(|t| t["status"] == "inProgress") {
            self.active = last["id"].as_str().map(str::to_owned);
            self.set_busy(true);
        }
        let rows = messages_to(&self.store, &self.id).map_err(store_error)?;
        let items = user_items(&turns);
        let found = match_items(
            &items.iter().map(|(_, u)| u.clone()).collect::<Vec<_>>(),
            &rows,
        );
        for ((_, item), row) in items.iter().zip(found) {
            let Some(row) = row else { continue };
            if rows
                .iter()
                .any(|r| r.id == row && r.state == DeliveryState::Queued)
            {
                log::line(&format!(
                    "{}: {row} found in thread history (turn {}); marked sent",
                    self.id, item.turn_id
                ));
            }
            self.confirm_logged(&row, &item.turn_id, " from the thread history");
        }
        let queued: Vec<Message> = messages_to(&self.store, &self.id)
            .map_err(store_error)?
            .into_iter()
            .filter(|r| r.state == DeliveryState::Queued)
            .collect();
        if queued.is_empty() {
            return Ok(());
        }
        let listed = match self.call_within(
            "thread/queue/list",
            json!({"threadId": self.thread}),
            CALL_WITHIN,
        ) {
            Ok(listed) => listed,
            Err(e @ RpcError::Rpc { .. }) => {
                log::line(&format!(
                    "{}: cannot list the thread queue ({e}); queued messages are sent",
                    self.id
                ));
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        for entry in listed["data"].as_array().into_iter().flatten() {
            let Some(client) = entry["clientUserMessageId"].as_str() else {
                continue;
            };
            if queued.iter().any(|r| r.id == client)
                && advance(&self.store, client, DeliveryState::Sent, None)
                    .map_err(store_error)?
                    .is_some()
            {
                log::line(&format!(
                    "{}: {client} found in thread queue; marked sent",
                    self.id
                ));
            }
        }
        Ok(())
    }

    /// Sends every `queued` message in `seq` order. A broken connection
    /// stops it (the messages stay `queued`); a refusal marks one `failed`.
    fn flush(&mut self) -> Result<(), RpcError> {
        let queued: Vec<Message> = messages_to(&self.store, &self.id)
            .map_err(store_error)?
            .into_iter()
            .filter(|r| r.state == DeliveryState::Queued)
            .collect();
        for row in queued {
            let text = text_of(&row);
            let thread = self.thread.clone();
            let level = messages::level_text(row.level);
            match send::send(self, &thread, &text, &row.id, row.level) {
                Ok(outcome) => {
                    advance(
                        &self.store,
                        &row.id,
                        DeliveryState::Sent,
                        outcome.turn_id.as_deref(),
                    )
                    .map_err(store_error)?;
                    log::line(&format!(
                        "{}: {} ({level}) → {} → sent{}",
                        self.id,
                        row.id,
                        outcome.via.join(", "),
                        outcome
                            .turn_id
                            .map(|t| format!(" (turn {t})"))
                            .unwrap_or_default()
                    ));
                }
                Err(e @ RpcError::Rpc { .. }) => {
                    advance(&self.store, &row.id, DeliveryState::Failed, None)
                        .map_err(store_error)?;
                    log::line(&format!("{}: {} failed: {e}", self.id, row.id));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

fn store_error(e: StoreError) -> RpcError {
    RpcError::Transport(format!("store: {e}"))
}

impl Rpc for Worker {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let result = self.call_within(method, params, CALL_WITHIN)?;
        if method == "turn/start"
            && let Some(turn) = result["turn"]["id"].as_str()
        {
            self.active = Some(turn.to_owned());
            self.set_busy(true);
        }
        Ok(result)
    }

    fn busy(&self) -> bool {
        self.busy
    }

    fn active_turn(&self) -> Option<String> {
        self.active.clone()
    }

    fn wait_turn_end(&mut self, turn: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while self.active.as_deref() == Some(turn) {
            if Instant::now() >= deadline {
                return false;
            }
            match self.conn.read_message() {
                Ok(Some(message)) => self.dispatch(message),
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        true
    }
}
