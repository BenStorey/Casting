//! Casting actions — the validated action vocabulary (docs/HARNESS.md D3b).
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
//!   - `action`  — the typed action vocabulary (`PmAction`, `DIRECTOR`, …)
//!   - `policy`  — the pure validation gate (`validate`, `PolicyError`, …)
//!   - `events`  — mapping a validated action into domain events (`to_events`)
//!   - `director`   — shared director-authored event-shape builders

mod action;
mod director;
mod events;
pub mod policy;

pub use action::{action_vocab_for, is_valid_assignee, PmAction, TaskSpec, DIRECTOR};
pub use director::{
    director_budget_set, director_decision_made, director_directive_created,
    director_policy_changed, director_work_paused, director_work_resumed,
};
pub use policy::{validate, PolicyError};

pub mod actions {
    //! Backward-compat re-exports so callers using the old `crate::actions::actions::*` paths
    //! (introduced during the module rename) keep resolving. Remove after a cleanup pass.
    pub use super::{
        action_vocab_for, director_budget_set, director_decision_made, director_directive_created,
        director_policy_changed, director_work_paused, director_work_resumed, is_valid_assignee,
        validate, PmAction, PolicyError, TaskSpec, DIRECTOR,
    };
}
