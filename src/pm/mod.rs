//! The Product Manager control loop — the "brain" of the organization.
//!
//! The PM reads the event stream through the projection, makes decisions
//! via the typed PmAction vocabulary (validated by the policy gate),
//! and appends new domain events. This module contains the loop (`pm`),
//! the planning engine (`planning`), plan data types (`plan`), the
//! decision-policy engine (`policy`), the budget/guard rails (`guard`),
//! the reconciler background pass (`reconciler`), and the triage
//! classifier (`triage`).

pub mod control;
pub mod guard;
pub mod plan;
pub mod planning;
pub mod policy;
pub mod reconciler;
pub mod triage;

pub use control::{drive_pm, run_pm, AppState, PlannedAction, PM_CONSUMER};
pub use guard::{Budget, BudgetStatus, PauseInfo};
pub use plan::{PlannedItem, Priority, ProjectPlan};
pub use policy::{check_proposal, Decider, DecisionClass, DecisionPolicy, OwnerInvolvement};
pub use reconciler::{default_passes, run_if_due, run_passes, ReconcilePass};
pub use triage::classify;
