//! PM wake≠act tiering.
//!
//! The expensive PM path (git-observe + drain + respond + reconciler) must NOT
//! run on every low-value event. Tier classification tells the loop whether a
//! wake warrants an ACT now, or should wait for a quiet window / a higher-priority
//! interrupt. Pure logic — fully unit-testable, no async.

use crate::event::EventType;

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
        DecisionMade
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
