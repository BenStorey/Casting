//! Consultants — the company's loadable, shareable team packages.
//!
//! A **consultant** is a self-contained config package (one TOML file + a
//! system prompt) that defines an identity bound to a catalog **role**. Role
//! remains the capability atom (see `cast.rs`); a consultant is the *packaged*
//! form: identity/persona + role + routing hints + model binding + verification
//! expectations, ready to be handed to a provider when D2 wiring lands.
//!
//! The curated defaults ship **embedded in the binary** (the `cast/` directory)
//! so a fresh `cast run` works with zero setup. A user/technical power user can
//! **drop additional TOML files** (or override a default by id) into
//! `<project>/.casting/consultants/` — the loader overlays them on top of the
//! embedded defaults. This is the \"drop a config file to add a consultant\"
//! story, and because every package is self-contained + id-namespaced, the same
//! files are what a sharing/marketplace layer would later distribute.
//!
//! Design decisions (director-aligned):
//! - **No tool allowlists / blocked paths / minions / token budgets.** Isolation
//!   is a platform property (worktrees); agents act only through the validated
//!   `PmAction` vocabulary; cost flows through the existing `CostIncurred` /
//!   budget-guard seam. A consultant declares *capability*, not dangerous
//!   permissions.
//! - **Routing hints, not a rules engine.** `specializations` / `trigger_patterns`
//!   are legible data fed to the PM's routing *context* (the PM reasons over
//!   them); `specialists_for` is a starting signal, never an enforcement layer.
//! - **Model binding is per-consultant** and feeds D2 metering (the existing
//!   `CostMetering` / `CostIncurred` path), consistent with per-role model tiers.
//! - `review_required` maps onto the existing InReview gate (a task does not
//!   reach Done on faith).
//! - **Playbooks** are named step recipes a consultant offers for solving a
//!   problem class. They compile onto the existing task graph (no second
//!   workflow engine). See `playbook.rs` and the plan at
//!   `.hermes/plans/2026-08-16_130752-consultant-playbooks.md`.

use crate::consultants::cast_role::CastRole;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub mod cast_role;
pub mod loader;
pub mod playbook;

/// The consultant's model tier. Drives how D2 meters a call (the `cost_tier`
/// is a config-sourced hint; actual spend lands via `CostIncurred`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CostTier {
    Budget,
    #[default]
    Standard,
    Premium,
}

/// Routing hints the PM reasons over — **not** a hard assignment engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// What this consultant is good at (fed to the PM's routing context).
    #[serde(default)]
    pub specializations: Vec<String>,
    /// Case-insensitive keyword hints; a *suggestion* for the PM, never binding.
    #[serde(default)]
    pub trigger_patterns: Vec<String>,
    /// `true` = part of the default cast (hired by default). Mirrors how
    /// `AgentHired` / `DEFAULT_CAST` model \"always on\" vs. \"summoned\".
    #[serde(default)]
    pub auto_join: bool,
}

/// The model binding for this consultant (D2). Provider/model feed metering.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelConfig {
    /// e.g. \"openrouter\".
    #[serde(default)]
    pub provider: Option<String>,
    /// e.g. \"anthropic/claude-sonnet-5\". **D2 wiring sets the real id.**
    #[serde(default)]
    pub model_id: Option<String>,
    /// Optional per-consultant endpoint override (e.g. a package that must hit
    /// a local LiteLLM). When unset, the provider resolver supplies the default
    /// base_url (openrouter → api/v1, litellm → localhost:4000/v1).
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub cost_tier: CostTier,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// Verification expectations — maps onto the existing review gate.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// `true` = this consultant's output must pass the InReview gate before Done.
    #[serde(default)]
    pub review_required: bool,
}

/// A named, self-contained knowledge/skill slice in a consultant's private
/// bank. Uniform shape used two ways:
/// - `skills` slices are *procedures* — what this consultant can do that
///   others can't (differentiator). Surfaced to the PM for routing and
///   injected when a playbook step requires them.
/// - `knowledge` slices are *declarative reference* — facts that make the
///   consultant smarter (a language reference, an API cheatsheet). Injected
///   only at the steps that declare them.
///
/// Both are sized (`char_budget`) so the orchestrator can cap injection and
/// meter it through the existing cost seam — the whole bank is never dumped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetSlice {
    /// Stable id, unique within the consultant's bank (e.g. "kdb-language").
    pub id: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: String,
    /// Source file reference (informational; the body is loaded at load time).
    #[serde(default)]
    pub file: String,
    /// Max characters of this slice's body the orchestrator may inject at a
    /// step. Bounded context by design.
    pub char_budget: usize,
    /// The loaded slice body (resolved from `file` during loading). Skipped on
    /// serialize so large bodies never bloat config/API responses; loaded in
    /// memory only.
    #[serde(skip)]
    pub body: String,
}

/// A resolved role identity (from the catalog or a loaded consultant's
/// cast_role). Owned (not `&'static`) so it can come from configuration.
#[derive(Debug, Clone, Serialize)]
pub struct RoleInfo {
    pub id: String,
    pub title: String,
    pub scope: String,
}

/// A normalized, validated consultant — the runtime/API shape (also what the
/// D2 orchestrator reads for model + prompt + routing).
#[derive(Debug, Clone, Serialize)]
pub struct ConsultantConfig {
    /// Machine key (also the file name without `.toml`). Stable for sharing.
    pub id: String,
    /// Human display name.
    pub name: String,
    /// Display title (defaults to the role's title).
    pub title: String,
    /// The CastRole this consultant fills (the source of truth for role
    /// identity, title, scope, and assignability).
    pub cast_role: CastRole,
    /// The catalog role id this binds to (derived from `cast_role`).
    pub role: String,
    /// The role's display title (derived from `cast_role`).
    pub role_title: String,
    /// The role's governance scope (derived from `cast_role`).
    pub scope: String,
    /// Persona avatar path (served by the embedded SPA, `/avatars/*.svg`).
    pub avatar: Option<String>,
    /// Free-text strengths — fed to the PM's routing context.
    pub summary: Option<String>,
    /// The packaged system prompt file (relative path inside the package).
    pub system_prompt_file: Option<String>,
    /// The loaded system prompt text (the D2 setup prompt).
    pub system_prompt: Option<String>,
    pub routing: RoutingConfig,
    /// The ordered model chain: `models[0]` is the PREFERRED model, each
    /// subsequent entry is a fallback tried in order when the previous is
    /// unavailable (region block, rate limit, outage). An arbitrarily long
    /// list collapses to \"first is priority, walk the rest\". Empty = no
    /// binding (the env base config is used).
    pub models: Vec<ModelConfig>,
    /// Whether this consultant can be ASSIGNED implementation work. `false`
    /// marks a SPECIAL role (the PM, the Advisor): they coordinate / advise /
    /// strategize, but never take tasks like the assignable cast. Enforced by
    /// the policy gate (`is_valid_assignee` + the reserved-special-actor
    /// guard). Defaults to true.
    pub assignable: bool,
    /// How many tasks this consultant may work on simultaneously. The platform
    /// gives them that many persistent worktrees (slots). Defaults to 1.
    pub max_concurrent: usize,
    pub verification: VerificationConfig,
    /// Playbooks this consultant offers — named step recipes for a problem
    /// class. Multiple playbooks may share a `problem` at different cost bands.
    /// Validated at load time; bad playbooks reject the whole package.
    #[serde(default)]
    pub playbooks: Vec<playbook::Playbook>,
    /// The consultant's private **skills** bank — procedure slices
    /// (differentiator). Injected only when a step that requires them runs.
    #[serde(default)]
    pub skills: Vec<AssetSlice>,
    /// The consultant's private **knowledge** bank — declarative reference
    /// slices (makes the consultant smarter). Injected only when a step that
    /// requires them runs.
    #[serde(default)]
    pub knowledge: Vec<AssetSlice>,
}

impl ConsultantConfig {
    /// Look up a skill slice by id.
    pub fn skill(&self, id: &str) -> Option<&AssetSlice> {
        self.skills.iter().find(|s| s.id == id)
    }

    /// Look up a knowledge slice by id.
    pub fn knowledge(&self, id: &str) -> Option<&AssetSlice> {
        self.knowledge.iter().find(|k| k.id == id)
    }

    /// Resolve an asset slice by id across both banks (skills then knowledge).
    /// Used by the step executor to fetch required slices for injection.
    pub fn asset(&self, id: &str) -> Option<&AssetSlice> {
        self.skill(id).or_else(|| self.knowledge(id))
    }

    /// Resolve a playbook step's `requires_skills`/`requires_knowledge` ids
    /// against this consultant's banks into injection-ready slices. Each body
    /// is truncated to its `char_budget` at a char boundary (loading already
    /// validated the ids resolve, so missing slices are logged, not fatal).
    pub fn required_slices(
        &self,
        step: &crate::consultants::playbook::PlaybookStep,
    ) -> Vec<crate::runtime::context::InjectedSlice> {
        let mut out = Vec::new();
        let mut push = |slices: &[AssetSlice], kind: &str| {
            for s in slices {
                let body = crate::runtime::context::truncate_chars(&s.body, s.char_budget);
                out.push(crate::runtime::context::InjectedSlice {
                    kind: kind.to_string(),
                    id: s.id.clone(),
                    title: if s.title.is_empty() {
                        s.id.clone()
                    } else {
                        s.title.clone()
                    },
                    body,
                });
            }
        };
        for sk_id in &step.requires_skills {
            if let Some(s) = self.skill(sk_id) {
                push(std::slice::from_ref(s), "skill");
            } else {
                log::warn!(
                    "[consultant {}] step '{}' requires skill '{}' but it is missing",
                    self.id,
                    step.id,
                    sk_id
                );
            }
        }
        for k_id in &step.requires_knowledge {
            if let Some(k) = self.knowledge(k_id) {
                push(std::slice::from_ref(k), "knowledge");
            } else {
                log::warn!(
                    "[consultant {}] step '{}' requires knowledge '{}' but it is missing",
                    self.id,
                    step.id,
                    k_id
                );
            }
        }
        out
    }
    /// True if this consultant can be assigned implementation work (not a
    /// special coordinator/adviser role).
    pub fn is_assignable(&self) -> bool {
        self.assignable
    }

    /// The preferred model binding (`models[0]`), if any. D2 starts here and
    /// walks `models[1..]` as fallbacks on failure.
    pub fn primary_model(&self) -> Option<&ModelConfig> {
        self.models.first()
    }

    /// Every model binding in priority order (preferred first). Empty when
    /// the consultant declares no binding (falls back to the env base).
    pub fn model_chain(&self) -> &[ModelConfig] {
        &self.models
    }

    /// Does this consultant declare a hint that plausibly matches the task?
    /// **A hint only** — the PM makes the routing judgment over these.
    pub fn hint_matches(&self, task_description: &str) -> bool {
        let t = task_description.to_lowercase();
        self.routing
            .specializations
            .iter()
            .chain(self.routing.trigger_patterns.iter())
            .any(|k| t.contains(&k.to_lowercase()))
    }
}

/// The in-memory registry of available consultants, keyed by `id` and by role.
///
/// Not authoritative state — it's loaded **configuration**. Who is *actually*
/// on the team remains the event log (`AgentHired`); this answers \"what
/// consultants exist and what are they configured to do\".
#[derive(Debug, Clone, Default)]
pub struct ConsultantRegistry {
    by_id: HashMap<String, Arc<ConsultantConfig>>,
    by_role: HashMap<String, Arc<ConsultantConfig>>,
    /// Insertion order, so `all()` is deterministic.
    order: Vec<String>,
}

impl ConsultantRegistry {
    /// Number of known consultants.
    pub fn count(&self) -> usize {
        self.by_id.len()
    }

    /// All consultants, in deterministic (load/insertion) order.
    pub fn all(&self) -> Vec<&ConsultantConfig> {
        self.order
            .iter()
            .filter_map(|id| self.by_id.get(id).map(|c| c.as_ref()))
            .collect()
    }

    /// Look up a consultant by its `id`.
    pub fn by_id(&self, id: &str) -> Option<&ConsultantConfig> {
        self.by_id.get(id).map(|c| c.as_ref())
    }

    /// The consultant bound to a catalog role id (the first one registered).
    pub fn for_role(&self, role_id: &str) -> Option<&ConsultantConfig> {
        self.by_role.get(role_id).map(|c| c.as_ref())
    }

    /// Look up a consultant by its `CastRole` enum. This is the preferred
    /// lookup — code should reference roles, not individual names.
    pub fn for_cast_role(&self, role: CastRole) -> Option<&ConsultantConfig> {
        self.by_role.get(role.role_id()).map(|c| c.as_ref())
    }

    /// The **actor id** (consultant name) that fills the given `CastRole`, if
    /// a consultant is bound. This is the single role→actor resolution the
    /// application should use to identify the PM, the Advisor, or any
    /// role-backed actor — never a hardcoded id string tied to a folder name.
    pub fn actor_for(&self, role: CastRole) -> Option<&str> {
        self.for_cast_role(role).map(|c| c.id.as_str())
    }

    /// True if `actor` is the consultant filling `role` (see `actor_for`).
    pub fn actor_is(&self, actor: &str, role: CastRole) -> bool {
        self.actor_for(role) == Some(actor)
    }

    /// Ensure all 7 CastRole variants have exactly one bound consultant.
    /// Call after loading to fail early on startup.
    pub fn validate_all_roles_present(&self) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();
        for role in cast_role::ALL_CAST_ROLES {
            if !self.by_role.contains_key(role.role_id()) {
                missing.push(format!("missing consultant for role: {}", role.role_id()));
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// The default cast: consultants with `auto_join = true` (hired by default).
    pub fn default_cast(&self) -> Vec<&ConsultantConfig> {
        self.all()
            .into_iter()
            .filter(|c| c.routing.auto_join)
            .collect()
    }

    /// Consultants that hint-match a task description, best-match first. A
    /// **starting signal** for the PM's routing reasoning — never an
    /// enforcement layer. Only consultants with a positive hint match return.
    pub fn specialists_for(&self, task_description: &str) -> Vec<&ConsultantConfig> {
        let t = task_description.to_lowercase();
        let mut scored: Vec<(&ConsultantConfig, usize)> = self
            .by_id
            .values()
            .map(|c| {
                let score = c
                    .routing
                    .specializations
                    .iter()
                    .filter(|k| t.contains(&k.to_lowercase()))
                    .count()
                    + c.routing
                        .trigger_patterns
                        .iter()
                        .filter(|k| t.contains(&k.to_lowercase()))
                        .count();
                (c.as_ref(), score)
            })
            .collect();
        scored.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
        scored
            .into_iter()
            .filter(|(_, s)| *s > 0)
            .map(|(c, _)| c)
            .collect()
    }

    /// Every known role, derived from the registered consultants' `cast_role`
    /// fields. This is the dynamic role set a director can hire into (the
    /// directory IS the roster — there is no separate hardcoded catalog).
    /// Deduped by role id; one consultant per role is enforced at load.
    pub fn known_roles(&self) -> Vec<RoleInfo> {
        let mut out: Vec<RoleInfo> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for c in self.by_id.values() {
            let ri = RoleInfo {
                id: c.role.clone(),
                title: c.role_title.clone(),
                scope: c.scope.clone(),
            };
            if seen.insert(c.role.clone()) {
                out.push(ri);
            }
        }
        out
    }

    /// Resolve a role id (catalog OR package-defined) to its identity.
    pub fn resolve_role(&self, id: &str) -> Option<RoleInfo> {
        self.known_roles().into_iter().find(|r| r.id == id)
    }

    /// Resolve a role by its exact title (catalog OR package-defined).
    pub fn resolve_role_by_title(&self, title: &str) -> Option<RoleInfo> {
        self.known_roles().into_iter().find(|r| r.title == title)
    }

    // ── Playbook lookups ────────────────────────────────────────────────

    /// All playbooks offered by consultants with the given CastRole.
    pub fn playbooks_for(&self, role: CastRole) -> Vec<&playbook::Playbook> {
        self.for_cast_role(role)
            .map(|c| c.playbooks.iter().collect())
            .unwrap_or_default()
    }

    /// All playbooks across all consultants matching a problem class.
    pub fn playbooks_for_problem(
        &self,
        problem: &str,
    ) -> Vec<(&ConsultantConfig, &playbook::Playbook)> {
        let mut results = Vec::new();
        for c in self.all() {
            for pb in &c.playbooks {
                if pb.problem == problem {
                    results.push((c, pb));
                }
            }
        }
        results
    }

    /// Look up a playbook by its qualified id (`"{consultant_id}/{playbook.id}"`).
    pub fn playbook(&self, qualified_id: &str) -> Option<(&ConsultantConfig, &playbook::Playbook)> {
        let (consultant_id, pb_id) = qualified_id.split_once('/')?;
        let c = self.by_id(consultant_id)?;
        c.playbooks
            .iter()
            .find(|pb| pb.id == pb_id)
            .map(|pb| (c, pb))
    }
}
