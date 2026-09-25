//! The contract suites have teeth, and the rule table in CONTRACTS.md is
//! covered mechanically:
//!
//! - every registered mutant (a deliberately broken implementation: a
//!   wrapper around a fake, or a real `sh` runner with one knob wrong) fails
//!   at least one case tagged with the rule it breaks;
//! - every rule in the table has at least one case and at least one mutant,
//!   the mutants the table names are exactly the registered ones, and no
//!   case or mutant names a rule the table does not have.
//!
//! To add a rule: add the row (with its mutant names) to CONTRACTS.md, a
//! case tagged with its id, and the mutants below.

mod clock;
mod driver;
mod forge;
mod notifier;
mod real_runner;
mod runner;
mod runtime;
mod store;

use std::collections::{BTreeMap, BTreeSet};

use agend_testkit::contract::Report;
use agend_testkit::contract::fakes::case_rules;

/// A deliberately broken implementation of one rule.
pub struct Mutant {
    pub rule: &'static str,
    pub name: &'static str,
    /// Runs the whole suite against the mutant, reported under `name`.
    pub run: fn(&'static str) -> Report,
}

fn all_mutants() -> Vec<Mutant> {
    [
        driver::mutants(),
        forge::mutants(),
        store::mutants(),
        runtime::mutants(),
        notifier::mutants(),
        clock::mutants(),
        runner::mutants(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// Rule-id prefix of each contract, as used in CONTRACTS.md.
const PREFIXES: [(&str, &str); 7] = [
    ("Driver", "DRV"),
    ("Forge", "FRG"),
    ("Store", "STO"),
    ("Runtime", "RTM"),
    ("Notifier", "NTF"),
    ("Clock", "CLK"),
    ("Runner", "RUN"),
];

const CONTRACTS_MD: &str = include_str!("../../CONTRACTS.md");

/// `(rule id, mutant names)` for every rule row of CONTRACTS.md: a table
/// row whose first cell is an id like `DRV-1`; the mutants are the
/// backticked names in its last cell.
fn rule_table() -> Vec<(String, Vec<String>)> {
    CONTRACTS_MD
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim().strip_prefix('|')?.split('|').collect();
            let id = cells.first()?.trim();
            is_rule_id(id).then(|| {
                let last = cells.iter().rev().find(|c| !c.trim().is_empty())?;
                let mutants = last
                    .split('`')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_owned)
                    .collect();
                Some((id.to_owned(), mutants))
            })?
        })
        .collect()
}

fn is_rule_id(cell: &str) -> bool {
    let Some((prefix, number)) = cell.split_once('-') else {
        return false;
    };
    PREFIXES.iter().any(|(_, p)| *p == prefix)
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

fn prefix_of(contract: &str) -> &'static str {
    PREFIXES
        .iter()
        .find(|(c, _)| *c == contract)
        .map(|(_, p)| *p)
        .unwrap_or_else(|| panic!("no rule prefix for contract {contract}"))
}

#[test]
fn every_rule_has_a_case_and_a_mutant_and_nothing_else_exists() {
    let table = rule_table();
    let mut problems = Vec::new();

    let mut ids = BTreeSet::new();
    for (id, mutants) in &table {
        if !ids.insert(id.as_str()) {
            problems.push(format!("CONTRACTS.md lists {id} twice"));
        }
        if mutants.is_empty() {
            problems.push(format!("CONTRACTS.md names no mutant for {id}"));
        }
    }
    for (_, prefix) in PREFIXES {
        let numbers: Vec<u32> = ids
            .iter()
            .filter_map(|id| id.strip_prefix(prefix)?.strip_prefix('-')?.parse().ok())
            .collect();
        let mut sorted = numbers.clone();
        sorted.sort();
        if sorted.is_empty() || sorted != (1..=sorted.len() as u32).collect::<Vec<_>>() {
            problems.push(format!(
                "{prefix} rules must be numbered 1..n without gaps, got {sorted:?}"
            ));
        }
    }

    let cases = case_rules();
    for (contract, rule, case) in &cases {
        if !ids.contains(rule) {
            problems.push(format!(
                "case {contract}.{case} names {rule}, which CONTRACTS.md does not have"
            ));
        }
        if !rule.starts_with(&format!("{}-", prefix_of(contract))) {
            problems.push(format!(
                "case {contract}.{case} names {rule}, a rule of another contract"
            ));
        }
    }

    let mutants = all_mutants();
    let mut registered: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for m in &mutants {
        if !names.insert(m.name) {
            problems.push(format!("mutant {} is registered twice", m.name));
        }
        if !ids.contains(m.rule) {
            problems.push(format!(
                "mutant {} names {}, which CONTRACTS.md does not have",
                m.name, m.rule
            ));
        }
        registered.entry(m.rule).or_default().insert(m.name);
    }

    for (id, listed) in &table {
        if !cases.iter().any(|(_, rule, _)| rule == id) {
            problems.push(format!("{id} has no contract case"));
        }
        let listed: BTreeSet<&str> = listed.iter().map(String::as_str).collect();
        let actual = registered.get(id.as_str()).cloned().unwrap_or_default();
        for missing in listed.difference(&actual) {
            problems.push(format!(
                "CONTRACTS.md lists mutant {missing} for {id}, but no such mutant is registered for it"
            ));
        }
        for unlisted in actual.difference(&listed) {
            problems.push(format!(
                "mutant {unlisted} is registered for {id}, but CONTRACTS.md does not list it"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "rule coverage broken:\n  {}",
        problems.join("\n  ")
    );
}

#[test]
fn every_mutant_fails_a_case_of_its_rule() {
    let mutants = all_mutants();
    // In parallel: runner mutants wait seconds for timed-out commands.
    let reports: Vec<Report> = std::thread::scope(|scope| {
        let handles: Vec<_> = mutants
            .iter()
            .map(|m| {
                scope.spawn(move || {
                    let started = std::time::Instant::now();
                    let report = (m.run)(m.name);
                    println!(
                        "mutant {} ({}): failing rules {:?} in {} ms",
                        m.name,
                        m.rule,
                        report.failing_rules(),
                        started.elapsed().as_millis()
                    );
                    report
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("mutant run panicked outside a case"))
            .collect()
    });
    let escaped: Vec<String> = mutants
        .iter()
        .zip(&reports)
        .filter(|(m, report)| !report.failing_rules().contains(&m.rule))
        .map(|(m, report)| format!("{} ({}) escaped:\n{report}", m.name, m.rule))
        .collect();
    assert!(
        escaped.is_empty(),
        "{} of {} mutants pass every case of their rule:\n{}",
        escaped.len(),
        mutants.len(),
        escaped.join("\n")
    );
}
