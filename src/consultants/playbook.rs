//! Playbook types — named, step-based recipes a consultant offers for solving a
//! problem class. Playbooks are loadable configuration, not a state machine.
//! They compile onto the existing task graph via `DecomposeTask` + `BlockTaskOn`.
//!
//! A consultant may offer multiple playbooks for the same `problem` at different
//! `cost_band` levels (cheap/medium/expensive). Cost band drives whether the PM
//! may apply it directly or must Ask the director first.
//!
//! See `/home/ben/casting/.hermes/plans/2026-08-16_130752-consultant-playbooks.md`
//! for the full design.

use serde::{Deserialize, Serialize};

/// How expensive a playbook is — drives director involvement through decision policy.
/// Missing cost_band in an ad-hoc recipe defaults to Expensive (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBand {
    Cheap,
    Medium,
    Expensive,
}

impl CostBand {
    /// Parse from a snake_case string as used in TOML.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cheap" => Some(Self::Cheap),
            "medium" => Some(Self::Medium),
            "expensive" => Some(Self::Expensive),
            _ => None,
        }
    }

    /// The stable id used in DecisionClass variant naming.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cheap => "cheap",
            Self::Medium => "medium",
            Self::Expensive => "expensive",
        }
    }
}

/// Where a playbook came from — packaged (embedded or overlay TOML) or
/// ad-hoc (PM-authored this run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybookSource {
    /// Shipped in active-cast/ or overlay TOML.
    #[default]
    Packaged,
    /// PM-authored for this run; recorded but not added to the catalog.
    AdHoc,
}

impl PlaybookSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Packaged => "packaged",
            Self::AdHoc => "ad_hoc",
        }
    }
}

/// A single step in a playbook recipe. Steps form a chain: step N reads
/// artifacts from step N-1, and all steps share the parent task's worktree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybookStep {
    /// Unique within the playbook (e.g. "survey", "critique", "ground").
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Which CostTier to use (budget|standard|premium). The resolver picks the
    /// first binding on the consultant's model chain that matches this tier.
    pub model: String,
    /// The step prompt — what this step must produce. Concatenated after the
    /// consultant's system_prompt at call time.
    pub prompt: String,
    /// Repo-relative path inside the parent worktree (e.g. "ARCHITECTURE.md").
    pub artifact: String,
    /// Token that later steps reference in `reads` (e.g. "survey").
    pub produces: String,
    /// Tokens from earlier steps this step depends on (each must match a prior
    /// step's `produces`).
    #[serde(default)]
    pub reads: Vec<String>,
    /// Skill slice ids (from the owning consultant's `skills` bank) that this
    /// step needs injected into its context to function. Resolved + inlined
    /// at dispatch time; only these exact slices (never the whole bank).
    #[serde(default)]
    pub requires_skills: Vec<String>,
    /// Knowledge slice ids (from the owning consultant's `knowledge` bank)
    /// that this step needs injected into its context. Same semantics as
    /// `requires_skills` — bounded, exact-slice injection.
    #[serde(default)]
    pub requires_knowledge: Vec<String>,
}

/// A named, versioned recipe for solving a problem class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playbook {
    /// Stable identifier, namespaced by consultant (e.g. "infra-review-deep").
    /// The full qualified id is `{consultant_id}/{playbook.id}`.
    pub id: String,
    /// Version number (0 for ad-hoc recipes).
    pub version: u32,
    /// Human-readable title.
    pub title: String,
    /// Problem class for routing (e.g. "architecture-review").
    pub problem: String,
    /// One-line summary.
    pub summary: String,
    /// Extra trigger keyword hints (advisory, fed to PM context).
    #[serde(default)]
    pub when: Vec<String>,
    /// Default CostIncurred.cost_class for steps (overrides role heuristic).
    #[serde(default = "default_cost_class")]
    pub cost_class: String,
    /// Cost band — required. Missing → the package is rejected.
    pub cost_band: CostBand,
    /// Where this playbook came from (packaged serialises as Packaged;
    /// ad-hoc serialises as AdHoc with version=0).
    #[serde(default)]
    pub source: PlaybookSource,
    /// Ordered steps. Non-empty, validated as a chain.
    #[serde(default)]
    pub steps: Vec<PlaybookStep>,
}

fn default_cost_class() -> String {
    "playbook".to_string()
}

/// Validate playbook configuration. Returns an error message for the first
/// problem found.
pub fn validate_playbook(pb: &Playbook, _consultant_id: &str) -> Result<(), String> {
    if pb.id.is_empty() {
        return Err("playbook id may not be empty".into());
    }
    if pb.title.is_empty() {
        return Err(format!("playbook '{}' title may not be empty", pb.id));
    }
    if pb.problem.is_empty() {
        return Err(format!("playbook '{}' problem may not be empty", pb.id));
    }
    if pb.steps.is_empty() {
        return Err(format!("playbook '{}' has no steps", pb.id));
    }

    let mut step_ids = std::collections::HashSet::new();
    let mut produced = std::collections::HashSet::new();
    for (i, step) in pb.steps.iter().enumerate() {
        if step.id.is_empty() {
            return Err(format!("playbook '{}' step {} has empty id", pb.id, i));
        }
        if !step_ids.insert(step.id.clone()) {
            return Err(format!(
                "playbook '{}' duplicate step id '{}'",
                pb.id, step.id
            ));
        }
        if step.title.is_empty() {
            return Err(format!(
                "playbook '{}' step '{}' has empty title",
                pb.id, step.id
            ));
        }
        // Validate model tier string
        if !matches!(step.model.as_str(), "budget" | "standard" | "premium") {
            return Err(format!(
                "playbook '{}' step '{}' unknown model '{}' (expected budget|standard|premium)",
                pb.id, step.id, step.model
            ));
        }
        // Artifact path must be relative and not escape
        if step.artifact.is_empty() {
            return Err(format!(
                "playbook '{}' step '{}' has empty artifact",
                pb.id, step.id
            ));
        }
        if step.artifact.starts_with('/') || step.artifact.contains("..") {
            return Err(format!(
                "playbook '{}' step '{}' artifact '{}' may not be absolute or contain '..'",
                pb.id, step.id, step.artifact
            ));
        }
        if step.produces.is_empty() {
            return Err(format!(
                "playbook '{}' step '{}' has empty produces",
                pb.id, step.id
            ));
        }
        if !produced.insert(step.produces.clone()) {
            return Err(format!(
                "playbook '{}' duplicate produce token '{}'",
                pb.id, step.produces
            ));
        }
        // Validate reads match prior produces
        for read in &step.reads {
            if !produced.contains(read.as_str()) {
                // reads should reference something already produced or the step itself
                // (allow self-read for survey steps that have no prior)
                if *read != step.produces {
                    return Err(format!(
                        "playbook '{}' step '{}' reads '{}' which is not produced by any earlier step",
                        pb.id, step.id, read
                    ));
                }
            }
        }
        // Validate required-slice ids are non-empty. Resolvability against the
        // owning consultant's banks is checked at package load (fail-closed).
        for (label, req) in [
            ("requires_skills", &step.requires_skills),
            ("requires_knowledge", &step.requires_knowledge),
        ] {
            for id in req {
                if id.trim().is_empty() {
                    return Err(format!(
                        "playbook '{}' step '{}' has an empty {label} id",
                        pb.id, step.id
                    ));
                }
            }
        }
    }

    // Validate cost_band
    // (already parsed by serde; just check it's not undefined — serde handles this)

    Ok(())
}
