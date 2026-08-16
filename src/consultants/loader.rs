//! Loaders for consultant packages: the **embedded curated defaults** (shipped
//! with the binary from the `active-cast/` directory) plus **filesystem overlays**
//! from `<project>/.casting/consultants/` (user-dropped or id-replacing packages).
//!
//! Every consultant TOML file is self-contained — the `system_prompt` field
//! carries the prompt text inline (not a file path). Playbooks are also inline
//! as `[[consultant.playbooks]]` tables with `[[consultant.playbooks.steps]]`.
//! This keeps each consultant fully self-contained.
//!
//! Validation is strict and fail-closed: a package with an unknown cast_role,
//! an empty id/name, an out-of-range temperature, or invalid playbook data is
//! rejected loudly.

use super::cast_role::CastRole;
use super::playbook::validate_playbook;
use super::{ConsultantConfig, ConsultantRegistry, ModelConfig, RoutingConfig, VerificationConfig};
use anyhow::{bail, Context, Result};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

/// The curated default consultant packages, embedded in the binary.
#[derive(RustEmbed)]
#[folder = "active-cast/"]
pub struct ConsultantAssets;

/// The `[consultant]` file wrapper.
#[derive(Debug, Deserialize)]
struct ConsultantFile {
    consultant: RawConsultant,
}

/// The raw on-disk shape (grouped tables), before validation/normalization.
#[derive(Debug, Deserialize)]
struct RawConsultant {
    id: String,
    name: String,
    #[serde(default)]
    title: Option<String>,
    /// Which CastRole this consultant fills (one of the 7 known roles).
    cast_role: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    /// Inline system prompt text. Self-contained — no file path resolution.
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    routing: RoutingConfig,
    /// The ordered model chain (preferred first). Backward-compatible with the
    /// legacy single `[consultant.model]` table: when `models` is empty the
    /// loader falls back to wrapping `model` (if any) as a one-entry chain.
    #[serde(default)]
    models: Vec<ModelConfig>,
    /// Legacy single-model binding. Kept for backward compatibility — the
    /// canonical form is the `models` list; `from_raw` normalizes a lone
    /// `model` into a one-element chain.
    #[serde(default)]
    model: Option<ModelConfig>,
    #[serde(default)]
    verification: VerificationConfig,
    /// Whether this consultant can be assigned implementation work. Marks a
    /// SPECIAL role (PM, Advisor) as `false`. Defaults to true.
    #[serde(default = "default_assignable")]
    assignable: bool,
    /// How many tasks this consultant may work on simultaneously (persistent
    /// worktree slots). Defaults to 1.
    #[serde(default = "default_max_concurrent")]
    max_concurrent: usize,
    /// Playbooks this consultant offers (inline TOML tables).
    #[serde(default)]
    playbooks: Vec<super::playbook::Playbook>,
}

fn default_assignable() -> bool {
    true
}

fn default_max_concurrent() -> usize {
    1
}

impl ConsultantRegistry {
    /// Load the curated default set embedded in the binary (the `active-cast/`
    /// directory of self-contained TOML packages). Validates that all 7
    /// CastRole variants are present. Fails loudly if a package is malformed.
    pub fn from_embedded() -> Result<Self> {
        let mut names: Vec<String> = ConsultantAssets::iter()
            .map(|p| p.to_string())
            .filter(|p| p.ends_with(".toml") && !p.contains('/'))
            .collect();
        names.sort();

        let mut configs = Vec::new();
        for name in names {
            let file = ConsultantAssets::get(&name).context("embed missing consultant package")?;
            let text =
                std::str::from_utf8(&file.data).context("consultant package not valid UTF-8")?;
            let wrapped: ConsultantFile =
                toml::from_str(text).with_context(|| format!("parse {name}"))?;
            configs.push(from_raw(wrapped.consultant).with_context(|| format!("validate {name}"))?);
        }
        let reg = build_defaults(configs)?;
        // Validate all 7 roles are present.
        reg.validate_all_roles_present().map_err(|missing| {
            anyhow::anyhow!(
                "active-cast/ missing consultants for roles: {}",
                missing.join(", ")
            )
        })?;
        Ok(reg)
    }

    /// Overlay user-supplied consultant packages from `dir` (the collocated
    /// `<project>/.casting/consultants/` directory) onto this registry.
    ///
    /// A new `id` adds a consultant; an id matching an existing one **replaces**
    /// it (the user overrides a default by reusing its id). A missing directory
    /// is a no-op; a malformed present file is an error the caller can surface.
    pub fn overlay_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .with_context(|| format!("read {}", dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|p| p.ends_with(".toml"))
            .collect();
        names.sort();

        let mut loaded = 0;
        for name in names {
            let path = dir.join(&name);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            let wrapped: ConsultantFile =
                toml::from_str(&text).with_context(|| format!("parse {name}"))?;
            let cfg = from_raw(wrapped.consultant).with_context(|| format!("validate {name}"))?;
            overlay_insert(self, cfg);
            loaded += 1;
        }
        Ok(loaded)
    }
}

/// Build a registry from the curated defaults (duplicate ids are a bug here).
fn build_defaults(configs: Vec<ConsultantConfig>) -> Result<ConsultantRegistry> {
    let mut reg = ConsultantRegistry::default();
    for cfg in configs {
        let id = cfg.id.clone();
        if reg.by_id.contains_key(&id) {
            bail!("duplicate default consultant id: {id}");
        }
        insert(&mut reg, cfg);
    }
    Ok(reg)
}

/// Overlay semantics: replace on id collision (a real override of a default).
/// When an overlay changes an id's `cast_role`, the old `by_role` entry is
/// removed so stale bindings never persist (P1.2 fix).
fn overlay_insert(reg: &mut ConsultantRegistry, cfg: ConsultantConfig) {
    let id = cfg.id.clone();
    if !reg.by_id.contains_key(&id) {
        reg.order.push(id.clone());
    } else {
        // Overriding an existing id: if the role changed, remove the old
        // by_role entry so stale bindings don't persist.
        if let Some(old) = reg.by_id.get(&id) {
            if old.role != cfg.role {
                reg.by_role.remove(&old.role);
            }
        }
    }
    reg.by_id.insert(id.clone(), Arc::new(cfg.clone()));
    reg.by_role.insert(cfg.role.clone(), Arc::new(cfg));
}

/// Default-set semantics: first role binding wins (keep the curated mapping).
fn insert(reg: &mut ConsultantRegistry, cfg: ConsultantConfig) {
    let role = cfg.role.clone();
    reg.order.push(cfg.id.clone());
    reg.by_id.insert(cfg.id.clone(), Arc::new(cfg.clone()));
    reg.by_role.entry(role).or_insert(Arc::new(cfg));
}

/// Validate + normalize a raw package into a `ConsultantConfig`. The
/// `system_prompt` is inline text (no file path resolution).
fn from_raw(raw: RawConsultant) -> Result<ConsultantConfig> {
    let id = raw.id.trim().to_string();
    if id.is_empty() {
        bail!("consultant id may not be empty");
    }
    if raw.name.trim().is_empty() {
        bail!("consultant '{id}' has an empty name");
    }

    // Resolve the cast_role to a CastRole variant — this is the source of
    // truth for role_id, title, scope, and assignability.
    let cast_role = CastRole::from_str(&raw.cast_role).with_context(|| {
        format!(
            "consultant '{id}' has unknown cast_role '{}'; expected one of: project_manager, advisor, lead_developer, testing_engineer, systems_architect, stage_manager, critic",
            raw.cast_role
        )
    })?;

    let role_id = cast_role.role_id().to_string();
    let role_title = cast_role.title().to_string();
    let scope = cast_role.scope().to_string();

    // Normalize the model chain: the canonical `models` list wins; a legacy
    // lone `model` is wrapped as a one-element chain. Every entry's temp
    // must be in range (fail loudly on a malformed package).
    let mut models = raw.models;
    if models.is_empty() {
        if let Some(m) = raw.model {
            models.push(m);
        }
    }
    for m in &models {
        if let Some(t) = m.temperature {
            if !(0.0..=2.0).contains(&t) {
                bail!("consultant '{id}' temperature {t} out of range [0, 2]");
            }
        }
    }

    // system_prompt is inline text — use it directly.
    let system_prompt = raw.system_prompt.filter(|s| !s.is_empty());

    // Validate playbooks
    let playbooks = raw.playbooks;
    for pb in &playbooks {
        validate_playbook(pb, &id)
            .map_err(|e| anyhow::anyhow!("playbook '{}' in consultant '{id}': {e}", pb.id))?;
    }

    Ok(ConsultantConfig {
        id,
        name: raw.name,
        title: raw.title.unwrap_or_else(|| role_title.clone()),
        cast_role,
        role: role_id,
        role_title,
        scope,
        avatar: raw.avatar,
        summary: raw.summary,
        system_prompt_file: None,
        system_prompt,
        routing: raw.routing,
        models,
        assignable: raw.assignable,
        max_concurrent: raw.max_concurrent.max(1),
        verification: raw.verification,
        playbooks,
    })
}
