//! Actual PTY acknowledgements and client-scoped owner transitions.
use super::actor::Actor;
use super::*;

impl Actor {
    pub(super) async fn control(&mut self, scope: ReplyScope, data: ClientTerminalControlData) {
        let id = &data.request_id;
        if !self.live() {
            scope.send(error(
                Some(id.clone()),
                "no_terminal",
                "instance has no live terminal",
            ));
            return;
        }
        if let Err(message) = self.valid_view(&scope, &data.view_id, &data.generation) {
            scope.send(error(Some(id.clone()), "stale_terminal", message));
            return;
        }
        let acquire = matches!(data.operation, ClientTerminalOperation::Acquire { .. });
        let sized = matches!(
            data.operation,
            ClientTerminalOperation::Acquire { .. } | ClientTerminalOperation::Resize { .. }
        );
        if matches!(
            data.operation,
            ClientTerminalOperation::Acquire { .. } | ClientTerminalOperation::Input { .. }
        ) && self
            .fleet
            .instance(&self.instance)
            .is_some_and(|v| v.backend == "codex")
        {
            scope.send(error(Some(id.clone()), "not_supported", CODEX_INPUT));
            return;
        }
        let operation = match data.operation {
            ClientTerminalOperation::Acquire { size } => {
                if !valid_size(size) {
                    scope.send(error(
                        Some(id.clone()),
                        "invalid_size",
                        "PTY size is invalid or its complete frame exceeds 8 MiB",
                    ));
                    return;
                }
                let Some(hub) = self.hub.upgrade() else {
                    return;
                };
                TerminalControlOperation::Acquire {
                    attach_id: token(&hub, "attach"),
                    size,
                }
            }
            other => {
                let attach = match &other {
                    ClientTerminalOperation::Resize { attach_id, .. }
                    | ClientTerminalOperation::Input { attach_id, .. }
                    | ClientTerminalOperation::Release { attach_id } => attach_id,
                    _ => unreachable!(),
                };
                if self
                    .owner
                    .as_ref()
                    .is_none_or(|o| o.view_id != data.view_id || o.attach_id != *attach)
                {
                    scope.send(error(
                        Some(id.clone()),
                        "control_lost",
                        "this view no longer controls the PTY; acquire control again",
                    ));
                    return;
                }
                match other {
                    ClientTerminalOperation::Resize { attach_id, size } => {
                        if !valid_size(size) {
                            scope.send(error(
                                Some(id.clone()),
                                "invalid_size",
                                "PTY size is invalid or its complete frame exceeds 8 MiB",
                            ));
                            return;
                        }
                        TerminalControlOperation::Resize { attach_id, size }
                    }
                    ClientTerminalOperation::Input {
                        attach_id,
                        bytes_base64,
                    } => TerminalControlOperation::Input {
                        attach_id,
                        bytes_base64,
                    },
                    ClientTerminalOperation::Release { attach_id } => {
                        TerminalControlOperation::Release { attach_id }
                    }
                    _ => unreachable!(),
                }
            }
        };
        let attach = operation.attach_id().to_owned();
        let releasing = matches!(operation, TerminalControlOperation::Release { .. });
        let result = self
            .connection
            .as_ref()
            .unwrap()
            .control(data.generation.clone(), operation)
            .await;
        match result {
            Ok(ack) => {
                if acquire && let Some(owner) = self.owner.take() {
                    self.changed(
                        &owner,
                        "another window acquired control; press i to acquire again",
                    );
                }
                if releasing {
                    self.owner = None;
                } else if acquire {
                    self.owner = Some(Owner {
                        view_id: data.view_id.clone(),
                        attach_id: attach.clone(),
                    });
                }
                // The real operation may complete after socket EOF. Finish it,
                // release any new owner, and never publish an orphaned grant.
                if !scope.alive.load(Ordering::SeqCst)
                    || self.views.get(&data.view_id).is_none_or(|v| !v.live())
                {
                    self.cleanup().await;
                    return;
                }
                let view = self.views.get_mut(&data.view_id).unwrap();
                if sized {
                    let Some(frame) = &ack.frame else {
                        self.invalidate("holder did not acknowledge a complete resized frame");
                        return;
                    };
                    view.size = frame.size;
                    view.revision = frame.revision;
                    view.selection = id.clone();
                    view.viewport = TerminalViewport {
                        top: None,
                        rows: frame.size.rows,
                    };
                    view.frames.send_replace(None);
                    let size = frame.size;
                    for view in self.views.values_mut() {
                        view.size = size;
                    }
                    self.last_sample = Some(Instant::now());
                    self.dirty = true;
                }
                scope.send(ClientResponse::TerminalControlAck {
                    data: ClientTerminalControlAck {
                        request_id: id.clone(),
                        instance_id: self.instance.clone(),
                        view_id: data.view_id,
                        generation: ack.generation,
                        control: if releasing {
                            TerminalControlState::ReadOnly
                        } else {
                            TerminalControlState::Controlled { attach_id: attach }
                        },
                        frame: ack.frame,
                    },
                });
            }
            Err(e) => {
                scope.send(error(Some(id.clone()), &e.code, e.message.clone()));
                // Local permission/size errors never reach this branch. A
                // native resize/write failure can already have changed the PTY.
                if !matches!(
                    e.code.as_str(),
                    "pty_busy" | "bad_request" | "invalid_request" | "request_too_large"
                ) {
                    if let Some(owner) = &self.owner {
                        self.changed(owner, "terminal operation failed; acquire control again");
                    }
                    self.release().await;
                    if self.connection.as_ref().is_some_and(|c| !c.is_current()) {
                        self.invalidate(&e.message);
                    }
                }
            }
        }
    }
}
