# Casting

## Project Brief, Product Vision & Initial Architecture

**Codename:** Casting\
**Status:** Initial project definition\
**Primary goal:** Build an autonomous software company in a box.

------------------------------------------------------------------------

# 1. Vision

Casting is an agent orchestration platform for building software.

The core idea is simple:

> A human owner describes what they want. A Project Manager agent
> organizes a team of specialist agents, coordinates their work, manages
> priorities and cost, records decisions, and asks the owner for input
> when necessary.

The product should feel less like operating a collection of AI chatbots
and more like **running a small software company**.

The human owner should not need to understand multi-agent protocols,
task queues, context windows, LLM providers, event buses, or agent
runtimes.

They should experience something like:

``` text
I have a project.

I have a team.

I tell my PM what I want.

The team figures out how to build it.

They work on it.

They ask me when they genuinely need me.

I can see what is happening.

I can understand why decisions were made.
```

The agents are the workforce.

The Project Manager is the management layer.

The human is the owner/director/CEO.

Casting is the operating system that keeps the whole thing coherent.

------------------------------------------------------------------------

# 2. Product Tenets

These principles should guide architectural and product decisions.

## 2.1 User experience is more important than implementation convenience

The user should not need to understand or manually manage Casting's
infrastructure.

The ideal first-run experience is:

``` text
🎬 Casting

Starting your project...

✓ Database ready
✓ Agent runtime ready
✓ Project Manager ready
✓ Web server ready

Your Casting workspace:

https://abc123.cast.dev

Owner login:
ben
Password: ********
```

The goal is for `cast run` to feel magical.

A user should not have to install a web framework, database, message
broker, runtime, or collection of supporting services merely to get
started.

If implementation is harder because we are making the user experience
dramatically better, that is generally a good trade.

------------------------------------------------------------------------

## 2.2 Agents should feel like people, but this is a product layer

Agents should have:

-   a name
-   a role
-   a profile picture/avatar
-   expertise
-   a CV/background
-   a working style
-   capabilities
-   permissions
-   a current assignment

For example:

``` text
Maya Patel
UX Consultant

Specialties:
- UX research
- Accessibility
- Design systems

Experience:
- SaaS
- Fintech
- Mobile applications

Working style:
- Pragmatic
- User-focused
```

This makes the system relatable and fun.

However, the fictional identity must never obscure the real technical
model underneath it.

An agent is ultimately a configured autonomous worker with:

-   a model
-   context
-   tools
-   capabilities
-   permissions
-   budget constraints
-   persistent identity
-   project history

The "CV" is the human-friendly representation of that configuration.

------------------------------------------------------------------------

## 2.3 Casting should feel like an autonomous software company

Prefer language such as:

-   hire a consultant
-   assign work
-   ask the PM
-   request an opinion
-   approve a decision
-   review a proposal
-   manage the team
-   project budget
-   company history

rather than exposing internal terminology such as:

-   spawn an LLM
-   send a prompt
-   execute a chain
-   create an agent node

The metaphor should be useful, not gimmicky.

------------------------------------------------------------------------

## 2.4 Shared project state is more important than individual agent context

Every agent must understand:

1.  What are we trying to accomplish?
2.  What has already been decided?
3.  What is currently true?
4.  What am I responsible for?
5.  What am I allowed to change?
6.  What should I do if I discover something unexpected?

Agents should not each maintain their own isolated interpretation of the
project.

The project itself owns the authoritative history and state.

------------------------------------------------------------------------

# 3. The Organizational Model

The initial hierarchy should be:

``` text
                         OWNER
                           │
                           │ goals / decisions / feedback
                           ▼
                    PROJECT MANAGER
                       /    |    \
                      /     |     \
                     ▼      ▼      ▼
                   UX      ENG     QA
                   │        │       │
                  ...      ...     ...
                            │
                       sub-agents
```

The PM is the primary human interface.

The owner generally communicates with the PM rather than directly
managing individual agents.

Consultants can have their own sub-agents.

However, the underlying communication architecture should not be rigidly
tree-shaped.

The real system is closer to a directed graph:

``` text
                    PM
                  / | \
                 /  |  \
              UX   ENG  QA
               \    |   /
                \   |  /
                 ARCH
```

Agents may need to communicate directly when appropriate, while the PM
remains the authority responsible for prioritization, planning, and
project coherence.

------------------------------------------------------------------------

# 4. The Project Manager

The PM is the most important agent in the initial system.

The PM's primary responsibility is not writing code.

The PM answers:

> What should happen next?

The PM should:

-   understand owner intent
-   turn intent into requirements
-   identify unknowns
-   create and prioritize work
-   hire/activate consultants
-   assign tasks
-   manage dependencies
-   reconcile conflicting advice
-   monitor progress
-   manage budget
-   determine when owner input is required
-   maintain project coherence
-   record and explain important decisions
-   respond to observations from consultants
-   re-plan when circumstances change

Example flow:

``` text
Owner:
"Let's build a SaaS app for managing climbing gyms."

PM:
→ Understand requirements
→ Identify unknowns
→ Ask important questions
→ Hire UX consultant
→ Hire architecture consultant
→ Create initial backlog
→ Investigate technology choices
→ Produce initial UX flows
→ Reconcile findings
→ Present recommendation to owner
→ Begin implementation
```

Later:

``` text
Engineer:
"I've discovered that the authentication architecture
we chose won't work cleanly with offline mode."

PM:
→ assess impact
→ request options
→ consult architect if necessary
→ determine whether owner approval is required
→ present recommendation
→ record decision
→ re-plan affected tasks
```

------------------------------------------------------------------------

# 5. Owner Autonomy

The owner should explicitly control how much autonomy the PM has.

The system should support an autonomy spectrum:

``` text
Ask me about everything  <----------------->  Just build it
```

But internally this should be more precise than a single slider.

Different decisions can have different policies.

Example:

  Decision                               Default owner involvement
  -------------------------------------- ---------------------------
  Rename internal variable               Never
  Internal refactor                      Never
  Choose testing library                 PM
  Add a consultant                       PM
  Change internal implementation         PM
  Change database                        Ask
  Change architecture                    Ask
  Change product requirements            Ask
  Spend more than configured threshold   Ask
  Production deployment                  Ask
  Security-critical issue                Notify / Ask
  Irreversible action                    Ask

The system should eventually have a **decision policy engine**.

The important concept is delegated authority.

The owner is not simply chatting with an AI; they are deciding what
decisions they trust the AI organization to make.

------------------------------------------------------------------------

# 6. Budget and Cost

LLM usage represents real money and should be treated as part of project
management.

The PM should understand:

-   current spend
-   budget
-   projected spend
-   cost by agent
-   cost by task
-   cost by activity
-   cost of alternative plans
-   urgency versus cost tradeoffs

Example:

``` text
CASTING INDUSTRIES

Budget this month       $250
Spent                    $73.42
Remaining               $176.58

Engineering              $41.20
UX                       $12.80
QA                        $9.42
PM                       $10.00

Forecast                 $184
```

Eventually the PM might reason:

``` text
Option A — Fast
6 agents concurrently
~$42
~2 hours

Option B — Balanced
3 agents
~$24
~4 hours

Option C — Cheap
1 engineer
~$9
~9 hours
```

The PM should optimize according to owner preferences around:

-   speed
-   quality
-   confidence
-   cost
-   risk

------------------------------------------------------------------------

# 7. Consultants

Consultants are specialist agents that can be added to a project.

Examples:

``` text
Marcus Reed
Principal Engineer
Architecture · Backend · Performance

Maya Patel
UX Consultant
UX · Accessibility · Product Design

James Wilson
Security Consultant
Security · Threat Modelling · Infrastructure
```

Consultants may have their own sub-agents.

A consultant should have explicit capabilities and authority.

Example:

``` yaml
name: Maya Patel
role: UX Consultant

expertise:
  - UX research
  - accessibility
  - design systems

experience:
  - fintech
  - SaaS
  - mobile applications

authority:
  can:
    - modify UX tasks
    - create design proposals
    - create observations

  cannot:
    - change architecture
    - spend budget
    - change product requirements
```

The fictional profile is simply a friendly UI representation of these
underlying properties.

------------------------------------------------------------------------

# 8. Agent Capabilities and Permissions

Do not give every agent unrestricted access to everything.

Agents should have explicit capabilities.

Example:

``` text
Marcus Reed

CAN:
✓ read repository
✓ modify source
✓ create branches
✓ run tests
✓ create tasks
✓ comment on architecture decisions

CANNOT:
✗ deploy production
✗ spend > $10 without approval
✗ change requirements
✗ hire consultants
```

Capabilities should become a first-class security and autonomy model.

Potential capabilities include:

-   read repository
-   write repository
-   create git branches
-   commit
-   run tests
-   execute shell commands
-   access internet
-   access selected services
-   create tasks
-   modify tasks
-   create decisions
-   request owner input
-   hire consultants
-   deploy
-   access production
-   spend budget

------------------------------------------------------------------------

# 9. Core Architectural Principle: Project History

Casting should maintain an append-only project history.

The fundamental model is:

``` text
                EVENTS
                  │
                  │
          ┌───────┴────────┐
          │                │
          ▼                ▼
    Task projection   Agent projection
          │                │
          ▼                ▼
       Kanban UI        Team UI

          │
          ▼
   Project projection
          │
          ▼
      Dashboard UI
```

The event history is the historical source of truth.

The projections are the fast queryable current state.

Do not make the UI reconstruct the project from the entire event history
on every request.

------------------------------------------------------------------------

# 10. Event Store

Initial recommendation:

**PostgreSQL as the long-term/server database, with SQLite as the
default zero-dependency deployment option.**

The logical event model should be database-independent.

Initial implementation can use SQLite.

Later, PostgreSQL can be added without changing domain semantics.

Conceptually:

``` text
EventStore
    │
    ├── SQLiteEventStore
    │
    └── PostgresEventStore
```

Only implement SQLite initially if that reduces complexity.

Do not build both implementations prematurely.

------------------------------------------------------------------------

# 11. Event Structure

A Casting event should contain enough information to answer:

-   what happened?
-   when?
-   where?
-   who caused it?
-   what did it affect?
-   what larger operation did it belong to?
-   what caused it?
-   what was the agent execution associated with it?

Illustrative event:

``` json
{
  "event_id": "01K...",
  "project_id": "...",
  "sequence": 1842,

  "timestamp": "2026-08-08T21:42:17Z",

  "actor": {
    "type": "agent",
    "id": "marcus-reed"
  },

  "type": "TaskCompleted",

  "aggregate": {
    "type": "task",
    "id": "task-184"
  },

  "data": {
    "result": "implemented authentication middleware",
    "commit": "a83f91c",
    "tests_passed": true
  },

  "metadata": {
    "correlation_id": "...",
    "causation_id": "...",
    "agent_run_id": "...",
    "model": "..."
  }
}
```

Important fields:

### `event_id`

Globally unique event identifier.

### `project_id`

The project to which the event belongs.

### `sequence`

A monotonically increasing sequence number within the project.

This gives an authoritative ordering:

``` text
1837
1838
1839
1840
1841
1842
```

Do not rely solely on timestamps for causal ordering.

### `timestamp`

When the event was created.

### `actor`

The human or agent responsible for the event.

### `event_type`

The semantic event type.

Examples:

``` text
ProjectCreated
AgentHired
TaskCreated
TaskAssigned
TaskStarted
TaskCompleted
DecisionRequested
DecisionMade
MessageSent
BudgetUpdated
RequirementChanged
ConsultantRequested
ConsultantReportSubmitted
CodeChangeProduced
ReviewRequested
ReviewCompleted
IncidentDetected
```

### `aggregate`

The entity primarily affected.

### `data`

Event-specific structured data.

JSON/JSONB is appropriate.

### `causation_id`

The event that directly caused this event.

This allows chains such as:

``` text
OwnerMessage
    ↓
PMDecision
    ↓
TaskCreated
    ↓
TaskAssigned
    ↓
AgentStarted
    ↓
TaskCompleted
```

### `correlation_id`

The larger operation to which the event belongs.

For example, an authentication feature might generate 37 events with one
correlation ID.

### `agent_run_id`

The underlying LLM/agent execution associated with the event.

This allows drill-down into model execution, prompts, tool calls,
tokens, etc.

------------------------------------------------------------------------

# 12. Domain Events vs Runtime Events

Do not mix meaningful project history with low-level execution
telemetry.

## Domain events

Things that matter to the project:

``` text
RequirementCreated
RequirementChanged
TaskCreated
TaskAssigned
TaskCompleted
DecisionProposed
DecisionApproved
DecisionRejected
AgentHired
AgentFired
ReviewRequested
ReviewCompleted
```

## Runtime events / telemetry

Things that happen inside the machinery:

``` text
AgentRunStarted
LLMRequestStarted
LLMRequestCompleted
ToolCalled
ToolFailed
ShellCommandExecuted
GitCommitCreated
ContainerStarted
ContainerStopped
```

Runtime telemetry can be stored separately while still referencing the
project/task/agent/event context.

Avoid polluting the semantic project history with token-stream chunks
and other low-level events.

------------------------------------------------------------------------

# 13. Time Travel and Historical State

The event model should make it possible to answer:

> What did the project believe was true at a particular point in time?

For example:

``` text
At 14:00 yesterday:

✓ PostgreSQL
✓ Authentication pending
✓ Marcus assigned to backend
✓ Maya working on onboarding
✓ Budget: $31.42
```

This will be valuable for:

-   debugging
-   auditing
-   explaining decisions
-   investigating agent failures
-   reproducing bugs
-   understanding why current state exists

------------------------------------------------------------------------

# 14. Don't Over-Apply Event Sourcing

Casting should use event sourcing where it provides real value, not as
dogma.

It is reasonable for tables such as:

``` text
tasks
agents
projects
decisions
requirements
messages
```

to represent current state/projections.

The event history tells us how that state was reached.

This is essentially a pragmatic CQRS/event-history architecture without
requiring the entire system to become a complicated event-sourcing
framework.

------------------------------------------------------------------------

# 15. Project State and Projections

At minimum, there should be logical stores for:

### Event history

Immutable historical record.

### Project state

Fast current-state queries.

Examples:

``` text
projects
agents
tasks
decisions
requirements
messages
observations
```

### Agent execution telemetry

Detailed operational data:

``` text
agent_runs
llm_calls
tool_calls
token_usage
```

Initially these can all live in the same database.

There is no need for multiple database technologies.

------------------------------------------------------------------------

# 16. Agent Cursors

Every consumer should have a position in the project history.

For example:

``` text
agent: PM
last_seen: 1842

agent: Marcus
last_seen: 1839

agent: Maya
last_seen: 1841

projection: task-board
last_processed: 1842
```

An agent should be able to resume from its last known position.

If a worker disappears, it should be able to continue from its cursor.

This is preferable to relying on transient messages as the authoritative
communication mechanism.

------------------------------------------------------------------------

# 17. Event Delivery

Separate:

1.  event persistence
2.  event notification

An event is safely written to the persistent store first.

Then consumers can be notified that new events are available.

Initially, a database-native notification mechanism can be sufficient.

The important property is:

> A notification is a hint to consume persisted events, not the event
> itself.

If a worker disappears, it can resume from its cursor.

Do not make transient messaging the source of truth.

------------------------------------------------------------------------

# 18. Event Relevance

Not every event should be presented to or injected into every agent's
context.

For example:

``` text
Maya changes button padding
```

does not need to wake the security consultant.

But:

``` text
Authentication architecture changed
```

probably should reach engineering, architecture, security, and the PM.

Events may eventually have:

``` text
scope:
    project
    team
    task
    agent

topics:
    authentication
    frontend
    security

importance:
    info
    normal
    important
    critical
```

The system should eventually have a relevance/context-assembly layer
that determines what each agent needs to know.

------------------------------------------------------------------------

# 19. Context Assembly

This is likely to become one of Casting's most important pieces of
intellectual property.

The PM should not receive a giant context containing the entire history.

Instead, an agent's context should be assembled from:

``` text
Project intent
+
Current project state
+
Recent relevant events
+
Decisions affecting the current task
+
Unresolved observations
+
Current assignments
+
Owner preferences/policies
+
Relevant task history
```

The goal is:

> Give the agent the information it needs, not everything that has ever
> happened.

------------------------------------------------------------------------

# 20. Communication Model

Agent communication should be structured project activity rather than
arbitrary chat whenever possible.

An agent may create an observation:

``` text
AgentObservationCreated
```

Example:

``` json
{
  "severity": "low",
  "subject": "HTTPS is not enabled",
  "body": "I noticed...",
  "recommended_action": "Create security task",
  "requires_owner": false
}
```

The PM can respond by:

``` text
CreateTask
DismissObservation
RequestMoreInformation
EscalateToOwner
```

This provides a traceable communication chain.

------------------------------------------------------------------------

# 21. Owner Inbox

The owner should not have to watch the event stream.

The PM should turn important situations into requests for attention.

Example:

``` text
Sarah Chen — Project Manager

We have two viable approaches to authentication.

A: OAuth
- Faster
- Estimated cost: $18

B: Custom authentication
- More control
- Estimated cost: $31

I recommend A.

Do you approve?
```

The owner replies:

``` text
Go with A.
```

This becomes a durable project event such as:

``` text
OwnerDecisionRecorded
```

The decision becomes part of the permanent project history.

The owner interaction can initially happen in the web UI.

Telegram/WhatsApp/etc. can be added later.

------------------------------------------------------------------------

# 22. "Email" Metaphor

Agent communication should be visually understandable.

For example:

``` text
Inbox

🔴 Maya Patel
UX Consultant

"I noticed that the onboarding flow doesn't account
for returning users. I haven't changed anything because
this may affect the product requirements. Should I
investigate?"

2 minutes ago


🟡 Marcus Reed
Principal Engineer

"I've identified a potential issue with the current
authentication approach..."
```

Agents can have project-specific addresses for fun:

``` text
maya@projectidea.com
marcus@projectidea.com
sarah@projectidea.com
```

The underlying system should remain structured messages/events rather
than an actual email infrastructure unless there is a later product
reason to implement real email.

------------------------------------------------------------------------

# 23. Task System / Kanban

Do not use Jira initially.

Casting does not need Jira's complexity.

The task board should be purpose-built for autonomous software
development.

Initial task model:

``` text
Task
────
id
title
description

status
priority

created_by
assigned_to

parent_task
blocked_by

created_at
updated_at

estimate
actual_cost

kind
```

Possible task kinds:

``` text
feature
bug
investigation
decision
refactor
```

The initial Kanban can be as simple as:

``` text
┌─────────────┬──────────────┬─────────────┬──────────┐
│ BACKLOG     │ WORKING      │ REVIEW      │ DONE     │
├─────────────┼──────────────┼─────────────┼──────────┤
│ Login       │ API          │ Landing     │ Database │
│ Billing     │ Auth         │ page        │ setup    │
│ Analytics   │              │             │          │
└─────────────┴──────────────┴─────────────┴──────────┘
```

The board is a projection of project activity.

It is not the source of truth.

The agents' work and project events are the source of truth.

------------------------------------------------------------------------

# 24. Task Hierarchy

Tasks should support meaningful hierarchy.

Example:

``` text
FEATURE: User authentication
│
├── Investigate authentication options
│
├── Design authentication architecture
│
├── Implement authentication
│   ├── API
│   ├── database
│   ├── sessions
│   └── frontend
│
├── Security review
│
└── End-to-end testing
```

The PM can dynamically create, restructure, split, or close tasks.

Agents should also be able to create tasks.

For example:

``` text
Task #721

Investigate database migration strategy

Parent:
Authentication implementation

Priority:
PM review
```

The PM decides whether to schedule it.

------------------------------------------------------------------------

# 25. Why the Task Board Is Different From Jira

Jira represents human project-management workflows.

Casting represents autonomous work.

A developer discovering a problem can create a task immediately.

The PM can prioritize it.

A task can be generated because an agent noticed something rather than
because a human created a ticket.

The system should optimize for:

> What work currently exists, why does it exist, who owns it, what is
> blocking it, and what should happen next?

not:

> Which box did someone move in a Scrum workflow?

------------------------------------------------------------------------

# 26. Technology Direction

## Initial language: Rust

Rust is attractive because Casting should be extremely easy to deploy.

The preferred user experience is:

``` bash
cast run
```

with minimal or no external prerequisites.

A native executable gives us:

-   simple distribution
-   low runtime overhead
-   predictable deployment
-   easy cross-platform releases
-   good fit for process execution and sandboxing
-   strong type safety
-   excellent async/concurrency support

Potential release artifacts:

``` text
cast-linux-amd64
cast-linux-arm64
cast-macos-arm64
cast-windows-amd64
```

The single-binary deployment model is a product feature, not merely a
technical preference.

------------------------------------------------------------------------

# 27. Why Not C#?

C#/.NET remains a very strong alternative.

A .NET application can be published self-contained and as a single-file
executable, so it does not necessarily require the user to manually
install .NET.

C# has excellent:

-   HTTP/server infrastructure
-   async support
-   WebSockets
-   JSON
-   background workers
-   database libraries
-   observability
-   framework maturity

The reason to prefer Rust is not that C# is inadequate.

The reason is that:

> Casting's deployment experience should be extremely simple, and a
> native executable is a compelling primitive for that experience.

Rust should be evaluated primarily as a product/deployment decision
rather than an ideological one.

------------------------------------------------------------------------

# 28. Potential Future Rust Architecture

If execution/sandboxing becomes sufficiently complex, Rust is also a
natural fit for the agent execution layer.

Possible future architecture:

``` text
                 Casting Control Plane
                         │
                ┌────────┴────────┐
                │                 │
             Control           Execution
               │                 │
              Rust              Rust
                                │
                       ┌────────┼────────┐
                       │        │        │
                     shell     git    sandbox
```

Do not introduce unnecessary architectural divisions early.

Start with one executable.

Split components only when there is a demonstrated reason.

------------------------------------------------------------------------

# 29. Database Direction

## Default initial database: SQLite

SQLite is extremely attractive for the first version because:

-   zero database installation
-   one database file
-   excellent local developer experience
-   easy backup
-   easy reproduction
-   simple deployment
-   works naturally with `cast run`

A project might look like:

``` text
my-project/
    .casting/
        casting.db
        config.toml
        agents/
        logs/
```

The entire project history could potentially be copied or backed up as
part of this local deployment model.

SQLite should use an appropriate concurrency configuration such as WAL
mode.

------------------------------------------------------------------------

# 30. PostgreSQL as the Scalable Backend

PostgreSQL should remain a planned backend.

It becomes increasingly attractive when:

-   many workers are writing concurrently
-   many users access the project
-   deployments become multi-node
-   query complexity grows
-   external integrations need database access
-   project scale increases significantly

Logical abstraction:

``` text
Casting persistence
       │
       ├── SQLite
       │
       └── PostgreSQL
```

Do not force the entire application into a lowest-common-denominator SQL
abstraction.

The domain semantics should be portable while database implementations
may use the strengths of their underlying systems.

------------------------------------------------------------------------

# 31. Deployment Philosophy

The deployment UX is a core product requirement.

The target should be:

``` bash
curl -fsSL https://casting.dev/install.sh | sh
cast run
```

or an equivalent simple installation mechanism.

The ideal experience:

``` text
🎬 Casting

Starting your project...

✓ Database ready
✓ Agent runtime ready
✓ Project Manager ready
✓ Web server ready

Your Casting workspace:

https://abc123.cast.dev

Owner login:
ben
Password:
********
```

The user should not need to know whether the database is SQLite,
PostgreSQL, or something else.

------------------------------------------------------------------------

# 32. Possible Deployment Modes

## Local / simple

``` bash
cast run
```

Uses SQLite.

No external services required.

## Server / serious

Potentially:

``` bash
cast run --database postgres://...
```

or a managed configuration.

## Future hosted mode

Potentially:

``` bash
cast cloud
```

or a managed Casting service.

The same logical project model should work across these modes.

------------------------------------------------------------------------

# 33. Web Architecture

The system will need a web interface with real-time updates.

The UI should expose at least:

### Work

What is everyone doing?

### People

Who is working on this?

What are they good at?

What are they currently doing?

### Timeline

What happened?

### Decisions

Why did we do this?

### Inbox

What does the owner need to decide?

The dashboard should update in real time as agents work.

------------------------------------------------------------------------

# 34. Initial Web UI

The first useful UI should contain:

## Owner ↔ PM chat

The primary human interaction.

## Team

Current cast, roles, status, capabilities.

## Tasks

Simple Kanban/task tree.

## Activity

Chronological project event stream.

## Decisions

Permanent decision history.

## Inbox

Requests requiring owner attention.

Avoid building a huge dashboard before the underlying workflows work.

------------------------------------------------------------------------

# 35. Realtime Updates

The architecture should support:

``` text
Agent
  ↓
Domain event
  ↓
Event store
  ↓
Projection
  ↓
Realtime notification
  ↓
Browser
```

Example:

``` text
Marcus completes "Implement authentication"

↓

TaskCompleted event

↓

Task projection updated

↓

Browser receives update

↓

Kanban moves task to Review
```

The UI should feel alive.

------------------------------------------------------------------------

# 36. First Vertical Slice

Do not begin by building a sophisticated multi-agent coding swarm.

First build a tiny simulated software company.

The first vertical slice should prove:

``` text
Owner
  │
  │ "Build me a todo app"
  ▼
PM
  │
  ├── creates requirement
  ├── creates tasks
  ├── hires Engineer
  │
  ▼
Engineer
  │
  ├── starts task
  ├── produces observation
  ├── produces work
  └── completes task
  │
  ▼
QA
  │
  └── finds bug
  │
  ▼
PM
  │
  └── reprioritizes
```

At every meaningful step:

**events are persisted.**

The UI is generated from projections.

This proves the core architecture before the system becomes complicated
with real coding agents.

------------------------------------------------------------------------

# 37. Suggested Initial Repository Structure

A possible Rust structure:

``` text
casting/
│
├── crates/
│   ├── casting-domain/
│   │   ├── events/
│   │   ├── tasks/
│   │   ├── agents/
│   │   ├── decisions/
│   │   └── projects/
│   │
│   ├── casting-application/
│   │   ├── commands/
│   │   ├── queries/
│   │   ├── projections/
│   │   └── agents/
│   │
│   ├── casting-infrastructure/
│   │   ├── persistence/
│   │   ├── llm/
│   │   ├── git/
│   │   └── execution/
│   │
│   ├── casting-web/
│   │   ├── api/
│   │   └── realtime/
│   │
│   └── casting-cli/
│
└── tests/
```

This is illustrative, not a requirement.

Avoid excessive crate/service decomposition until the boundaries have
proven useful.

A single Rust binary should initially be the default.

------------------------------------------------------------------------

# 38. Initial Domain Primitives

The first domain model should probably include:

``` text
Project
Owner
Agent
Role
Capability
Task
Decision
Requirement
Observation
Message
Event
AgentRun
Budget
```

Potential relationships:

``` text
Project
├── Owner
├── Agents
├── Requirements
├── Tasks
├── Decisions
├── Messages
├── Observations
├── Events
└── Budget
```

------------------------------------------------------------------------

# 39. Important Distinctions

These concepts should not be collapsed together.

## Event

Something that happened.

## Task

Something that needs to happen.

## Decision

A choice about how the project should proceed.

## Observation

Something an agent noticed.

## Message

Human-readable communication between participants.

## Requirement

Something the product is intended to achieve.

## Agent run

One execution of an agent/model.

These objects may produce events, but they are not interchangeable.

------------------------------------------------------------------------

# 40. Example Decision History

Suppose the team chooses PostgreSQL and later changes its mind.

The history should look something like:

``` text
14:02
DecisionProposed
PostgreSQL recommended

14:04
DecisionApproved
Owner approved PostgreSQL

18:21
ObservationCreated
Engineer found an issue

18:29
DecisionProposed
SQLite recommended for this deployment mode

18:33
OwnerDecisionRequested

18:37
OwnerDecisionRecorded
Owner approved SQLite

18:38
DecisionSuperseded
PostgreSQL decision superseded
```

Current state:

``` text
Database = SQLite
```

Historical truth:

``` text
PostgreSQL was previously chosen and later superseded.
```

Both are important.

------------------------------------------------------------------------

# 41. The "Why?" Experience

One of the long-term killer features should be answering:

> Why does this code/architecture/task exist?

Potential chain:

``` text
Code
 ↓
Commit
 ↓
Task
 ↓
Decision
 ↓
Conversation
 ↓
Original requirement
 ↓
Owner
```

Example:

``` text
Why are we using PostgreSQL?

Decision #184 — 14 September

Sarah Chen proposed PostgreSQL because:
- transactional requirements
- existing team expertise
- expected data volume

Architecture consultant agreed.

Owner approved the decision.

Tasks #391–#417 were created from this decision.
```

This is a major advantage of persistent project history.

------------------------------------------------------------------------

# 42. Product Differentiation

Do not position Casting primarily as:

> "A multi-agent coding framework."

That space will become increasingly commoditized.

A stronger positioning is:

> **An autonomous software company in a box.**

Or:

> **Hire an AI team. Give them a goal. Manage them like a company.**

Casting's differentiation is:

-   orchestration
-   governance
-   memory
-   decision history
-   human delegation
-   cost management
-   project state
-   contextual communication
-   agent identity
-   real-time visibility

The underlying coding agents can evolve independently.

------------------------------------------------------------------------

# 43. What Not To Build Yet

Avoid premature complexity.

Do not initially build:

-   Kafka
-   Kubernetes
-   Temporal
-   EventStoreDB
-   distributed microservices
-   complex external message brokers
-   full email infrastructure
-   WhatsApp integration
-   Telegram integration
-   agent marketplace
-   elaborate fictional character systems
-   Jira compatibility
-   Scrum/Agile methodology features
-   huge analytics dashboards

Build only what proves the core model.

The architecture should be capable of evolving into these things, but
they should not distract from the core loop.

------------------------------------------------------------------------

# 44. Core Success Criterion

The fundamental success criterion is not:

> Can several LLMs edit the same repository?

That problem is solvable.

The real question is:

> Can a human owner feel like they are running a capable little software
> company rather than babysitting a swarm of chatbots?

Casting succeeds if the answer is yes.

The owner should be able to say:

``` text
"I want this."
```

and then trust the system to:

``` text
understand
plan
delegate
build
review
adapt
ask
remember
```

while keeping the owner informed and in control.

------------------------------------------------------------------------

# 45. Initial Engineering Priorities

Prioritize in roughly this order:

1.  **Excellent first-run experience**
2.  **Clean project/domain model**
3.  **Reliable append-only event history**
4.  **Current-state projections**
5.  **Owner ↔ PM interaction**
6.  **Simple task system**
7.  **Agent identity/capabilities**
8.  **Decision history**
9.  **Context assembly**
10. **Real agent execution**
11. **Cost tracking**
12. **Realtime dashboard**
13. **External owner messaging**
14. **More advanced orchestration**

Do not invert this order by starting with agent swarm sophistication.

------------------------------------------------------------------------

# 46. Initial Product Milestone

The first milestone should be something that feels like Casting even if
the agents are partially simulated.

A user should be able to:

``` bash
cast run
```

receive a workspace URL and credentials, open the UI, and:

1.  See their project.
2.  Meet their PM.
3.  Tell the PM what they want.
4.  See the PM create requirements/tasks.
5.  See consultants appear.
6.  See tasks move through the board.
7.  See activity in real time.
8.  See messages/observations.
9.  See decisions being requested.
10. Make a decision.
11. See that decision recorded permanently.
12. See the project state change accordingly.
13. Reload the application and have everything still present.
14. Inspect the history and understand why the current state exists.

Only after this feels good should the system become a serious autonomous
coding environment.

------------------------------------------------------------------------

# 47. Guiding Philosophy

Casting should optimize for:

**Human clarity over agent cleverness.**

**User experience over implementation convenience.**

**Persistent shared state over isolated agent context.**

**Explainability over mysterious autonomy.**

**Delegated authority over unrestricted autonomy.**

**Simple deployment over infrastructure purity.**

**Useful history over raw logs.**

**Purpose-built workflows over enterprise feature bloat.**

**Real product value over impressive demos.**

The system should make autonomous software development feel
understandable, controlled, enjoyable, and surprisingly human.

------------------------------------------------------------------------

# 48. The North Star

The ideal interaction is eventually this:

``` text
Owner:

"I want a SaaS product for managing climbing gyms.
I'd like a first version in the next few days.
Keep costs reasonable and don't bother me unless
you genuinely need a decision."

                     ↓

                 CASTING

Sarah Chen — Project Manager

"Understood. I've broken this into the initial product
areas and brought in an engineer and UX consultant.

I have three questions before we begin..."

                     ↓

                 OWNER

Answers three questions.

                     ↓

                 CASTING

The team works.

The owner occasionally sees:

"Sarah Chen:
The team has completed the initial architecture."

"Maya Patel:
I've identified a UX issue worth investigating.
I've created an observation rather than interrupting you."

"Marcus Reed:
Authentication is implemented and ready for review."

"Sarah Chen:
QA has found an architectural issue.
I recommend option A. This will add approximately
$12 and one hour. Shall I proceed?"

                     ↓

                 OWNER

"Yes."

                     ↓

                 CASTING

The team continues.

Everything that happened is preserved.

Every important decision can be explained.

The owner remains in control.

The software gets built.
```

**That is Casting.**
