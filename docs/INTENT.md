In response to the question:

"where do you think priorities lives not in terms of tasks (task A is priority 1, after that we do B..) but in terms of:

Backwards compatibility should NOT be taken into consideration, this is a new project...
We need to be using a test driven development model....
lets frequently have a consultant look at the codebase and refactor as we go to ensure the project remains coherent

etc? would this be outside the event model and something just fed to each agent each time? or is there a cleaner way?"

This is the answer:




Yes. I think those are **not really priorities at all**, despite the fact that they influence priorities.

They are closer to **project policies, principles, constraints, and operating rules**.

And I would absolutely make them first-class, persistent project state rather than stuffing them into every agent prompt.

The cleanest model I see is:

```text
                    PROJECT
                       │
        ┌──────────────┼──────────────┐
        │              │              │
        ▼              ▼              ▼
   OBJECTIVES      POLICIES       CONSTRAINTS
   "What we want"  "How we work"  "What must be true"
        │              │              │
        │              │              │
        ▼              ▼              ▼
      TASKS        DECISIONS       VALIDATION
```

For your examples:

| Statement                                    | I would model it as                  |
| -------------------------------------------- | ------------------------------------ |
| "Build the MVP in 3 days"                    | Objective / target                   |
| "Authentication is not important right now"  | Priority / planning state            |
| "No backwards compatibility"                 | **Project policy / constraint**      |
| "Use TDD"                                    | **Engineering practice / policy**    |
| "Frequently refactor with a consultant"      | **Operating policy / practice**      |
| "Never deploy without owner approval"        | **Decision policy / authority rule** |
| "Keep monthly spend below $250"              | **Budget constraint**                |
| "We are targeting climbing gyms"             | Product requirement / objective      |
| "Use Rust"                                   | Architecture decision                |
| "Prefer simplicity over performance for MVP" | **Engineering principle / policy**   |

I think this distinction could become quite important to Casting.

## I'd call the general concept "Project Directives"

You could have a durable object along these lines:

```text
Project Directive
────────────────────────────

id
project_id

type
status

statement
scope

priority
strength

created_by
created_at

effective_from
effective_until

supersedes

rationale
```

And then types such as:

```text
Policy
Constraint
Principle
Practice
Preference
Objective
```

I'm not necessarily saying these should all be separate database entities. They may initially be one `directives` table with a `kind`.

For example:

```yaml
kind: policy

statement: >
  This project uses test-driven development.

scope:
  areas:
    - engineering

strength: required

status: active
```

Another:

```yaml
kind: principle

statement: >
  Backwards compatibility is not a concern for this project.

scope:
  areas:
    - architecture
    - api
    - database

strength: strong
```

And:

```yaml
kind: practice

statement: >
  A consultant should periodically review the codebase
  and identify opportunities for refactoring.

scope:
  areas:
    - engineering

cadence:
  every: 20_tasks
```

That last one is particularly interesting because it isn't merely information.

**It's an instruction to the organization.**

---

# The key distinction: "context" vs "governance"

I would resist the idea of simply injecting these into every prompt:

```text
Project principles:

- Use TDD
- No backwards compatibility
- Refactor frequently
- Keep things simple
...
```

That works initially, but it makes them essentially **prompt configuration**.

That's too weak for what Casting is trying to become.

Instead, they should be **authoritative project state**.

Then the context assembler decides which directives are relevant to a particular agent.

For example, Marcus receives:

```text
PROJECT DIRECTIVES

Engineering practices:
✓ TDD is required
✓ Refactor periodically
✓ No backwards compatibility requirement

Architecture:
✓ Prefer simplicity over premature optimisation

Authority:
✓ You may modify source
✗ You may not change product requirements
```

While Maya might receive:

```text
PROJECT DIRECTIVES

Product:
✓ Prioritise accessibility

Engineering:
✓ No backwards compatibility requirement

Authority:
✓ You may modify UX proposals
✗ You may not alter architecture
```

So the directive exists **once**, but gets selectively surfaced.

---

# This also fits beautifully with your event architecture

I wouldn't make directives "outside the event model."

I'd make them **domain objects whose lifecycle is recorded by events**.

For example:

```text
ProjectDirectiveCreated
```

Then:

```text
ProjectDirectiveActivated
ProjectDirectiveModified
ProjectDirectiveSuspended
ProjectDirectiveSuperseded
ProjectDirectiveExpired
```

The projection might simply contain:

```text
project_directives

id
kind
statement
scope
strength
status
created_at
updated_at
supersedes
```

The history tells you:

```text
10:31
Owner created:
"TDD is required."

12:42
Owner modified:
"TDD applies to production code but not exploratory prototypes."

16:21
PM proposed:
"Refactoring review every 20 tasks."

16:24
Owner approved.
```

Current state tells you:

```text
TDD:
Required

Refactoring review:
Every 20 tasks
```

Again, **history + current projection**.

---

# And I think "directive" has another huge advantage

It gives the PM something concrete to reason about.

Imagine the PM receives:

> "Let's move faster. Don't worry too much about backwards compatibility."

The PM can interpret that into a structured change:

```text
DirectiveModified

directive:
    backwards_compatibility

new_policy:
    not_required

scope:
    architecture
    API
```

Now every downstream decision can take that into account.

The PM doesn't need to remember the owner's sentence.

The **organization has a policy**.

---

# There should probably be different strengths

This is where I think it gets particularly powerful.

Not everything the owner says has equal authority.

For example:

### Principle

> "Prefer simple solutions."

Useful guidance.

### Preference

> "I'd rather use Postgres."

A preference.

### Policy

> "All production code must have tests."

A rule.

### Constraint

> "Budget cannot exceed $250."

A hard constraint.

### Requirement

> "Users must be able to export their data."

A product obligation.

That gives the PM a hierarchy for reasoning.

Something roughly like:

```text
Hard constraints
       ↓
Owner decisions / requirements
       ↓
Policies
       ↓
Project principles
       ↓
Preferences
       ↓
Agent recommendations
```

Not necessarily a rigid universal ordering, but enough to resolve conflicts.

---

# Your TDD example is especially revealing

Suppose Marcus says:

> "Writing tests first is slowing me down. I'm going to implement this and add tests afterwards."

The PM should not need to rediscover from historical conversations that the owner wanted TDD.

It can simply know:

```text
Active project directive:

TDD
Strength: REQUIRED
Scope: engineering
```

Marcus's behaviour conflicts with an active directive.

That can become:

```text
PolicyViolationDetected
```

or perhaps simply an observation:

```text
AgentObservationCreated

severity: important

subject:
TDD policy not being followed

recommended_action:
Ask Marcus to revise implementation workflow
```

That is a **much more robust system** than prompt engineering.

---

# The refactoring example is even more interesting

You said:

> "Let's frequently have a consultant look at the codebase and refactor as we go to ensure the project remains coherent."

That's not merely a policy.

It's an **organizational rule**.

I'd potentially represent it as something like:

```yaml
kind: practice

name: periodic_architecture_review

statement: >
  A senior engineering consultant should periodically review
  the codebase for coherence and refactoring opportunities.

trigger:
  type: task_count
  every: 15

action:
  create_task:
    kind: review
    role: principal-engineer
```

Now you've crossed into something really interesting:

**Project directives can generate organizational behaviour.**

The PM doesn't have to remember:

> "Oh yeah, Ben wanted us to do a refactor every so often."

The system can notice:

```text
15 tasks completed
        ↓
Directive triggered
        ↓
Create review task
        ↓
Assign consultant
```

That's beginning to feel much more like a company operating system.

---

# I would therefore distinguish two kinds of directives

This is probably worth putting into the architecture document.

### Declarative directives

They describe how the organization should behave.

```text
Use TDD.
Don't care about backwards compatibility.
Prefer simple architecture.
Security issues require owner notification.
```

### Procedural directives

They cause something to happen.

```text
Review architecture every 15 tasks.
Run security review before production deployment.
Ask owner before spending > $25.
Have QA review every authentication change.
```

The second category is effectively **organizational automation**.

And I think that could become one of Casting's strongest concepts.

---

# There's also a relationship to the PM's memory

I would give the PM a special "organizational context" assembled from these things:

```text
PROJECT INTENT

OBJECTIVES

ACTIVE DIRECTIVES
    Policies
    Constraints
    Principles
    Practices

CURRENT PLAN

CURRENT STATE

OPEN DECISIONS

RISKS

RECENT RELEVANT EVENTS
```

So the PM's context might begin:

```text
You are the Project Manager for Casting Project X.

PROJECT OBJECTIVE
Build a climbing-gym SaaS MVP.

ACTIVE POLICIES
- TDD is required.
- Backwards compatibility is not required.
- Production deployments require owner approval.

ENGINEERING PRACTICES
- Architecture review every 15 completed tasks.
- Prefer simple solutions during MVP.

CURRENT PLAN
...

CURRENT RISKS
...

OPEN DECISIONS
...
```

That's far better than throwing 500 events into the model.

---

# And agents should not necessarily see all directives

This is another reason to make them first-class.

The context assembler can answer:

> Which directives are relevant to this agent, task, and decision?

For a database migration task:

```text
Relevant:
- No backwards compatibility
- TDD
- Simplicity principle
- Security policy
```

For a UX task:

```text
Relevant:
- Accessibility principle
- Target customer objective
```

For the PM:

```text
Everything relevant to organizational governance.
```

So you get:

```text
             PROJECT DIRECTIVES
                     │
             Context Resolver
             /       |       \
            /        |        \
          PM       Marcus     Maya
```

One authoritative source, many tailored contexts.

---

# One subtle thing I'd be careful about

I wouldn't let agents casually edit these.

That's where **authority** comes back into play.

For example:

```text
Marcus:
"Let's turn off TDD because it's slowing us down."
```

He shouldn't be able to emit:

```text
ProjectDirectiveModified
```

directly.

He can emit:

```text
AgentObservationCreated
```

or:

```text
DirectiveChangeRequested
```

Then the PM evaluates it.

And if the directive requires owner authority:

```text
OwnerDecisionRequested
```

The owner decides.

So:

```text
Agent
  ↓
Observation / proposal
  ↓
PM
  ↓
Policy check
  ↓
Owner if necessary
  ↓
Directive changed
```

Again, delegated authority.

---

# I'd also avoid putting too much prose into directives

There's a temptation to turn this into:

```text
The owner has told us that generally we should...
```

and then build a giant "project constitution" document.

I wouldn't.

Keep directives **small, explicit and structured**.

Instead of:

> "When working on the project, engineers should generally try to write tests first where practical, but exploratory work can be done differently..."

prefer:

```text
kind: policy
name: test_driven_development
strength: required
scope: production_engineering
```

If there's nuance, attach it:

```text
exceptions:
  - exploratory_prototype
```

The LLM can turn human language into these structures when necessary, but the system stores the resulting semantic representation.

---

# So I think the overall model becomes quite elegant

You effectively have:

```text
                         PROJECT
                            │
        ┌───────────────────┼───────────────────┐
        │                   │                   │
        ▼                   ▼                   ▼
     INTENT             DIRECTIVES            STATE
        │                   │                   │
   What we want       How we operate       What is true
        │                   │                   │
        └───────────────────┼───────────────────┘
                            │
                            ▼
                           PM
                            │
                 interprets new information
                            │
                            ▼
                         COMMANDS
                            │
                            ▼
                          EVENTS
                            │
                            ▼
                       PROJECTIONS
                            │
                            └──────────→ STATE
```

And then the **context assembler** sits between all of that and the agents:

```text
Project intent
Current state
Active directives
Relevant decisions
Relevant history
Current task
Owner policies
        │
        ▼
 Context Assembly
        │
        ▼
      Agent
```

---

## I think this is actually a significant architectural insight for Casting

The project doesn't just have **state**.

It has three fundamentally different kinds of knowledge:

**Intent** — *What are we trying to accomplish?*

**Governance** — *How have we agreed to operate?*

**State** — *What is currently true?*

And then:

**History** — *How did we get here?*

That gives you a very strong conceptual foundation:

```text
                ┌──────────────┐
                │    INTENT    │
                │  What / Why  │
                └──────┬───────┘
                       │
                ┌──────▼───────┐
                │  GOVERNANCE  │
                │ How / Rules  │
                └──────┬───────┘
                       │
                ┌──────▼───────┐
                │    STATE     │
                │   Current    │
                └──────┬───────┘
                       │
                ┌──────▼───────┐
                │   HISTORY    │
                │   Why/When   │
                └──────────────┘
```

I would **absolutely put this into the architecture**, rather than treating these principles as prompt text. It makes the PM substantially more powerful because it can reason against a persistent organizational constitution instead of having to "remember" what the owner wanted.
