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
