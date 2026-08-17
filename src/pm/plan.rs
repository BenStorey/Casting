//! Project Plan — the deterministic "current state" of what the company is doing.
//!
//! Per docs/SEMANTIC_EVENTS.md: events are mutations, projections are state.
//! This module defines the *view types* for the plan (objective, ordered
//! priorities, open decisions) and the `Priority` enum that tasks carry. The
//! actual derivation lives on `Projection::plan()` (it needs the folded
//! projection), but the shapes are here so they can be serialized and shared.
//!
//! This is the first dogfooding artifact: our own roadmap/priorities will
//! eventually live as this derived state rather than hand-edited `.md`.

use serde::{Deserialize, Serialize};

/// How important a task is. Fully ordered so the plan can rank work.
/// Declaration order is reverse-importance because `derive(Ord)` ranks the
/// FIRST variant as the SMALLEST — so `High` must precede `Critical` for
/// `Critical > High` to hold (matches the docs: Critical > High > Medium > Low).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

/// One ranked item in the plan (a task and its current priority).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannedItem {
    pub task_id: String,
    pub title: String,
    pub priority: Priority,
}

/// The derived, current plan: what we're building, in priority order, and what's
/// waiting on the director. Always recomputed from the projection — never stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectPlan {
    /// The current objective — the most recent open requirement's title.
    pub objective: Option<String>,
    /// Tasks ranked by priority (Critical..Low), for the current work.
    pub priorities: Vec<PlannedItem>,
    /// The tasks currently at the lowest priority (deprioritized).
    pub deprioritized: Vec<PlannedItem>,
    /// Subjects of open risks (SEMANTIC_EVENTS §8 semantic objects).
    pub open_risks: Vec<String>,
    /// The active governing directives (docs/INTENT.md), ordered by strength.
    pub active_directives: Vec<String>,
    /// Subjects of decisions still awaiting the director.
    pub open_decisions: Vec<String>,
}
