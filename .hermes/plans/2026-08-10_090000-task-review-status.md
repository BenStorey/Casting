# Task Review Status — Implementation Plan

> **For Hermes:** implement with TDD; commit after each task.

**Goal:** Close the task lifecycle with a **review step** — work doesn't count as
"Done" until someone verifies it. Rounds out the last incomplete part of the
domain state machine and stress-tests the reducer + gate + read views.

## Model

- `TaskStatus::InReview` (between Working and Done).
- `Task` gains `review: Option<TaskReview>` where `TaskReview { reviewer: String,
  note: String, approved: bool }` (the verdict, kept for provenance).

## Events

- `TaskReadyForReview { reviewer }` → status = InReview (task was Working).
- `TaskReviewed { approved, note }` → if approved: status = Done, record review;
  if rejected: status = Working (rework), record review note.

## Actions (through the gate)

- `RequestReview { task_id, reviewer }` → TaskReadyForReview. validate: task
  exists and is Working.
- `ReviewTask { task_id, approved, note }` → TaskReviewed. validate: task exists
  and is InReview.

## PM wiring (plan_onboard)

After `CompleteTask task-core` (by Marcus), the PM assigns review to QA:
Marcus CompleteTask → PM RequestReview(task-core, maya) → Maya ReviewTask.

## Read views

- `persona_for`: highlights = *reviewed* (approved) Done tasks; a QA reviewer
  gets credit for reviews.
- context: no change needed (Done already excluded); open tasks incl. InReview.

## Files

| File | Change |
|---|---|
| `src/event.rs` | `TaskReadyForReview`, `TaskReviewed` |
| `src/projection.rs` | `TaskStatus::InReview`, `Task.review`, reducers |
| `src/actions.rs` | `RequestReview`, `ReviewTask` + gate + to_events |
| `src/persona.rs` | highlights only approved/reviewed done |
| `src/pm.rs` | onboarding review flow |
| `tests/task_review.rs` (new) | reducer + gate + e2e via drive_pm |

## Tasks (commit each)

1. model + reducer (status + review record) → commit
2. actions + gate → commit
3. PM onboarding wiring + persona view → commit
4. docs + count → commit

## Validation

- tests: ready→InReview; approved→Done+review recorded; rejected→Working
  (rework); gate rejects non-Working RequestReview / non-InReview ReviewTask;
  e2e: Marcus completes → PM requests QA review → Maya approves → task Done.
- Full gate `make`. Currently 123 tests.