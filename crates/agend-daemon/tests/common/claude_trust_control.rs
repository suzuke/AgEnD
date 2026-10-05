//! Bounded operator probe, not production startup automation. It only moves
//! from the captured No selection to Yes, then confirms that exact selection.
//! A separate opt-in can confirm the recorded local agend channel warning once.
use super::*;

#[derive(Default)]
pub struct Progress {
    pub started: usize,
    pub completed: usize,
}
impl Progress {
    pub fn decide(
        &mut self,
        frame: &TerminalFrame,
        workspace: &Path,
        accept_development_channels: bool,
    ) -> Result<Option<(&'static str, &'static str)>, String> {
        if self.started == 3 || self.started == 2 && !accept_development_channels {
            return Ok(None);
        }
        let text = frame
            .cells
            .iter()
            .map(|row| {
                row.iter()
                    .filter(|cell| cell.width != 0 && !cell.leading_spacer)
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let lines = text.lines().map(str::trim).collect::<Vec<_>>();
        let nonempty = lines
            .iter()
            .copied()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if self.started == 2 {
            // Match the complete recorded menu, ignoring only whitespace.
            // Presence of a selected-local line cannot disambiguate a second
            // selection or a decoy menu. Any extra content remains unknown.
            let recorded = match frame.size.columns {
                100 => include_str!(
                    "../../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-100x24.txt"
                ),
                140 => include_str!(
                    "../../../agend-core/tests/fixtures/screens/claude-2.1.284-development-channels-140x24.txt"
                ),
                _ => return Ok(None),
            };
            if self.completed == 2
                && normalized == recorded.split_whitespace().collect::<Vec<_>>().join(" ")
            {
                self.started = 3;
                return Ok(Some(("development-enter", "DQ==")));
            }
            return Ok(None);
        }
        let workspace = workspace.display().to_string();
        // This is a newly created, private workspace. Never accept another path
        // by a prefix match or by mentioning our path elsewhere on the screen.
        if nonempty
            .iter()
            .filter(|line| **line == "Accessing workspace:")
            .count()
            != 1
            || !nonempty
                .windows(2)
                .any(|pair| pair == ["Accessing workspace:", workspace.as_str()])
            || !normalized
                .contains("Quick safety check: Is this a project you created or one you trust?")
            || !normalized.contains("Claude Code'll be able to read, edit, and execute files here.")
            || !normalized.contains("Enter to confirm · Esc to cancel")
        {
            return Ok(None);
        }
        let no = lines.contains(&"❯ No, exit") && lines.contains(&"Yes, I trust this folder");
        let yes = lines.contains(&"No, exit") && lines.contains(&"❯ Yes, I trust this folder");
        match (self.started, no, yes) {
            (0, true, false) => {
                self.started = 1;
                Ok(Some(("down", "G1tC")))
            }
            (1, false, true) if self.completed == 1 => {
                self.started = 2;
                Ok(Some(("enter", "DQ==")))
            }
            (0, false, true) => Err("unexpected initial trust selection; no input sent".into()),
            _ => Ok(None),
        }
    }
}

pub struct Grant<'a> {
    pub instance: &'a str,
    pub view: &'a str,
    pub attach: &'a str,
}
impl Grant<'_> {
    pub fn send(
        &self,
        client: &mut ProbeClient,
        frame: &TerminalFrame,
        key: &str,
        bytes: &str,
        deadline: Instant,
    ) -> Result<Vec<TerminalFrame>, String> {
        let id = format!("capture-trust-{key}");
        client
            .send(&ClientRequest::TerminalControl {
                data: ClientTerminalControlData {
                    request_id: id.clone(),
                    instance_id: self.instance.into(),
                    view_id: self.view.into(),
                    generation: frame.generation.clone(),
                    operation: ClientTerminalOperation::Input {
                        attach_id: self.attach.into(),
                        bytes_base64: bytes.into(),
                    },
                },
            })
            .map_err(|_| "trust input write failed; outcome unknown; no replay")?;
        let deadline = deadline.min(Instant::now() + Duration::from_secs(2));
        let mut frames = Vec::new();
        loop {
            match next(client, deadline)? {
                ClientResponse::TerminalControlAck { data } if data.request_id == id => {
                    if data.instance_id != self.instance
                        || data.view_id != self.view
                        || data.generation != frame.generation
                        || !matches!(data.control, TerminalControlState::Controlled { attach_id } if attach_id == self.attach)
                    {
                        return Err(
                            "trust input identity changed; outcome unknown; no replay".into()
                        );
                    }
                    return Ok(frames);
                }
                ClientResponse::TerminalFrame { data } => {
                    if data.instance_id != self.instance
                        || data.view_id != self.view
                        || data.frame.generation != frame.generation
                        || data.frame.size != frame.size
                        || frames.len() >= 512
                    {
                        return Err("trust capture terminal changed; no replay".into());
                    }
                    frames.push(data.frame);
                }
                ClientResponse::Error { .. } | ClientResponse::TerminalControlChanged { .. } => {
                    return Err("trust input refused or control lost; no replay".into());
                }
                _ => {
                    if Instant::now() >= deadline {
                        return Err("trust input completion unknown; no replay".into());
                    }
                }
            }
        }
    }
}
