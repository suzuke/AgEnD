//! `cargo xtask accept <gate>`: acceptance checks for one build gate
//! (docs/ROADMAP.md).
//!
//! For gate 1 run workspace formatting and clippy, test agend-core and its
//! protocol compatibility contract, run check-deps, then show the core demo.
//! For gate 2 run the per-crate checks (the testkit tests include every
//! contract suite and the fake agent binaries), check-deps, then the testkit
//! demo.
//! Gate 3 runs the per-crate checks plus the `agend` crate's tests (argv[0]
//! dispatch, and the shim and hook tests against real repos, which need the
//! real binary), then the shim demo. Gate 4 runs the per-crate checks of
//! agend-holder and agend (which holds the cross-process tests), check-deps,
//! then the holder demo (`holder_probe demo` against the built `agend`).
//! Gate 5 runs the agend-daemon checks (its tests include the STO-1..12
//! contract against the real store and the cross-process restart/crash
//! tests), check-deps, then the store demo. Gate 6 runs the checks of
//! agend-daemon, agend-holder, agend (RTM-1..9 against real holders, the
//! re-executed four boots, the real `agend daemon` tests) and agend-core,
//! check-deps, then `daemon_probe demo` against the built `agend`.
//! Gate 11 runs the per-crate checks, check-deps, then the TUI demo (screens,
//! navigation, resolve, disconnect against the testkit fake daemon).
//! Other gates use the per-crate checks until their acceptance flow is built.

use crate::{cargo, check_deps, workspace_root};
use std::process::Command;

pub struct Gate {
    pub number: u8,
    pub name: &'static str,
    pub crates: &'static [&'static str],
}

pub const GATES: &[Gate] = &[
    Gate {
        number: 1,
        name: "core",
        crates: &["agend-core"],
    },
    Gate {
        number: 2,
        name: "testkit",
        crates: &["agend-testkit"],
    },
    Gate {
        number: 3,
        name: "shim",
        crates: &["agend-shim"],
    },
    Gate {
        number: 4,
        name: "holder",
        // `agend` holds the cross-process tests (tests/holder_process.rs).
        crates: &["agend-holder", "agend"],
    },
    Gate {
        number: 5,
        name: "store",
        crates: &["agend-daemon"],
    },
    Gate {
        number: 6,
        name: "daemon-holder",
        // `agend` holds the tests with the real binary (holder_runtime.rs,
        // daemon_process.rs); `agend-core` gets the D26 version test.
        crates: &["agend-daemon", "agend-holder", "agend", "agend-core"],
    },
    Gate {
        number: 7,
        name: "codex",
        crates: &["agend-daemon"],
    },
    Gate {
        number: 8,
        name: "client",
        crates: &["agend-client", "agend-daemon"],
    },
    Gate {
        number: 9,
        name: "cli",
        crates: &["agend"],
    },
    Gate {
        number: 10,
        name: "pipeline",
        crates: &["agend-daemon"],
    },
    Gate {
        number: 11,
        name: "tui",
        crates: &["agend-tui"],
    },
    Gate {
        number: 12,
        name: "adapters",
        crates: &["agend-daemon"],
    },
    Gate {
        number: 13,
        name: "install",
        crates: &["agend-core", "agend-daemon", "agend"],
    },
];

/// Finds a gate by number (`1`) or name (`core`).
pub fn find(arg: &str) -> Option<&'static Gate> {
    GATES
        .iter()
        .find(|g| g.name == arg || arg.parse::<u8>() == Ok(g.number))
}

pub fn run(arg: Option<&str>) -> Result<(), String> {
    let gate = arg.and_then(find).ok_or_else(|| {
        let names: Vec<String> = GATES
            .iter()
            .map(|g| format!("{} {}", g.number, g.name))
            .collect();
        format!(
            "usage: cargo xtask accept <gate>; gates: {}",
            names.join(", ")
        )
    })?;
    println!(
        "== gate {} ({}) == docs/gates/gate-{:02}-{}.md",
        gate.number, gate.name, gate.number, gate.name
    );

    if gate.number == 1 {
        step(&["fmt", "--all", "--", "--check"])?;
        step(&[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])?;
        step(&["test", "-p", "agend-core"])?;
        step(&["test", "-p", "xtask", "--test", "protocol_compat"])?;
        step(&["test", "-p", "xtask", "--test", "workflow_toml"])?;
    } else {
        for krate in gate.crates {
            step(&["fmt", "-p", krate, "--", "--check"])?;
            step(&[
                "clippy",
                "-p",
                krate,
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ])?;
            step(&["test", "-p", krate])?;
        }
        if gate.number == 3 {
            step(&["test", "-p", "agend"])?;
        }
    }
    check_deps::run(false)?;

    if gate.number == 1 {
        crate::core_demo::run()?;
        println!("gate 1 (core): checks passed");
    } else if gate.number == 2 {
        step(&["build", "--quiet", "-p", "agend-testkit", "--bins"])?;
        step(&[
            "run",
            "--quiet",
            "-p",
            "agend-testkit",
            "--example",
            "testkit_demo",
        ])?;
        println!("gate 2 (testkit): checks passed");
    } else if gate.number == 3 {
        crate::shim_demo::run()?;
        println!("gate 3 (shim): checks passed");
    } else if gate.number == 4 {
        step(&["build", "--quiet", "-p", "agend"])?;
        step(&[
            "run",
            "--quiet",
            "-p",
            "agend-holder",
            "--example",
            "holder_probe",
            "--",
            "demo",
        ])?;
        println!("gate 4 (holder): checks passed");
    } else if gate.number == 5 {
        step(&[
            "run",
            "--quiet",
            "-p",
            "agend-daemon",
            "--example",
            "store_demo",
        ])?;
        println!("gate 5 (store): checks passed");
    } else if gate.number == 6 {
        step(&["build", "--quiet", "-p", "agend"])?;
        step(&[
            "run",
            "--quiet",
            "-p",
            "agend-daemon",
            "--example",
            "daemon_probe",
            "--",
            "demo",
        ])?;
        println!("gate 6 (daemon-holder): checks passed");
    } else if gate.number == 11 {
        step(&[
            "run",
            "--quiet",
            "-p",
            "agend-tui",
            "--example",
            "tui_accept",
        ])?;
        println!("gate 11 (tui): checks passed");
    } else {
        println!(
            "gate {} ({}): checks passed; demo not implemented yet (it is added when this gate is built)",
            gate.number, gate.name
        );
    }
    Ok(())
}

fn step(args: &[&str]) -> Result<(), String> {
    println!("-- cargo {}", args.join(" "));
    let status = Command::new(cargo())
        .current_dir(workspace_root())
        .args(args)
        .status()
        .map_err(|e| format!("cannot run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_are_numbered_one_to_thirteen_in_order() {
        let numbers: Vec<u8> = GATES.iter().map(|g| g.number).collect();
        assert_eq!(numbers, (1..=13).collect::<Vec<u8>>());
    }

    #[test]
    fn finds_gates_by_number_or_name() {
        assert_eq!(find("1").map(|g| g.name), Some("core"));
        assert_eq!(find("core").map(|g| g.number), Some(1));
        assert_eq!(find("12").map(|g| g.name), Some("adapters"));
        assert_eq!(find("13").map(|g| g.name), Some("install"));
        assert!(find("14").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn every_gate_has_a_gate_page() {
        let root = workspace_root();
        for gate in GATES {
            let page = format!("docs/gates/gate-{:02}-{}.md", gate.number, gate.name);
            assert!(root.join(&page).is_file(), "missing {page}");
        }
    }

    #[test]
    fn every_gate_crate_is_a_workspace_crate() {
        let root = workspace_root();
        for gate in GATES {
            for krate in gate.crates {
                assert!(
                    root.join("crates").join(krate).join("Cargo.toml").is_file(),
                    "gate {} names unknown crate {krate}",
                    gate.number
                );
            }
        }
    }
}
