//! Deadline-aware socket reads without changing descriptor flags or socket
//! options. Poll then MSG_DONTWAIT also drains bytes after the peer closes.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Instant;

pub(super) struct SocketReader {
    stream: UnixStream,
    pub(super) deadline: Option<Instant>,
}

impl SocketReader {
    pub(super) fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            deadline: None,
        }
    }
}

impl Read for SocketReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            let timeout = match self.deadline {
                None => -1,
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "no reply from holder",
                        ));
                    }
                    left.as_millis().clamp(1, i32::MAX as u128) as i32
                }
            };
            let mut poll = libc::pollfd {
                fd: self.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one initialized pollfd backed by our owned stream.
            let ready = unsafe { libc::poll(&mut poll, 1, timeout) };
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if ready == 0 {
                continue;
            }
            // SAFETY: live owned socket and a writable buffer of this length.
            // MSG_DONTWAIT affects this call only, preserving the writer's
            // blocking flags. HUP is still read so buffered bytes survive EOF.
            let read = unsafe {
                libc::recv(
                    self.stream.as_raw_fd(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if read >= 0 {
                return Ok(read as usize);
            }
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            return Err(error);
        }
    }
}
