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
});
