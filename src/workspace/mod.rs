//! Project workspace, setup, CLI infrastructure, secrets, and role catalog.
//!
//! A Casting project is a git repository whose Casting state lives OUTSIDE it,
//! under `~/.casting/<slug>/` (one directory per project). The workspace module
//! manages that directory, git integration, worktrees, and the ownership
//! boundary that keeps the artifact repo byte-identical (Casting never writes
//! into it).
//! and self-hosting mode. Setup initialises new projects. Secrets provides
//! the `@secret:NAME@` resolution layer. Auth handles bearer-token access
//! control. Cast defines the built-in role catalog and default cast members.

pub mod auth;
pub mod cast;
pub mod git_observer;
pub mod project;
pub mod provenance;
pub mod repo_metrics;
pub mod secrets;
pub mod setup;

pub use auth::{authorized, bearer_token};
pub use project::{ProvisionedWorktree, Selfhost, Workspace};
pub use secrets::SecretStore;
pub use setup::{RuntimeConfig, SetupPlan, SetupSpec};
