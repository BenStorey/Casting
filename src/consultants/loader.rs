//! Loaders for consultant packages: the **embedded curated defaults** (shipped
//! with the binary from the `active-cast/` directory) plus **filesystem overlays**
//! from `~/.casting/<slug>/consultants/` (user-dropped or id-replacing packages).
//!
//! Since 2026-08-17 each consultant is its own **directory named by consultant
//! id**, not a single flat TOML. A directory package has a fixed structure:
//!
//! ```text
//! <id>/
//!   consultant.toml        // manifest: identity, role, models, routing,
//!                          //   skills/knowledge indices, playbook refs
//!   system_prompt.md       // optional persona (referenced by system_prompt_file)
//!   skills/<slice>.md      // optional capability/procedure slices
//!   knowledge/<slice>.md   // optional declarative-reference slices
//!   playbooks/<pb>.toml    // optional, one playbook per file
//! ```
//!
//! The manifest keeps identity/role/models/routing inline; the persona and
//! asset/playbook *bodies* live in referenced files (resolved at load time),
//! so large language references never bloat the manifest. A legacy single-file
//! package (inline `system_prompt` + inline `[[consultant.playbooks]]`) is still
//! accepted for backward compatibility in overlays.
//!
//! Validation is strict and fail-closed: an unknown cast_role, empty id/name,
//! out-of-range temperature, invalid playbook, unresolved referenced file, or
//! a playbook step requiring a skill/knowledge slice the consultant does not
//! own — each rejects the whole package loudly.

use super::cast_role::CastRole;
use super::playbook::validate_playbook;
use super::{
    AssetSlice, ConsultantConfig, ConsultantRegistry, ModelConfig, RoutingConfig,
    VerificationConfig,
};
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
    /// Inline system prompt text (legacy single-file packages, backward compat).
    #[serde(default)]
    system_prompt: Option<String>,
    /// Reference to the package-relative persona file (e.g. "system_prompt.md").
    /// Preferred over inline. Unresolved ref rejects the package.
    #[serde(default)]
    system_prompt_file: Option<String>,
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
    /// Playbooks this consultant offers. In the directory layout these are
    /// references (`[[consultant.playbooks]] file = "playbooks/x.toml"`); the
    /// inline full-playbook form (`id`/`version`/... with steps) is kept for
    /// legacy single-file packages.
    #[serde(default)]
    playbooks: Vec<RawPlaybookRef>,
    /// The consultant's private **skills** bank — procedure slices.
    #[serde(default)]
    skills: Vec<RawAssetSlice>,
    /// The consultant's private **knowledge** bank — declarative-reference slices.
    #[serde(default)]
    knowledge: Vec<RawAssetSlice>,
}

/// A playbook entry in the manifest: either a reference to a package-relative
/// file (directory layout) or an inline playbook (legacy single-file).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPlaybookRef {
    /// `file = "playbooks/x.toml"` — one playbook per file.
    File { file: String },
    /// Inline full playbook with steps (legacy).
    Inline(super::playbook::Playbook),
}

/// A skill/knowledge slice reference in the manifest (body lives in a file).
#[derive(Debug, Deserialize)]
struct RawAssetSlice {
    id: String,
    #[serde(default)]
    title: String,
    /// Package-relative file (e.g. "skills/kdb-language.md").
    file: String,
    /// Max chars of this slice's body the orchestrator may inject at a step.
    #[serde(default)]
    char_budget: usize,
}

fn default_assignable() -> bool {
    true
}

fn default_max_concurrent() -> usize {
    1
}

/// Reads a package-relative file. Returns `Ok(None)` if absent, `Err` if it
/// exists but cannot be read (embedded UTF-8 error, filesystem IO error).
impl ConsultantRegistry {
    /// Load the curated default set embedded in the binary (the `active-cast/`
    /// directory of per-consultant package directories). Validates that all 7
    /// CastRole variants are present. Fails loudly if a package is malformed.
    pub fn from_embedded() -> Result<Self> {
        // Enumerate package ids = top-level directories under active-cast/.
        let ids: std::collections::BTreeSet<String> = ConsultantAssets::iter()
            .map(|p| p.to_string())
            .filter_map(|p| p.split('/').next().map(String::from))
            .collect();

        let mut configs = Vec::new();
        for id in ids {
            let manifest = ConsultantAssets::get(&format!("{id}/consultant.toml"))
                .with_context(|| format!("package '{id}' missing consultant.toml"))?;
            let text = std::str::from_utf8(&manifest.data)
                .with_context(|| format!("package '{id}' manifest not valid UTF-8"))?
                .to_string();
            let wrapped: ConsultantFile =
                toml::from_str(&text).with_context(|| format!("parse {id}/consultant.toml"))?;
            let mut read = |rel: &str| -> Result<Option<String>> {
                let Some(f) = ConsultantAssets::get(&format!("{id}/{rel}")) else {
                    return Ok(None);
                };
                Ok(Some(
                    std::str::from_utf8(&f.data)
                        .with_context(|| format!("package '{id}' file '{rel}' not UTF-8"))?
                        .to_string(),
                ))
            };
            let cfg = from_package(wrapped.consultant, &mut read)
                .with_context(|| format!("validate {id}"))?;
            configs.push(cfg);
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

    /// Overlay user-supplied consultant packages from `dir` (the
    /// `~/.casting/<slug>/consultants/` directory) onto this registry.
    ///
    /// A package directory here is `<dir>/<id>/` (its own consultant.toml +
    /// referenced files). A legacy flat `<dir>/<id>.toml` single-file package
    /// is also accepted for backward compatibility and normalized the same way.
    ///
    /// A new `id` adds a consultant; an id matching an existing one **replaces**
    /// it (the user overrides a default by reusing its id). A missing directory
    /// is a no-op; a malformed present package is an error the caller can surface.
    pub fn overlay_dir(&mut self, dir: &Path) -> Result<usize> {
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut entries: Vec<String> = std::fs::read_dir(dir)
            .with_context(|| format!("read {}", dir.display()))?
            .filter_map(|e| e.ok())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let full = dir.join(&name);
                (name, full)
            })
            .filter(|(name, full)| name.ends_with(".toml") || full.is_dir())
            .map(|(name, _)| name)
            .collect();
        entries.sort();

        let mut loaded = 0;
        for name in entries {
            let path = dir.join(&name);
            if path.is_dir() {
                // Directory package: <id>/consultant.toml + referenced files.
                let manifest_path = path.join("consultant.toml");
                let text = match std::fs::read_to_string(&manifest_path) {
                    Ok(t) => t,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // A directory that isn't a consultant package (e.g. a
                        // stray README dir) is skipped, not fatal.
                        continue;
                    }
                    Err(e) => {
                        return Err(e).with_context(|| format!("read {}", manifest_path.display()))
                    }
                };
                let wrapped: ConsultantFile = toml::from_str(&text)
                    .with_context(|| format!("parse {}/consultant.toml", path.display()))?;
                let mut read = |rel: &str| -> Result<Option<String>> {
                    let p = path.join(rel);
                    if !p.is_file() {
                        return Ok(None);
                    }
                    Ok(Some(std::fs::read_to_string(&p)?))
                };
                let cfg = from_package(wrapped.consultant, &mut read)
                    .with_context(|| format!("validate {}", path.display()))?;
                let id = cfg.id.clone();
                if id != name {
                    bail!(
                        "overlay package directory '{}' must be named by its consultant id ('{id}')",
                        name
                    );
                }
                overlay_insert(self, cfg);
                loaded += 1;
            } else {
                // Legacy flat single-file package: <id>.toml (inline everything).
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("read {}", path.display()))?;
                let wrapped: ConsultantFile =
                    toml::from_str(&text).with_context(|| format!("parse {name}"))?;
                let cfg = from_package(wrapped.consultant, &mut |_| Ok(None))
                    .with_context(|| format!("validate {name}"))?;
                overlay_insert(self, cfg);
                loaded += 1;
            }
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

/// Load referenced files for an asset slice ref. A missing referenced file
/// rejects the package (fail-closed).
fn load_slice(
    id: &str,
    bank: &str,
    raw: RawAssetSlice,
    read: &mut dyn FnMut(&str) -> Result<Option<String>>,
) -> Result<AssetSlice> {
    if raw.id.trim().is_empty() {
        bail!("consultant '{id}' {bank} slice has empty id");
    }
    if raw.file.trim().is_empty() {
        bail!("consultant '{id}' {bank} slice '{}' has no file", raw.id);
    }
    let body = read(&raw.file)
        .with_context(|| format!("consultant '{id}' {bank} slice '{}'", raw.id))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "consultant '{id}' {bank} slice '{}' references missing file '{}'",
                raw.id,
                raw.file
            )
        })?;
    Ok(AssetSlice {
        id: raw.id,
        title: raw.title,
        file: raw.file,
        char_budget: raw.char_budget,
        body,
    })
}

/// Validate + normalize a raw package into a `ConsultantConfig`. The persona
/// and asset/playbook bodies are resolved through `read` (package-relative;
/// a closure over the embedded archive or the overlay filesystem).
fn from_package(
    raw: RawConsultant,
    read: &mut dyn FnMut(&str) -> Result<Option<String>>,
) -> Result<ConsultantConfig> {
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

    // Persona: prefer the referenced file, else the inline text (legacy).
    let (system_prompt_file, system_prompt) = match &raw.system_prompt_file {
        Some(f) if !f.trim().is_empty() => {
            let body = read(f)
                .with_context(|| format!("consultant '{id}' system_prompt_file '{f}'"))?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "consultant '{id}' system_prompt_file '{f}' references a missing file"
                    )
                })?;
            (Some(f.clone()), Some(body))
        }
        _ => (None, raw.system_prompt.filter(|s| !s.is_empty())),
    };

    // Load the private skills + knowledge banks (referenced files).
    let mut skills = Vec::with_capacity(raw.skills.len());
    for raw_slice in raw.skills {
        skills.push(load_slice(&id, "skills", raw_slice, read)?);
    }
    let mut knowledge = Vec::with_capacity(raw.knowledge.len());
    for raw_slice in raw.knowledge {
        knowledge.push(load_slice(&id, "knowledge", raw_slice, read)?);
    }
    // Slice ids must be unique within a bank (fail-closed, ambiguous injection).
    for bank in ["skills", "knowledge"] {
        let slices = if bank == "skills" {
            &skills
        } else {
            &knowledge
        };
        let mut seen = std::collections::HashSet::new();
        for s in slices {
            if !seen.insert(s.id.clone()) {
                bail!("consultant '{id}' duplicate {bank} slice id '{}'", s.id);
            }
        }
    }

    // Playbooks: inline (legacy) + referenced files, in declared order.
    let mut playbooks = Vec::new();
    for entry in raw.playbooks {
        match entry {
            RawPlaybookRef::Inline(pb) => playbooks.push(pb),
            RawPlaybookRef::File { file } => {
                if file.trim().is_empty() {
                    bail!("consultant '{id}' has an empty playbook file reference");
                }
                let text = read(&file)
                    .with_context(|| format!("consultant '{id}' playbook file '{file}'"))?
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "consultant '{id}' playbook file '{file}' references a missing file"
                        )
                    })?;
                #[derive(Deserialize)]
                struct PlaybookFile {
                    playbook: super::playbook::Playbook,
                }
                let pbf: PlaybookFile = toml::from_str(&text)
                    .with_context(|| format!("consultant '{id}' playbook file '{file}'"))?;
                playbooks.push(pbf.playbook);
            }
        }
    }
    for pb in &playbooks {
        validate_playbook(pb, &id)
            .map_err(|e| anyhow::anyhow!("playbook '{}' in consultant '{id}': {e}", pb.id))?;
        // Fail-closed: every step's requires_skills/requires_knowledge must
        // resolve against this consultant's banks.
        for step in &pb.steps {
            for sk in &step.requires_skills {
                if !skills.iter().any(|s| &s.id == sk) {
                    bail!(
                        "playbook '{}' in consultant '{id}' step '{}' requires skill '{}' which '{}' does not own",
                        pb.id, step.id, sk, id
                    );
                }
            }
            for k in &step.requires_knowledge {
                if !knowledge.iter().any(|s| &s.id == k) {
                    bail!(
                        "playbook '{}' in consultant '{id}' step '{}' requires knowledge '{}' which '{}' does not own",
                        pb.id, step.id, k, id
                    );
                }
            }
        }
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
        system_prompt_file,
        system_prompt,
        routing: raw.routing,
        models,
        assignable: raw.assignable,
        max_concurrent: raw.max_concurrent.max(1),
        verification: raw.verification,
        playbooks,
        skills,
        knowledge,
    })
}
