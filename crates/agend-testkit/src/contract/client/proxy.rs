//! A line-rewriting proxy in front of a client protocol server: the
//! deliberately broken servers of the CLP rules (mutants) and the negative
//! check are the fake daemon behind one of these, so the fake itself has no
//! knob for being wrong.
//!
//! Each client connection gets its own upstream connection; every line in
//! either direction goes through the transform, which may drop it, change
//! it or add lines. Writes to the client have the servers' 5 s timeout.
//!
//! Must NOT: be used as a server in production code.

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::fakes::lock;
use crate::tempdir::TempDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    ToServer,
    ToClient,
}

/// Per client connection, shared by both directions.
#[derive(Debug, Default)]
pub struct ConnState {
    /// 1 for the proxy's first client connection, 2 for the second, …
    pub number: u64,
    /// Free for the transform (for example "lines seen so far").
    pub counter: u64,
    pub flag: bool,
    /// A line the transform holds back (to reorder).
    pub held: Option<String>,
}

/// Rewrites one line (without its newline) into zero or more lines.
pub type Transform = Arc<dyn Fn(&mut ConnState, Direction, String) -> Vec<String> + Send + Sync>;

/// How the proxy misbehaves.
#[derive(Clone)]
pub struct Options {
    pub transform: Transform,
    /// Keep the client's connection open when the server's closes.
    pub keep_client_open: bool,
    /// Read the server as fast as it writes and queue without limit, so a
    /// slow client never makes the server fall behind.
    pub unbounded: bool,
}

impl Options {
    pub fn rewrite(transform: Transform) -> Options {
        Options {
            transform,
            keep_client_open: false,
            unbounded: false,
        }
    }
}

pub struct Proxy {
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<UnixStream>>>,
    _dir: TempDir,
}

impl Proxy {
    /// Listens on a new socket and forwards to `upstream`.
    pub fn start(upstream: PathBuf, options: Options) -> io::Result<Proxy> {
        let dir = TempDir::new("px")?;
        let path = dir.path().join("proxy.sock");
        let listener = UnixListener::bind(&path)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let (stop, conns) = (Arc::clone(&stopping), Arc::clone(&connections));
        let accept = std::thread::Builder::new()
            .name("clp-proxy-accept".into())
            .spawn(move || {
                let next = AtomicU64::new(1);
                for client in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(client) = client else { continue };
                    let number = next.fetch_add(1, Ordering::SeqCst);
                    let Ok(server) = UnixStream::connect(&upstream) else {
                        continue;
                    };
                    for s in [&client, &server] {
                        if let Ok(clone) = s.try_clone() {
                            lock(&conns).push(clone);
                        }
                    }
                    let _ = serve(client, server, number, options.clone());
                }
            })?;
        Ok(Proxy {
            path,
            stopping,
            accept: Some(accept),
            connections,
            _dir: dir,
        })
    }

    pub fn socket(&self) -> &Path {
        &self.path
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for s in lock(&self.connections).drain(..) {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

fn write_lines(mut to: &UnixStream, lines: Vec<String>) -> io::Result<()> {
    for mut line in lines {
        line.push('\n');
        to.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// A queue without limit between the server reader and the client writer.
type Queue = Arc<(Mutex<(VecDeque<String>, bool)>, Condvar)>;

fn serve(client: UnixStream, server: UnixStream, number: u64, options: Options) -> io::Result<()> {
    client.set_write_timeout(Some(Duration::from_secs(5)))?;
    let state = Arc::new(Mutex::new(ConnState {
        number,
        ..ConnState::default()
    }));
    // Client -> server.
    let (from_client, to_server) = (client.try_clone()?, server.try_clone()?);
    let (st, transform) = (Arc::clone(&state), Arc::clone(&options.transform));
    std::thread::spawn(move || {
        for line in BufReader::new(&from_client).lines() {
            let Ok(line) = line else { break };
            let out = transform(&mut lock(&st), Direction::ToServer, line);
            if write_lines(&to_server, out).is_err() {
                break;
            }
        }
        let _ = to_server.shutdown(Shutdown::Write);
    });
    // Server -> client.
    let (from_server, to_client) = (server, client);
    let transform = Arc::clone(&options.transform);
    let keep_open = options.keep_client_open;
    if !options.unbounded {
        std::thread::spawn(move || {
            for line in BufReader::new(&from_server).lines() {
                let Ok(line) = line else { break };
                let out = transform(&mut lock(&state), Direction::ToClient, line);
                if write_lines(&to_client, out).is_err() {
                    let _ = to_client.shutdown(Shutdown::Both);
                    return;
                }
            }
            if !keep_open {
                let _ = to_client.shutdown(Shutdown::Both);
            }
        });
        return Ok(());
    }
    let queue: Queue = Arc::new((Mutex::new((VecDeque::new(), false)), Condvar::new()));
    let q = Arc::clone(&queue);
    std::thread::spawn(move || {
        for line in BufReader::new(&from_server).lines() {
            let Ok(line) = line else { break };
            let out = transform(&mut lock(&state), Direction::ToClient, line);
            lock(&q.0).0.extend(out);
            q.1.notify_all();
        }
        lock(&q.0).1 = true;
        q.1.notify_all();
    });
    std::thread::spawn(move || {
        loop {
            let mut guard = lock(&queue.0);
            while guard.0.is_empty() && !guard.1 {
                guard = queue.1.wait(guard).unwrap_or_else(|e| e.into_inner());
            }
            let Some(line) = guard.0.pop_front() else {
                break;
            };
            drop(guard);
            if write_lines(&to_client, vec![line]).is_err() {
                break;
            }
        }
        let _ = to_client.shutdown(Shutdown::Both);
    });
    Ok(())
}
