//! Mapping a validated action into the ordered domain events (with provenance).
use super::action::PmAction;
use crate::event::{Actor, Aggregate, Event, EventType, Metadata};
use crate::pm::DecisionClass;
use serde_json::{json, Value};

impl PmAction {
    /// Map an action, performed by `who`, into the ordered domain events that
    /// record it — with provenance (`causation`/`correlation`/`agent_run_id`)
    /// already attached. Returns zero events for `NoOp`.
    pub fn to_events(
        &self,
        project: &str,
        who: &str,
        cause: &Event,
        correlation: &str,
    ) -> Vec<Event> {
        let actor = actor_for(who);
        let meta = linked(cause, correlation);
        match self {
            PmAction::HireAgent { agent_id, role } => vec![ev(
                project,
                actor,
                agent_id,
                "agent",
                EventType::AgentHired,
                json!({ "role": role }),
                meta,
            )],
            PmAction::CreateRequirement {
                id,
                title,
                description,
            } => vec![ev(
                project,
                actor,
                id,
                "requirement",
                EventType::RequirementCreated,
                json!({ "title": title, "description": description }),
                meta,
            )],
            PmAction::CreateTask { id, title, kind } => vec![ev(
                project,
                actor,
                id,
                "task",
                EventType::TaskCreated,
                json!({ "title": title, "kind": kind }),
                meta,
            )],
            // Fan-out a task into parallel children. Emits one TaskDecomposed
            // (the decomposition intent / provenance) then one TaskCreated per
            // child, each carrying `parent_id` so the graph can aggregate them
            // into the parent's joint resolution.
            PmAction::DecomposeTask { parent, children } => {
                let mut events = vec![ev(
                    project,
                    actor.clone(),
                    parent,
                    "task",
                    EventType::TaskDecomposed,
                    json!({
                        "parent": parent,
                        "children": children
                            .iter()
                            .map(|c| c.id.clone())
                            .collect::<Vec<String>>()
                    }),
                    meta.clone(),
                )];
                for c in children {
                    events.push(ev(
                        project,
                        actor.clone(),
                        &c.id,
                        "task",
                        EventType::TaskCreated,
                        json!({ "title": c.title, "kind": c.kind, "parent_id": parent }),
                        meta.clone(),
                    ));
                }
                events
            }
            // A hard dependency edge: `task_id` (the aggregate) waits on
            // `blocking_task_id` until `required_state`. Aggregate = the
            // DEPENDENT task; the blocker is carried in the payload.
            PmAction::BlockTaskOn {
                task_id,
                blocking_task_id,
                required_state,
            } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskBlockedOn,
                json!({
                    "blocking_task_id": blocking_task_id,
                    "required_state": required_state,
                }),
                meta,
            )],
            PmAction::AssignTask {
                task_id,
                assignee,
                merge_authority,
            } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskAssigned,
                json!({ "assignee": assignee, "merge_authority": merge_authority }),
                meta,
            )],
            PmAction::SetMergeAuthority {
                task_id,
                merge_authority,
            } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::MergeAuthorityChanged,
                // `from` omitted here (to_events lacks the projection); the
                // reducer only needs `to`. Mirrors SetTaskPriority.
                json!({ "task_id": task_id, "to": merge_authority }),
                meta,
            )],
            PmAction::ProvisionWorktree {
                task_id,
                assignee,
                slug,
                cargo_target_dir,
                slot,
                port,
            } => {
                // The worktree ROOT is the parent of the cargo target dir (the
                // build target is a private subdir inside the worktree). The
                // projection records both: `path` = the worktree root (which
                // physically exists), `cargo_target_dir` = its private target.
                let path = cargo_target_dir
                    .strip_suffix("/target")
                    .or_else(|| cargo_target_dir.strip_suffix("\\target"))
                    .unwrap_or(cargo_target_dir)
                    .to_string();
                // Branch follows the casting/task-* convention: the slug is an
                // optional suffix on the task id.
                let branch = if slug.is_empty() {
                    format!("casting/{task_id}")
                } else {
                    format!("casting/{task_id}-{slug}")
                };
                vec![ev(
                    project,
                    actor,
                    // aggregate keyed by the worktree (one per task)
                    &format!("wt-{task_id}"),
                    "worktree",
                    EventType::WorktreeProvisioned,
                    json!({
                        "task_id": task_id,
                        "consultant": assignee,
                        "slot": slot,
                        "branch": branch,
                        "path": path,
                        "cargo_target_dir": cargo_target_dir,
                        "port": port,
                    }),
                    meta,
                )]
            }
            PmAction::StartTask { task_id } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskStarted,
                json!({}),
                meta,
            )],
            // The thin agent git surface: record the commit intent. The actual
            // commit is made physically in the worktree by run_planned via the
            // pinned runner (and the observer later records CommitObserved).
            PmAction::CommitToChangeSet { task_id, message } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::CommitRequested,
                json!({ "message": message }),
                meta,
            )],
            PmAction::CompleteTask { task_id, result } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskCompleted,
                json!({ "result": result }),
                meta,
            )],
            PmAction::RequestReview { task_id, reviewer } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskReadyForReview,
                json!({ "reviewer": reviewer }),
                meta,
            )],
            PmAction::ReviewTask {
                task_id,
                approved,
                note,
            } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskReviewed,
                json!({
                    "approved": approved,
                    "note": note,
                }),
                meta,
            )],
            PmAction::BlockTask { task_id, reason } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskBlocked,
                json!({ "reason": reason }),
                meta,
            )],
            PmAction::SetTaskPriority { task_id, priority } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskPriorityChanged,
                // `from` is omitted here (to_events lacks the projection); the
                // reducer only needs `to` for current state.
                json!({ "task_id": task_id, "to": priority }),
                meta,
            )],
            PmAction::RaiseRisk {
                id,
                subject,
                severity,
            } => vec![ev(
                project,
                actor,
                id,
                "risk",
                EventType::RiskRaised,
                json!({ "subject": subject, "severity": severity }),
                meta,
            )],
            PmAction::ResolveRisk { risk_id, status } => vec![ev(
                project,
                actor,
                risk_id,
                "risk",
                EventType::RiskUpdated,
                json!({ "status": status }),
                meta,
            )],
            PmAction::RecordAssumption { id, body } => vec![ev(
                project,
                actor,
                id,
                "assumption",
                EventType::AssumptionRecorded,
                json!({ "body": body }),
                meta,
            )],
            PmAction::RecordConstraint { id, body } => vec![ev(
                project,
                actor,
                id,
                "constraint",
                EventType::ConstraintRecorded,
                json!({ "body": body }),
                meta,
            )],
            PmAction::RecordOpinion {
                id,
                subject,
                category,
                statement,
                supersedes,
            } => vec![ev(
                project,
                actor,
                id,
                "opinion",
                EventType::OpinionRecorded,
                json!({
                    "subject": subject,
                    "category": category,
                    "statement": statement,
                    "supersedes": supersedes,
                }),
                meta,
            )],
            PmAction::RecordFact {
                id,
                kind,
                statement,
            } => vec![ev(
                project,
                actor,
                id,
                "fact",
                EventType::FactRecorded,
                json!({ "kind": kind, "statement": statement }),
                meta,
            )],
            PmAction::SupersedeOpinion {
                opinion_id,
                by_opinion_id,
            } => vec![ev(
                project,
                actor,
                opinion_id,
                "opinion",
                EventType::OpinionSuperseded,
                json!({ "superseded_by": by_opinion_id }),
                meta,
            )],
            PmAction::ImportBriefing {
                id,
                source,
                subject,
                title,
                body,
                assets,
            } => {
                let brought_in_by = match &actor {
                    Actor::Owner => "owner".to_string(),
                    Actor::Agent { id } => id.clone(),
                    Actor::System => "system".to_string(),
                };
                vec![ev(
                    project,
                    actor,
                    id,
                    "briefing",
                    EventType::AdvisoryBriefingImported,
                    json!({
                        "source": source,
                        "subject": subject,
                        "title": title,
                        "body": body,
                        "assets": assets,
                        "brought_in_by": brought_in_by,
                        "supersedes": null,
                    }),
                    meta,
                )]
            }
            PmAction::ReceiveExternalRequest {
                id,
                source,
                external_id,
                title,
                body,
                reporter,
                labels,
                url,
            } => {
                // Deterministic triage — single source of truth in crate::pm::triage
                // (also used by Projection::triage_request, so they can't drift).
                let (classification, severity) = crate::pm::triage::classify(title, body, labels);
                vec![ev(
                    project,
                    actor,
                    id,
                    "external_request",
                    EventType::ExternalRequestReceived,
                    json!({
                        "source": source,
                        "external_id": external_id,
                        "title": title,
                        "body": body,
                        "reporter": reporter,
                        "labels": labels,
                        "url": url,
                        "classification": classification,
                        "severity": severity,
                    }),
                    meta,
                )]
            }
            PmAction::SaveDiagram { id, title, data } => {
                let saved_by = match &actor {
                    Actor::Owner => "owner".to_string(),
                    Actor::Agent { id } => id.clone(),
                    Actor::System => "system".to_string(),
                };
                vec![ev(
                    project,
                    actor,
                    id,
                    "diagram",
                    EventType::DiagramSaved,
                    json!({
                        "title": title,
                        "data": data,
                        "saved_by": saved_by,
                    }),
                    meta,
                )]
            }
            PmAction::CreateDirective {
                id,
                kind,
                statement,
                scope,
                strength,
                supersedes,
            } => vec![ev(
                project,
                actor,
                id,
                "directive",
                EventType::ProjectDirectiveCreated,
                json!({
                    "kind": kind,
                    "statement": statement,
                    "scope": scope,
                    "strength": strength,
                    "created_by": null, // derived from the event actor by the reducer
                    "supersedes": supersedes,
                }),
                meta,
            )],
            PmAction::SuspendDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveSuspended,
                json!({}),
                meta,
            )],
            PmAction::ResumeDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveResumed,
                json!({}),
                meta,
            )],
            PmAction::ExpireDirective { directive_id } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveExpired,
                json!({}),
                meta,
            )],
            PmAction::SupersedeDirective {
                directive_id,
                by_directive_id,
            } => vec![ev(
                project,
                actor,
                directive_id,
                "directive",
                EventType::ProjectDirectiveSuperseded,
                json!({ "superseded_by": by_directive_id }),
                meta,
            )],
            PmAction::CreateObservation {
                id,
                severity,
                subject,
                body,
                pm_action_required,
            } => vec![ev(
                project,
                actor,
                id,
                "observation",
                EventType::ObservationCreated,
                json!({
                    "severity": severity,
                    "subject": subject,
                    "body": body,
                    "pm_action_required": pm_action_required,
                }),
                meta,
            )],
            PmAction::ProposeDecision {
                id,
                subject,
                options,
                recommendation,
                class,
                involvement,
            } => vec![ev(
                project,
                actor,
                id,
                "decision",
                EventType::DecisionProposed,
                json!({
                    "subject": subject,
                    "options": options,
                    "recommendation": recommendation,
                    "class": class,
                    "involvement": involvement,
                }),
                meta,
            )],
            PmAction::ProposeDirectiveChange {
                id,
                subject,
                kind,
                statement,
                scope,
                strength,
                supersedes,
            } => vec![ev(
                project,
                actor,
                id,
                "decision",
                EventType::DecisionProposed,
                json!({
                    "subject": subject,
                    "options": serde_json::json!({
                        "governance_change": {
                            "kind": kind,
                            "statement": statement,
                            "scope": scope,
                            "strength": strength,
                            "supersedes": supersedes,
                        },
                    }),
                    "recommendation": format!("Approve governance change: {subject}"),
                    "class": crate::pm::DecisionClass::GovernanceChange,
                    "involvement": crate::pm::OwnerInvolvement::Ask,
                }),
                meta,
            )],
            PmAction::MakeDecision {
                decision_id,
                approved,
                note,
            } => vec![ev(
                project,
                actor,
                decision_id,
                "decision",
                EventType::DecisionMade,
                // Same shape as the owner-authored builder (owner_decision_made /
                // decision_made_event): note normalized to a string field.
                json!({
                    "subject": "",
                    "approved": approved,
                    "note": note.as_deref().unwrap_or(""),
                }),
                meta,
            )],
            PmAction::SupersedeDecision {
                decision_id,
                by_decision_id,
            } => vec![ev(
                project,
                actor,
                decision_id,
                "decision",
                EventType::DecisionSuperseded,
                json!({ "superseded_by": by_decision_id }),
                meta,
            )],
            PmAction::ProposeConsultant {
                id,
                subject,
                role_id,
                involvement,
            } => vec![ev(
                project,
                actor,
                id,
                "decision",
                EventType::DecisionProposed,
                json!({
                    "subject": subject,
                    "options": serde_json::json!({
                        "consultant": { "role_id": role_id },
                    }),
                    "recommendation": format!("Approve adding a consultant: {subject}"),
                    "class": DecisionClass::AddConsultant,
                    "involvement": involvement,
                }),
                meta,
            )],
            PmAction::SendMessage { to, body } => vec![ev(
                project,
                actor,
                &format!("msg-{}", cause.sequence),
                "message",
                EventType::MessageSent,
                json!({ "to": to, "body": body }),
                meta,
            )],
            // --- Harness guards (2026-08-13) ---
            PmAction::SetBudget { limit_usd, warn_at } => vec![ev(
                project,
                actor,
                "budget",
                "budget",
                EventType::BudgetSet,
                json!({
                    "limit_usd": limit_usd,
                    "warn_at": warn_at.unwrap_or(0.80),
                }),
                meta,
            )],
            PmAction::PauseWork { reason } => vec![ev(
                project,
                actor,
                "work-pause",
                "guard",
                EventType::WorkPaused,
                json!({ "reason": reason, "by": who }),
                meta,
            )],
            PmAction::ResumeWork => vec![ev(
                project,
                actor,
                "work-pause",
                "guard",
                EventType::WorkResumed,
                json!({ "by": who }),
                meta,
            )],
            PmAction::NoOp => vec![],
        }
    }
}

/// Convert a `who` label to the typed actor. `"system"` and `"owner"` map to
/// their domain actors; anything else is an agent id.
pub fn actor_for(who: &str) -> Actor {
    match who {
        "system" => Actor::System,
        "owner" => Actor::Owner,
        id => Actor::Agent { id: id.to_string() },
    }
}

/// Build metadata linking the new events to the owner event that caused them —
/// the "why?" provenance chain (brief §11, addendum §24).
fn linked(causation: &Event, correlation: &str) -> Metadata {
    Metadata {
        correlation_id: Some(correlation.to_string()),
        causation_id: Some(causation.event_id),
        agent_run_id: Some(format!("sim-run-{}", causation.sequence)),
    }
}

/// Make a domain event with provenance already attached.
fn ev(
    project: &str,
    actor: Actor,
    id: &str,
    kind: &str,
    event_type: EventType,
    data: Value,
    meta: Metadata,
) -> Event {
    let mut e = Event::new(
        project,
        actor,
        event_type,
        Aggregate {
            kind: kind.to_string(),
            id: id.to_string(),
        },
        data,
    );
    e.metadata = meta;
    e
}
