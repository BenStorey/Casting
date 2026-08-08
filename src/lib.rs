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

pub mod cursor;
pub mod event;
pub mod sqlite_store;
pub mod store;
