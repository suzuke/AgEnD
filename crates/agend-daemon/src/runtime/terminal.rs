//! Correlated terminal requests on one holder connection. Requests are bounded,
//! written in the background, and never replayed across a reconnect. Cancelling
//! an in-flight control operation closes its connection so a late grant cannot survive.

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::holder::{HolderRequest, HolderResponse, MAX_REQUEST_LINE, V1_1};
use agend_core::protocol::terminal::{
    TerminalControlData, TerminalControlOperation, TerminalControlRequest, TerminalFrame,
    TerminalFrameRequest, TerminalOperationError, TerminalViewport,
};
use tokio::sync::oneshot;

const PENDING: usize = 64;
const RESPONSE_WITHIN: Duration = Duration::from_secs(15);

type Result<T> = std::result::Result<T, TerminalOperationError>;
type Stream = Arc<Mutex<Option<UnixStream>>>;

pub(crate) fn failure(id: &str, code: &str, message: &str) -> TerminalOperationError {
    TerminalOperationError {
        request_id: id.into(),
        code: code.into(),
        message: message.into(),
    }
}

struct Waiting {
    deadline: Instant,
    reply: oneshot::Sender<Result<HolderResponse>>,
}
#[derive(Default)]
struct State {
    epoch: u64,
    version: Option<ProtocolVersion>,
    next_id: u64,
    pending: BTreeMap<String, Waiting>,
}
struct Job {
    epoch: u64,
    request_id: String,
    deadline: Instant,
    line: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct Channel {
    state: Arc<Mutex<State>>,
    jobs: mpsc::SyncSender<Job>,
    stream: Stream,
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|e| e.into_inner())
}

impl Channel {
    pub(super) fn start(
        stream: Stream,
        write: Arc<Mutex<()>>,
        stopping: Arc<AtomicBool>,
    ) -> (Self, JoinHandle<()>) {
        let (jobs, receiver) = mpsc::sync_channel::<Job>(PENDING);
        let channel = Self {
            state: Arc::new(Mutex::new(State::default())),
            jobs,
            stream,
        };
        let worker = channel.clone();
        let thread = std::thread::Builder::new()
            .name("holder-terminal-writer".into())
            .spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    let expired = {
                        let state = lock(&worker.state);
                        state
                            .pending
                            .values()
                            .any(|p| p.deadline <= Instant::now())
                            .then_some(state.epoch)
                    };
                    if let Some(epoch) = expired {
                        worker.close_epoch(
                            epoch,
                            "terminal_timeout",
                            "terminal operation exceeded 15 seconds; acquire control again",
                        );
                    }
                    match receiver.recv_timeout(Duration::from_millis(50)) {
                        Ok(job) => {
                            // Same whole-line lock as legacy writes; capture the
                            // actual socket only while checking the pinned epoch.
                            let whole_line = loop {
                                if stopping.load(Ordering::SeqCst) { return; }
                                match write.try_lock() {
                                    Ok(guard) => break Some(guard),
                                    Err(std::sync::TryLockError::Poisoned(error)) => break Some(error.into_inner()),
                                    Err(std::sync::TryLockError::WouldBlock) => {
                                        if Instant::now() >= job.deadline {
                                            worker.close_epoch(job.epoch, "terminal_timeout", "terminal operation exceeded 15 seconds; acquire control again");
                                            break None;
                                        }
                                        std::thread::sleep(Duration::from_millis(5));
                                    }
                                }
                            };
                            let Some(_whole_line) = whole_line else { continue };
                            let stream = {
                                let slot = lock(&worker.stream);
                                let state = lock(&worker.state);
                                if state.epoch == job.epoch
                                    && state.version.is_some()
                                    && state.pending.contains_key(&job.request_id)
                                {
                                    slot.as_ref().and_then(|s| s.try_clone().ok())
                                } else {
                                    None
                                }
                            };
                            let Some(mut stream) = stream else { continue };
                            if write_until(&mut stream, &job.line, job.deadline.min(Instant::now() + super::link::WRITE_WITHIN)).is_err() {
                                let _ = stream.shutdown(std::net::Shutdown::Both);
                                worker.close_epoch(
                                    job.epoch,
                                    "stale_terminal",
                                    "holder write failed; acquire control again",
                                );
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => (),
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .expect("spawn holder terminal writer");
        (channel, thread)
    }

    // The publisher holds the stream slot lock while connecting.
    pub(super) fn connected(&self, version: ProtocolVersion) {
        let mut state = lock(&self.state);
        fail_pending(
            &mut state,
            "stale_terminal",
            "holder connection changed; acquire control again",
        );
        state.epoch += 1;
        state.version = Some(version);
    }

    pub(super) fn disconnected(&self, message: &str) {
        let mut state = lock(&self.state);
        state.version = None;
        fail_pending(&mut state, "stale_terminal", message);
    }

    pub(super) fn connection(&self) -> Result<TerminalConnection> {
        let state = lock(&self.state);
        match state.version {
            Some(version) if version >= V1_1 => Ok(TerminalConnection {
                epoch: state.epoch,
                channel: self.clone(),
            }),
            Some(_) => Err(failure(
                "",
                "not_supported",
                "full terminal requires holder protocol 1.1; upgrade the holder",
            )),
            None => Err(failure(
                "",
                "no_terminal",
                "the holder is disconnected; acquire control after reconnecting",
            )),
        }
    }

    pub(super) fn response(&self, response: HolderResponse) {
        let id = match &response {
            HolderResponse::TerminalFrame { data } => &data.request_id,
            HolderResponse::TerminalControl { data } => &data.request_id,
            HolderResponse::TerminalOperationError { data } => &data.request_id,
            _ => return,
        };
        if let Some(waiting) = lock(&self.state).pending.remove(id) {
            let _ = waiting.reply.send(Ok(response));
        }
    }

    fn close_epoch(&self, epoch: u64, code: &str, message: &str) {
        let mut stream = lock(&self.stream);
        let mut state = lock(&self.state);
        if state.epoch != epoch {
            return;
        }
        state.version = None;
        fail_pending(&mut state, code, message);
        if let Some(stream) = stream.take() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

fn write_until(stream: &mut UnixStream, mut line: &[u8], deadline: Instant) -> std::io::Result<()> {
    while !line.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "holder request write timed out",
            ));
        }
        stream.set_write_timeout(Some(left.max(Duration::from_millis(1))))?;
        match stream.write(line) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "holder accepted no request bytes",
                ));
            }
            Ok(count) => line = &line[count..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn fail_pending(state: &mut State, code: &str, message: &str) {
    for (id, waiting) in std::mem::take(&mut state.pending) {
        let _ = waiting.reply.send(Err(failure(&id, code, message)));
    }
}

/// Capability and connection epoch, distinct from the holder process generation.
/// Clone to share one connection; an old clone never follows a reconnect.
#[derive(Clone)]
pub struct TerminalConnection {
    epoch: u64,
    channel: Channel,
}

impl TerminalConnection {
    pub fn is_current(&self) -> bool {
        let state = lock(&self.channel.state);
        state.epoch == self.epoch && state.version.is_some()
    }

    pub async fn frame(&self, viewport: TerminalViewport) -> Result<TerminalFrame> {
        match self
            .request(|id| HolderRequest::GetTerminalFrame {
                data: TerminalFrameRequest {
                    request_id: id,
                    viewport,
                },
            })
            .await?
        {
            HolderResponse::TerminalFrame { data } => Ok(data.frame),
            _ => {
                self.channel.close_epoch(
                    self.epoch,
                    "invalid_response",
                    "holder returned the wrong terminal response",
                );
                Err(failure(
                    "",
                    "invalid_response",
                    "holder did not return a terminal frame",
                ))
            }
        }
    }

    pub async fn control(
        &self,
        generation: String,
        operation: TerminalControlOperation,
    ) -> Result<TerminalControlData> {
        match self
            .request(|id| HolderRequest::TerminalControl {
                data: TerminalControlRequest {
                    request_id: id,
                    generation,
                    operation,
                },
            })
            .await?
        {
            HolderResponse::TerminalControl { data } => Ok(data),
            _ => {
                self.channel.close_epoch(
                    self.epoch,
                    "invalid_response",
                    "holder returned the wrong terminal response",
                );
                Err(failure(
                    "",
                    "invalid_response",
                    "holder did not acknowledge terminal control",
                ))
            }
        }
    }

    async fn request(&self, make: impl FnOnce(String) -> HolderRequest) -> Result<HolderResponse> {
        let (reply, response) = oneshot::channel();
        let (id, invalidate_on_cancel) = {
            let mut state = lock(&self.channel.state);
            if state.epoch != self.epoch || state.version.is_none() {
                return Err(failure(
                    "",
                    "stale_terminal",
                    "holder connection changed; acquire control again",
                ));
            }
            if state.pending.len() >= PENDING {
                return Err(failure(
                    "",
                    "pty_busy",
                    "too many pending terminal requests; nothing was written",
                ));
            }
            state.next_id += 1;
            let id = format!("terminal-{}-{}", self.epoch, state.next_id);
            let request = make(id.clone());
            let invalidate_on_cancel = matches!(request, HolderRequest::TerminalControl { .. });
            let mut line = serde_json::to_vec(&request)
                .map_err(|e| failure(&id, "invalid_request", &e.to_string()))?;
            line.push(b'\n');
            if line.len() > MAX_REQUEST_LINE {
                return Err(failure(
                    &id,
                    "invalid_request",
                    &agend_core::protocol::holder::operator_input_too_long(line.len()),
                ));
            }
            let deadline = Instant::now() + RESPONSE_WITHIN;
            state
                .pending
                .insert(id.clone(), Waiting { deadline, reply });
            if self
                .channel
                .jobs
                .try_send(Job {
                    epoch: self.epoch,
                    request_id: id.clone(),
                    deadline,
                    line,
                })
                .is_err()
            {
                state.pending.remove(&id);
                return Err(failure(
                    &id,
                    "pty_busy",
                    "terminal writer is unavailable; nothing was written",
                ));
            }
            (id, invalidate_on_cancel)
        };
        let mut cancellation = Cancellation {
            connection: self.clone(),
            id,
            invalidate_on_cancel,
            completed: false,
        };
        let reply = response.await;
        cancellation.completed = true;
        match reply {
            Ok(Ok(HolderResponse::TerminalOperationError { data })) => Err(data),
            Ok(result) => result,
            Err(_) => Err(failure(
                "",
                "stale_terminal",
                "holder connection ended; acquire control again",
            )),
        }
    }
}

struct Cancellation {
    connection: TerminalConnection,
    id: String,
    invalidate_on_cancel: bool,
    completed: bool,
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        lock(&self.connection.channel.state)
            .pending
            .remove(&self.id);
        // A reply may already be queued in the oneshot while its consumer is
        // cancelled. Absence from pending does not prove it consumed a grant.
        if self.invalidate_on_cancel {
            self.connection.channel.close_epoch(
                self.connection.epoch,
                "stale_terminal",
                "terminal operation cancelled; acquire control again",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::holder::V1;
    use std::io::Read;

    #[test]
    fn old_holder_capability_is_rejected_without_sending_a_new_request() {
        let (ours, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let stream = Arc::new(Mutex::new(Some(ours)));
        let stopping = Arc::new(AtomicBool::new(false));
        let (channel, thread) =
            Channel::start(stream, Arc::new(Mutex::new(())), Arc::clone(&stopping));
        channel.connected(V1);
        let error = match channel.connection() {
            Ok(_) => panic!("old holder enabled full terminal"),
            Err(error) => error,
        };
        assert_eq!(error.code, "not_supported");
        let mut byte = [0];
        assert_eq!(
            peer.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        stopping.store(true, Ordering::SeqCst);
        channel.disconnected("test ended");
        thread.join().unwrap();
    }
    #[test]
    fn a_legacy_write_lock_cannot_prevent_terminal_timeout_or_link_shutdown() {
        let (ours, _peer) = UnixStream::pair().unwrap();
        let write = Arc::new(Mutex::new(()));
        let held = lock(&write);
        let stopping = Arc::new(AtomicBool::new(false));
        let (channel, thread) = Channel::start(
            Arc::new(Mutex::new(Some(ours))),
            Arc::clone(&write),
            Arc::clone(&stopping),
        );
        channel.connected(V1_1);
        let connection = channel.connection().unwrap();
        let (done, reply) = mpsc::channel();
        let started = Instant::now();
        let caller = std::thread::spawn(move || {
            let result =
                agend_testkit::block_on(connection.frame(TerminalViewport { top: None, rows: 1 }));
            let _ = done.send(result);
        });
        let result = reply.recv_timeout(RESPONSE_WITHIN + Duration::from_secs(2));
        // Release even on a failed assertion so the owned writer can finish.
        drop(held);
        stopping.store(true, Ordering::SeqCst);
        channel.disconnected("test ended");
        thread.join().unwrap();
        caller.join().unwrap();
        assert_eq!(result.unwrap().unwrap_err().code, "terminal_timeout");
        assert!(started.elapsed() < RESPONSE_WITHIN + Duration::from_secs(2));
    }
}
