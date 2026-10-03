//! Bounded, nonblocking UI seam for a dedicated client 1.4 connection.
//! Control replies are ordered; intermediate screen updates may be coalesced.
//! Closing cancels queued writes, shuts down both readers and never replays.

use super::{FullTerminalEvent, SourceError};
use agend_client::{Client, ClientError, FullTerminalUpdate, Sender};
use agend_core::protocol::client::{ClientRequest, ErrorData};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

const WRITE_QUEUE: usize = 16;
const REPLY_QUEUE: usize = 64;

#[derive(Default)]
struct Mailbox {
    replies: VecDeque<FullTerminalEvent>,
    frame: Option<FullTerminalEvent>,
    closed: Option<String>,
}

struct Count(Arc<AtomicUsize>);
impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(super) struct FullConnection {
    close: Sender,
    alive: Arc<AtomicBool>,
    writes: mpsc::SyncSender<ClientRequest>,
    mailbox: Arc<Mutex<Mailbox>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

impl FullConnection {
    pub(super) fn start(
        mut client: Client,
        threads: Arc<AtomicUsize>,
    ) -> Result<Self, SourceError> {
        let close = client.sender().map_err(super::connect_error)?;
        let mut write = client.sender().map_err(super::connect_error)?;
        let read_close = client.sender().map_err(super::connect_error)?;
        let alive = Arc::new(AtomicBool::new(true));
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let (writes, jobs) = mpsc::sync_channel(WRITE_QUEUE);
        let read_alive = alive.clone();
        let read_box = mailbox.clone();
        let read_count = threads.clone();
        threads.fetch_add(1, Ordering::SeqCst);
        let reader = match std::thread::Builder::new()
            .name("tui-full-reader".into())
            .spawn(move || {
                let _count = Count(read_count);
                while read_alive.load(Ordering::SeqCst) {
                    let update = match client.next_full_terminal() {
                        Ok(update) => update,
                        Err(error) => {
                            fail(&read_box, &read_alive, &read_close, error.to_string());
                            break;
                        }
                    };
                    let event = match update {
                        FullTerminalUpdate::Frame(data) => FullTerminalEvent::Frame(Box::new(data)),
                        FullTerminalUpdate::ControlAck(data) => {
                            FullTerminalEvent::ControlAck(Box::new(data))
                        }
                        FullTerminalUpdate::ControlChanged(data) => {
                            FullTerminalEvent::ControlChanged(data)
                        }
                        FullTerminalUpdate::Rejected(data) => FullTerminalEvent::Refused(data),
                    };
                    if !publish(&read_box, event) {
                        fail(
                            &read_box,
                            &read_alive,
                            &read_close,
                            "terminal reply queue overflow; reconnect read-only".into(),
                        );
                        break;
                    }
                }
            }) {
            Ok(reader) => reader,
            Err(error) => {
                threads.fetch_sub(1, Ordering::SeqCst);
                close.close();
                return Err(SourceError::Disconnected(error.to_string()));
            }
        };
        let write_alive = alive.clone();
        let write_box = mailbox.clone();
        let write_count = threads.clone();
        threads.fetch_add(1, Ordering::SeqCst);
        let writer = match std::thread::Builder::new()
            .name("tui-full-writer".into())
            .spawn(move || {
                let _count = Count(write_count);
                while write_alive.load(Ordering::SeqCst) {
                    let request = match jobs.recv_timeout(Duration::from_millis(10)) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    if !write_alive.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Err(error) = write.send_terminal(&request) {
                        match error {
                            ClientError::Daemon { code, message } => {
                                let id = match request {
                                    ClientRequest::SubscribeTerminalFrames { data } => {
                                        data.request_id
                                    }
                                    ClientRequest::SetTerminalViewport { data } => data.request_id,
                                    ClientRequest::TerminalControl { data } => data.request_id,
                                    _ => unreachable!("only full terminal requests are queued"),
                                };
                                if !publish(
                                    &write_box,
                                    FullTerminalEvent::Refused(ErrorData {
                                        request_id: Some(id),
                                        code,
                                        message,
                                    }),
                                ) {
                                    fail(
                                        &write_box,
                                        &write_alive,
                                        &write,
                                        "terminal reply queue overflow".into(),
                                    );
                                    break;
                                }
                            }
                            other => {
                                fail(&write_box, &write_alive, &write, other.to_string());
                                break;
                            }
                        }
                    }
                }
            }) {
            Ok(writer) => writer,
            Err(error) => {
                threads.fetch_sub(1, Ordering::SeqCst);
                alive.store(false, Ordering::SeqCst);
                close.close();
                let _ = reader.join();
                return Err(SourceError::Disconnected(error.to_string()));
            }
        };
        Ok(Self {
            close,
            alive,
            writes,
            mailbox,
            reader: Some(reader),
            writer: Some(writer),
        })
    }

    pub(super) fn send(&self, request: ClientRequest) -> Result<(), SourceError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(SourceError::Disconnected(
                "terminal connection ended; input was not replayed".into(),
            ));
        }
        // Bound the queue's memory as well as its number of operations. The
        // writer performs the exact serialized-line check before sending.
        if matches!(&request, ClientRequest::TerminalControl { data }
            if matches!(&data.operation, agend_core::protocol::client::ClientTerminalOperation::Input { bytes_base64, .. }
                if bytes_base64.len() > agend_core::protocol::holder::MAX_REQUEST_LINE))
        {
            return Err(SourceError::Rejected {
                code: "invalid_request".into(),
                message: "terminal input exceeds 1 MiB; nothing was queued".into(),
            });
        }
        match self.writes.try_send(request) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(SourceError::Rejected {
                code: "busy".into(),
                message: "terminal writer queue is full; nothing was queued".into(),
            }),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(SourceError::Disconnected("terminal writer ended".into()))
            }
        }
    }

    pub(super) fn poll(&self) -> Vec<FullTerminalEvent> {
        let mut mailbox = self.mailbox.lock().unwrap();
        let mut events: Vec<_> = mailbox.replies.drain(..).collect();
        if let Some(frame) = mailbox.frame.take() {
            events.push(frame);
        }
        if let Some(reason) = mailbox.closed.take() {
            events.push(FullTerminalEvent::Closed(reason));
        }
        events
    }
}
impl Drop for FullConnection {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.close.close();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}
fn publish(mailbox: &Mutex<Mailbox>, event: FullTerminalEvent) -> bool {
    let mut mailbox = mailbox.lock().unwrap();
    if mailbox.closed.is_some() {
        return true;
    }
    if matches!(event, FullTerminalEvent::Frame(_)) {
        mailbox.frame = Some(event);
    } else {
        if mailbox.replies.len() >= REPLY_QUEUE {
            return false;
        }
        mailbox.replies.push_back(event);
    }
    true
}
fn fail(mailbox: &Mutex<Mailbox>, alive: &AtomicBool, sender: &Sender, reason: String) {
    alive.store(false, Ordering::SeqCst);
    sender.close();
    let mut mailbox = mailbox.lock().unwrap();
    mailbox.frame = None;
    mailbox.replies.clear();
    mailbox.closed.get_or_insert(reason);
}
