//! Client protocol mutants: the fake daemon behind a proxy that breaks one
//! rule (`contract::client::proxy`). Each runs only the cases of its rule.

use std::sync::Arc;

use agend_core::protocol::client::SUPPORTED_VERSIONS;
use agend_testkit::contract::Report;
use agend_testkit::contract::client::proxy::{ConnState, Direction, Options, Transform};
use agend_testkit::contract::client::{self, FakeDaemonFixture, Proxied};
use serde_json::{Value, json};

use super::Mutant;

fn with(rule: &'static str, name: &'static str, options: fn() -> Options) -> Report {
    client::run_rules(name, &[rule], || {
        Proxied::new(FakeDaemonFixture::new(), options())
    })
}

/// A transform over parsed lines: `f` returns the lines to pass on.
fn parsed(f: fn(&mut ConnState, Direction, Value) -> Vec<Value>) -> Options {
    let transform: Transform =
        Arc::new(
            move |state, direction, line| match serde_json::from_str::<Value>(&line) {
                Ok(value) => f(state, direction, value)
                    .into_iter()
                    .map(|v| v.to_string())
                    .collect(),
                Err(_) => vec![line],
            },
        );
    Options::rewrite(transform)
}

fn identity() -> Transform {
    Arc::new(|_, _, line| vec![line])
}

fn is_error(v: &Value, code: &str) -> bool {
    v["type"] == "error" && v["data"]["code"] == code
}

pub fn mutants() -> Vec<Mutant> {
    vec![
        // CLP-1: the proxy says hello for the client, so any first line works.
        Mutant {
            rule: "CLP-1",
            name: "NoHelloRequired",
            run: |name| {
                with("CLP-1", name, || {
                    parsed(|state, direction, v| match direction {
                        Direction::ToServer if state.counter == 0 => {
                            state.counter = 1;
                            if v["type"] == "hello" {
                                return vec![v];
                            }
                            state.flag = true;
                            let hello = serde_json::to_value(
                                agend_core::protocol::client::ClientRequest::hello(),
                            )
                            .unwrap();
                            vec![hello, v]
                        }
                        Direction::ToClient if state.flag && v["type"] == "hello" => {
                            state.flag = false;
                            vec![]
                        }
                        _ => vec![v],
                    })
                })
            },
        },
        // CLP-2: any major is rewritten to the supported one.
        Mutant {
            rule: "CLP-2",
            name: "AcceptsAnyMajor",
            run: |name| {
                with("CLP-2", name, || {
                    parsed(|_, direction, mut v| {
                        if direction == Direction::ToServer && v["type"] == "hello" {
                            v["data"]["supported"] =
                                serde_json::to_value(SUPPORTED_VERSIONS).unwrap();
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-3: the first event after subscribing is lost.
        Mutant {
            rule: "CLP-3",
            name: "SkipsFirstEventAfterAsOf",
            run: |name| {
                with("CLP-3", name, || {
                    parsed(|state, direction, v| {
                        if direction == Direction::ToClient && v["type"] == "event" && !state.flag {
                            state.flag = true;
                            return vec![];
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-4 (the negative check of P9): event ids counted from 1 again.
        Mutant {
            rule: "CLP-4",
            name: "IdsFromOne",
            run: |name| client::run_rules(name, &["CLP-4"], client::fake_with_ids_from_one),
        },
        // CLP-4: `event_gap` never reaches the client.
        Mutant {
            rule: "CLP-4",
            name: "HidesEventGap",
            run: |name| {
                with("CLP-4", name, || {
                    parsed(|_, direction, v| {
                        if direction == Direction::ToClient && is_error(&v, "event_gap") {
                            return vec![];
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-5: no cursor is taken as cursor 0 (no 1.0 replay).
        Mutant {
            rule: "CLP-5",
            name: "NoneBecomesZero",
            run: |name| {
                with("CLP-5", name, || {
                    parsed(|_, direction, mut v| {
                        if direction == Direction::ToServer
                            && v["type"] == "subscribe_events"
                            && v["data"]["after_event_id"].is_null()
                        {
                            v["data"]["after_event_id"] = json!(0);
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-6: after an unknown request nothing more reaches the client.
        Mutant {
            rule: "CLP-6",
            name: "GoesSilentOnUnknown",
            run: |name| {
                with("CLP-6", name, || {
                    parsed(|state, direction, v| {
                        if direction == Direction::ToClient {
                            if state.flag {
                                return vec![];
                            }
                            if is_error(&v, "unknown_request") {
                                state.flag = true;
                            }
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-7: the second client gets events in pairs swapped.
        Mutant {
            rule: "CLP-7",
            name: "ReordersForSecondClient",
            run: |name| {
                with("CLP-7", name, || {
                    let transform: Transform = Arc::new(|state, direction, line| {
                        if direction != Direction::ToClient
                            || state.number != 2
                            || !line.contains(r#""type":"event""#)
                        {
                            return vec![line];
                        }
                        match state.held.take() {
                            None => {
                                state.held = Some(line);
                                vec![]
                            }
                            Some(first) => vec![line, first],
                        }
                    });
                    Options::rewrite(transform)
                })
            },
        },
        // CLP-8: a proxy that buffers without limit hides a slow client.
        Mutant {
            rule: "CLP-8",
            name: "NeverDropsLaggers",
            run: |name| {
                with("CLP-8", name, || Options {
                    transform: identity(),
                    keep_client_open: false,
                    unbounded: true,
                })
            },
        },
        // CLP-9: the client's connection outlives the server's.
        Mutant {
            rule: "CLP-9",
            name: "KeepsOldConnection",
            run: |name| {
                with("CLP-9", name, || Options {
                    transform: identity(),
                    keep_client_open: true,
                    unbounded: false,
                })
            },
        },
        // CLP-10: a request that is not supported is answered as accepted.
        Mutant {
            rule: "CLP-10",
            name: "AcceptsUnsupported",
            run: |name| {
                with("CLP-10", name, || {
                    parsed(|_, direction, v| {
                        if direction == Direction::ToClient && is_error(&v, "not_supported") {
                            return vec![json!({
                                "type": "command_result",
                                "data": {"request_id": "clp", "result": {"result": "accepted"}}
                            })];
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-11: the caller is dropped from hello, so agents are operators.
        Mutant {
            rule: "CLP-11",
            name: "AgentMayResolve",
            run: |name| {
                with("CLP-11", name, || {
                    parsed(|_, direction, mut v| {
                        if direction == Direction::ToServer
                            && v["type"] == "hello"
                            && let Some(data) = v["data"].as_object_mut()
                        {
                            data.remove("caller");
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-11: `attention_resolved` never reaches the client.
        Mutant {
            rule: "CLP-11",
            name: "HidesResolvedEvent",
            run: |name| {
                with("CLP-11", name, || {
                    parsed(|_, direction, v| {
                        if direction == Direction::ToClient
                            && v["data"]["event"]["event"] == "attention_resolved"
                        {
                            return vec![];
                        }
                        vec![v]
                    })
                })
            },
        },
        // CLP-12: the screen is dropped; the terminal starts with bytes.
        Mutant {
            rule: "CLP-12",
            name: "DropsSnapshot",
            run: |name| {
                with("CLP-12", name, || {
                    parsed(|_, direction, v| {
                        if direction == Direction::ToClient && v["type"] == "terminal_snapshot" {
                            return vec![];
                        }
                        vec![v]
                    })
                })
            },
        },
    ]
}
