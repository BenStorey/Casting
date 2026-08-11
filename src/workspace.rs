//! The Ownership Boundary (docs/OWNERSHIP_BOUNDARY.md, D5).
//!
//! Casting operates on exactly one repo — the one it is explicitly handed at
//! startup — and keeps its internal state (the `--state-dir`) permanently
//! separate from that artifact repo. By construction it never conducts on the
//! repo that built it (the self-identity guard), and all Git runs through a
//! single pinned runner so a bare `git` call can never resolve to the wrong
//! repo.
//!
//! This is the foundation the Git slice builds on (§9 of that doc). Git
//! *semantics* (branches, ChangeSets, provenance) are deliberately out of scope
//! here — only the boundary itself.

use anyhow::{bail, Context, Result};
use std::path::{Component, Path, PathBuf};

/// Whether self-hosting (Casting building Casting) has been explicitly enabled.
/// Off by default; the §3 refusal only demotes to a banner when enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selfhost {
    Disabled,
    Enabled,
}

/// The canonical workspace: which artifact repo Casting drives, where its own
/// state lives, and whether self-hosting is permitted. Both paths are absolute
/// and canonical; they are guaranteed distinct and non-nested.
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Canonical absolute path to the artifact repo agents operate in.
    pub repo: PathBuf,
    /// Canonical absolute path to Casting's internal state dir (`--state-dir`).
    /// Never inside `repo` (and `repo` is never inside it).
    pub state_dir: PathBuf,
    selfhost: Selfhost,
}

impl Workspace {
    /// Open a workspace for `repo` (the artifact/project repo), enforcing the
    /// boundary:
    ///
    /// 1. the repo resolves to a canonical absolute path;
    /// 2. Casting's own state lives **collocated** in `<repo>/.casting/` (a
    ///    gitignored directory — the whole dir is self-ignored so it never
    ///    pollutes the user's git history);
    /// 3. unless `Selfhost::Enabled`, refuse to operate on the repo that built
    ///    this binary (embedded source root) or any repo whose identity is the
    ///    Casting crate (`name = "casting"`).
    pub fn open(repo: &Path, selfhost: Selfhost) -> Result<Self> {
        let repo = repo
            .canonicalize()
            .with_context(|| format!("canonicalize artifact repo {}", repo.display()))?;

        if selfhost == Selfhost::Disabled && is_casting_source(&repo) {
            bail!(
                "refusing to operate on the Casting source repo at {} — this is the \
                 repo that built this binary. To explicitly build Casting with Casting, \
                 re-run with --selfhost (or CAST_SELFHOST=1); see docs/OWNERSHIP_BOUNDARY.md.",
                repo.display()
            );
        }

        let state_dir = repo.join(".casting");
        Ok(Workspace {
            repo,
            state_dir,
            selfhost,
        })
    }

    /// The `.casting/` directory (Casting's internal state lives here,
    /// collocated inside the project repo and self-ignored by git).
    pub fn casting_dir(&self) -> &Path {
        &self.state_dir
    }

    /// Ensure Casting's state directory exists and is **self-ignored**: write
    /// `<repo>/.casting/.gitignore` = `*` (idempotent) so the whole directory
    /// never shows up as untracked/pending in the user's repo. Also makes sure
    /// the directory itself exists. Called at startup.
    pub fn ensure_self_ignored(&self) -> Result<()> {
        std::fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("create casting dir {}", self.state_dir.display()))?;
        let gi = self.state_dir.join(".gitignore");
        if !gi.exists() {
            std::fs::write(&gi, "*\n").with_context(|| format!("write {}", gi.display()))?;
        }
        Ok(())
    }

    /// Whether self-hosting is enabled on this workspace.
    pub fn selfhost(&self) -> Selfhost {
        self.selfhost
    }

    /// Resolve an agent-supplied path against the artifact repo, refusing any
    /// path that escapes it. Relative inputs resolve under `repo`; absolute or
    /// `..`-escaping inputs are rejected. Agents get NO cwd-relative resolution.
    pub fn resolve_under(&self, requested: &Path) -> Result<PathBuf> {
        let mut out = self.repo.clone();
        for c in requested.components() {
            match c {
                Component::CurDir => {}
                Component::Normal(seg) => out.push(seg),
                // `..` is allowed only while still inside the repo.
                Component::ParentDir => {
                    if !out.pop() {
                        bail!("path escapes the artifact repo: {}", requested.display());
                    }
                }
                // Absolute / prefix components would escape our anchor.
                Component::RootDir | Component::Prefix(_) => {
                    bail!(
                        "absolute component not allowed in a workspace path: {}",
                        requested.display()
                    );
                }
            }
        }
        if !out.starts_with(&self.repo) {
            bail!("path escapes the artifact repo: {}", requested.display());
        }
        Ok(out)
    }

    /// The ONLY way to invoke Git. Returns a `Command` pinned to `repo` (via
    /// `-C`) with `GIT_WORK_TREE`/`GIT_DIR` set, so raw, unscoped `git` is never
    /// exposed to agent code and a wrong-cwd `git` cannot reach another repo.
    pub fn git_command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C")
            .arg(&self.repo)
            .env("GIT_WORK_TREE", &self.repo)
            .env("GIT_DIR", self.repo.join(".git"));
        cmd
    }

    /// The repo's current HEAD (or `None` if it is not yet a git repo or has no
    /// commits yet). Uses the pinned runner, so exercizes the boundary on the
    /// correct repo only.
    pub fn head(&self) -> Option<String> {
        let out = self
            .git_command()
            .arg("rev-parse")
            .arg("HEAD")
            .output()
            .ok()?;
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!sha.is_empty()).then_some(sha)
        } else {
            None
        }
    }

    /// Ensure the artifact repo is a real git repository. If it has no `.git`,
    /// run `git init` (through the pinned runner) so the workspace is ready for
    /// agent-driven branch/commit workflows. Idempotent: a repo that already
    /// has a `.git` is left untouched. Returns `true` if a repo was created,
    /// `false` if one already existed.
    ///
    /// This wires Git into the workspace at startup (Git slice increment 1).
    /// After this call, `head()` will resolve (though it may still be `None`
    /// if the fresh repo has no commits yet).
    pub fn ensure_repo(&self) -> Result<bool> {
        if self.repo.join(".git").exists() {
            return Ok(false);
        }
        let out = self
            .git_command()
            .arg("init")
            .output()
            .with_context(|| format!("git init in {}", self.repo.display()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("git init failed in {}: {stderr}", self.repo.display());
        }
        Ok(true)
    }

    /// The current branch name (or `None` if HEAD is detached or the repo has
    /// no commits). Uses the pinned runner.
    pub fn current_branch(&self) -> Option<String> {
        let out = self
            .git_command()
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .output()
            .ok()?;
        if out.status.success() {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if name.is_empty() || name == "HEAD" {
                None
            } else {
                Some(name)
            }
        } else {
            None
        }
    }
}

fn is_casting_source(repo: &Path) -> bool {
    // Signal 1: this is the actual repo this binary was compiled from.
    if let Some(root) = option_env!("CASTING_SOURCE_ROOT") {
        if let Ok(rt) = Path::new(root).canonicalize() {
            if let Some(toplevel) = git_toplevel(repo) {
                if toplevel == rt {
                    return true;
                }
            }
        }
    }

    // Signal 2: identity — a Cargo.toml at the repo root naming the crate.
    if let Ok(manifest) = std::fs::read_to_string(repo.join("Cargo.toml")) {
        if manifest.contains("name = \"casting\"") {
            return true;
        }
    }

    false
}

/// Walk up from `from` to the nearest directory containing a `.git` entry.
fn git_toplevel(from: &Path) -> Option<PathBuf> {
    let mut cur = from;
    loop {
        if cur.join(".git").exists() {
            return Some(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p,
            _ => return None,
        }
    }
}
