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
    /// External requests (product intake surface): issues/PRs raised outside.
    pub requests: RequestsView,
    /// Diagrams drawn + saved in the app (visual artifacts).
    pub diagrams: DiagramsView,
    /// Cost attribution (HARNESS #6) — total spend + per-agent, so budget is
    /// visible to the PM/owner, not (only) tracked implicitly.
    pub spend: SpendView,
    /// Per-actor operating context — EXACTLY what each model is handed when it
    /// plans. This is the heart of "see what the models are seeing".
    pub actor_contexts: Vec<crate::context::AgentContext>,
    /// The isolated consultant workspaces currently provisioned (2026-08-12):
    /// each summoned consultant's desk (task, branch, path, build target,
    /// port). The platform's structural-isolation boundary, visible at a glance.
    pub worktrees: Vec<WorktreeView>,
    /// Signals a stale/inconsistent state (e.g. same-subject opinion
    /// contradiction the reconciler hasn't fixed yet) the owner may want to see.
    pub drift_signals: Vec<String>,
}

/// A consultant's isolated workspace, as surfaced in the operating picture.
#[derive(Debug, Clone, Serialize)]
pub struct WorktreeView {
    pub task_id: String,
    pub branch: String,
    pub path: String,
    pub cargo_target_dir: String,
    pub port: u16,
}

/// Aggregated cost attribution for the operating picture.
#[derive(Debug, Clone, Serialize)]
pub struct SpendView {
    /// Total estimated USD across all cost entries.
    pub total_estimated_usd: f64,
    /// Total prompt & completion tokens across all entries.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Total input tokens served from the provider's prompt cache.
    pub cached_input_tokens: u64,
    /// Derived cache-hit ratio: cached / (prompt + cached) across all input,
    /// in 0..1. 0 when there's no input (or caching is unreported).
    pub cache_hit_ratio: f64,
    /// Mean wall-clock latency across entries that reported one (>0 ms).
    /// `None` when nothing reports latency (e.g. only scripted/mock calls).
    pub avg_latency_ms: Option<f64>,
    pub entries: usize,
    /// Per-agent spend (agent_id -> total USD), so individual consultants can
    /// be budgeted.
    pub by_agent: std::collections::BTreeMap<String, f64>,
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
    /// EXTERNAL advisor briefings imported into the project.
    pub briefings: AdvisoryView,
}

/// External advisor content, kept clearly separate from authoritative state.
/// `briefings.active` are the currently-considered advisories; `superseded`
/// keeps the audit trail. Each is marked with its `source` so it's never
/// confusable with the owner's own intent. Advisory can inform context, never
/// sets rules (directives remain the only authority mechanism).
#[derive(Debug, Clone, Serialize)]
pub struct AdvisoryView {
    /// Active briefings: "subject — title (source): body-preview".
    pub active: Vec<String>,
    /// Superseded briefings (history preserved, no longer at full weight).
    pub superseded: Vec<String>,
    /// How many advisory items are currently shaping context.
    pub active_count: usize,
}

/// External requests (product intake surface) — issues/PRs raised OUTSIDE, kept
/// separate from the owner's own Requirements. Each is triaged deterministically
/// (classification + severity). The PM later decides whether to act (D2).
#[derive(Debug, Clone, Serialize)]
pub struct RequestsView {
    /// Count of open (un-triaged/not-closed) requests.
    pub open_count: usize,
    /// Open requests, triaged: "[classification / severity] title (from reporter)".
    pub open: Vec<String>,
}

/// Diagrams drawn + saved in the app (visual artifacts). Index/titles summary;
/// the full Excalidraw JSON lives per-diagram in the projection for reload.
#[derive(Debug, Clone, Serialize)]
pub struct DiagramsView {
    /// Total diagrams saved.
    pub count: usize,
    /// "[title] — saved by X (id)" for each, newest-first.
    pub diagrams: Vec<String>,
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

        // External advisor briefings — kept separate, clearly marked advisory.
        use crate::projection::BriefingStatus;
        let active_briefings = self
            .briefings
            .iter()
            .filter(|b| b.status == BriefingStatus::Active)
            .map(|b| {
                let preview: String = b.body.chars().take(120).collect::<String>()
                    + if b.body.chars().count() > 120 {
                        "…"
                    } else {
                        ""
                    };
                format!("{} — {} ({}): {preview}", b.subject, b.title, b.source)
            })
            .collect::<Vec<_>>();
        let superseded_briefings = self
            .briefings
            .iter()
            .filter(|b| b.status == BriefingStatus::Superseded)
            .map(|b| format!("{} — {} (superseded)", b.subject, b.title))
            .collect::<Vec<_>>();
        let advisory_briefings = AdvisoryView {
            active_count: active_briefings.len(),
            active: active_briefings,
            superseded: superseded_briefings,
        };

        // External requests: deterministic triage summary for the operating picture.
        use crate::projection::ExternalRequestStatus;
        let open_requests = self
            .external_requests
            .iter()
            .filter(|r| r.status == ExternalRequestStatus::Open)
            .map(|r| {
                format!(
                    "[{}/{}] {} (from {})",
                    r.classification, r.severity, r.title, r.reporter
                )
            })
            .collect::<Vec<_>>();
        let requests = RequestsView {
            open_count: open_requests.len(),
            open: open_requests,
        };

        // Diagrams: summary index (newest-first) for the operating picture.
        let diagrams = DiagramsView {
            count: self.diagrams.len(),
            diagrams: self
                .diagrams
                .iter()
                .rev()
                .map(|d| format!("{} — saved by {} ({})", d.title, d.saved_by, d.id))
                .collect(),
        };

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
                briefings: advisory_briefings,
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
            requests,
            diagrams,
            spend: SpendView {
                total_estimated_usd: self.total_spend_usd(),
                prompt_tokens: self.total_prompt_tokens(),
                completion_tokens: self.spend.iter().map(|c| c.completion_tokens).sum(),
                cached_input_tokens: self.spend.iter().map(|c| c.cached_input_tokens).sum(),
                cache_hit_ratio: {
                    let cached: u64 = self.spend.iter().map(|c| c.cached_input_tokens).sum();
                    let total_input: u64 = self
                        .spend
                        .iter()
                        .map(|c| c.prompt_tokens + c.cached_input_tokens)
                        .sum();
                    if total_input > 0 {
                        cached as f64 / total_input as f64
                    } else {
                        0.0
                    }
                },
                avg_latency_ms: {
                    let lats: Vec<u64> = self
                        .spend
                        .iter()
                        .map(|c| c.latency_ms)
                        .filter(|&v| v > 0)
                        .collect();
                    if lats.is_empty() {
                        None
                    } else {
                        Some(lats.iter().sum::<u64>() as f64 / lats.len() as f64)
                    }
                },
                entries: self.spend.len(),
                by_agent: {
                    let mut m = std::collections::BTreeMap::new();
                    for c in &self.spend {
                        *m.entry(c.agent_id.clone()).or_insert(0.0) += c.estimated_usd;
                    }
                    m
                },
            },
            actor_contexts,
            worktrees: self
                .worktrees
                .iter()
                .map(|w| WorktreeView {
                    task_id: w.task_id.clone(),
                    branch: w.branch.clone(),
                    path: w.path.clone(),
                    cargo_target_dir: w.cargo_target_dir.clone(),
                    port: w.port,
                })
                .collect(),
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
