//! Finite writes to the nonblocking native PTY. The polling descriptor is
//! owned so it cannot be reused when the holder drops its master handle.

use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

const WRITE_WITHIN: Duration = Duration::from_secs(5);

pub(super) struct DeadlinedWriter {
    out: Box<dyn Write + Send>,
    poll_file: File,
}

impl DeadlinedWriter {
    pub(super) fn new(out: Box<dyn Write + Send>, poll_file: File) -> Self {
        Self { out, poll_file }
    }

    fn write_until(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PTY write exceeded 5 seconds; partial input may have been written",
                ));
            }
            match self.out.write(bytes) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => self.wait_writable(deadline)?,
                result => return result,
            }
        }
    }

    fn wait_writable(&self, deadline: Instant) -> io::Result<()> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PTY write exceeded 5 seconds; partial input may have been written",
                ));
            }
            let mut poll = libc::pollfd {
                fd: self.poll_file.as_raw_fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            // SAFETY: one initialized pollfd backed by our owned file.
            let rc = unsafe {
                libc::poll(
                    &mut poll,
                    1,
                    remaining.as_millis().clamp(1, i32::MAX as u128) as i32,
                )
            };
            if rc < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if rc > 0 {
                if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "PTY closed while writing",
                    ));
                }
                if poll.revents & libc::POLLOUT != 0 {
                    return Ok(());
                }
            }
        }
    }
}

impl Write for DeadlinedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_until(bytes, Instant::now() + WRITE_WITHIN)
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        // A single deadline covers the whole operation, including partial
        // progress. Large input cannot keep resetting the timeout forever.
        let deadline = Instant::now() + WRITE_WITHIN;
        while !bytes.is_empty() {
            let count = self.write_until(bytes, deadline)?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "PTY accepted no input",
                ));
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
