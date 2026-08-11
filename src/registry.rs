//! The project registry (home-dir launcher, owner decision 2026-08-10).
//!
//! Multi-user is deliberately NOT supported — git is the collaboration surface,
//! and each human runs their own Casting setup. This registry is what a single
//! owner uses to keep a list of their projects in **`~/.casting/projects.json`**
//! (name → repo path). It answers "what projects do I have?" so `cast` needs no
//! path params; per-project *state* still lives collocated in `<repo>/.casting/`.
//!
//! Two different `.casting/` dirs:
//!   - `~/.casting/projects.json`  (this registry — the launcher)
//!   - `<repo>/.casting/`          (a project's live state, gitignored)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One registered project: a friendly name + the path to its git repo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectEntry {
    pub name: String,
    pub repo: PathBuf,
}

/// The default home-directory registry file.
pub fn default_registry_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".casting")
        .join("projects.json")
}

/// The registry: an ordered list of projects. Empty by default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Registry {
    pub projects: Vec<ProjectEntry>,
}

impl Registry {
    /// Load from `path` (or `default_registry_path()` when `None`). A missing
    /// file is an empty registry, not an error.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(default_registry_path);
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
        };
        let reg =
            serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
        Ok(reg)
    }

    /// Save to `path` (or `default_registry_path()` when `None`). Creates the
    /// `.casting/` home dir if missing.
    pub fn save(&self, path: Option<&Path>) -> Result<()> {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(default_registry_path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))
    }

    /// Register (or update, by name) a project. Returns whether it was new.
    pub fn register(&mut self, name: String, repo: PathBuf) -> bool {
        match self.projects.iter_mut().find(|p| p.name == name) {
            Some(existing) => {
                existing.repo = repo;
                false
            }
            None => {
                self.projects.push(ProjectEntry { name, repo });
                true
            }
        }
    }

    /// Remove a project by name. Returns whether it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.projects.len();
        self.projects.retain(|p| p.name != name);
        self.projects.len() != before
    }

    /// Look up a project's repo path by name.
    pub fn lookup(&self, name: &str) -> Option<&ProjectEntry> {
        self.projects.iter().find(|p| p.name == name)
    }
}
