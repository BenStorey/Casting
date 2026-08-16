//! Project workspace, setup, CLI infrastructure, secrets, and role catalog.
//!
//! A Casting project is a git repository with a `.casting/` state directory.
//! The workspace module manages that directory, git integration, worktrees,
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
pub use cast::role_catalog;
pub use cast::{role_by_id, role_by_title, CastMember, Role, DEFAULT_CAST};
pub use project::{ProvisionedWorktree, Selfhost, Workspace};
pub use secrets::SecretStore;
pub use setup::{RuntimeConfig, SetupPlan, SetupSpec};
