//! Tests for PM wake≠act tiering (docs/plans/2026-08-14_pm-wakeact-tiering.md).

use casting::event::{Actor, Aggregate, Event, EventType};
use casting::wake::{highest_tier, should_act, tier_of, WakeTier};

fn ev(et: EventType) -> Event {
    Event::new(
        "proj",
        Actor::Owner,
        et,
        Aggregate {
            kind: "x".into(),
            id: "x".into(),
        },
        serde_json::json!({}),
    )
}

#[test]
fn owner_message_is_immediate() {
    assert_eq!(tier_of(EventType::MessageSent), WakeTier::Immediate);
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

#[test]
fn should_act_interrupts_immediately_even_without_quiet_window() {
    // A lone Tier-0/1 event → act now (no quiet window needed).
    assert!(should_act(&[ev(EventType::MessageSent)], false));
    assert!(should_act(&[ev(EventType::TaskBlocked)], false));
    // A lone batch event with NO quiet window → defer (the cost lever).
    assert!(!should_act(&[ev(EventType::TaskCompleted)], false));
    assert!(!should_act(&[ev(EventType::CommitObserved)], false));
}

#[test]
fn should_act_quiet_window_flushes_batch() {
    // Batch-only but the quiet window elapsed → flush.
    assert!(should_act(&[ev(EventType::TaskCompleted)], true));
    assert!(should_act(&[ev(EventType::CommitObserved)], true));
    // An interrupt still acts even in a mixed batch.
    assert!(should_act(
        &[ev(EventType::TaskCompleted), ev(EventType::MessageSent)],
        false
    ));
}

#[test]
fn highest_tier_reports_the_interrupt() {
    assert_eq!(
        highest_tier(&[ev(EventType::TaskCompleted)]),
        Some(WakeTier::Batch)
    );
    assert_eq!(
        highest_tier(&[ev(EventType::TaskCompleted), ev(EventType::TaskBlocked)]),
        Some(WakeTier::Single)
    );
    assert_eq!(
        highest_tier(&[ev(EventType::MessageSent), ev(EventType::TaskCompleted)]),
        Some(WakeTier::Immediate)
    );
    assert_eq!(highest_tier(&[]), None);
}
