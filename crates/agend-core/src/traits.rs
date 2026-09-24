//! Boundary traits between the daemon's domain logic and its adapters:
//! `Driver`, `Forge`, `Store`, `Runtime`, `Notifier`, `Clock` (D9, D11).
//!
//! Every trait gets a fake in `agend-testkit` and a contract test suite that
//! runs against both the fake and the real implementation.
//!
//! Status: the trait signatures are not designed yet (stage 1). They are added
//! together with their first real implementation and fake, not before.
//!
//! Must NOT: contain implementations.
