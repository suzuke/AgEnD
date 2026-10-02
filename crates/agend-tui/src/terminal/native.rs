//! Restore outer terminal input modes on normal exit, error and unwinding.
use ratatui::crossterm::{
    cursor::{SetCursorStyle, Show},
    event::{
        DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture,
    },
    style::{
        Attribute, Color, Colored, SetAttribute, SetBackgroundColor, SetForegroundColor,
        SetUnderlineColor,
    },
};
use std::io::{self, Write};
pub struct OuterModes<W: Write> {
    writer: W,
    colors: ColorOutput,
}
impl<W: Write> OuterModes<W> {
    pub fn enter(writer: W) -> io::Result<Self> {
        // Construct first: partial setup errors still run the destructor.
        let mut guard = Self {
            writer,
            colors: ColorOutput::enter(),
        };
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
        let _ = ratatui::crossterm::execute!(self.writer, Show);
        let _ = ratatui::crossterm::execute!(self.writer, SetAttribute(Attribute::Reset));
        let _ = ratatui::crossterm::execute!(self.writer, SetForegroundColor(Color::Reset));
        let _ = ratatui::crossterm::execute!(self.writer, SetBackgroundColor(Color::Reset));
        let _ = ratatui::crossterm::execute!(self.writer, SetUnderlineColor(Color::Reset));
        // Restore the previous process setting after emitting real reset SGRs.
        self.colors.restore();
    }
}
/// A full terminal reproduces the agent's colors even when a launcher exports
/// NO_COLOR for ordinary command output. Restore the previous library setting
/// after the outer terminal resets, including partial setup failures.
struct ColorOutput(bool);
impl ColorOutput {
    fn enter() -> Self {
        let previous = Colored::ansi_color_disabled_memoized();
        Colored::set_ansi_color_disabled(false);
        Self(previous)
    }
    fn restore(&self) {
        Colored::set_ansi_color_disabled(self.0);
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
            "\x1b[?25h",
            "\x1b[0m",
            "\x1b[39m",
            "\x1b[49m",
            "\x1b[59m",
        ] {
            assert!(
                text.contains(reset),
                "missing mode reset {reset:?}: {text:?}"
            );
        }
    }
    #[test]
    fn modes_restore_after_normal_exit_setup_error_and_unwind() {
        let previous_colors = Colored::ansi_color_disabled_memoized();
        Colored::set_ansi_color_disabled(true);
        let writer = Writer::default();
        let guard = OuterModes::enter(writer.clone()).unwrap();
        assert!(!Colored::ansi_color_disabled_memoized());
        drop(guard);
        assert!(Colored::ansi_color_disabled_memoized());
        restored(&writer);
        let writer = Writer::default();
        *writer.fail_next.lock().unwrap() = true;
        assert!(OuterModes::enter(writer.clone()).is_err());
        assert!(Colored::ansi_color_disabled_memoized());
        restored(&writer);
        let writer = Writer::default();
        let result = std::panic::catch_unwind(|| {
            let _guard = OuterModes::enter(writer.clone()).unwrap();
            panic!("injected unwind");
        });
        assert!(result.is_err());
        assert!(Colored::ansi_color_disabled_memoized());
        restored(&writer);
        Colored::set_ansi_color_disabled(previous_colors);
    }
}
