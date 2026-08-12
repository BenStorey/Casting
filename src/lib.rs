//! Casting — headless core, slice one.
//!
//! This is the smallest LLM-free foundation everything else bolts onto:
//!
//! - `event`: typed domain events (append-only history is the source of truth)
//! - `store` + `sqlite_store`: append-only event persistence, read-by-sequence
//! - `cursor`: durable per-consumer position in the event history
//!
//! See docs/ADDENDUM.md (PM control loop, Git/provenance) and
//! docs/PM_INVOCATION_TRIGGERS.md (wake/act) for the surrounding design.

pub mod actions;
pub mod auth;
pub mod backend;
pub mod cast;
pub mod context;
pub mod cursor;
pub mod directive;
pub mod event;
pub mod git_observer;
pub mod integrity;
pub mod mental;
pub mod orchestrator;
pub mod persona;
pub mod plan;
pub mod pm;
pub mod policy;
pub mod postgres_store;
pub mod projection;
pub mod provenance;
pub mod reconciler;
pub mod registry;
pub mod replay;
pub mod setup;
pub mod snapshot;
pub mod sqlite_store;
pub mod store;
pub mod triage;
pub mod web;
pub mod workspace;
