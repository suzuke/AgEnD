//! Every contract suite passes against its fake.

use agend_testkit::contract::fakes::{FAKE, FakeDriverFixture, FakeRunnerFixture};
use agend_testkit::contract::{
    clock, driver, forge, notifier, run_all_fakes, runner, runtime, store,
};
use agend_testkit::fakes::{FakeClock, FakeForge, FakeNotifier, FakeRuntime, FakeStore};

#[test]
fn driver_fake_meets_the_contract() {
    driver::run(FAKE, FakeDriverFixture::new).assert_passed();
}

#[test]
fn forge_fake_meets_the_contract() {
    forge::run(FAKE, FakeForge::new).assert_passed();
}

#[test]
fn store_fake_meets_the_contract() {
    store::run(FAKE, FakeStore::new).assert_passed();
}

#[test]
fn runtime_fake_meets_the_contract() {
    runtime::run(FAKE, FakeRuntime::new).assert_passed();
}

#[test]
fn notifier_fake_meets_the_contract() {
    notifier::run(FAKE, FakeNotifier::new).assert_passed();
}

#[test]
fn clock_fake_meets_the_contract() {
    clock::run(FAKE, FakeClock::default).assert_passed();
}

#[test]
fn runner_fake_meets_the_contract() {
    runner::run(FAKE, FakeRunnerFixture::new).assert_passed();
}

#[test]
fn run_all_fakes_covers_every_trait_once() {
    let contracts: Vec<&str> = run_all_fakes().iter().map(|r| r.contract).collect();
    assert_eq!(
        contracts,
        [
            "Driver", "Forge", "Store", "Runtime", "Notifier", "Clock", "Runner"
        ]
    );
}

/// Gate 8 P9: the fake daemon meets the client protocol contract (the real
/// `agend daemon` runs the same cases in `crates/agend/tests/`).
#[cfg(unix)]
#[test]
fn client_protocol_fake_meets_the_contract() {
    use agend_testkit::contract::client::{self, FakeDaemonFixture};
    let report = client::run(FAKE, FakeDaemonFixture::new);
    println!("{report}");
    report.assert_passed();
}

/// Gate 8 P9, negative check: the fake with event ids counted from 1 again
/// must fail the old-cursor rule.
#[cfg(unix)]
#[test]
fn client_protocol_fake_with_ids_from_one_fails_clp_4() {
    use agend_testkit::contract::client::{self, fake_with_ids_from_one};
    let report = client::run_rules("ids-from-one", &["CLP-4"], fake_with_ids_from_one);
    println!("{report}");
    assert_eq!(report.failing_rules(), ["CLP-4"], "{report}");
}
