//! Decision policy engine — *delegated authority* (docs/CASTING_PROJECT_BRIEF.md §5).
//!
//! The owner should not have to reason about every move the organization makes.
//! Instead they configure, per class of decision, how much owner involvement is
//! required. This module is that engine, expressed as a pure, deterministic,
//! LLM-free policy layer:
//!
//! ```text
//!   DecisionClass ──(DecisionPolicy.resolve)──► OwnerInvolvement
//!                                                    │
//!                                                    ▼
//!                                        Owner (ask)  |  PM/agent (decide)
//! ```
//!
//! Crucially, the engine does NOT decide *whether* a decision is recorded —
//! every decision in Casting is recorded via the universal `DecisionProposed`
//! → `DecisionMade` event pair. The engine only decides **who the
//! decision-maker is** (the owner, or a delegated PM/agent) and **whether the
//! owner's inbox is involved**.
//!
//! It sits directly in front of the LLM seam: today a scripted PM consults it;
//! a future provider's plans flow through the exact same gate (see `actions.rs`).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How much owner involvement a class of decision requires, along the autonomy
/// spectrum from least to most owner control:
///
/// ```text
/// Never < Pm < Notify < Ask
/// ```
///
/// The derived ordering (`Ord`) is what powers the authority-downgrade gate:
/// a producer must never claim *less* restrictive involvement than the policy
/// requires of a class (e.g. it cannot claim `Pm` for a class whose policy
/// says `Ask`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OwnerInvolvement {
    /// The organization may act; the owner is never consulted.
    #[serde(rename = "never")]
    Never,
    /// The PM may decide on its own; the owner is not asked.
    #[serde(rename = "pm")]
    Pm,
    /// The owner is informed, but work proceeds without their verdict.
    #[serde(rename = "notify")]
    Notify,
    /// The owner must decide first; work is blocked until they do.
    #[serde(rename = "ask")]
    Ask,
}

/// The delegated decision-maker for a level of owner involvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decider {
    /// The human owner decides.
    Owner,
    /// A PM/agent decides on the organization's behalf.
    Agent,
}

/// The stable taxonomy of decision kinds. Mirrors the default table in
/// docs/CASTING_PROJECT_BRIEF.md §5. This is the seed of "what is
/// decision-worthy" — keep it a deliberate, curated set (like `EventType`) so
/// the decision log does not get noisy. A decision is a *structured, recorded*
/// choice (options + recommendation + class), distinct from ordinary messaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionClass {
    /// Renaming an internal variable/symbol. Default: Never.
    InternalRename,
    /// An internal refactor with no product-facing change. Default: Never.
    InternalRefactor,
    /// Choosing a testing library/framework. Default: Pm.
    TestingLibrary,
    /// Bringing a new consultant into the company. Default: Pm.
    AddConsultant,
    /// A change in internal implementation approach. Default: Pm.
    InternalImplementation,
    /// Choosing/changing the database. Default: Ask.
    Database,
    /// A change in system/architecture. Default: Ask.
    Architecture,
    /// A change in product requirements/scope. Default: Ask.
    ProductRequirement,
    /// Spending more than a configured threshold. Default: Ask.
    SpendingThreshold,
    /// A production deployment. Default: Ask.
    ProductionDeployment,
    /// A security-critical issue/action. Default: Notify.
    SecurityCritical,
    /// An irreversible action. Default: Ask.
    Irreversible,
}

/// The built-in default involvement for each class (brief §5). These are
/// *seeds* — the owner will reconfigure per-class autonomy later, so no single
/// default is load-bearing. New/unclassified classes fall back to
/// [`DecisionPolicy::default_involvement`].
pub fn builtin_involvement(class: DecisionClass) -> OwnerInvolvement {
    use DecisionClass::*;
    match class {
        InternalRename | InternalRefactor => OwnerInvolvement::Never,
        TestingLibrary | AddConsultant | InternalImplementation => OwnerInvolvement::Pm,
        Database | Architecture | ProductRequirement | SpendingThreshold | ProductionDeployment
        | Irreversible => OwnerInvolvement::Ask,
        SecurityCritical => OwnerInvolvement::Notify,
    }
}

/// A per-project decision policy: which `OwnerInvolvement` each `DecisionClass`
/// currently requires, resolved as override → builtin → default.
///
/// This is the owner's "autonomy knobs." It is immutable-on-construction in
/// this slice (built from [`DecisionPolicy::defaults`] or custom overrides);
/// persisting per-project policy changes as domain events is round two.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionPolicy {
    /// Fallback involvement for any class with no override and no builtin.
    /// Defaults to `Ask` (fail safe), but defaults are seeds — overrideable.
    #[serde(default = "default_involvement")]
    pub default_involvement: OwnerInvolvement,
    /// Per-class overrides on top of the builtin table.
    #[serde(default)]
    overrides: HashMap<DecisionClass, OwnerInvolvement>,
}

fn default_involvement() -> OwnerInvolvement {
    OwnerInvolvement::Ask
}

impl Default for DecisionPolicy {
    fn default() -> Self {
        DecisionPolicy {
            default_involvement: default_involvement(),
            overrides: HashMap::new(),
        }
    }
}

impl DecisionPolicy {
    /// The builtin policy: the brief §5 table, no overrides.
    pub fn defaults() -> Self {
        DecisionPolicy::default()
    }

    /// Resolve the required owner involvement for `class`.
    /// Precedence: explicit override → builtin table → `default_involvement`.
    pub fn resolve(&self, class: DecisionClass) -> OwnerInvolvement {
        self.overrides
            .get(&class)
            .copied()
            .unwrap_or_else(|| builtin_involvement(class))
    }

    /// Set (or override) the involvement for a class. This is how the owner's
    /// per-class autonomy is configured (round two persists these as events).
    pub fn set(&mut self, class: DecisionClass, involvement: OwnerInvolvement) {
        if involvement == builtin_involvement(class) {
            // Equal to the builtin: drop the override so `resolve` is stable.
            self.overrides.remove(&class);
        } else {
            self.overrides.insert(class, involvement);
        }
    }
}

impl OwnerInvolvement {
    /// Whether this level requires the owner to give an explicit verdict before
    /// the organization acts. Only `Ask` blocks; `Notify` informs but proceeds.
    pub fn requires_owner_verdict(self) -> bool {
        self == OwnerInvolvement::Ask
    }

    /// The delegated decision-maker for this level.
    pub fn decider(self) -> Decider {
        if self.requires_owner_verdict() {
            Decider::Owner
        } else {
            Decider::Agent
        }
    }
}
