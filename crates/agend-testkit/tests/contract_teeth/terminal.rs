//! Deliberate wire corruptions over a real parser-backed fake server.
#[path = "../common/terminal_parser.rs"]
mod parser;
use super::Mutant;
use agend_testkit::contract::Report;
use agend_testkit::contract::client::proxy::{Direction, Options, Proxy};
use agend_testkit::contract::terminal::{self, FullTerminalFixture};
use serde_json::Value;
use std::sync::Arc;

struct Broken {
    proxy: Proxy,
    fake: parser::Fake,
}
impl Broken {
    fn new(rule: &'static str) -> Self {
        let fake = parser::Fake::default();
        let transform = Arc::new(move |_: &mut _, direction, line: String| {
            if direction != Direction::ToClient {
                return vec![line];
            }
            let Ok(mut v) = serde_json::from_str::<Value>(&line) else {
                return vec![line];
            };
            let tag = v["type"].as_str().unwrap_or_default().to_owned();
            match rule {
                "CLP-23" if tag == "terminal_control_ack" && v["data"]["frame"].is_object() => {
                    let rows = v["data"]["frame"]["size"]["rows"].as_u64().unwrap();
                    v["data"]["frame"]["size"]["rows"] = (rows + 1).into();
                }
                "CLP-24" if tag == "terminal_control_changed" => return vec![],
                "CLP-25" if tag == "error" && v["data"]["code"] == "forbidden" => {
                    v["data"]["code"] = "invalid_request".into()
                }
                "CLP-26" if tag == "error" && v["data"]["code"] == "control_lost" => {
                    v["data"]["code"] = "stale_terminal".into()
                }
                "CLP-27" if tag == "terminal_frame" && v["data"]["request_id"] == "pinned" => {
                    v["data"]["frame"]["viewport_top"] = 0.into()
                }
                "CLP-28" if tag == "error" && v["data"]["code"] == "not_supported" => {
                    v["data"]["code"] = "no_terminal".into()
                }
                _ => (),
            }
            vec![v.to_string()]
        });
        let proxy = Proxy::start(
            fake.daemon.socket_path().to_path_buf(),
            Options::rewrite(transform),
        )
        .unwrap();
        Self { proxy, fake }
    }
}
impl FullTerminalFixture for Broken {
    fn socket(&self) -> std::path::PathBuf {
        self.proxy.socket().to_path_buf()
    }
    fn instance(&self) -> String {
        self.fake.instance()
    }
    fn received(&self) -> String {
        self.fake.received()
    }
    fn output(&mut self) {
        self.fake.output();
    }
}
fn run(name: &str, rule: &'static str) -> Report {
    terminal::run_rules(name, &[rule], || Box::new(Broken::new(rule)))
}
pub fn mutants() -> Vec<Mutant> {
    vec![
        Mutant {
            rule: "CLP-23",
            name: "GrantBeforeMatchingFrame",
            run: |n| run(n, "CLP-23"),
        },
        Mutant {
            rule: "CLP-24",
            name: "HidesLostControl",
            run: |n| run(n, "CLP-24"),
        },
        Mutant {
            rule: "CLP-25",
            name: "IgnoresTerminalCallerPriority",
            run: |n| run(n, "CLP-25"),
        },
        Mutant {
            rule: "CLP-26",
            name: "ForgetsOldOwnerRefusal",
            run: |n| run(n, "CLP-26"),
        },
        Mutant {
            rule: "CLP-27",
            name: "UnpinsHistoryViewport",
            run: |n| run(n, "CLP-27"),
        },
        Mutant {
            rule: "CLP-28",
            name: "ForgetsOldTerminalCapability",
            run: |n| run(n, "CLP-28"),
        },
    ]
}
