Document-only review of /home/ben/casting/ARCHITECTURE.md at the claimed commit ef4cd25. I did not open source. Everything below is internal to this file: contradictions, incomplete inventories, and places the plan is weaker than the stated philosophy.

Biggest problems first.

1. Fail-closed is the slogan, not the default
The principles (ARCHITECTURE.md:39) say restrictive defaults. The actual rails are optional:
- Budget can be Disabled until the owner sets one (743-749). First live LLM run has no spend ceiling.
- Write-time integrity is opt-in via with_integrity() (301-306). Offline verify is thinner still: only DecisionMade→Proposed and TaskCompleted→Created (307-312), against ~60 event types.
- Auth is Option; None disables it (633-637). All of /api/state, /api/events, SSE, context, provenance, persona are listed as public reads (589-610).
- SecurityCritical maps to Notify, not Ask (468-472). Security work proceeds; the owner is only informed.

If the plan is “autonomous company, fail-closed,” the first improvements are: budget on by default, integrity on by default, read auth, and SecurityCritical → Ask.

2. The event inventory is already lying to the reader
The doc claims a complete verified inventory (871-872) and “all ~60 variants” (144), then:
- Governance events ProjectDirectiveCreated/Suspended/Resumed/Superseded/Expired are specified in §7.2 (433-437) but missing from the §3.2 catalog (144-164).
- Git events are cited as “ADDENDUM §23” (152) but the document ends at §19. That addendum does not exist here.
- Activity kinds include LlmCall, GitPush, Shell, Inline (486), but workspace_activity_for only maps WorktreeProvisioned and CommitRequested (501-506). LLM calls happen outside the durable-execution protocol (548-555). Half the execution model is aspirational.

An architecture doc that cannot list its own events is the first place drift will hide. Finish the catalog, delete the phantom §23 pointer, and either wire or delete unused ActivityKinds.

3. Two role systems, and the doc admits they disagree
§14.1: Stage Manager is “treated as non-assignable” even though is_assignable permits it; default_cast excludes it instead (702-703).
§14.4 still has a legacy catalog (engineer, qa, security, devops) plus CastRole-derived roles (718-722).
Registry must bind all 7 roles (715); default hired cast is 5 (722). Fine if intentional, but assignability now lives in three places: enum, catalog, setup list.

This is the highest-leverage structural cleanup: one CastRole, one assignability function, delete workspace/cast.rs as an authority.

4. Vocabulary vs authority will train the LLM to fail
§3.5 puts HARNESS GUARDS in the LLM-visible ACTION_VOCAB table, then marks them owner-only (222).
§3.5 says GOVERNANCE is “PM/owner only” (219); §7.3 says only the owner may create/change directives (441). Those two sentences cannot both be true.

If validate() has no wildcard and rejects owner-only actions (848, 228-234), advertising them in the prompt wastes tokens and teaches the model that the vocab is a suggestion. Diff advertised actions against the gate per actor and shrink the prompt.

5. “Properly inert without an orchestrator” is overstated
§1 and §19.2 say no orchestrator ⇒ recorded but no action (36, 843). The same doc has the git observer emitting Branch/Commit/Merge events on every PM tick (646-651), and the reconciler emitting EntityArchived / opinion supersedes on a 25-event cadence (728-739). Those are real mutations with no LLM.

Decide what “inert” means. Either the observer/reconciler are operator services (and the sentence changes) or they must not write when the orchestrator is absent.

6. Backend parity and sequence theology do not line up
SQLite: MAX+1 in an IMMEDIATE transaction, and “UNIQUE(project_id, sequence) enforces no gaps” (263-268). UNIQUE enforces no duplicates, not contiguity.
Postgres: atomic counter ON CONFLICT … RETURNING, retry up to 5 on UNIQUE_VIOLATION (270-274). That pattern usually burns sequences.
Offline verify treats 1..max gaps as corruption (308-309).

If the verifier is the contract, Postgres retries are a designed way to fail it. If gaps are allowed, the verifier is wrong. Pick one and make both backends and cast log --verify agree. Also resolve “single file events.db” (264) vs Backend::Sqlite { events, cursors, snapshots } (295) — one file or three?

Postgres as a first-class backend for a “one project, one binary, collocated .casting/” product (38, 684) is cloud-shaped complexity sitting next to a local-tool story. Worth asking whether it should stay in the plan or be deferred with multi-project.

7. Cost and context will grow faster than the plan admits
Phase 1 is one PM plan() per owner interrupt; Phase 2 is every actor with work, up to 10 iterations (343-345, 409). That is O(actors × 10) LLM calls per drain, plus a 220ms UI animation delay per planned step (394).
AgentContext ships full roster, directives, risks, briefings, requests (238-246) with no truncation or token budget. Archive is a cadence pass every 25 events (730, 739), not a per-call cap.
MessageSent is Tier-0 Interrupt (376). The wording “from owner” may or may not apply to agent-to-agent send_message. If it does not, chatter is a spend amplifier.
Budget default Disabled (748) makes all of the above unbounded on first live run.

Improvements: per-drain/per-turn token cap, context slimming before the LLM seam, wake-tier by sender, drop the 220ms delay from the control loop (UI can animate on SSE).

8. Dual write paths, thin integrity
Web/Telegram/git append via AppState::append (800-802). PM/actor actions go through validate() + to_events + append (384-390). The doc never says owner HTTP mutations hit the same gate. Integrity checks a handful of task/decision preconditions against a rebuilt projection (301-306) — check-then-act, TOCTOU under concurrency, O(N) if it rebuilds.

One append pipeline: every write, including owner and observer, should be gate + integrity + store. Integrity should be exhaustive or explicitly listed as “advisory, these N types only.”

Other structural cracks worth a look

Worktree story is two products. Per-task destroy-on-done vs persistent warm slots (664-667), plus write-time teardown on TaskCompleted/ChangeSetReady/MergeCompleted (393) and a StaleWorktreePass safety net (738). Persistent + eager teardown fight each other. Pick one primary model.

Git observer is coupled to the PM loop (observe before drain, 332, 806-807). A 5s debounce inside a 500ms wake loop mixes I/O polling with orchestration. It should be its own consumer, not a preface to respond().

Dedup key is (event_type_debug, aggregate_id, correlation_id) (383, 861). Debug formatting is not an identity. Use the serde tag / EventType discriminant.

Telegram first-DM-is-owner (785) plus Interrupt-tier MessageSent plus optional auth is an unauthenticated spend and control path.

OwnerChannel::send_message has no mention of going through the event log (775-778). Outbound owner mail may be a side channel the projection never sees.

Module map includes watchdog, mental, persona, provenance, repo_metrics (88, 83-84, 102-103) with almost no architecture. Either they are load-bearing (specify invariants) or they are premature surface area.

No frontend in a “complete inventory.” If the UI is in-repo, the doc is not complete. If it is not, say so.

list_projects exists on EventStore (259) while multi-project is explicitly deferred (38). Accidental multi-tenancy in a single-project binary.

Where to improve, in order

1. Close the philosophy gaps: default budget, default integrity, read auth, SecurityCritical involvement.
2. Make the event/action/activity inventories true and complete; kill unused ActivityKinds and the missing §23.
3. Collapse role authority to CastRole; delete the legacy catalog as a source of truth; fix Stage Manager in one place.
4. Align LLM vocab with the gate per actor.
5. Define inert vs always-on writers (observer, reconciler).
6. One sequence/gap contract across SQLite, Postgres, and verify.
7. Bound LLM cost: wake-by-sender, context budget, per-drain cap; move UI delay out of run_planned.
8. One write pipeline for owner, PM, observer.
9. Reconcile persistent vs ephemeral worktrees.
10. Decide whether Postgres/multi-project belongs in this binary’s plan at all.

The bones are coherent: event log as authority, LLM behind Orchestrator, no-wildcard policy gate, leapfrog-safe cursor, snapshot folded-through sequence (839-867). The risk is not missing ideas — it is duplicated authority, optional safety, and a surface area (activities, backends, role catalogs, public reads) larger than the single-project inert-by-default product described in §1.
