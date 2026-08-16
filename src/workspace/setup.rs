//! The **setup engine** (`cast init`) — onboarding as a shared, deterministic
//! flow (owner decision 2026-08-10: "offer both CLI wizard and UI, one engine").
//!
//! A fresh company is configured here: name, initial cast (roles from the
//! catalog), an optional owner token, and optional starting governance
//! directives. `SetupPlan` writes these as the initial event sequence into the
//! state store — **idempotently** (re-running never double-hires or re-creates).
//!
//! The engine is deliberately separate from _onboarding_: it seeds the company
//! and cast but does NOT fire the objective/message. That stays the owner's
//! first real message, which triggers `plan_onboard`. Because the engine and
//! onboarding both hire cast members, `plan_onboard` skips already-hired
//! agents (see pm.rs). This keeps setup and onboarding from fighting.
//!
//! Future first-run UI drives this SAME engine (no second copy of setup logic).

use crate::actions;
use crate::event::{Actor, Aggregate, Event, EventType};
use crate::runtime::directive::{DirectiveKind, DirectiveStrength};
use crate::store::EventStore;
use crate::store::SqliteEventStore;
use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;

/// What the owner wants for a fresh company.
#[derive(Debug, Clone, Default)]
pub struct SetupSpec {
    /// Human-readable company/product name.
    pub name: String,
    /// Role ids from the catalog to hire (e.g. ["engineer","qa"]). Empty =
    /// default cast.
    pub roles: Vec<String>,
    /// Optional owner bearer token (enables auth). Empty = auth off.
    pub owner_token: Option<String>,
    /// Optional starting governance directives (`ProjectDirectiveCreated`).
    pub directives: Vec<StartDirective>,
}

/// A starting governance directive (reuses the owner-authored event builder).
#[derive(Debug, Clone)]
pub struct StartDirective {
    pub id: String,
    pub kind: DirectiveKind,
    pub statement: String,
    pub scope: Vec<String>,
    pub strength: DirectiveStrength,
}

/// The plan to create a company, ready to apply against a state store.
pub struct SetupPlan {
    pub spec: SetupSpec,
    /// Which cast members will be hired (id + role title), for the summary.
    pub hires: Vec<(String, String)>,
}

impl SetupPlan {
    /// Build the plan from a spec. Resolves the default cast when `roles` is
    /// empty; validates every role is in the catalog.
    pub fn build(spec: SetupSpec) -> Result<Self> {
        let hires = resolve_hires(&spec.roles)?;
        Ok(SetupPlan { spec, hires })
    }

    /// Apply the setup to a state dir: open (or create) the DBs, append the
    /// initial events idempotently, and persist the runtime config (name +
    /// optional owner token) that `cast run` reads. Returns the number of
    /// events written (0 if the company is already set up — in which case the
    /// existing config is left untouched).
    pub fn apply(&self, dir: &std::path::Path) -> Result<u32> {
        let store = open_store(dir)?;
        let written = apply_to_store(&store, &self.spec, &self.hires)?;
        if written > 0 {
            write_config(dir, &self.spec)?;
        }
        Ok(written)
    }
}

/// Open (or create) the state-dir event store.
pub fn open_store(dir: &std::path::Path) -> Result<SqliteEventStore> {
    std::fs::create_dir_all(dir).context("create state dir")?;
    SqliteEventStore::open(dir.join("events.db"))
}

/// Resolve a list of role ids into concrete hires (agent id + role title),
/// using the default cast when `roles` is empty. Validates each role.
pub fn resolve_hires(roles: &[String]) -> Result<Vec<(String, String)>> {
    let roles: Vec<String> = if roles.is_empty() {
        crate::workspace::DEFAULT_CAST
            .iter()
            .map(|m| m.role_id.to_string())
            .collect()
    } else {
        roles.to_vec()
    };

    let mut hires = Vec::new();
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for role_id in &roles {
        let role = crate::workspace::role_by_id(role_id)
            .with_context(|| format!("unknown role in cast catalog: {role_id}"))?;
        // Canonical default-cast agent for that role if it's a default one,
        // else a per-role occurrence counter (role-1, role-2, ...).
        let agent_id = match default_agent_for(role_id) {
            Some(id) => id,
            None => {
                let n = seen.entry(role_id.clone()).or_insert(0);
                *n += 1;
                format!("{role_id}-{n}")
            }
        };
        hires.push((agent_id, role.title.to_string()));
    }
    Ok(hires)
}

/// Idempotently ensure a set of cast members are hired against a RUNNING
/// AppState (used by the web setup endpoint). Skips anyone already in the
/// projection. Returns the hires that were actually issued.
pub fn ensure_hires(
    state: &crate::pm::AppState,
    roles: &[String],
) -> Result<Vec<(String, String)>> {
    let hires = resolve_hires(roles)?;
    let existing: Vec<String> = state
        .projection()
        .ok()
        .map(|p| p.agents.iter().map(|a| a.id.clone()).collect())
        .unwrap_or_default();
    let mut issued = Vec::new();
    for (agent_id, role_title) in hires {
        if existing.contains(&agent_id) {
            continue;
        }
        state.append(Event::new(
            &state.project,
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: agent_id.clone(),
            },
            serde_json::json!({ "role": role_title }),
        ))?;
        issued.push((agent_id, role_title));
    }
    Ok(issued)
}

/// Persist the runtime config (name + owner token) that `cast run` reads.
pub fn persist_config(dir: &std::path::Path, name: &str, owner_token: Option<&str>) -> Result<()> {
    let spec = SetupSpec {
        name: name.to_string(),
        roles: vec![],
        owner_token: owner_token.map(str::to_string),
        directives: vec![],
    };
    write_config(dir, &spec)
}

/// Persist setup-time LLM api key and owner preferences (name, experience level)
/// into the existing config, MERGING so nothing is clobbered.
pub fn persist_setup_prefs(
    dir: &std::path::Path,
    owner_name: Option<&str>,
    experience_level: Option<&str>,
    api_key: Option<&str>,
) -> Result<()> {
    let prior = read_config(dir).unwrap_or(RuntimeConfig {
        name: String::new(),
        owner_name: None,
        experience_level: None,
        owner_token: None,
        api_key: None,
        telegram_token: None,
        telegram_chat_id: None,
    });
    let cfg = RuntimeConfig {
        name: prior.name,
        owner_name: owner_name.map(|s| s.to_string()).or(prior.owner_name),
        experience_level: experience_level
            .map(|s| s.to_string())
            .or(prior.experience_level),
        owner_token: prior.owner_token,
        api_key: api_key.map(|s| s.to_string()).or(prior.api_key),
        telegram_token: prior.telegram_token,
        telegram_chat_id: prior.telegram_chat_id,
    };
    let json = serde_json::to_string_pretty(&cfg)?;
    std::fs::write(dir.join(CONFIG_FILE), json).context("write persisted setup prefs")
}

/// The canonical agent id for a default-cast role, if any (so the wizard's
/// "add security" numbers don't collide with Marcus/Maya).
fn default_agent_for(role_id: &str) -> Option<String> {
    crate::workspace::DEFAULT_CAST
        .iter()
        .find(|m| m.role_id == role_id)
        .map(|m| m.agent_id.to_string())
}

/// Append the initial company events against an existing store, idempotently.
/// Assumes the store already has a seeded PM (like `cast run`); detects an
/// existing company by the project's first event.
fn apply_to_store(
    store: &SqliteEventStore,
    spec: &SetupSpec,
    hires: &[(String, String)],
) -> Result<u32> {
    use crate::projection::Projection;
    let project = "project-demo"; // single-project for now (multi-project later)

    if store.latest_sequence(project)? > 0 {
        return Ok(0); // company already exists — no-op
    }

    let mut written = 0u32;

    // 1. Create the company.
    store.append(Event::new(
        project,
        Actor::System,
        EventType::ProjectCreated,
        Aggregate {
            kind: "project".into(),
            id: project.into(),
        },
        serde_json::json!({ "name": spec.name }),
    ))?;
    written += 1;

    // 2. Hire the PM.
    store.append(Event::new(
        project,
        Actor::System,
        EventType::AgentHired,
        Aggregate {
            kind: "agent".into(),
            id: "pm".into(),
        },
        serde_json::json!({ "role": "Project Manager" }),
    ))?;
    written += 1;

    // 3. Hire the chosen cast members.
    for (agent_id, role_title) in hires {
        store.append(Event::new(
            project,
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: agent_id.clone(),
            },
            serde_json::json!({ "role": role_title }),
        ))?;
        written += 1;
    }

    // 4. Optionally write starting governance directives (owner-authored).
    for d in &spec.directives {
        store.append(actions::owner_directive_created(
            project,
            &d.id,
            d.kind,
            &d.statement,
            d.scope.clone(),
            d.strength,
        ))?;
        written += 1;
    }

    // 5. Seed a default budget so the spend breaker is never Disabled (§4.2.8).
    store.append(Event::new(
        project,
        Actor::System,
        EventType::BudgetSet,
        Aggregate {
            kind: "budget".into(),
            id: "budget".into(),
        },
        serde_json::json!({ "limit_usd": 10.0, "warn_at": 0.80 }),
    ))?;
    written += 1;

    // Rebuild the empty projection once to confirm it folds (cheap sanity).
    Projection::build(store, project)?;

    Ok(written)
}

/// Runtime config persisted by setup and read by `cast run` (name + auth).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeConfig {
    pub name: String,
    /// What the owner wants to be called (e.g. "Ben").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// How familiar the owner is with software dev — "novice" | "somewhat" | "confident".
    /// Used by the PM to calibrate how technically it explains things.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_token: Option<String>,
    /// LLM provider API key (e.g. OpenRouter). Persisted at setup so the user
    /// doesn't need the CAST_LLM_API_KEY env var for the default provider.
    /// Falls through as a fallback to the env var in the LLM config loader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Persisted Telegram channel config (2026-08-14). Set via the UI
    /// `POST /api/telegram/configure` so a user of Casting never touches env.
    /// Both are secrets-adjacent (a bot token; a user's chat id) and live in
    /// the gitignored `.casting/` dir, never in committed config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_chat_id: Option<i64>,
}

const CONFIG_FILE: &str = "config.json";

fn write_config(dir: &std::path::Path, spec: &SetupSpec) -> Result<()> {
    let cfg = RuntimeConfig {
        name: spec.name.clone(),
        owner_name: None,
        experience_level: None,
        owner_token: spec.owner_token.clone(),
        api_key: None,
        telegram_token: None,
        telegram_chat_id: None,
    };
    let json = serde_json::to_string_pretty(&cfg)?;
    let path = dir.join(CONFIG_FILE);
    std::fs::write(&path, json).context("write state-dir config")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .context("set config file permissions to 0600")?;
    Ok(())
}

/// Read the persisted runtime config (owner token + name), if present.
pub fn read_config(dir: &std::path::Path) -> Option<RuntimeConfig> {
    let raw = std::fs::read_to_string(dir.join(CONFIG_FILE)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Persist the Telegram channel config, MERGING into any existing config (so
/// an already-persisted owner token / name is never clobbered). If no config
/// exists yet we create a minimal one with an empty name. Mirrors the setup
/// "fresh-only" rule in the reverse direction: a UI Telegram configure never
/// wipes the owner token that `cast init`/setup already wrote.
pub fn persist_telegram_config(
    dir: &std::path::Path,
    token: impl Into<String>,
    chat_id: i64,
) -> Result<()> {
    let prior = read_config(dir).unwrap_or(RuntimeConfig {
        name: String::new(),
        owner_name: None,
        experience_level: None,
        owner_token: None,
        api_key: None,
        telegram_token: None,
        telegram_chat_id: None,
    });
    let cfg = RuntimeConfig {
        name: prior.name,
        owner_name: prior.owner_name,
        experience_level: prior.experience_level,
        owner_token: prior.owner_token,
        api_key: prior.api_key,
        telegram_token: Some(token.into()),
        telegram_chat_id: Some(chat_id),
    };
    let json = serde_json::to_string_pretty(&cfg)?;
    let path = dir.join(CONFIG_FILE);
    std::fs::write(&path, json).context("write persisted telegram config")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .context("set telegram config file permissions to 0600")?;
    Ok(())
}

/// Write a NO-SECRETS `casting.example.json` template to `dir` (the repo
/// root), documenting the canonical config shape. The real token never appears
/// here — this is the committed "like .env.example", never live state.
pub fn write_template(dir: &std::path::Path, name: &str) -> Result<()> {
    let cfg = serde_json::json!({
        "name": name,
        "owner_token": "<set at cast init; never commit a real token>",
    });
    let json = serde_json::to_string_pretty(&cfg)?;
    std::fs::write(dir.join("casting.example.json"), json).context("write casting.example.json")
}
