# Worktree commit & merge hygiene (skill — how to do this)

Procedure applied whenever you finish changes in your worktree and prepare
them to merge.

1. **Build first, never commit blind.** Run the crate's build/tests in your
   own worktree (your private `CARGO_TARGET_DIR` keeps you from colliding with
   other consultants' parallel builds). Nothing merges unless CI is green.
2. **Keep the diff scoped.** Only files in the task's declared scope. If the
   change spills into unrelated files, stop and flag it rather than expanding
   the diff.
3. **Write the regression test first** for the behaviour you changed, then the
   implementation. `tests/` is the integration home.
4. **Commit with a clear message** naming the task/feature and the change.
   Reference the design (`DESIGN.md`) artifact produced by the survey step.
5. **Respect merge authority.** `merge_authority = self` lets you merge small,
   low-blast-radius work directly after CI. `pm` means you request merge and
   the PM handles it. When in doubt, escalate to `pm`.
6. **Flag surprises** — anything the design didn't anticipate (schema change,
   API break, security implication) goes back to the PM via an observation,
   never silently into the merge.