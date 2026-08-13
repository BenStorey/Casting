//! Loaders for consultant packages: the **embedded curated defaults** (shipped
//! with the binary from the `cast/` directory) plus **filesystem overlays** from
//! `<project>/.casting/consultants/` (user-dropped or id-replacing packages).
//!
//! Validation is strict and fail-closed: a package that references an unknown
//! role, has an empty id/name, names a missing system prompt, or sets an out-of
//! -range temperature is rejected loudly (a broken package must be visible, not
//! silently dropped).

use super::{
    ConsultantConfig, ConsultantRegistry, ModelConfig, NewRole, RoutingConfig, VerificationConfig,
};
use crate::cast::role_by_id;
use anyhow::{bail, Context, Result};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

/// The curated default consultant packages, embedded in the binary. The folder
/// always exists (it's tracked in git), so no build.rs placeholder is needed.
#[derive(RustEmbed)]
#[folder = "cast/"]
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
    /// Catalog role id this binds to. Optional: a package may define its own
    /// role via `[consultant.new_role]` instead of binding to the catalog.
    #[serde(default)]
    role: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    /// Relative path to the system prompt inside the package.
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    routing: RoutingConfig,
    #[serde(default)]
    model: ModelConfig,
    #[serde(default)]
    verification: VerificationConfig,
    /// An OPTIONAL self-defined role. When present, this consultant OWNS a new
    /// capability (role id/title/scope) instead of binding to a catalog role.
    #[serde(default)]
    new_role: Option<NewRole>,
}

impl ConsultantRegistry {
    /// Load the curated default set embedded in the binary (the `cast/`
    /// directory of TOML + prompt packages). Fails loudly if a default package
    /// is malformed — our shipped defaults should always be valid.
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
            let resolve_prompt = |p: &str| {
                ConsultantAssets::get(p).map(|f| String::from_utf8_lossy(&f.data).into_owned())
            };
            configs.push(
                from_raw(wrapped.consultant, &resolve_prompt)
                    .with_context(|| format!("validate {name}"))?,
            );
        }
        build_defaults(configs)
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
            let base = dir.to_path_buf();
            let resolve_prompt = move |p: &str| std::fs::read_to_string(base.join(p)).ok();
            let cfg = from_raw(wrapped.consultant, &resolve_prompt)
                .with_context(|| format!("validate {name}"))?;
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
fn overlay_insert(reg: &mut ConsultantRegistry, cfg: ConsultantConfig) {
    let id = cfg.id.clone();
    if !reg.by_id.contains_key(&id) {
        reg.order.push(id.clone());
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

/// Validate + normalize a raw package into a `ConsultantConfig`. `resolve_prompt`
/// loads the system prompt file given its package-relative path.
fn from_raw(
    raw: RawConsultant,
    resolve_prompt: &dyn Fn(&str) -> Option<String>,
) -> Result<ConsultantConfig> {
    let id = raw.id.trim().to_string();
    if id.is_empty() {
        bail!("consultant id may not be empty");
    }
    if raw.name.trim().is_empty() {
        bail!("consultant '{id}' has an empty name");
    }

    // The effective role: an inline `new_role` marks this consultant as the
    // owner of a brand-new capability; otherwise it binds to a catalog role.
    let (role_id, role_title, scope) = match raw.new_role {
        Some(nr) => {
            let rid = nr.id.trim().to_string();
            if rid.is_empty() {
                bail!("consultant '{id}' new_role needs a non-empty id");
            }
            let title = if nr.title.trim().is_empty() {
                rid.clone()
            } else {
                nr.title
            };
            (rid, title, nr.scope)
        }
        None => {
            if raw.role.trim().is_empty() {
                bail!("consultant '{id}' must bind to a `role` or define a `new_role`");
            }
            let role = role_by_id(&raw.role).with_context(|| {
                format!("consultant '{id}' references unknown role '{}'", raw.role)
            })?;
            (
                role.id.to_string(),
                role.title.to_string(),
                role.scope.to_string(),
            )
        }
    };

    if let Some(t) = raw.model.temperature {
        if !(0.0..=2.0).contains(&t) {
            bail!("consultant '{id}' temperature {t} out of range [0, 2]");
        }
    }

    let (system_prompt_file, system_prompt) = match &raw.system_prompt {
        None => (None, None),
        Some(p) => {
            let text = resolve_prompt(p)
                .with_context(|| format!("consultant '{id}' system_prompt '{p}' not found"))?;
            (Some(p.clone()), Some(text))
        }
    };

    Ok(ConsultantConfig {
        id,
        name: raw.name,
        title: raw.title.unwrap_or_else(|| role_title.clone()),
        role: role_id,
        role_title,
        scope,
        avatar: raw.avatar,
        summary: raw.summary,
        system_prompt_file,
        system_prompt,
        routing: raw.routing,
        model: raw.model,
        verification: raw.verification,
    })
}
