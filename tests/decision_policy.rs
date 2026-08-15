//! Pure unit tests for the decision policy engine (`src/policy.rs`).
//!
//! The engine is the *delegated authority* layer (brief §5): a deterministic
//! map from decision class → owner involvement, with a fail-safe default. It is
//! deliberately pure (no I/O, no store) so it is trivially testable and safe to
//! run in front of an arbitrary producer — exactly the guarantee the LLM seam
//! needs once a real provider is wired in.

use casting::actions::PolicyError;
use casting::pm::policy::{
    builtin_involvement, check_proposal, Decider, DecisionClass, DecisionPolicy, OwnerInvolvement,
};

#[test]
fn builtin_table_matches_brief_defaults() {
    use DecisionClass::*;
    assert_eq!(builtin_involvement(InternalRename), OwnerInvolvement::Never);
    assert_eq!(
        builtin_involvement(InternalRefactor),
        OwnerInvolvement::Never
    );
    assert_eq!(builtin_involvement(TestingLibrary), OwnerInvolvement::Pm);
    assert_eq!(builtin_involvement(AddConsultant), OwnerInvolvement::Pm);
    assert_eq!(
        builtin_involvement(InternalImplementation),
        OwnerInvolvement::Pm
    );
    assert_eq!(builtin_involvement(Database), OwnerInvolvement::Ask);
    assert_eq!(builtin_involvement(Architecture), OwnerInvolvement::Ask);
    assert_eq!(
        builtin_involvement(ProductRequirement),
        OwnerInvolvement::Ask
    );
    assert_eq!(
        builtin_involvement(SpendingThreshold),
        OwnerInvolvement::Ask
    );
    assert_eq!(
        builtin_involvement(ProductionDeployment),
        OwnerInvolvement::Ask
    );
    assert_eq!(builtin_involvement(Irreversible), OwnerInvolvement::Ask);
    assert_eq!(
        builtin_involvement(SecurityCritical),
        OwnerInvolvement::Notify
    );
}

#[test]
fn resolve_uses_builtin_by_default() {
    let policy = DecisionPolicy::defaults();
    assert_eq!(
        policy.resolve(DecisionClass::Database),
        OwnerInvolvement::Ask
    );
    assert_eq!(
        policy.resolve(DecisionClass::TestingLibrary),
        OwnerInvolvement::Pm
    );
}

#[test]
fn override_changes_resolve_and_reset_restores_builtin() {
    let mut policy = DecisionPolicy::defaults();
    // Escalate SecurityCritical from Notify -> Ask.
    policy.set(DecisionClass::SecurityCritical, OwnerInvolvement::Ask);
    assert_eq!(
        policy.resolve(DecisionClass::SecurityCritical),
        OwnerInvolvement::Ask
    );
    // Setting back to the builtin drops the override.
    policy.set(DecisionClass::SecurityCritical, OwnerInvolvement::Notify);
    assert_eq!(
        policy.resolve(DecisionClass::SecurityCritical),
        OwnerInvolvement::Notify
    );
}

#[test]
fn policy_round_trips_through_json() {
    let mut policy = DecisionPolicy::defaults();
    policy.set(DecisionClass::SecurityCritical, OwnerInvolvement::Ask);
    let json = serde_json::to_string(&policy).unwrap();
    let back: DecisionPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back, policy);
    assert_eq!(
        back.resolve(DecisionClass::SecurityCritical),
        OwnerInvolvement::Ask
    );
}

#[test]
fn involvement_ordering_is_never_pm_notify_ask() {
    assert!(OwnerInvolvement::Never < OwnerInvolvement::Pm);
    assert!(OwnerInvolvement::Pm < OwnerInvolvement::Notify);
    assert!(OwnerInvolvement::Notify < OwnerInvolvement::Ask);
    assert!(OwnerInvolvement::Ask > OwnerInvolvement::Never);
}

#[test]
fn requires_owner_verdict_only_true_for_ask() {
    assert!(OwnerInvolvement::Ask.requires_owner_verdict());
    assert!(!OwnerInvolvement::Notify.requires_owner_verdict());
    assert!(!OwnerInvolvement::Pm.requires_owner_verdict());
    assert!(!OwnerInvolvement::Never.requires_owner_verdict());
}

#[test]
fn decider_locks_to_owner_only_for_ask() {
    assert_eq!(OwnerInvolvement::Ask.decider(), Decider::Owner);
    assert_eq!(OwnerInvolvement::Notify.decider(), Decider::Agent);
    assert_eq!(OwnerInvolvement::Pm.decider(), Decider::Agent);
    assert_eq!(OwnerInvolvement::Never.decider(), Decider::Agent);
}

#[test]
fn enums_round_trip_through_json() {
    for class in [
        DecisionClass::InternalRename,
        DecisionClass::TestingLibrary,
        DecisionClass::Database,
        DecisionClass::SecurityCritical,
    ] {
        let back: DecisionClass =
            serde_json::from_str(&serde_json::to_string(&class).unwrap()).unwrap();
        assert_eq!(back, class);
    }
    for level in [
        OwnerInvolvement::Never,
        OwnerInvolvement::Pm,
        OwnerInvolvement::Notify,
        OwnerInvolvement::Ask,
    ] {
        let back: OwnerInvolvement =
            serde_json::from_str(&serde_json::to_string(&level).unwrap()).unwrap();
        assert_eq!(back, level);
    }
}

// --- Task 2: decider routing + authority-downgrade gate ---

#[test]
fn check_proposal_accepts_claim_at_least_as_restrictive() {
    let policy = DecisionPolicy::defaults();
    // Database requires Ask; claiming Ask (equal) is fine.
    assert!(check_proposal(DecisionClass::Database, OwnerInvolvement::Ask, &policy).is_ok());
    // Database requires Ask; claiming more-restrictive is still fine (Never/Pm/Notify < Ask,
    // so those are LESS restrictive — only equal or MORE restrictive passes; Ask is the max,
    // so equal is the only acceptance here).
    // InternalRefactor requires Never; claiming anything ≥ Never passes.
    assert!(check_proposal(
        DecisionClass::InternalRefactor,
        OwnerInvolvement::Never,
        &policy
    )
    .is_ok());
    assert!(check_proposal(
        DecisionClass::InternalRefactor,
        OwnerInvolvement::Ask,
        &policy
    )
    .is_ok());
    // SecurityCritical default Notify; claiming Notify or Ask passes.
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Notify,
        &policy
    )
    .is_ok());
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Ask,
        &policy
    )
    .is_ok());
}

#[test]
fn check_proposal_rejects_authority_downgrade() {
    let policy = DecisionPolicy::defaults();
    // Database requires Ask; claiming Pm (less restrictive) is a downgrade.
    let err = check_proposal(DecisionClass::Database, OwnerInvolvement::Pm, &policy)
        .expect_err("claiming Pm for an Ask-required class must be rejected");
    assert!(matches!(
        err,
        PolicyError::AuthorityDowngrade {
            class: DecisionClass::Database,
            required: OwnerInvolvement::Ask,
            claimed: OwnerInvolvement::Pm,
        }
    ));
    // SecurityCritical requires Notify; claiming Never or Pm is a downgrade.
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Never,
        &policy
    )
    .is_err());
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Pm,
        &policy
    )
    .is_err());
}

#[test]
fn downgrade_is_rejected_even_after_owner_override() {
    // Owner escalates SecurityCritical to Ask; the producer must claim Ask.
    let mut policy = DecisionPolicy::defaults();
    policy.set(DecisionClass::SecurityCritical, OwnerInvolvement::Ask);
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Notify,
        &policy
    )
    .is_err());
    assert!(check_proposal(
        DecisionClass::SecurityCritical,
        OwnerInvolvement::Ask,
        &policy
    )
    .is_ok());
}

#[test]
fn error_display_is_human_readable() {
    let err = PolicyError::AuthorityDowngrade {
        class: DecisionClass::Database,
        required: OwnerInvolvement::Ask,
        claimed: OwnerInvolvement::Pm,
    };
    let msg = err.to_string();
    assert!(msg.contains("Database"));
    assert!(msg.contains("Ask"));
    assert!(msg.contains("Pm"));
}
