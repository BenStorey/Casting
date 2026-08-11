//! The operating picture — "what the models are seeing".
//!
//! `/api/state` returns the raw projection (every task, message, event); that's a
//! lot and isn't shaped for debugging misbehavior. This module derives a curated
//! read-model: the current objective and priorities, governance (directives +
//! decision policy + open decisions), knowledge (active opinions + facts),
//! assumptions/constraints/risks, and — crucially — the per-actor operating
//! context each model sees (`context_for`), so an owner or a tester debugging a
//! wrong-priority PM can see exactly what the models were working from. Pure
//! derivation; the event log stays the only authority.

use crate::directive;
use crate::plan::PlannedItem;
use crate::policy::DecisionPolicy;
use serde::Serialize;

/// The owner-facing / debugging "operating picture" of a project.
#[derive(Debug, Clone, Serialize)]
pub struct OperatingModel {
    pub project_id: String,
    pub objective: Option<String>,
    /// Tasks ranked by priority (Critical..Low).
    pub priorities: Vec<PlannedItem>,
    /// The governance posture: what's allowed/required and what's awaiting the
    /// owner.
    pub governance: GovernanceView,
    /// Knowledge the company has recorded (the "don't re-derive" layer).
    pub knowledge: KnowledgeView,
    /// The environmental state the company reasons about.
    pub context: ContextView,
    /// Per-actor operating context — EXACTLY what each model is handed when it
    /// plans. This is the heart of "see what the models are seeing".
    pub actor_contexts: Vec<crate::context::AgentContext>,
    /// Signals a stale/inconsistent state (e.g. same-subject opinion
    /// contradiction the reconciler hasn't fixed yet) the owner may want to see.
    pub drift_signals: Vec<String>,
}

/// Governance posture (directives + decision policy + open decisions).
#[derive(Debug, Clone, Serialize)]
pub struct GovernanceView {
    /// Active directives, strongest-first.
    pub active_directives: Vec<String>,
    /// The delegated-authority decision policy (per-class autonomy).
    pub decision_policy: DecisionPolicy,
    /// Decisions still awaiting the owner.
    pub open_decisions: Vec<String>,
}

/// Recorded knowledge: what the company believes / measured.
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeView {
    /// Currently-valid opinions, grouped as "subject: statement".
    pub opinions: Vec<String>,
    /// Superseded opinions (audit trail — deliberately visible so owners can
    /// see beliefs changed).
    pub superseded_opinions: Vec<String>,
    /// Recorded objective facts (point-in-time measures).
    pub facts: Vec<String>,
    pub assumptions: Vec<String>,
    pub constraints: Vec<String>,
}

/// The environmental state the company reasons about.
#[derive(Debug, Clone, Serialize)]
pub struct ContextView {
    pub open_risks: Vec<String>,
    pub open_requirements: Vec<String>,
    /// Counts, so a tester can sanity-check the model's view of scale.
    pub task_counts: TaskCounts,
    pub active_agents: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskCounts {
    pub total: usize,
    pub open: usize,
    pub in_review: usize,
    pub done: usize,
}

impl crate::projection::Projection {
    /// Derive the operating picture for this project (see module docs).
    pub fn operating_model(&self) -> OperatingModel {
        let plan = self.plan();

        let active_directives: Vec<String> = directive::active(self)
            .into_iter()
            .map(|d| format!("[{}] {}", d.kind.label(), d.statement))
            .collect();

        let open_decisions = plan.open_decisions.clone();

        let opinions = self
            .active_opinions()
            .into_iter()
            .map(|o| {
                let subj = if o.subject.is_empty() {
                    "(unspecified)".to_string()
                } else {
                    o.subject.clone()
                };
                format!("{subj}: {}{}", o.statement, tag_for(&o.category))
            })
            .collect::<Vec<_>>();

        let superseded_opinions = self
            .opinions
            .iter()
            .filter(|o| o.status == crate::projection::OpinionStatus::Superseded)
            .map(|o| format!("{}: {} (superseded)", o.subject, o.statement))
            .collect::<Vec<_>>();

        let facts = self
            .facts
            .iter()
            .map(|f| format!("[{}] {}", f.kind, f.statement))
            .collect::<Vec<_>>();

        let assumptions = self.assumptions.iter().map(|a| a.body.clone()).collect();
        let constraints = self.constraints.iter().map(|c| c.body.clone()).collect();

        let open_risks = self
            .risks
            .iter()
            .filter(|r| r.status == crate::projection::RiskStatus::Open)
            .map(|r| r.subject.clone())
            .collect::<Vec<_>>();

        let open_requirements = self
            .requirements
            .iter()
            .map(|r| r.title.clone())
            .collect::<Vec<_>>();

        let (open, in_review, done) =
            self.tasks
                .iter()
                .fold((0usize, 0usize, 0usize), |(o, i, d), t| {
                    use crate::projection::TaskStatus::*;
                    match t.status {
                        Done => (o, i, d + 1),
                        InReview => (o, i + 1, d),
                        _ => (o + 1, i, d),
                    }
                });

        let active_agents = self
            .agents
            .iter()
            .filter(|a| a.id != "pm")
            .map(|a| format!("{} ({})", a.id, a.role))
            .collect::<Vec<_>>();

        let actor_ids = {
            let mut v: Vec<String> = self.agents.iter().map(|a| a.id.clone()).collect();
            v.push("owner".to_string());
            v
        };
        let actor_contexts = actor_ids
            .into_iter()
            .map(|actor| self.context_for(&actor))
            .collect::<Vec<_>>();

        let drift_signals = same_subject_drift(self);

        OperatingModel {
            project_id: self.project_id.clone(),
            objective: plan.objective.clone(),
            priorities: plan.priorities.clone(),
            governance: GovernanceView {
                active_directives,
                decision_policy: self.policy.clone(),
                open_decisions,
            },
            knowledge: KnowledgeView {
                opinions,
                superseded_opinions,
                facts,
                assumptions,
                constraints,
            },
            context: ContextView {
                open_risks,
                open_requirements,
                task_counts: TaskCounts {
                    total: self.tasks.len(),
                    open,
                    in_review,
                    done,
                },
                active_agents,
            },
            actor_contexts,
            drift_signals,
        }
    }
}

/// Mechanical drift visible to an owner: same-subject opinions still marked
/// Active (a contradiction the reconciler hasn't cleaned up this window). This
/// is exactly the "PM keeps prioritizing wrong — what are they seeing?" cue.
fn same_subject_drift(proj: &crate::projection::Projection) -> Vec<String> {
    use std::collections::HashMap;
    let mut seen: HashMap<&str, &crate::projection::Opinion> = HashMap::new();
    let mut signals = Vec::new();
    for op in proj
        .opinions
        .iter()
        .filter(|o| o.status == crate::projection::OpinionStatus::Active)
    {
        if op.subject.trim().is_empty() {
            continue;
        }
        if let Some(prev) = seen.insert(&op.subject, op) {
            signals.push(format!(
                "subject {:?} has two Active opinions: {}/{} vs {}/{}",
                op.subject, prev.id, prev.statement, op.id, op.statement
            ));
        }
    }
    signals
}

/// A short tag for the category, so the dump reads naturally.
fn tag_for(category: &str) -> String {
    if category.is_empty() {
        String::new()
    } else {
        format!("  [{category}]")
    }
}
