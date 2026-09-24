//! Assignment rules for role templates (D18): a role template holds allowed
//! backends, model tier, instructions, min/max headcount and session policy.
//!
//! Rules: a review excludes the author and prefers a different backend;
//! requested changes go back to the original author; above max, tasks queue;
//! a parent waiting on its fanout does not hold a slot; detect mutual waiting
//! inside a team and notify; a missing role turns into an `ask`; on usage
//! limit, reassign to another allowed backend. Agents never create instances.
//!
//! These are pure rules (D25): the daemon supplies the inputs (candidate
//! members, current load); this module decides.
//!
//! Must NOT: spawn instances or talk to drivers.
