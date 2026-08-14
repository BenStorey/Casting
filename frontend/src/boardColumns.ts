import type { TaskStatus } from "./api";

export interface TaskColumn {
  key: TaskStatus;
  label: string;
}

// The kanban board columns. Kept in a dependency-free module so it can be
// unit-tested without pulling in React, AND so the "does every TaskStatus have
// a column?" invariant is checkable (a task whose status has no column silently
// vanishes from the board — that was a real bug).
export const TASK_COLUMNS: TaskColumn[] = [
  { key: "backlog", label: "Backlog" },
  { key: "working", label: "Working" },
  { key: "blocked", label: "Blocked" },
  { key: "in_review", label: "In Review" },
  { key: "done", label: "Done" },
];

export const ALL_TASK_STATUSES: TaskStatus[] = [
  "backlog",
  "working",
  "blocked",
  "in_review",
  "done",
];
