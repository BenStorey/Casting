Yes — you've identified a **very important problem**, and I think it points to a refinement of the event model.

The mistake would be to think:

> "The event stream is the state."

It isn't.

I'd make the architecture explicitly have **three layers**:

```text
                    HISTORY
                       │
             immutable events/facts
                       │
                       ▼
                 STATE / PROJECTION
                       │
             current authoritative truth
                       │
                       ▼
                  CONTEXT VIEW
                       │
             what this agent needs now
```

The key is that **merging should normally be deterministic, not an LLM operation**.

---

# 1. Your priority example is exactly right

Suppose initially we have:

```text
Task A — Authentication     Priority: High
Task B — Billing            Priority: Medium
Task C — Onboarding         Priority: Medium
Task D — Analytics          Priority: Low
```

Then the owner says:

> Deprioritise authentication. It's not important right now.

A naïve event might be:

```text
PriorityChanged
{
    task: auth,
    priority: low
}
```

That's fine.

But if we instead treat the owner's statement as some kind of new "priority state":

```text
"Authentication isn't important right now."
```

we've got a problem.

The system still needs to know:

```text
Billing       Medium
Onboarding    Medium
Analytics     Low
Authentication Low
```

And if six more changes happen, we don't want to make the PM repeatedly reconstruct the entire priority system from natural-language statements.

---

# 2. The answer is: events are mutations, projections are state

I'd make this a fundamental rule:

> **Events describe changes. Projections represent the resulting state.**

So:

```text
TaskPriorityChanged
    task_id = auth
    from = high
    to = low
```

is an event.

The projection becomes:

```text
tasks
────────────────────────
auth          low
billing       medium
onboarding    medium
analytics     low
```

Then another event:

```text
TaskPriorityChanged
    task_id = billing
    from = medium
    to = high
```

produces:

```text
auth          low
billing       high
onboarding    medium
analytics     low
```

**No LLM is involved in this merge.**

It's simply a deterministic state transition.

---

# 3. This applies far beyond priorities

And this is where I think your observation is really important.

You will encounter this everywhere.

For example:

### Assignment

```text
Marcus owns authentication
```

then:

```text
Move authentication to Sarah
```

Current state:

```text
Authentication → Sarah
```

You don't need an LLM to reconcile the two statements.

---

### Requirement

Initially:

```text
Requirement:
Users must be able to authenticate with email/password.
```

Later:

```text
Owner:
Actually, we're going to use Google OAuth instead.
```

You don't want:

```text
Requirement #1
Requirement #2
```

with an agent expected to figure out which one is current every time.

Instead:

```text
RequirementChanged
```

updates the current requirement projection.

The history still remembers:

```text
RequirementCreated
    ↓
RequirementChanged
```

So you have both:

**Current truth:**

```text
Authentication uses Google OAuth.
```

**Historical truth:**

```text
Originally email/password.
Later changed to Google OAuth.
```

---

# 4. This suggests a very important distinction

I'd explicitly distinguish:

### Facts/events

What happened.

```text
OwnerRequested
TaskPriorityChanged
TaskAssigned
RequirementChanged
DecisionApproved
```

### State

What is currently true.

```text
auth.priority = low
auth.assignee = sarah
authentication.method = oauth
```

### History

How we arrived there.

```text
PriorityChanged(high → low)
AssignmentChanged(marcus → sarah)
RequirementChanged(password → oauth)
```

### Context

What a particular agent needs to know.

```text
Current authentication requirement:
OAuth

Priority:
Low

Owner recently said:
Authentication is not important right now.

Relevant decision:
OAuth approved.

Current task:
Paused.
```

That's a much cleaner mental model.

---

# 5. Don't make agents merge the whole world

This is the part I'd be particularly strict about.

**Never do this:**

```text
Here are the last 10,000 project events.

Please figure out the current project state.
```

That's expensive, unreliable, and makes the model responsible for something your database should be doing.

Instead:

```text
Event
  ↓
Deterministic projection
  ↓
Current state
```

Then:

```text
Current state
+
relevant history
+
recent events
+
agent-specific context
```

goes to the LLM.

The LLM should reason about **what the state means**, not reconstruct the state.

---

# 6. But there's a harder case

There *are* things that cannot be cleanly reduced to:

```text
field = value
```

For example:

> "I don't think we should launch until we're confident users won't lose their data."

That's not necessarily a simple property.

Or:

> "We're probably targeting enterprise customers now, although we haven't completely decided."

Or:

> "Marcus thinks PostgreSQL is safer, while Maya thinks SQLite would make the initial experience simpler."

These are semantic/project-level things.

You don't want to build a giant deterministic state machine for every possible human statement.

So I think Casting needs a distinction between **structured state** and **semantic knowledge**.

---

# 7. Structured state should be deterministic

Things like:

```text
task.status
task.priority
task.assignee
task.parent
agent.status
agent.capabilities
decision.status
budget.remaining
requirement.status
branch.status
review.status
```

should all have deterministic reducers.

For example:

```text
TaskPriorityChanged
    ↓
TaskProjectionReducer
    ↓
task.priority = low
```

No LLM.

This should be extremely cheap.

---

# 8. Semantic state needs a different mechanism

For less structured information, I'd use something more like:

```text
Observation
Decision
Requirement
Plan
Risk
Assumption
Constraint
```

These should be **first-class objects**, rather than expecting the event stream to magically represent them.

For example:

```text
Risk #42

Subject:
Data loss during migration

Status:
Open

Severity:
High

Evidence:
...

Discovered by:
Marcus

Related tasks:
#381
#392
```

Then later:

```text
RiskResolved
```

changes:

```text
Status:
Resolved
```

Again, the current projection is deterministic.

But the **creation of the semantic object** may require intelligence.

That's where the PM comes in.

---

# 9. I think the PM should maintain a "Project Plan"

This may actually solve a lot of the concern you're raising.

The PM should have a first-class current plan.

Something like:

```text
Project Plan
────────────────────────

Objective
Build climbing gym SaaS MVP.

Current priorities
1. Customer onboarding
2. Gym management
3. Billing

Deprioritized
- Authentication enhancements

Active strategy
...

Known risks
...

Open decisions
...

Current milestones
...

Constraints
...
```

That plan is **current state**, not history.

The event history tells us how the plan evolved.

---

# 10. Then "deprioritise auth" is a mutation

The owner says:

> Deprioritise auth.

The PM doesn't need to regenerate the entire plan.

It produces something like:

```text
UpdateTask
{
    task_id: auth,
    priority: low
}
```

or perhaps:

```text
UpdatePlan
{
    operation:
        SetPriority(
            task = auth,
            priority = low
        )
}
```

The system applies that mutation.

Current plan becomes:

```text
1. Onboarding
2. Gym management
3. Billing

Low priority:
- Authentication enhancements
```

The event history records exactly what happened.

---

# 11. The interesting bit is when the owner says something ambiguous

Suppose they say:

> "Actually, authentication isn't that important anymore. Let's focus on the customer experience."

That's not necessarily a simple `priority = low`.

The PM should interpret it.

Perhaps it concludes:

```text
Plan mutation:

Deprioritize:
Authentication

Increase priority:
Onboarding

Create objective:
Improve customer onboarding experience

Potentially pause:
Auth enhancements
```

That **interpretation is where the expensive model belongs**.

But once the PM has decided what the mutation means, the resulting state should be deterministic.

So:

```text
Owner language
      ↓
       PM
      ↓
structured mutations
      ↓
projection
```

not:

```text
Owner language
      ↓
LLM-generated giant state
      ↓
hope it merged correctly
```

---

# 12. This also gives you a very useful architectural concept: Commands

I'd introduce a distinction between:

```text
Event
Command
State
```

For example:

### Command

```text
DeprioritizeTask
{
    task_id: auth
}
```

### Event

```text
TaskPriorityChanged
{
    task_id: auth
    old_priority: high
    new_priority: low
}
```

### State

```text
task.priority = low
```

The flow becomes:

```text
PM
 ↓
Command
 ↓
Validation / policy
 ↓
Domain logic
 ↓
Event
 ↓
Projection
 ↓
Current state
```

This is a **very healthy architecture for Casting**.

---

# 13. It also solves conflicting agents

Suppose:

```text
Marcus:
"Authentication should be high priority."

Maya:
"Onboarding is more important."

Owner:
"Focus on onboarding."
```

You don't want the PM to somehow continuously merge all three opinions into "the current priority."

Instead:

```text
Observations
    ↓
PM reasoning
    ↓
Decision
    ↓
TaskPriorityChanged
    ↓
Current state
```

The opinions remain historical evidence.

The decision becomes current organizational truth.

That's a really important distinction:

> **Opinions don't mutate project state. Decisions do.**

---

# 14. And this applies beautifully to architecture decisions

Imagine:

```text
Marcus:
Use PostgreSQL.

Maya:
SQLite would be simpler.

Security:
PostgreSQL gives us stronger controls.

PM:
Recommend PostgreSQL.

Owner:
Approved.
```

You don't want to repeatedly feed all four messages to every agent and ask:

> "What database are we using?"

Instead:

```text
Decision #184
────────────────────
Subject:
Database

Decision:
PostgreSQL

Status:
Approved

Rationale:
...

Alternatives considered:
SQLite

Approved by:
Owner
```

Current state:

```text
database = PostgreSQL
```

History:

```text
Marcus recommended PostgreSQL
Maya recommended SQLite
Security supported PostgreSQL
PM recommended PostgreSQL
Owner approved
```

That's enormously cleaner.

---

# 15. I would therefore avoid "merging" wherever possible

The word **merge** is slightly dangerous here because it implies the system has to reconcile arbitrary documents.

Instead, I'd think in terms of:

> **state transitions**

For structured concepts:

```text
State₀
  ↓
Event
  ↓
State₁
  ↓
Event
  ↓
State₂
```

For example:

```text
Priority:
High
  ↓
TaskPriorityChanged
  ↓
Low
  ↓
TaskPriorityChanged
  ↓
High
```

You don't need to merge the statements.

You just have a state transition history.

---

# 16. But what about multiple things changing concurrently?

This is where you'll eventually need concurrency semantics.

Imagine:

```text
11:02 Owner:
Deprioritize authentication.

11:03 PM:
Pause authentication work.

11:03 Marcus:
Authentication is actually blocking onboarding.
```

Now you have a legitimate conflict.

I would **not** immediately solve this with sophisticated distributed CRDT machinery.

Instead, give Casting commands/events a project sequence:

```text
1842 Owner request
1843 PM pause
1844 Marcus observation
```

Then the PM wakes because of 1844 and decides:

> The new evidence materially changes the situation.

It may then produce:

```text
1845 DecisionProposed
```

and potentially ask the owner.

The event stream gives you ordering.

The PM handles semantic conflict.

---

# 17. This gives the PM a very clean job

The PM's job isn't:

> "Maintain a giant mental model of everything."

It's:

> **Maintain the project's current plan and respond when new information might invalidate it.**

That is much more tractable.

The PM essentially does:

```text
Current state
+
new evidence
        ↓
Does this invalidate the plan?
        │
   ┌────┴────┐
   │         │
  No        Yes
   │         │
 continue   re-evaluate
             │
             ▼
          mutate plan
```

That is the control loop.

---

# 18. I'd go one step further: snapshots

There is another optimization that I think will become important.

You don't want to reconstruct even the projections from 50 million events every time.

So periodically:

```text
Events 1 ─────────── 10,000
                     ↓
                Snapshot
                     ↓
Events 10,001 ────── 11,000
```

The projection can start from the snapshot and apply only subsequent events.

This is a standard optimization and doesn't change the semantics.

More importantly, **the PM itself can have a durable state snapshot**.

For example:

```text
PM State Snapshot #73

Cursor:
1842

Current plan:
...

Open questions:
...

Active work:
...

Known risks:
...

Last reasoning:
...

Next wake conditions:
...
```

Then it doesn't have to rebuild its entire mental operating context after restart.

---

# 19. But don't let the snapshot become another source of truth

This is important.

The hierarchy should remain:

```text
Events
  ↓
authoritative history

Projections
  ↓
current structured state

Snapshots
  ↓
optimization

LLM summaries
  ↓
context optimization
```

If a snapshot becomes corrupted, you should be able to throw it away and reconstruct it.

Likewise, an LLM-generated summary should never be treated as authoritative project state.

---

# 20. I think this is also the answer to your cost concern

You were worried:

> "If every little change requires a merge step, won't this become expensive?"

Yes — **if the merge is an LLM operation.**

It shouldn't be.

For 95% of state:

```text
Event
 ↓
deterministic reducer
 ↓
current state
```

is essentially free.

The expensive model is only invoked when something requires **interpretation**.

For example:

```text
"Deprioritise auth."
```

might require the PM once.

It turns that into:

```text
SetPriority(auth, low)
```

After that, every downstream agent gets:

```text
Current priority:
auth = low
```

No model required.

---

# 21. The context assembler then becomes extremely important

Suppose Marcus is working on billing.

He shouldn't receive:

```text
All 18,492 events
```

He gets:

```text
Project:
Climbing gym SaaS

Current objectives:
...

Current priorities:
1. Onboarding
2. Gym management
3. Billing

Your task:
Implement billing API

Relevant decisions:
...

Relevant requirements:
...

Relevant recent events:
...

Known risks:
...

Repository state:
...
```

The context assembler queries **current state** and selectively retrieves history.

That's where Casting becomes efficient.

---

# 22. I'd add one more concept: supersession

For some objects, explicit supersession is valuable.

For example:

```text
Decision #184
Use PostgreSQL
```

later:

```text
Decision #217
Use SQLite for local deployment
```

Don't delete #184.

Instead:

```text
Decision #184
Status:
Superseded by #217
```

Now you have:

```text
Current:
SQLite

History:
PostgreSQL was previously chosen.
```

This is better than pretending the first decision never happened.

---

# 23. So I'd modify the original architecture slightly

Your original statement:

> The event history is the historical source of truth.

is correct.

But I'd make it more precise:

> **The event history is the authoritative record of what happened. Current project state is maintained through deterministic projections of that history. Semantic interpretation is performed by agents only where the state transition cannot be expressed deterministically.**

That's a much stronger architectural principle.

And then:

```text
                 EVENTS
                    │
                    ▼
          deterministic reducers
                    │
                    ▼
             CURRENT STATE
                    │
                    ├──────────────┐
                    │              │
                    ▼              ▼
              context views    UI projections
                    │
                    ▼
                  AGENTS
                    │
                    ▼
            commands / decisions
                    │
                    ▼
                  EVENTS
```

That's the loop I'd build around.

---

# 24. The really important philosophical distinction

I think this leads to a nice rule for Casting:

> **Agents interpret. The system records.**

More specifically:

**The database should know:**

```text
Authentication priority = low.
```

**The PM should know/reason:**

```text
Authentication was deprioritized because the owner wants
to focus on customer onboarding.
```

**The event history should know:**

```text
At 11:04, the owner requested authentication be deprioritized.
```

And an agent may be shown:

```text
Authentication
Priority: Low

Reason:
Owner wants to focus on onboarding.

Previous priority:
High

Changed:
Today, 11:04
```

That gives you the best of all three worlds:

* cheap deterministic state
* rich historical memory
* intelligent interpretation

And I think this is **exactly the architecture you want for Casting**, because it prevents the PM from becoming an expensive "summarize the entire company every time something happens" machine.
