//! Mapping a validated action into the ordered domain events (with provenance).
use super::action::PmAction;
use crate::event::{Actor, Aggregate, Event, EventType, Metadata};
use crate::policy::DecisionClass;
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
            PmAction::AssignTask { task_id, assignee } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskAssigned,
                json!({ "assignee": assignee }),
                meta,
            )],
            PmAction::ProvisionWorktree {
                task_id,
                slug,
                cargo_target_dir,
                port,
            } => vec![ev(
                project,
                actor,
                // aggregate keyed by the worktree (one per task)
                &format!("wt-{task_id}"),
                "worktree",
                EventType::WorktreeProvisioned,
                json!({
                    "task_id": task_id,
                    "branch": format!("casting/task-{slug}"),
                    "path": cargo_target_dir,
                    "cargo_target_dir": cargo_target_dir,
                    "port": port,
                }),
                meta,
            )],
            PmAction::StartTask { task_id } => vec![ev(
                project,
                actor,
                task_id,
                "task",
                EventType::TaskStarted,
                json!({}),
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
                // Deterministic triage — single source of truth in crate::triage
                // (also used by Projection::triage_request, so they can't drift).
                let (classification, severity) = crate::triage::classify(title, body, labels);
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
                    "class": crate::policy::DecisionClass::GovernanceChange,
                    "involvement": crate::policy::OwnerInvolvement::Ask,
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
