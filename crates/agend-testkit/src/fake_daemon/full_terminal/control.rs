//! Fake server ownership policy; the producer rechecks at actual operations.
use super::*;

impl Actor {
    pub(super) fn control(&mut self, scope: Scope, data: ClientTerminalControlData) {
        self.cleanup();
        let id = data.request_id.clone();
        if !self.valid_view(&scope, &data.view_id, &data.generation) {
            scope.fail(
                Some(id),
                error_code::STALE_TERMINAL,
                "view or generation changed",
            );
            return;
        }
        let operation = match data.operation {
            ClientTerminalOperation::Acquire { size } => TerminalControlOperation::Acquire {
                attach_id: self.token("attach"),
                size,
            },
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
                    .is_none_or(|o| o.view != data.view_id || o.attach != *attach)
                {
                    scope.fail(
                        Some(id),
                        error_code::CONTROL_LOST,
                        "this view no longer controls the PTY",
                    );
                    return;
                }
                match other {
                    ClientTerminalOperation::Resize { attach_id, size } => {
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
        if let TerminalControlOperation::Acquire { size, .. }
        | TerminalControlOperation::Resize { size, .. } = &operation
            && !size.is_valid()
        {
            scope.fail(Some(id), error_code::INVALID_SIZE, "invalid PTY size");
            return;
        }
        let acquire = matches!(operation, TerminalControlOperation::Acquire { .. });
        let released = matches!(operation, TerminalControlOperation::Release { .. });
        let attach = operation.attach_id().to_owned();
        match self.producer.control(TerminalControlRequest {
            request_id: id.clone(),
            generation: data.generation.clone(),
            operation,
        }) {
            Err(error) => scope.fail(Some(id), &error.code, &error.message),
            Ok(reply) => {
                // An in-flight write/resize finishes before a subsequent grant.
                // A grant orphaned by EOF is released immediately, never replayed.
                if acquire {
                    if let Some(old) = self.owner.take()
                        && let Some(view) = self.views.get(&old.view)
                    {
                        view.scope.publish(ClientResponse::TerminalControlChanged {
                            data: TerminalControlChangedData {
                                instance_id: self.instance.clone(),
                                view_id: old.view,
                                generation: old.generation,
                                control: TerminalControlState::ReadOnly,
                                reason: "another window acquired control".into(),
                            },
                        });
                    }
                    self.owner = Some(Owner {
                        view: data.view_id.clone(),
                        attach: attach.clone(),
                        generation: data.generation.clone(),
                    });
                } else if released {
                    self.owner = None;
                }
                if !self.valid_view(&scope, &data.view_id, &data.generation) {
                    self.cleanup();
                    return;
                }
                if let Some(frame) = &reply.frame {
                    lock(&scope.0.frame).take();
                    self.size = Some(frame.size);
                    let view = self.views.get_mut(&data.view_id).unwrap();
                    view.viewport = TerminalViewport {
                        top: None,
                        rows: frame.size.rows,
                    };
                    view.request_id = id.clone();
                    view.revision = frame.revision;
                }
                scope.publish(ClientResponse::TerminalControlAck {
                    data: ClientTerminalControlAck {
                        request_id: id,
                        instance_id: self.instance.clone(),
                        view_id: data.view_id,
                        generation: reply.generation,
                        control: if released {
                            TerminalControlState::ReadOnly
                        } else {
                            TerminalControlState::Controlled { attach_id: attach }
                        },
                        frame: reply.frame,
                    },
                });
                self.capture();
            }
        }
    }
}
