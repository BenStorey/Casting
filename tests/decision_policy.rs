//! Pure unit tests for the decision policy engine (`src/policy.rs`).
//!
//! The engine is the *delegated authority* layer (brief §5): a deterministic
//! map from decision class → owner involvement, with a fail-safe default. It is
//! deliberately pure (no I/O, no store) so it is trivially testable and safe to
//! run in front of an arbitrary producer — exactly the guarantee the LLM seam
//! needs once a real provider is wired in.

use casting::policy::{
    builtin_involvement, Decider, DecisionClass, DecisionPolicy, OwnerInvolvement,
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
