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
    /// Harness guard rails (2026-08-13): the budget-breaker phase + any active
    /// pause. The owner reads this to see whether the cast is self-halted and
    /// why, especially while traveling / unattended.
    pub guards: GuardsView,
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
    /// Diagnostics audit trail (2026-08): refused PM actions + recorded
    /// orchestrator planning passes — the "what failed / what did the model
    /// do" surface for testing the LLM seam.
    pub diagnostics: DiagnosticsView,
    /// Owner engagement — is the owner engaging or muting? (metric for the
    /// "am I being escalated to death / is the owner AWOL?" meta-signal.)
    pub engagement: OwnerEngagementView,
    /// Code diff quality over time — language-agnostic churn from git, so a
    /// tester can see if the codebase is trending toward "LLM soup".
    pub diff_quality: DiffQualityView,
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
    /// Input tokens READ FROM the provider's prompt cache (a cache "hit").
    pub cache_read_input_tokens: u64,
    /// Input tokens WRITTEN to the provider's prompt cache (a "creation", NOT a
    /// hit — ~10x the read cost, so tracked separately to see caching's value).
    pub cache_creation_input_tokens: u64,
    /// Derived cache-hit ratio: cache_read / (prompt + read + creation) across
    /// all input, in 0..1. 0 when there's no input (or caching is unreported).
    pub cache_hit_ratio: f64,
    /// Mean wall-clock latency across entries that reported one (>0 ms).
    /// `None` when nothing reports latency (e.g. only scripted/mock calls).
    pub avg_latency_ms: Option<f64>,
    pub entries: usize,
    /// Per-agent spend (agent_id -> total USD), so individual consultants can
    /// be budgeted.
    pub by_agent: std::collections::BTreeMap<String, f64>,
}

/// Harness guard-rail status (2026-08-13, docs/plans/2026-08-13_harness-guards.md):
/// the budget breaker phase + any active pause. Read-only derivation; the event
/// log is the authority.
#[derive(Debug, Clone, Serialize)]
pub struct GuardsView {
    /// The owner-set budget, if any.
    pub budget: Option<BudgetView>,
    /// An active resumable pause, if any (reason/by/at).
    pub paused: Option<crate::guard::PauseInfo>,
}

/// The curated budget phase, as the owner reads it.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetView {
    pub limit_usd: f64,
    pub warn_at: f64,
    /// One of: disabled | ok | warn | halted.
    pub status: String,
    /// Current spend as a fraction of the limit (0..1+, 0 when unset).
    pub spend_fraction: f64,
}

/// The diagnostics audit trail (2026-08): refused PM actions + recorded
/// orchestrator planning passes. Surface (not the event log) for "what did
/// the model try, and what was refused / what did it cost."
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiagnosticsView {
    /// Total refused actions (count, for a badge).
    pub rejection_count: usize,
    /// Most recent refusals, newest first (bounded).
    pub recent_rejections: Vec<crate::projection::ActionRejection>,
    /// Total recorded orchestrator passes.
    pub orchestration_count: usize,
    /// Most recent orchestrator passes, newest first (bounded).
    pub recent_orchestration: Vec<crate::projection::OrchestrationRun>,
}

/// Owner engagement — "is the owner engaging, or muting?" (a week-1 metric,
/// from the meta-pattern: measure owner response rate). Purely derived from the
/// decision log. The signal: a growing `awaiting_owner` backlog with a falling
/// `response_rate` is escalation fatigue / owner abandonment — the PM is asking
/// things the owner isn't answering.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OwnerEngagementView {
    /// Open decisions that still REQUIRE the owner (involvement Ask, not yet
    /// decided or superseded). Work is blocked on each of these.
    pub awaiting_owner: usize,
    /// Decisions the owner has ruled on (decided_by == owner).
    pub owner_decided: usize,
    /// Decisions handled autonomously (decided by the PM/agent, not the owner).
    pub delegated_decided: usize,
    /// `owner_decided / (owner_decided + awaiting_owner)`. 1.0 = fully caught
    /// up; falls toward 0 as unanswered escalations pile up.
    pub response_rate: f64,
}

/// Code diff quality over time — "is the code getting worse?" (a week-1 metric,
/// from the meta-pattern: code diff quality). Language-agnostic churn captured
/// from git `--numstat` at observe time, so no formatter/linter assumption. The
/// signals: average churn per commit (rising = soup accretion / whole-section
/// rewrites) and the count of "large rewrite" commits.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffQualityView {
    /// Commits with recorded churn stats.
    pub commit_count: usize,
    pub total_additions: u64,
    pub total_deletions: u64,
    pub total_files: u64,
    /// Mean lines changed (add + del) per commit.
    pub avg_churn_per_commit: f64,
    /// Mean files touched per commit.
    pub avg_files_per_commit: f64,
    /// Commits that changed more than `large_rewrite_threshold` lines net —
    /// the "chef rewrote the whole dish" smell.
    pub large_rewrites: usize,
    /// Lines-changed threshold that counts a commit as a "large rewrite".
    pub large_rewrite_threshold: u64,
    /// Recent commits with per-commit churn, newest-first (bounded).
    pub recent: Vec<CommitChurnView>,
}

/// Per-commit churn, as surfaced in the diff-quality view.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CommitChurnView {
    pub sha: String,
    pub branch: String,
    pub task_id: Option<String>,
    pub message: String,
    pub additions: u64,
    pub deletions: u64,
    pub files: u64,
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

        // Owner engagement: escalation-fatigue signal from the decision log.
        let engagement = owner_engagement(self);
        // Code diff quality: language-agnostic churn from the observed commits.
        let diff_quality = diff_quality(self);

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
                cache_read_input_tokens: self.spend.iter().map(|c| c.cache_read_input_tokens).sum(),
                cache_creation_input_tokens: self
                    .spend
                    .iter()
                    .map(|c| c.cache_creation_input_tokens)
                    .sum(),
                cache_hit_ratio: {
                    let read: u64 = self.spend.iter().map(|c| c.cache_read_input_tokens).sum();
                    let creation: u64 = self
                        .spend
                        .iter()
                        .map(|c| c.cache_creation_input_tokens)
                        .sum();
                    let fresh: u64 = self.spend.iter().map(|c| c.prompt_tokens).sum();
                    let total_input = fresh + read + creation;
                    if total_input > 0 {
                        read as f64 / total_input as f64
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
            guards: GuardsView {
                budget: self.budget.as_ref().map(|b| {
                    let status = crate::guard::budget_status(self);
                    BudgetView {
                        limit_usd: b.limit_usd,
                        warn_at: b.warn_at,
                        status: status.label().to_string(),
                        spend_fraction: crate::guard::budget_fraction(self),
                    }
                }),
                paused: self.paused.clone(),
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
            diagnostics: DiagnosticsView {
                rejection_count: self.rejections.len(),
                recent_rejections: self.rejections.iter().rev().take(20).cloned().collect(),
                orchestration_count: self.orchestration.len(),
                recent_orchestration: self.orchestration.iter().rev().take(20).cloned().collect(),
            },
            engagement,
            diff_quality,
        }
    }
}

/// Derive the owner-engagement view from the decision log. See
/// [`OwnerEngagementView`] for semantics.
fn owner_engagement(proj: &crate::projection::Projection) -> OwnerEngagementView {
    use crate::policy::OwnerInvolvement;
    use crate::types::DecisionStatus;
    let mut awaiting = 0usize;
    let mut owner_decided = 0usize;
    let mut delegated = 0usize;
    for d in &proj.decisions {
        match d.decided_by.as_deref() {
            Some("owner") => owner_decided += 1,
            Some(_) => delegated += 1,
            None => {}
        }
        if d.involvement == OwnerInvolvement::Ask && d.status == DecisionStatus::Proposed {
            awaiting += 1;
        }
    }
    let denom = owner_decided + awaiting;
    OwnerEngagementView {
        awaiting_owner: awaiting,
        owner_decided,
        delegated_decided: delegated,
        response_rate: if denom > 0 {
            owner_decided as f64 / denom as f64
        } else {
            1.0
        },
    }
}

/// Threshold (lines added + deleted) above which a commit counts as a "large
/// rewrite" — the soup-accretion smell a tester wants flagged.
const LARGE_REWRITE_THRESHOLD: u64 = 500;

/// Derive the code diff-quality view from the observed commits' churn. See
/// [`DiffQualityView`] for semantics.
fn diff_quality(proj: &crate::projection::Projection) -> DiffQualityView {
    let commits = &proj.commits;
    let total_additions: u64 = commits.iter().map(|c| c.additions).sum();
    let total_deletions: u64 = commits.iter().map(|c| c.deletions).sum();
    let total_files: u64 = commits.iter().map(|c| c.files).sum();
    let n = commits.len();
    let total_churn = total_additions + total_deletions;
    let large_rewrites = commits
        .iter()
        .filter(|c| c.additions + c.deletions > LARGE_REWRITE_THRESHOLD)
        .count();
    DiffQualityView {
        commit_count: n,
        total_additions,
        total_deletions,
        total_files,
        avg_churn_per_commit: if n > 0 {
            total_churn as f64 / n as f64
        } else {
            0.0
        },
        avg_files_per_commit: if n > 0 {
            total_files as f64 / n as f64
        } else {
            0.0
        },
        large_rewrites,
        large_rewrite_threshold: LARGE_REWRITE_THRESHOLD,
        recent: commits
            .iter()
            .rev()
            .take(20)
            .map(|c| CommitChurnView {
                sha: c.sha.clone(),
                branch: c.branch.clone(),
                task_id: c.task_id.clone(),
                message: c.message.clone(),
                additions: c.additions,
                deletions: c.deletions,
                files: c.files,
            })
            .collect(),
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
