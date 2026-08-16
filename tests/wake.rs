//! Tests for PM wake≠act tiering (docs/plans/2026-08-14_pm-wakeact-tiering.md).

use casting::event::EventType;
use casting::runtime::wake::{tier_of, WakeTier};

#[test]
fn owner_message_is_immediate() {
    assert_eq!(tier_of(EventType::DecisionMade), WakeTier::Immediate);
    assert_eq!(tier_of(EventType::WorkPaused), WakeTier::Immediate);
    assert_eq!(tier_of(EventType::BudgetSet), WakeTier::Immediate);
}

#[test]
fn gated_work_wakes_on_single() {
    assert_eq!(tier_of(EventType::TaskBlocked), WakeTier::Single);
    assert_eq!(tier_of(EventType::ChangeSetReady), WakeTier::Single);
    assert_eq!(tier_of(EventType::MergeConflictDetected), WakeTier::Single);
    assert_eq!(tier_of(EventType::TaskReadyForReview), WakeTier::Single);
}

#[test]
fn progress_is_batched() {
    assert_eq!(tier_of(EventType::TaskCompleted), WakeTier::Batch);
    assert_eq!(tier_of(EventType::CommitObserved), WakeTier::Batch);
    assert_eq!(tier_of(EventType::TaskCreated), WakeTier::Batch);
    assert_eq!(tier_of(EventType::CostIncurred), WakeTier::Batch);
    // Unlisted variants default to Batch (safe, cost-conservative).
    assert_eq!(tier_of(EventType::OrchestrationRun), WakeTier::Batch);
}
