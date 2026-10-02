//! Restore outer terminal input modes on normal exit, error and unwinding.
use ratatui::crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
};
use std::io::{self, Write};
pub struct OuterModes<W: Write> {
    writer: W,
}
impl<W: Write> OuterModes<W> {
    pub fn enter(writer: W) -> io::Result<Self> {
        // Construct first: partial setup errors still run the destructor.
        let mut guard = Self { writer };
        ratatui::crossterm::execute!(
            guard.writer,
            EnableMouseCapture,
            EnableBracketedPaste,
            EnableFocusChange
        )?;
        Ok(guard)
    }
}
impl<W: Write> Drop for OuterModes<W> {
    fn drop(&mut self) {
        // Independent attempts: an error restoring one mode must not skip the
        // remaining modes. Ratatui separately restores raw/alternate screen.
        let _ = ratatui::crossterm::execute!(self.writer, DisableMouseCapture);
        let _ = ratatui::crossterm::execute!(self.writer, DisableBracketedPaste);
        let _ = ratatui::crossterm::execute!(self.writer, DisableFocusChange);
        let _ = ratatui::crossterm::execute!(self.writer, SetCursorStyle::DefaultUserShape);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Writer {
        bytes: Arc<Mutex<Vec<u8>>>,
        fail_next: Arc<Mutex<bool>>,
    }
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if std::mem::take(&mut *self.fail_next.lock().unwrap()) {
                return Err(io::Error::other("injected output failure"));
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn restored(writer: &Writer) {
        let bytes = writer.bytes.lock().unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for reset in [
            "\x1b[?1000l",
            "\x1b[?1002l",
            "\x1b[?1003l",
            "\x1b[?1006l",
            "\x1b[?2004l",
            "\x1b[?1004l",
            "\x1b[0 q",
        ] {
            assert!(
                text.contains(reset),
                "missing mode reset {reset:?}: {text:?}"
            );
        }
    }
    #[test]
    fn modes_restore_after_normal_exit_setup_error_and_unwind() {
        let writer = Writer::default();
        drop(OuterModes::enter(writer.clone()).unwrap());
        restored(&writer);
        let writer = Writer::default();
        *writer.fail_next.lock().unwrap() = true;
        assert!(OuterModes::enter(writer.clone()).is_err());
        restored(&writer);
        let writer = Writer::default();
        let result = std::panic::catch_unwind(|| {
            let _guard = OuterModes::enter(writer.clone()).unwrap();
            panic!("injected unwind");
        });
        assert!(result.is_err());
        restored(&writer);
    }
}
