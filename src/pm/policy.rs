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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum OwnerInvolvement {
    /// The organization may act; the owner is never consulted.
    #[serde(rename = "never")]
    #[default]
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
    /// Changing project governance (a directive). Default: Ask — governance is
    /// owner-only, so the PM proposing a directive change must route to the
    /// owner for approval before it can be applied.
    GovernanceChange,
    /// Applying a CHEAP-cost-band playbook. Default: Pm so the PM can fire
    /// inexpensive, everyday recipes without the owner. Owner can tighten to
    /// Ask if desired.
    PlaybookCheap,
    /// Applying a MEDIUM-cost-band playbook. Default: Pm.
    PlaybookMedium,
    /// Applying an EXPENSIVE-cost-band playbook. Default: Ask so the owner
    /// must approve expensive scans and full-repo operations. Owner can loosen
    /// to Pm if they trust the PM's judgment on cost.
    PlaybookExpensive,
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
        | Irreversible | GovernanceChange => OwnerInvolvement::Ask,
        SecurityCritical => OwnerInvolvement::Notify,
        // Playbook cost-band involvement
        PlaybookCheap | PlaybookMedium => OwnerInvolvement::Pm,
        PlaybookExpensive => OwnerInvolvement::Ask,
    }
}

/// A per-project decision policy: which `OwnerInvolvement` each `DecisionClass`
/// currently requires, resolved as override → builtin → default.
///
/// This is the owner's "autonomy knobs." It is immutable-on-construction in
/// this slice (built from [`DecisionPolicy::defaults`] or custom overrides);
/// mutations happen via `DecisionPolicyChanged` events.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DecisionPolicy {
    /// Per-class overrides (user-set via DecisionPolicyChanged events).
    overrides: HashMap<DecisionClass, OwnerInvolvement>,
    /// Default involvement for any unclassified class.
    default_involvement: OwnerInvolvement,
}

/// Check that a claimed involvement for a class is at least as restrictive
/// as the policy requires. This is the authority-downgrade guard: an LLM
/// cannot silently skip the owner by under-claiming involvement.
pub fn check_proposal(
    class: DecisionClass,
    claimed: OwnerInvolvement,
    policy: &DecisionPolicy,
) -> Result<(), crate::actions::policy::PolicyError> {
    let required = policy.resolve(class);
    if claimed >= required {
        Ok(())
    } else {
        Err(crate::actions::policy::PolicyError::AuthorityDowngrade {
            class,
            required,
            claimed,
        })
    }
}

impl DecisionPolicy {
    /// Default policy with all classes at their builtin involvement.
    pub fn defaults() -> Self {
        DecisionPolicy {
            overrides: HashMap::new(),
            default_involvement: OwnerInvolvement::Ask,
        }
    }

    /// Apply an owner-set override (from a `DecisionPolicyChanged` event).
    pub fn set(&mut self, class: DecisionClass, involvement: OwnerInvolvement) {
        self.overrides.insert(class, involvement);
    }

    /// Resolve the required involvement for a class: override wins, then
    /// builtin, then default (Ask).
    pub fn resolve(&self, class: DecisionClass) -> OwnerInvolvement {
        self.overrides
            .get(&class)
            .copied()
            .unwrap_or_else(|| builtin_involvement(class))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playbook_bands_builtin() {
        // Cheap and medium are PM-fireable by default
        assert_eq!(
            builtin_involvement(DecisionClass::PlaybookCheap),
            OwnerInvolvement::Pm
        );
        assert_eq!(
            builtin_involvement(DecisionClass::PlaybookMedium),
            OwnerInvolvement::Pm
        );
        // Expensive requires owner ask
        assert_eq!(
            builtin_involvement(DecisionClass::PlaybookExpensive),
            OwnerInvolvement::Ask
        );
    }

    #[test]
    fn test_playbook_band_policy_resolve() {
        let policy = DecisionPolicy::defaults();
        // Default resolves to builtin
        assert_eq!(
            policy.resolve(DecisionClass::PlaybookCheap),
            OwnerInvolvement::Pm
        );
        assert_eq!(
            policy.resolve(DecisionClass::PlaybookExpensive),
            OwnerInvolvement::Ask
        );
    }

    #[test]
    fn test_playbook_band_override() {
        let mut policy = DecisionPolicy::defaults();
        // Owner can loosen expensive to Pm
        policy.set(DecisionClass::PlaybookExpensive, OwnerInvolvement::Pm);
        assert_eq!(
            policy.resolve(DecisionClass::PlaybookExpensive),
            OwnerInvolvement::Pm
        );
        // Owner can tighten cheap to Ask
        policy.set(DecisionClass::PlaybookCheap, OwnerInvolvement::Ask);
        assert_eq!(
            policy.resolve(DecisionClass::PlaybookCheap),
            OwnerInvolvement::Ask
        );
    }

    #[test]
    fn test_proposal_check() {
        let policy = DecisionPolicy::defaults();
        // Claiming Pm for an expensive band is rejected (Pm < Ask)
        assert!(check_proposal(
            DecisionClass::PlaybookExpensive,
            OwnerInvolvement::Pm,
            &policy
        )
        .is_err());
        // Claiming Ask for cheap is accepted (Ask >= Pm)
        assert!(
            check_proposal(DecisionClass::PlaybookCheap, OwnerInvolvement::Ask, &policy).is_ok()
        );
    }
}
