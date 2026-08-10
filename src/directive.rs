//! Project Directives — the **governance** layer (docs/INTENT.md).
//!
//! Up to now Casting tracks *state* (what is true) and *intent* (what we want).
//! Directives add **how we operate**: policies, constraints, principles,
//! practices, preferences, and objectives as first-class, event-sourced
//! project state — NOT prompt text. They are authoritative, selected per agent
//! by a context resolver, and changed through the authority gate (never
//! silently mutated by a plain agent).
//!
//! Per INTENT.md, the general concept is: directives exist ONCE and are
//! selectively surfaced. `relevant(projection, areas)` is that resolver.

use crate::projection::Projection;
use serde::{Deserialize, Serialize};

/// What kind of directive this is (INTENT.md "Directive kinds").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveKind {
    /// "How we work" rule for the whole project (e.g. use TDD).
    Policy,
    /// A hard limit that must hold ("budget cannot exceed $250").
    Constraint,
    /// A guiding value ("prefer simple solutions").
    Principle,
    /// An operating cadence ("review architecture every 15 tasks").
    Practice,
    /// A soft inclination ("I'd rather use Postgres").
    Preference,
    /// A target we're trying to achieve ("build the MVP in 3 days").
    Objective,
}

/// How much authority a directive carries. Ordering is REVERSE-declaration so
/// that `derive(Ord)` makes `Required` the GREATEST, because Rust ranks the
/// first variant as smallest (same trick as `plan::Priority`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveStrength {
    Recommended,
    Strong,
    Required,
}

/// Lifecycle of a directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveStatus {
    Active,
    Suspended,
    /// Replaced by another directive (history is preserved, not overwritten).
    Superseded,
    Expired,
}

/// A first-class project directive — the authoritative "how we operate" state.
/// Creation may need the PM/LLM to *interpret* owner language, but lifecycle
/// transitions are deterministic reducers through the authority gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directive {
    pub id: String,
    pub kind: DirectiveKind,
    pub statement: String,
    /// Areas this governs, e.g. `["engineering"]`, `["architecture", "api"]`.
    pub scope: Vec<String>,
    pub strength: DirectiveStrength,
    pub status: DirectiveStatus,
    pub created_by: String,
    /// id of the directive this one replaces, if any (supersession, not edit).
    pub supersedes: Option<String>,
}

impl Directive {
    pub fn new(
        id: String,
        kind: DirectiveKind,
        statement: String,
        scope: Vec<String>,
        strength: DirectiveStrength,
        created_by: String,
        supersedes: Option<String>,
    ) -> Self {
        Directive {
            id,
            kind,
            statement,
            scope,
            strength,
            status: DirectiveStatus::Active,
            created_by,
            supersedes,
        }
    }
}

/// Select IN ACTIVE directives whose scope intersects `areas`, for the given
/// agent/task context (INTENT.md "agents should not necessarily see all
/// directives"). Sorted by strength (Required..Recommended, strongest first),
/// then by declaration order for stability.
pub fn relevant<'a>(projection: &'a Projection, areas: &[&str]) -> Vec<&'a Directive> {
    let mut out: Vec<&Directive> = projection
        .directives
        .iter()
        .filter(|d| d.status == DirectiveStatus::Active)
        .filter(|d| d.scope.iter().any(|s| areas.contains(&s.as_str())))
        .collect();
    // Strongest first (Required < Recommended is false; Reverse puts Required first).
    out.sort_by_key(|d| std::cmp::Reverse(d.strength));
    out
}
