//! The cast — the company's team.
//!
//! The roster is **`active-cast/` IS the roster**: the single source of truth
//! is the consultant registry (loaded from `active-cast/`, embedded in the
//! binary). Who is *actually* on the team is the event log (`AgentHired` /
//! `AgentRemoved`, reconciled against the directory by `CastReconcilePass`).
//!
//! There is **no** hardcoded role catalog or default-cast list here anymore —
//! the old `Role` / `LEGACY_ROLES` / `role_catalog` / `DEFAULT_CAST` tables
//! were removed. Role identity, title, scope, and assignability all come from
//! `crate::consultants::ConsultantConfig` (via `ConsultantRegistry`), and the
//! registry enforces exactly one consultant per role at load.
//!
//! Code that needs the list of roles a director can hire should use
//! `ConsultantRegistry::known_roles()`. This module exists only to keep the
//! `workspace::cast` path stable; it carries no data.
