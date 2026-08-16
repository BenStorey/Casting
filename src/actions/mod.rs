//! The PM's structured action vocabulary + policy gate (review refactor
//! 2026-08-10: split from a single God-file into four coherent submodules).
//!
//! This is the seam between *reasoning* and *execution* (docs/ADDENDUM.md §16):
//!
//! ```text
//! reasoning → structured proposed actions → policy validation → execution → domain events
//! ```
//!
//! Today the reasoning is a deterministic scripted policy in `pm.rs`. Tomorrow
//! it will be an LLM client. Both produce the SAME typed `PmAction`s, which are
//! validated by the pure policy gate here before anything touches the event
//! store. That gate is what stops a wrong model (or a wrong script) from
//! violating the project's invariants.
//!
//! This module is a thin FACADE: it just re-exports the public surface from the
//! submodules so every existing `crate::actions::X` reference resolves
//! unchanged. The bulk lives in:
//!   - `action`  — the typed action vocabulary (`PmAction`, `OWNER`, …)
//!   - `policy`  — the pure validation gate (`validate`, `PolicyError`, …)
//!   - `events`  — mapping a validated action into domain events (`to_events`)
//!   - `owner`   — shared owner-authored event-shape builders

mod action;
mod events;
mod owner;
pub mod policy;

pub use action::{action_vocab_for, is_valid_assignee, PmAction, TaskSpec, OWNER};
pub use owner::{
    owner_budget_set, owner_decision_made, owner_directive_created, owner_policy_changed,
    owner_work_paused, owner_work_resumed,
};
pub use policy::{validate, PolicyError};

/// Resolve the acting `Actor` from a `who` label ("owner" / agent id). Used by
/// the event builders.
pub use events::actor_for;
