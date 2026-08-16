import { describe, it, expect } from "vitest";
import { TASK_COLUMNS, ALL_TASK_STATUSES } from "../boardColumns";

// Regression guard for the bug where a task in InReview vanished from the
// board because TaskStatus had no matching column.
describe("board columns", () => {
  it("has exactly one column for every valid TaskStatus", () => {
    const keys = TASK_COLUMNS.map((c) => c.key);
    expect(new Set(keys).size).toBe(keys.length); // no duplicates
    for (const s of ALL_TASK_STATUSES) {
      expect(keys).toContain(s); // every status is reachable on the board
    }
  });

  it("has columns in standard kanban order (backlog → working → blocked → in_review → done)", () => {
    const keys = TASK_COLUMNS.map((c) => c.key);
    expect(keys).toEqual(["backlog", "working", "blocked", "in_review", "done"]);
  });

  it("has non-empty labels for every column", () => {
    for (const col of TASK_COLUMNS) {
      expect(col.label).toBeTruthy();
      expect(col.label.trim().length).toBeGreaterThan(0);
    }
  });

  it("has human-readable labels that match their status keys", () => {
    // Map each status to the expected label text
    const labelMap: Record<string, string> = {
      backlog: "Backlog",
      working: "Working",
      blocked: "Blocked",
      in_review: "In Review",
      done: "Done",
    };
    for (const col of TASK_COLUMNS) {
      expect(col.label).toBe(labelMap[col.key]);
    }
  });
});