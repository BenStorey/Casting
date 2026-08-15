//! Casting — the autonomous software company in a box.
//!
//! Core slice: project lifecycle, typed domain events, event-sourced
//! projections, persistence backends, PM control loop, agent runtime,
//! git/provenance integration, web API, and LLM wiring seam.

pub mod actions;
pub mod consultants;
pub mod llm;
pub mod web;

pub mod event;
pub mod pm;
pub mod projection;
pub mod runtime;
pub mod store;
pub mod types;
pub mod workspace;
