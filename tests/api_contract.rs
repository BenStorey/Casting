//! Frontend contract guard (2026-08-14, review item #8/#15).
//!
//! `frontend/src/api.ts` mirrors these Rust serde encodings BY HAND. When they
//! diverge, the UI silently breaks — a real bug: tasks in `InReview` vanished
//! from the board because the TS `TaskStatus` union lacked `"in_review"`, and a
//! `DecisionStatus::Superseded` rendered as a red failure because the TS mirror
//! was missing `"superseded"`. This file pins the Rust-side serialization so a
//! serde/rename change here FAILS this test, forcing `api.ts` to be updated in
//! the same commit. It is the deterministic half of keeping the single-authority
//! invariant from leaking into a drifting hand-maintained mirror.
//!
//! NOTE: the source of truth for the strings is `api.ts`; this test locks what
//! Rust PRODUCES. Keep the two in exact agreement; never weaken one side of the
//! mirror without the other.

use casting::event::EventType;
use casting::plan::Priority;
use casting::projection::{DecisionStatus, TaskStatus};

fn s<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap()
}

#[test]
fn task_status_serde_matches_frontend_union() {
    // frontend/src/api.ts: export type TaskStatus = "backlog" | "working" |
    // "blocked" | "in_review" | "done";
    assert_eq!(s(&TaskStatus::Backlog), "\"backlog\"");
    assert_eq!(s(&TaskStatus::Working), "\"working\"");
    assert_eq!(s(&TaskStatus::Blocked), "\"blocked\"");
    assert_eq!(s(&TaskStatus::InReview), "\"in_review\"");
    assert_eq!(s(&TaskStatus::Done), "\"done\"");
}

#[test]
fn decision_status_serde_matches_frontend() {
    // frontend/src/api.ts: export type DecisionStatus = "proposed" | "approved"
    // | "rejected" | "superseded";
    assert_eq!(s(&DecisionStatus::Proposed), "\"proposed\"");
    assert_eq!(s(&DecisionStatus::Approved), "\"approved\"");
    assert_eq!(s(&DecisionStatus::Rejected), "\"rejected\"");
    assert_eq!(s(&DecisionStatus::Superseded), "\"superseded\"");
}

#[test]
fn priority_serde_is_snake_case() {
    // frontend/src/api.ts Task.priority is a string; Rust emits snake_case.
    assert_eq!(s(&Priority::Low), "\"low\"");
    assert_eq!(s(&Priority::Medium), "\"medium\"");
    assert_eq!(s(&Priority::High), "\"high\"");
    assert_eq!(s(&Priority::Critical), "\"critical\"");
}

#[test]
fn event_type_serde_is_pascal_case_for_ui() {
    // ActivityView + the event stream match on the PascalCase `event_type` — to
    // write `event_type` matches, the TS filter must compare against these exact
    // strings (a previous bug lowercased one side and matched nothing).
    assert_eq!(s(&EventType::TaskCreated), "\"TaskCreated\"");
    assert_eq!(s(&EventType::ActivityFailed), "\"ActivityFailed\"");
    assert_eq!(s(&EventType::PlanActionRejected), "\"PlanActionRejected\"");
    assert_eq!(s(&EventType::MergeCompleted), "\"MergeCompleted\"");
}
