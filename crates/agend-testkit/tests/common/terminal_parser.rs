//! Fake daemon fixture backed by the actual holder parser, never hand cells.
use agend_core::protocol::client::{AgentState, InstanceView};
use agend_core::protocol::terminal::*;
use agend_core::traits::TerminalProducer;
use agend_holder::screen::{ReplySink, Screen};
use agend_testkit::contract::terminal::FullTerminalFixture;
use agend_testkit::fake_daemon::FakeDaemon;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::sync::{Arc, Mutex};

pub const ID: &str = "g11-contract";
#[allow(dead_code)] // Used by the native fixture in agend, not testkit.
pub const BURST: &str = "burst";
#[allow(dead_code)] // Used by the native fixture in agend, not testkit.
pub const SCRIPT: &str = "stty -echo; printf 'READY\\r\\n'; while IFS= read -r line; do printf '%s\\n' \"$line\" >> delivered; if [ \"$line\" = burst ]; then i=0; while [ \"$i\" -lt 120 ]; do printf 'number-%s\\r\\n' \"$i\"; i=$((i+1)); done; printf 'FINAL-DIRTY\\r\\n'; else printf 'GOT:%s\\r\\n' \"$line\"; fi; done";
struct State {
    screen: Screen,
    owner: Option<String>,
    input: Vec<u8>,
}
#[derive(Clone)]
pub struct Parser(Arc<Mutex<State>>);
impl Default for Parser {
    fn default() -> Self {
        let mut screen = Screen::new(50, 200, ReplySink::default());
        screen.process(b"READY\r\n");
        Self(Arc::new(Mutex::new(State {
            screen,
            owner: None,
            input: Vec::new(),
        })))
    }
}
fn failure(id: &str, code: &str, message: &str) -> TerminalOperationError {
    TerminalOperationError {
        request_id: id.into(),
        code: code.into(),
        message: message.into(),
    }
}
impl Parser {
    #[allow(dead_code)] // Real parser stimuli for TUI renderer and input-mode cases.
    pub fn feed(&self, bytes: &[u8]) {
        self.0.lock().unwrap().screen.process(bytes);
    }

    #[allow(dead_code)] // Specific lifecycle regression, not all shared suites.
    pub fn restart(&self) {
        let mut state = self.0.lock().unwrap();
        let (rows, columns) = state.screen.size();
        state.screen = Screen::new(rows, columns, ReplySink::default());
        state.screen.process(b"NEW-GENERATION\r\n");
        state.owner = None;
    }
    #[allow(dead_code)] // Binary mouse reports may not be valid UTF-8.
    pub fn received_bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().input.clone()
    }
    pub fn received(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap().input).into_owned()
    }
    pub fn output(&self) {
        let mut state = self.0.lock().unwrap();
        for i in 0..120 {
            state.screen.process(format!("number-{i}\r\n").as_bytes());
        }
        state.screen.process(b"FINAL-DIRTY\r\n");
    }
    fn input(state: &mut State, id: &str, bytes: &str) -> Result<(), TerminalOperationError> {
        let bytes = STANDARD
            .decode(bytes)
            .map_err(|_| failure(id, "bad_request", "invalid base64"))?;
        state.input.extend(&bytes);
        if bytes == b"burst\n" {
            for i in 0..120 {
                state.screen.process(format!("number-{i}\r\n").as_bytes());
            }
            state.screen.process(b"FINAL-DIRTY\r\n");
        } else {
            state.screen.process(b"GOT:");
            state.screen.process(&bytes);
        }
        Ok(())
    }
}
impl TerminalProducer for Parser {
    fn frame(
        &mut self,
        request: TerminalFrameRequest,
    ) -> Result<TerminalFrameData, TerminalOperationError> {
        let frame = self
            .0
            .lock()
            .unwrap()
            .screen
            .sampled_frame(request.viewport)
            .map_err(|error| failure(&request.request_id, error.code(), error.message()))?;
        Ok(TerminalFrameData {
            request_id: request.request_id,
            frame,
        })
    }
    fn legacy_input(&mut self, bytes: String) -> Result<(), TerminalOperationError> {
        let mut state = self.0.lock().unwrap();
        if state.owner.is_some() {
            return Err(failure("", "control_required", "PTY has an owner"));
        }
        Self::input(&mut state, "", &bytes)
    }
    fn control(
        &mut self,
        request: TerminalControlRequest,
    ) -> Result<TerminalControlData, TerminalOperationError> {
        let mut state = self.0.lock().unwrap();
        let id = request.request_id;
        if request.generation != state.screen.generation() {
            return Err(failure(&id, "stale_terminal", "generation changed"));
        }
        if !matches!(request.operation, TerminalControlOperation::Acquire { .. })
            && state.owner.as_deref() != Some(request.operation.attach_id())
        {
            return Err(failure(&id, "control_lost", "owner changed"));
        }
        let mut data = TerminalControlData {
            request_id: id.clone(),
            generation: request.generation,
            attach_id: Some(request.operation.attach_id().into()),
            frame: None,
        };
        match request.operation {
            TerminalControlOperation::DaemonKey { .. } => {
                return Err(failure(
                    &id,
                    "not_supported",
                    "operator fixture has no daemon key transport",
                ));
            }
            TerminalControlOperation::Acquire { attach_id, size }
            | TerminalControlOperation::Resize { attach_id, size } => {
                Screen::validate_frame_size(size)
                    .map_err(|error| failure(&id, error.code(), error.message()))?;
                state.screen.resize(size.rows, size.columns);
                data.frame = Some(
                    state
                        .screen
                        .frame(TerminalViewport {
                            top: None,
                            rows: size.rows,
                        })
                        .map_err(|error| failure(&id, error.code(), error.message()))?,
                );
                state.owner = Some(attach_id);
            }
            TerminalControlOperation::Input { bytes_base64, .. } => {
                Self::input(&mut state, &id, &bytes_base64)?
            }
            TerminalControlOperation::Release { .. } => {
                state.owner = None;
                data.attach_id = None;
            }
        }
        Ok(data)
    }
}
pub struct Fake {
    pub daemon: FakeDaemon,
    pub parser: Parser,
}
impl Default for Fake {
    fn default() -> Self {
        let daemon = FakeDaemon::start().unwrap();
        daemon.set_instance(InstanceView {
            program: None,
            instance_id: ID.into(),
            team_id: "general".into(),
            backend: "claude".into(),
            state: AgentState::Idle,
            working_directory: None,
        });
        let parser = Parser::default();
        daemon.set_terminal_producer(ID, parser.clone()).unwrap();
        Self { daemon, parser }
    }
}
impl FullTerminalFixture for Fake {
    fn socket(&self) -> std::path::PathBuf {
        self.daemon.socket_path().to_path_buf()
    }
    fn instance(&self) -> String {
        ID.into()
    }
    fn received(&self) -> String {
        self.parser.received()
    }
    fn output(&mut self) {
        self.parser.output();
    }
}
