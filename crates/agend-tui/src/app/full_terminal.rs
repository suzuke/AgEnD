//! Explicit full-mode control. Completed matching frames enable typing;
//! input acknowledgements never enable or restore control. Reconnect is read-only.
use super::*;
use crate::source::FullTerminalEvent;
use agend_core::protocol::client::{
    ClientTerminalControlData, ClientTerminalFrameData, ClientTerminalOperation,
    TerminalControlState, TerminalViewportData,
};
use agend_core::protocol::terminal::{TerminalSize, TerminalViewport};

#[cfg(test)]
mod ready_tests;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    id: String,
    size: TerminalSize,
    acquire: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullView {
    pub data: Option<ClientTerminalFrameData>,
    pub expanded: bool,
    pub owner: Option<String>,
    pub ready: bool,
    pub lost_control: bool,
    selection: String,
    viewport: TerminalViewport,
    pending: Option<Pending>,
    next: u64,
    input_pending: BTreeSet<String>,
    follow_top: u64,
    fit_requested: Option<TerminalSize>,
}
impl FullView {
    fn new(rows: u16) -> Self {
        Self {
            data: None,
            expanded: false,
            owner: None,
            ready: false,
            lost_control: false,
            selection: "tui-view".into(),
            viewport: TerminalViewport { top: None, rows },
            pending: None,
            next: 0,
            input_pending: BTreeSet::new(),
            follow_top: 0,
            fit_requested: None,
        }
    }
    pub fn follows_live(&self) -> bool {
        self.viewport.top.is_none()
    }
    fn id(&mut self) -> String {
        self.next += 1;
        format!("tui-terminal-{}", self.next)
    }
    pub(super) fn revoke(&mut self) {
        self.owner = None;
        self.pending = None;
        self.expanded = false;
        self.input_pending.clear();
        self.lost_control = false;
        self.fit_requested = None;
    }
}
impl App {
    fn full_rows(&self, expanded: bool) -> u16 {
        self.outer_size
            .rows
            .saturating_sub(if expanded { 1 } else { 2 })
            .clamp(1, 1000)
    }
    pub fn resize(&mut self, columns: u16, rows: u16) {
        if self.outer_size == (TerminalSize { rows, columns }) {
            return;
        }
        self.outer_size = TerminalSize { rows, columns };
        if self.term.as_ref().is_some_and(|term| term.full.is_some()) {
            self.resize_full();
        }
    }
    pub fn full_mode(&self) -> bool {
        self.term
            .as_ref()
            .and_then(|term| term.full.as_ref())
            .is_some_and(|full| full.expanded)
    }
    fn full_size(&self) -> TerminalSize {
        TerminalSize {
            rows: self.outer_size.rows.saturating_sub(1),
            columns: self.outer_size.columns,
        }
    }
    pub(super) fn new_full_term(
        &mut self,
        agent: &str,
        mode: TermMode,
    ) -> Result<Option<Term>, SourceError> {
        // Learn the complete live grid without resizing another controller's
        // PTY. Read-only rendering follows its last occupied/cursor row.
        let full = FullView::new(1000);
        if !self
            .source
            .open_full_terminal(agent, full.selection.clone(), full.viewport)?
        {
            return Ok(None);
        }
        let mut term = Term::new(agent, mode);
        term.full = Some(full);
        Ok(Some(term))
    }
    pub(super) fn begin_full(&mut self) {
        let size = self.full_size();
        let Some(term) = self.term.as_mut() else {
            return;
        };
        if term.mode != TermMode::Live {
            return;
        }
        let Some(full) = term.full.as_mut() else {
            return;
        };
        if !full.ready || full.pending.is_some() {
            self.message = Some(self.lang.tr(Text::FullWaiting).into());
            return;
        }
        let Some(frame) = &full.data else {
            return;
        };
        let view_id = frame.view_id.clone();
        let generation = frame.frame.generation.clone();
        let id = full.id();
        let data = ClientTerminalControlData {
            request_id: id.clone(),
            instance_id: term.agent.clone(),
            view_id,
            generation,
            operation: ClientTerminalOperation::Acquire { size },
        };
        term.typing = false;
        full.expanded = true;
        full.pending = Some(Pending {
            id,
            size,
            acquire: true,
        });
        if let Err(error) = self.source.terminal_control(data) {
            self.full_send_error(error);
        }
    }
    fn full_send_error(&mut self, error: SourceError) {
        self.message = Some(error_text(&error));
        if let Some(term) = self.term.as_mut() {
            term.typing = false;
            if let Some(full) = &mut term.full {
                full.revoke();
            }
            if matches!(
                error,
                SourceError::Disconnected(_) | SourceError::Version(_)
            ) {
                term.mode = TermMode::Ended;
                term.retried = Instant::now();
            }
        }
        // Any failed enqueue while retaining an owner must retire that socket,
        // including a pending acquire whose grant could arrive later.
        self.source.close_terminal();
        if let Some(term) = self.term.as_mut() {
            term.mode = TermMode::Ended;
            term.retried = Instant::now();
        }
    }
    fn resize_full(&mut self) {
        let size = self.full_size();
        let readonly_rows = self.full_rows(false);
        let Some(term) = self.term.as_mut() else {
            return;
        };
        if term.mode != TermMode::Live {
            return;
        }
        let Some(full) = term.full.as_mut() else {
            return;
        };
        if !full.ready {
            return;
        }
        if full.expanded {
            if full.pending.is_none()
                && full
                    .data
                    .as_ref()
                    .is_some_and(|data| data.frame.size == size)
            {
                return;
            }
            term.typing = false;
            if full.pending.is_some() {
                return;
            }
            let (Some(attach), Some(frame)) = (&full.owner, &full.data) else {
                return;
            };
            let attach_id = attach.clone();
            let view_id = frame.view_id.clone();
            let generation = frame.frame.generation.clone();
            let id = full.id();
            full.pending = Some(Pending {
                id: id.clone(),
                size,
                acquire: false,
            });
            let data = ClientTerminalControlData {
                request_id: id,
                instance_id: term.agent.clone(),
                view_id,
                generation,
                operation: ClientTerminalOperation::Resize { attach_id, size },
            };
            if let Err(error) = self.source.terminal_control(data) {
                self.full_send_error(error);
            }
        } else {
            if let Some(data) = &full.data {
                if full.viewport.top.is_none() {
                    full.follow_top = data.frame.live_top
                        + crate::terminal::full::live_offset(&data.frame, readonly_rows) as u64;
                } else if full.viewport.rows != readonly_rows {
                    full.follow_top = full.follow_top.saturating_add_signed(
                        i64::from(full.viewport.rows) - i64::from(readonly_rows),
                    );
                }
            }
            let rows = if full.viewport.top.is_none() {
                1000
            } else {
                readonly_rows
            };
            let fit = TerminalSize {
                rows: readonly_rows,
                columns: size.columns.clamp(1, 1000),
            };
            if full.viewport.rows != rows || full.fit_requested != Some(fit) {
                self.select_full_viewport(full_view_top(self), rows);
            }
        }
    }
    pub fn scroll_terminal(&mut self, delta: i64) {
        let rows = self.full_rows(self.full_mode());
        let Some(full) = self.term.as_ref().and_then(|term| term.full.as_ref()) else {
            return;
        };
        let Some(frame) = &full.data else {
            return;
        };
        if frame.frame.alternate_screen {
            return;
        }
        let bottom = if full.expanded {
            frame.frame.live_top
        } else {
            full.follow_top.max(frame.frame.live_top)
        };
        let top = full.viewport.top.unwrap_or(bottom);
        let target = top
            .saturating_add_signed(delta)
            .clamp(frame.frame.history_oldest, bottom);
        let selection = (target < bottom).then_some(target);
        let rows = if selection.is_none() && !self.full_mode() {
            1000
        } else {
            rows
        };
        self.select_full_viewport(selection, rows);
    }
    fn select_full_viewport(&mut self, top: Option<u64>, rows: u16) {
        let fit = TerminalSize {
            rows: self.full_rows(false),
            columns: self.outer_size.columns.clamp(1, 1000),
        };
        let Some(term) = self.term.as_mut() else {
            return;
        };
        let Some(full) = term.full.as_mut() else {
            return;
        };
        if !full.ready || full.pending.is_some() {
            return;
        }
        let Some(frame) = &full.data else {
            return;
        };
        let view_id = frame.view_id.clone();
        let generation = frame.frame.generation.clone();
        let id = full.id();
        let viewport = TerminalViewport { top, rows };
        let request = TerminalViewportData {
            fit_size: (!full.expanded && full.fit_requested != Some(fit)).then_some(fit),
            request_id: id.clone(),
            instance_id: term.agent.clone(),
            view_id,
            generation,
            viewport,
        };
        match self.source.terminal_viewport(request) {
            Ok(()) => {
                full.selection = id;
                full.viewport = viewport;
                if !full.expanded {
                    full.fit_requested = Some(fit);
                }
            }
            Err(error) => self.full_send_error(error),
        }
    }
    pub(super) fn pump_full_terminal(&mut self) {
        if !self.pump_full_terminal_ready() {
            self.resize_full();
        }
    }
    /// Drain received updates between ticks; an empty mailbox has no effects.
    pub(crate) fn pump_full_terminal_ready(&mut self) -> bool {
        let events = self.source.poll_full_terminal();
        if events.is_empty() {
            return false;
        }
        for event in events {
            let size = self.full_size();
            let readonly_rows = self.full_rows(false);
            let Some(term) = self.term.as_mut() else {
                continue;
            };
            let Some(full) = term.full.as_mut() else {
                continue;
            };
            match event {
                FullTerminalEvent::Frame(data) => {
                    if data.instance_id != term.agent || data.request_id != full.selection {
                        continue;
                    }
                    if full.ready
                        && let Some(old) = &full.data
                    {
                        if data.view_id != old.view_id {
                            continue;
                        }
                        if data.frame.generation != old.frame.generation {
                            full.revoke();
                            term.typing = false;
                            term.mode = TermMode::Ended;
                            term.retried = Instant::now();
                            self.source.close_terminal();
                            continue;
                        }
                        if data.frame.revision < old.frame.revision {
                            continue;
                        }
                    }
                    if data.frame.viewport_clamped {
                        self.message = Some(self.lang.tr(Text::FullHistoryClamped).into());
                        if full.viewport.top.is_some() {
                            full.viewport.top = Some(data.frame.viewport_top);
                        }
                    }
                    if full.viewport.top.is_none() {
                        full.follow_top = data.frame.live_top
                            + crate::terminal::full::live_offset(&data.frame, readonly_rows) as u64;
                    } else if full
                        .data
                        .as_ref()
                        .is_some_and(|old| data.frame.live_top > old.frame.live_top)
                    {
                        full.follow_top = data.frame.live_top
                            + u64::from(data.frame.size.rows.saturating_sub(readonly_rows));
                    }
                    full.ready = true;
                    full.data = Some(*data);
                    if term.mode != TermMode::Stopped {
                        term.mode = TermMode::Live;
                    }
                }
                FullTerminalEvent::ControlAck(data) => {
                    if full.data.as_ref().is_some_and(|frame| {
                        frame.view_id == data.view_id && frame.frame.generation == data.generation
                    }) && full.input_pending.remove(&data.request_id)
                    {
                        continue;
                    }
                    let Some(pending) = full.pending.as_ref() else {
                        continue;
                    };
                    if data.request_id != pending.id || data.instance_id != term.agent {
                        continue;
                    }
                    let Some(old) = &full.data else {
                        continue;
                    };
                    if data.view_id != old.view_id || data.generation != old.frame.generation {
                        continue;
                    }
                    let Some(frame) = data.frame else {
                        self.full_send_error(SourceError::Disconnected(
                            "resize acknowledgement has no complete frame".into(),
                        ));
                        continue;
                    };
                    if frame.size != pending.size
                        || frame.cells.len() != usize::from(frame.size.rows)
                        || frame.generation != old.frame.generation
                        || frame.revision < old.frame.revision
                    {
                        self.full_send_error(SourceError::Disconnected(
                            "resize acknowledgement does not match the requested complete frame"
                                .into(),
                        ));
                        continue;
                    }
                    let TerminalControlState::Controlled { attach_id } = data.control else {
                        full.revoke();
                        term.typing = false;
                        continue;
                    };
                    full.owner = Some(attach_id);
                    full.lost_control = false;
                    full.pending = None;
                    full.selection = data.request_id.clone();
                    full.viewport = TerminalViewport {
                        top: None,
                        rows: frame.size.rows,
                    };
                    full.data = Some(ClientTerminalFrameData {
                        request_id: data.request_id,
                        instance_id: data.instance_id,
                        view_id: data.view_id,
                        frame,
                    });
                    term.typing = full.expanded
                        && size == full.data.as_ref().unwrap().frame.size
                        && term.mode == TermMode::Live;
                    if full.expanded && !term.typing {
                        self.resize_full();
                    }
                }
                FullTerminalEvent::ControlChanged(data) => {
                    if full.data.as_ref().is_some_and(|frame| {
                        frame.view_id == data.view_id && frame.frame.generation == data.generation
                    }) && data.control == TerminalControlState::ReadOnly
                    {
                        full.owner = None;
                        // A notice for the preceding owner can precede the
                        // acknowledgement of our explicit new Acquire.
                        if !full.pending.as_ref().is_some_and(|pending| pending.acquire) {
                            full.revoke();
                        }
                        full.lost_control = true;
                        term.typing = false;
                        self.message = Some(self.lang.tr(Text::FullControlLost).into());
                    }
                }
                FullTerminalEvent::Refused(data) => {
                    let was_input = data
                        .request_id
                        .as_ref()
                        .is_some_and(|id| full.input_pending.remove(id));
                    if let Some(id) = &data.request_id {
                        let relevant = id == &full.selection
                            || full
                                .pending
                                .as_ref()
                                .is_some_and(|pending| &pending.id == id)
                            || was_input;
                        if !relevant {
                            continue;
                        }
                    }
                    if was_input
                        && (data.code == error_code::INVALID_REQUEST || data.code == "busy")
                    {
                        // These refusals happen before I/O. The complete input
                        // was rejected; no ownership change or partial write.
                        self.message = Some(format!("{}: {}", data.code, data.message));
                        continue;
                    }
                    if full.pending.as_ref().is_some_and(|pending| pending.acquire)
                        && full.owner.is_none()
                        && (data.code == error_code::NOT_SUPPORTED
                            || data.code == error_code::FORBIDDEN)
                    {
                        full.revoke();
                        term.typing = false;
                        self.message = Some(format!("{}: {}", data.code, data.message));
                        continue;
                    }
                    let initial_unsupported = !full.ready && data.code == error_code::NOT_SUPPORTED;
                    if initial_unsupported {
                        let agent = term.agent.clone();
                        self.source.close_terminal();
                        match self.source.open_terminal(&agent) {
                            Ok(screen) => {
                                self.set_screen(screen);
                                if let Some(term) = &mut self.term {
                                    term.full = None;
                                    term.upgrade_required = true;
                                    term.typing = false;
                                }
                            }
                            Err(error) => self.full_send_error(error),
                        }
                        self.message = Some(self.lang.tr(Text::FullUpgrade).into());
                    } else {
                        full.revoke();
                        term.typing = false;
                        self.message = Some(format!("{}: {}", data.code, data.message));
                        // A refusal can leave a completed owner alive. Retire
                        // the entire scope so a late reply cannot re-enable it.
                        self.source.close_terminal();
                        term.mode = TermMode::Ended;
                        term.retried = Instant::now();
                    }
                }
                FullTerminalEvent::Closed(reason) => {
                    full.revoke();
                    full.ready = false;
                    term.typing = false;
                    term.mode = TermMode::Ended;
                    term.retried = Instant::now();
                    self.message = Some(reason);
                }
            }
        }
        self.resize_full();
        true
    }
    pub(super) fn full_key(&mut self, key: KeyEvent) {
        if stops_typing(&key) {
            let agent = self.term.as_ref().unwrap().agent.clone();
            self.source.close_terminal();
            self.reopen_terminal(&agent);
            self.message = None;
            return;
        }
        if self.term.as_ref().is_some_and(|term| term.typing) {
            let modes = self
                .term
                .as_ref()
                .and_then(|term| term.full.as_ref())
                .and_then(|full| full.data.as_ref())
                .unwrap()
                .frame
                .modes;
            self.send_full_bytes(crate::terminal::input::key_bytes(&key, modes));
        }
    }
    /// Dispatch native input without translating paste into a sequence of keys.
    pub fn event(&mut self, event: ratatui::crossterm::event::Event) {
        use ratatui::crossterm::event::Event;
        match event {
            Event::Key(key) => self.key(key),
            Event::Resize(columns, rows) => self.resize(columns, rows),
            Event::Mouse(event) => self.mouse(event),
            Event::Paste(text) => self.paste(&text),
            Event::FocusGained => self.focus(true),
            Event::FocusLost => self.focus(false),
        }
    }
    fn refresh_input_state(&mut self) {
        if self.is_connected() {
            self.pull();
            self.pump_terminal();
        }
    }
    pub fn paste(&mut self, text: &str) {
        self.refresh_input_state();
        if !self.is_connected() || self.finder.is_some() {
            return;
        }
        let Some(full) = self
            .term
            .as_ref()
            .filter(|term| term.typing)
            .and_then(|term| term.full.as_ref())
        else {
            return;
        };
        let Some(data) = &full.data else {
            return;
        };
        // Reject known oversized input before copying or base64 allocation.
        // The writer checks the exact serialized request as a whole as well.
        if text.len() > agend_core::protocol::holder::MAX_REQUEST_LINE {
            self.message = Some(self.lang.tr(Text::FullPasteTooLarge).into());
            return;
        }
        let bytes = if data.frame.modes.bracketed_paste {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            bytes
        } else {
            text.as_bytes().to_vec()
        };
        self.send_full_bytes(bytes);
    }
    pub fn mouse(&mut self, mut event: ratatui::crossterm::event::MouseEvent) {
        use agend_core::protocol::terminal::TerminalMouseTracking;
        use ratatui::crossterm::event::MouseEventKind;
        self.refresh_input_state();
        if !self.is_connected() || self.finder.is_some() {
            return;
        }
        let Some(term) = &self.term else {
            return;
        };
        let Some(full) = &term.full else {
            return;
        };
        let Some(data) = &full.data else {
            return;
        };
        let top = u16::from(!full.expanded);
        let height = self
            .outer_size
            .rows
            .saturating_sub(top + 1)
            .min(data.frame.cells.len() as u16);
        if !full.ready
            || event.column >= self.outer_size.columns.min(data.frame.size.columns)
            || event.row < top
            || event.row - top >= height
        {
            return;
        }
        let wheel = match event.kind {
            MouseEventKind::ScrollUp => Some(-3),
            MouseEventKind::ScrollDown => Some(3),
            _ => None,
        };
        let local = !term.typing
            || full.viewport.top.is_some()
            || data.frame.modes.mouse_tracking == TerminalMouseTracking::None
            || event.modifiers.contains(KeyModifiers::SHIFT);
        if local {
            if let Some(delta) = wheel {
                self.scroll_terminal(delta);
            }
            return;
        }
        event.row -= top;
        let bytes = crate::terminal::input::mouse_bytes(event, data.frame.modes);
        self.send_full_bytes(bytes);
    }
    pub fn focus(&mut self, gained: bool) {
        self.refresh_input_state();
        if !self.is_connected() || self.finder.is_some() {
            return;
        }
        if self
            .term
            .as_ref()
            .and_then(|term| term.full.as_ref())
            .and_then(|full| full.data.as_ref())
            .is_some_and(|data| data.frame.modes.focus_reporting)
        {
            self.send_full_bytes(if gained { b"\x1b[I" } else { b"\x1b[O" }.to_vec());
        }
    }
    pub(super) fn send_full_bytes(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let Some(term) = self.term.as_mut() else {
            return;
        };
        if !term.typing {
            return;
        }
        let Some(full) = &mut term.full else {
            return;
        };
        if full.input_pending.len() >= 64 {
            self.message = Some(self.lang.tr(Text::FullInputBusy).into());
            return;
        }
        let (Some(attach), Some(frame)) = (&full.owner, &full.data) else {
            return;
        };
        let attach_id = attach.clone();
        let view_id = frame.view_id.clone();
        let generation = frame.frame.generation.clone();
        let id = full.id();
        let data = ClientTerminalControlData {
            request_id: id.clone(),
            instance_id: term.agent.clone(),
            view_id,
            generation,
            operation: agend_client::terminal::input_operation(&attach_id, &bytes),
        };
        match self.source.terminal_control(data) {
            Ok(()) => {
                full.input_pending.insert(id);
            }
            Err(SourceError::Rejected { code, message }) => {
                self.message = Some(format!("{code}: {message}"))
            }
            Err(error) => self.full_send_error(error),
        }
    }
}
fn full_view_top(app: &App) -> Option<u64> {
    app.term
        .as_ref()
        .and_then(|term| term.full.as_ref())
        .and_then(|full| full.viewport.top)
}
