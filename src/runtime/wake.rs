//! PM wake≠act tiering.
//!
//! The expensive PM path (git-observe + drain + respond + reconciler) must NOT
//! run on every low-value event. Tier classification tells the loop whether a
//! wake warrants an ACT now, or should wait for a quiet window / a higher-priority
//! interrupt. Pure logic — fully unit-testable, no async.

use crate::event::{Event, EventType};

/// How a wake should be handled once classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeTier {
    /// WAKE now, always — could be one call, but these are rare + high-value.
    Immediate,
    /// One occurrence is enough to wake the PM.
    Single,
    /// Batch — do NOT wake per-event; these accumulate and only flush on a
    /// quiet window / a higher-tier interrupt / all-idle.
    Batch,
}

/// Classify an event type into its wake tier. Events not explicitly listed
/// default to `Batch` (the safe, cost-conservative default — batching progress
/// is almost always fine; a wrongly-batched high-value event still flushes on
/// the quiet window).
pub fn tier_of(et: EventType) -> WakeTier {
    use EventType::*;
    use WakeTier::*;
    match et {
        // ---- Tier 0: immediate, always ----
        MessageSent
        | DecisionMade
        | RequirementChanged
        | AdvisoryBriefingImported
        | ExternalRequestReceived
        | AdvisorHandoff
        | BudgetSet
        | WorkPaused
        | WorkResumed => Immediate,

        // ---- Tier 1: wake on a single occurrence ----
        TaskBlocked
        | TaskBlockedOn
        | TaskReadyForReview
        | TaskReviewed
        | ChangeSetReady
        | MergeConflictDetected
        | WorktreeProvisioned
        | WorktreeRemoved
        | WorktreeBound
        | WorktreeReleased
        | RiskRaised
        | RiskUpdated
        | ActivityScheduled
        | ActivityFailed
        | PlanActionRejected => Single,

        // ---- Tier 2: batch (default) ----
        ProjectCreated
        | AgentHired
        | RequirementCreated
        | TaskCreated
        | TaskAssigned
        | TaskStarted
        | TaskCompleted
        | TaskPriorityChanged
        | MergeAuthorityChanged
        | EntityArchived
        | TaskDecomposed
        | AssumptionRecorded
        | ConstraintRecorded
        | OpinionRecorded
        | OpinionSuperseded
        | FactRecorded
        | CostIncurred
        | ProjectDirectiveCreated
        | ProjectDirectiveSuspended
        | ProjectDirectiveResumed
        | ProjectDirectiveSuperseded
        | ProjectDirectiveExpired
        | ObservationCreated
        | DecisionProposed
        | DecisionSuperseded
        | DecisionPolicyChanged
        | BranchCreated
        | CommitObserved
        | MergeCompleted
        | CommitRequested
        | AdvisorMessageSent
        | DiagramSaved
        | ActivityCompleted => Batch,

        // Any unlisted (incl. OrchestrationRun) + future variants default to
        // Batch — the safe, cost-conservative default (see module docs).
        _ => Batch,
    }
}

/// Whether the PM should ACT now given the newly-arrived events, or defer.
///
/// The ACT path: a non-batch (interrupt) event arrived, or the quiet window
/// elapsed (a poll timeout with nothing higher-priority pending means we
/// flush the batch). Only-batch arrival with NO quiet window → defers (the
/// cursor keeps accumulating; a later interrupt or the poll timeout flushes
/// it).
///
/// NOTE: The PM loop in control.rs reimplements this logic inline rather than
/// calling this function, so this function is currently unused. It is kept
/// as documentation of the wake decision algorithm.
#[allow(dead_code)]
pub fn should_act(new_events: &[Event], quiet_elapsed: bool) -> bool {
    if quiet_elapsed {
        return true;
    }
    new_events
        .iter()
        .any(|e| tier_of(e.event_type) != WakeTier::Batch)
}

/// The highest-tier event in the batch, if any is an interrupt (used for
/// diagnostics/logging — "woke on Tier-0 interrupt X"). Currently unused
/// but kept for documentation.
#[allow(dead_code)]
pub fn highest_tier(new_events: &[Event]) -> Option<WakeTier> {
    new_events
        .iter()
        .map(|e| tier_of(e.event_type))
        .max_by_key(|t| match t {
            WakeTier::Immediate => 2,
            WakeTier::Single => 1,
            WakeTier::Batch => 0,
        })
}
