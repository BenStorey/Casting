//! Per-project secret store — the ONLY place secret VALUES live.
//! (docs/plans/2026-08-13_harness-guards.md, feature 2)
//!
//! Why out of the event log: an append-only log is replayed forever and can
//! never be scrubbed. If a secret's raw value ever lands in an `Activity` (a
//! `Shell { cmd }` or `LlmCall { prompt }`), `ActivityScheduled` persists it
//! into the durable history immortally. So:
//!
//! - Values live only in this store, on disk under `<project>/.casting/`
//!   (already gitignored), NEVER in an event.
//! - An `Activity` references a secret by NAME using a placeholder token
//!   `@secret:NAME@`; the [runner/executor] substitutes at EXECUTION time in
//!   memory, never persisting the resolved value.
//! - [`ensure_no_raw_secrets`] is the fail-closed invariant: refuse to
//!   schedule/execute an activity that embeds a stored value verbatim. This is
//!   the one guard that is genuinely "hard to add later" — once leaked into the
//!   log, a secret is un-leak-able.
//!
//! The full request-scoped vault ceremony is deliberately deferred; the harness
//! performs side effects, so the runner holds the key and its value never needs
//! to enter an LLM context window at all.

use crate::executor::{Activity, ActivityKind};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Values shorter than this are not scanned for verbatim leaks (a one-char
/// secret like "a" would false-positive on any command containing "a"). API
/// keys / tokens are far longer in practice.
const MIN_SCAN_LEN: usize = 8;

/// The per-project secret store. Owned once per project and held by
/// `AppState`; a runner reads a value only at execution time, by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SecretStore {
    values: HashMap<String, String>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

impl SecretStore {
    /// Load from `<dir>/.casting/secrets.json` (gitignored), if present. A
    /// missing file yields an empty store (with persistence armed for `set`).
    pub fn load(casting_dir: &Path) -> Result<SecretStore> {
        let path = casting_dir.join("secrets.json");
        let store = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let s: SecretStore = serde_json::from_str(&raw).unwrap_or_default();
            SecretStore {
                path: Some(path),
                ..s
            }
        } else {
            SecretStore {
                path: Some(path),
                ..Default::default()
            }
        };
        Ok(store)
    }

    /// Read a secret by name (`None` if absent).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Set (and persist) a secret value. Callers reference it by name from
    /// this point on; the value must never be embedded in an activity.
    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        self.values.insert(name.to_string(), value.to_string());
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(p, serde_json::to_string_pretty(&self)?)?;
        }
        Ok(())
    }

    /// Whether `name` is present (so callers can validate before use).
    pub fn has(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// Resolve `@secret:NAME@` placeholder tokens in free text to the secret's
    /// VALUE. This is the runner seam: the resolved string exists only in
    /// memory at execution time and is never persisted. Unknown names resolve
    /// to the empty string (a caller that needs it may validate via `has`).
    pub fn resolve(&self, text: &str) -> String {
        let mut out = text.to_string();
        while let Some(start) = out.find("@secret:") {
            let after = &out[start + "@secret:".len()..];
            let Some(end_rel) = after.find('@') else {
                break;
            };
            let name = &after[..end_rel];
            let value = self.values.get(name).cloned().unwrap_or_default();
            let end = start + "@secret:".len() + end_rel + 1;
            out.replace_range(start..end, &value);
        }
        out
    }
}

/// Fail-closed invariant: refuse an `Activity` that embeds a stored secret's
/// RAW VALUE verbatim, so a secret can never be persisted to the event log
/// (via `ActivityScheduled`). Values are referenced by `@secret:NAME@` token
/// instead. Short values (below [`MIN_SCAN_LEN`]) are exempt to avoid
/// false-positive matches on ordinary text.
pub fn ensure_no_raw_secrets(store: &SecretStore, activity: &Activity) -> Result<()> {
    let payload = match &activity.kind {
        ActivityKind::LlmCall { prompt } => prompt,
        ActivityKind::Shell { cmd } => cmd,
        ActivityKind::GitPush { .. } | ActivityKind::Inline => return Ok(()),
    };
    for (name, value) in &store.values {
        if value.len() < MIN_SCAN_LEN || value.is_empty() {
            continue;
        }
        if payload.contains(value.as_str()) {
            bail!(
                "activity {} embeds the RAW value of secret '{name}'; reference it as \
                 @secret:{name}@ so it never reaches the event log",
                activity.id
            );
        }
    }
    Ok(())
}
