# Casting — Architectural Addendum

## PM Control Loop, Version Control & Code Provenance

> **AUTHORITATIVE (kept current).** This is a live design doc — referenced
> throughout the codebase as "brief §X". Update it when the PM loop, Git, or
> provenance design changes; do not treat it as archival.

**Status:** Architectural direction
**Purpose:** Clarify two areas that are fundamental to turning Casting from an agent orchestration framework into an autonomous software company:

1. How the Project Manager actually operates as a continuous decision-making system.
2. How Casting relates to Git and the software artifacts its agents produce.

These concerns are deeply related.

The PM should not merely manage a Kanban board. It should manage a continuous chain:

```text
Intent
  ↓
Requirements
  ↓
Decisions
  ↓
Plans
  ↓
Tasks
  ↓
Agent work
  ↓
Code artifacts
  ↓
Evidence / reviews
  ↓
New observations
  ↓
New decisions
  ↓
Re-plan
```

The goal is to create a system where autonomous software development feels like an organization operating over time rather than a collection of LLM calls.

------

# 1. The Project Manager Is a Control Loop

The PM should be understood as a long-running autonomous control loop over the project's event history.

It is not simply:

```text
user message
    ↓
LLM
    ↓
response
```

It is closer to:

```text
                    PROJECT EVENT STREAM
                            │
                            ▼
                     PM WAKE-UP
                            │
                            ▼
                    CONTEXT ASSEMBLY
                            │
                            ▼
                      PM REASONING
                            │
                            ▼
                     STATE ASSESSMENT
                            │
                            ▼
                    DECISION / ACTION
                            │
                ┌───────────┼────────────┐
                ▼           ▼            ▼
             observe      act         ask owner
                │           │            │
                └───────────┴────────────┘
                            │
                            ▼
                     NEW EVENTS
                            │
                            ▼
                       PM SLEEPS
```

The PM should wake when something meaningful happens, determine whether the project requires action, take whatever actions it is authorized to take, record those actions, and then return to a waiting state.

This is the core loop that makes the system feel like a company.

------

# 2. The PM Has a Cursor

The PM should maintain a durable position in the project's event history.

For example:

```text
PM cursor:
project = project-123
last_seen_sequence = 1842
```

When the PM wakes, it can determine:

```text
Events since 1842:

1843 TaskCompleted
1844 ObservationCreated
1845 CommitObserved
1846 ReviewCompleted
```

The PM does not need to reconstruct the entire project from scratch every time.

It receives:

- the current project projection
- its durable cursor
- relevant events since the cursor
- relevant historical context
- current assignments
- unresolved observations
- pending decisions
- owner policies
- current budget
- relevant repository state

The PM then determines what, if anything, should happen.

The cursor must be durable.

If the PM crashes, restarts, or is replaced by another worker, it should be able to resume from its last known position.

------

# 3. Not Every Event Should Wake the PM

The existence of an event should not automatically imply that the PM needs to reason about it.

Low-level events may occur constantly:

```text
LLM token received
Git status checked
Shell command executed
File opened
Container started
```

These should not wake the PM.

The PM should primarily respond to meaningful project events such as:

```text
RequirementCreated
RequirementChanged

TaskCreated
TaskBlocked
TaskCompleted

ObservationCreated

DecisionProposed
DecisionApproved
DecisionRejected

AgentHired
AgentUnavailable

ReviewRequested
ReviewCompleted

ChangeSetReady
MergeCompleted
MergeConflictDetected

BudgetThresholdReached

OwnerMessageReceived

IncidentDetected
```

The system should distinguish between:

> Something happened inside the machinery.

and:

> Something happened that may require the organization to reconsider what it is doing.

Only the latter should generally trigger PM reasoning.

------

# 4. PM Wake-Up Conditions

The PM should wake for several broad categories of reasons.

## 4.1 Owner input

The owner sends a message or makes a decision.

```text
OwnerMessageReceived
DecisionMade (owner-authored)
RequirementChanged
```

The PM should immediately evaluate whether this changes project priorities, requirements, or plans.

------

## 4.2 Work completion

An agent completes meaningful work.

```text
TaskCompleted
ChangeSetReady
ReviewCompleted
InvestigationCompleted
```

The PM should determine what this unlocks.

For example:

```text
Authentication implementation completed
        ↓
PM evaluates
        ↓
Security review required
        ↓
Security consultant assigned
```

------

## 4.3 Work failure or blockage

Something prevents planned work from continuing.

```text
TaskBlocked
TestsFailed
BuildFailed
AgentFailed
MergeConflictDetected
DependencyUnavailable
```

The PM should determine whether to:

- retry
- reassign
- split the task
- create an investigation
- change the plan
- escalate
- ask the owner

------

## 4.4 Important observations

A consultant or agent notices something potentially important.

```text
ObservationCreated
```

The PM determines whether the observation is:

```text
informational
actionable
planning-relevant
owner-relevant
critical
```

Not every observation deserves a task or owner interruption.

------

## 4.5 Decision points

The project reaches a point where a decision is required.

```text
DecisionRequested
```

The PM should gather relevant information, evaluate alternatives, apply the owner's policies, and either:

- make the decision itself,
- delegate it,
- request additional investigation,
- or ask the owner.

------

## 4.6 Budget and policy thresholds

The PM may wake because a policy threshold has been reached.

For example:

```text
Projected spend exceeds task budget
```

or:

```text
Production deployment requested
```

or:

```text
Security-sensitive capability requested
```

The PM evaluates the applicable delegation policy.

------

# 5. The PM Should Not React Blindly

A critical design principle:

> An event is a reason to reconsider the project, not necessarily a reason to act.

For example:

```text
TaskCompleted
```

does not necessarily mean:

```text
Create another task
```

The PM should first ask:

```text
What does this change?

Does it affect the plan?

Does it unblock anything?

Does it create new risk?

Does it require review?

Does it require owner involvement?

Does nothing need to happen?
```

The final answer may simply be:

```text
No action required.
```

This prevents the organization from becoming an endless chain of unnecessary agent activity.

------

# 6. The PM's Reasoning Cycle

A useful conceptual cycle is:

```text
1. Observe
2. Understand
3. Evaluate
4. Decide
5. Act
6. Verify
7. Record
8. Wait
```

### 1. Observe

Read relevant new events and current project state.

### 2. Understand

Determine what changed and why it matters.

### 3. Evaluate

Compare the new state against:

- project goals
- requirements
- current plan
- dependencies
- risks
- budget
- owner policies
- agent capabilities
- repository state

### 4. Decide

Determine the next organizational action.

### 5. Act

Execute authorized actions such as:

- create task
- assign task
- reassign work
- hire consultant
- request review
- create decision
- change priority
- update plan
- request owner input

### 6. Verify

Where appropriate, check that the intended action actually occurred.

### 7. Record

Persist meaningful decisions and actions as domain events.

### 8. Wait

Advance the PM cursor and wait for the next meaningful trigger.

------

# 7. Avoiding PM Thrashing

This is one of the most important engineering problems.

A naïve autonomous loop could produce:

```text
Task completed
    ↓
PM creates task
    ↓
Agent completes task
    ↓
PM creates another task
    ↓
Agent completes task
    ↓
PM creates another task
    ↓
...
```

Or:

```text
Agent A makes observation
    ↓
PM asks Agent B
    ↓
Agent B makes observation
    ↓
PM asks Agent A
    ↓
Agent A makes another observation
    ↓
...
```

Casting must have explicit mechanisms against this.

------

# 8. Planning Should Have Stable Objectives

The PM should maintain a current plan rather than generating an entirely new plan after every event.

A plan may contain:

```text
Objective
Current strategy
Active work
Dependencies
Known risks
Open questions
Decision points
Exit conditions
```

An event should cause the PM to ask:

> Does this invalidate or materially change the current plan?

If not, the PM should usually leave the plan alone.

This gives the organization stability.

------

# 9. Replanning Should Have Triggers

Replanning should happen when meaningful conditions occur.

Examples:

```text
Requirement changed
Critical dependency failed
Major technical assumption disproved
Task blocked
Important discovery made
Budget forecast changes substantially
Owner changes priority
Security issue discovered
Architecture decision invalidated
External system changes
```

Routine progress should not automatically cause wholesale replanning.

The PM should preserve continuity whenever possible.

------

# 10. PM Actions Should Be Bounded

The PM should not be allowed to produce unlimited work from a single observation.

For example:

```text
Maximum tasks created in one cycle
Maximum consultants activated
Maximum concurrent work
Maximum budget committed
Maximum retries
Maximum replanning depth
```

These should eventually become policy/configuration rather than hard-coded magic numbers.

The general principle is:

> Autonomy needs boundaries.

------

# 11. The PM Should Have a Decision Budget

A useful future concept is a reasoning/action budget per wake cycle.

For example:

```text
PM Wake #842

Reason:
Authentication task completed

Budget:
- max 5 agent calls
- max $2.00
- max 10 minutes of delegated work

Actions:
1. Review implementation
2. Request security review
3. Update task dependencies
4. Wait
```

This prevents one event from triggering an uncontrolled cascade.

------

# 12. The PM Should Prefer Existing Information

Before asking another agent a question, the PM should determine whether the answer already exists in:

- project state
- event history
- decisions
- task history
- previous observations
- repository state
- existing reviews
- agent reports

This is important for both cost and stability.

The PM should not repeatedly ask:

> What do we know about authentication?

if the project already has a decision, architecture document, investigation, and review answering that question.

------

# 13. The PM Should Prefer Delegation Over Doing Specialist Work

The PM is the management layer.

It should not become the world's most expensive general-purpose engineer.

For example:

```text
Bad:

PM personally investigates OAuth libraries.
PM personally reviews frontend accessibility.
PM personally performs security analysis.
PM personally implements code.
```

Better:

```text
PM:
"We need to determine the best authentication strategy."

→ Architecture consultant
→ Security consultant
→ Engineer

PM:
reconciles their findings
```

The PM's unique value is coordination and judgment.

------

# 14. The PM Is the Strongest Reasoning Agent

The PM will likely justify using one of the strongest available models.

This is appropriate.

The PM is responsible for:

- interpreting ambiguous intent
- reconciling competing recommendations
- understanding long-term project state
- deciding when to act
- deciding when not to act
- deciding when to ask the owner
- managing tradeoffs
- maintaining coherence

However, "strongest model" should not mean:

> Give the PM unlimited context and unlimited tool access.

The PM should still operate through:

```text
structured context
+
explicit capabilities
+
delegated tools
+
budget
+
decision policies
+
durable state
```

Model intelligence and system architecture should complement each other.

------

# 15. PM Context Assembly

The PM should not receive the entire event history on every wake.

Its context should be assembled from:

```text
Project intent
+
Current project state
+
Current plan
+
Recent relevant events
+
Unresolved observations
+
Active tasks
+
Pending decisions
+
Owner policies
+
Budget state
+
Relevant agent reports
+
Relevant repository/change-set state
```

The PM should have access to deeper history when needed, but the default context should remain focused.

This is both a cost optimization and a reasoning-quality optimization.

------

# 16. PM Actions Should Be Structured

The PM should not merely produce prose such as:

```text
"I think Marcus should investigate this."
```

It should produce structured intentions/actions.

Conceptually:

```text
PMDecision
{
    assessment: "...",

    actions: [
        {
            type: "CreateTask",
            ...
        },
        {
            type: "AssignTask",
            ...
        },
        {
            type: "RequestOwnerDecision",
            ...
        }
    ]
}
```

The system validates those actions against:

- capabilities
- permissions
- autonomy policy
- budget
- project state

Only then are they executed.

This creates an important boundary:

```text
LLM reasoning
      ↓
structured proposed actions
      ↓
policy validation
      ↓
execution
      ↓
domain events
```

The LLM should not directly mutate the database or invoke arbitrary infrastructure.

------

# 17. PM Decisions Should Be Durable

If the PM makes an important decision, the reasoning should become part of project history.

For example:

```text
DecisionProposed

Subject:
Authentication architecture

Options:
A: OAuth
B: Custom authentication

Recommendation:
A

Reasoning:
...

Evidence:
...

Risk:
...

Estimated cost:
...

Owner involvement:
Required
```

If the owner approves:

```text
DecisionMade (universal event; actor = who decided — the owner if asked, or a delegated PM/agent)
```

The resulting state should be durable and explainable.

------

# 18. Git and Casting

Casting should treat Git as a first-class external system.

The core principle is:

> **Git knows what code exists. Casting knows why it exists.**

Casting should not attempt to replace Git's version-control semantics.

Git remains authoritative for:

- repository contents
- branches
- commits
- diffs
- tags
- merge history
- working-tree state

Casting remains authoritative for:

- requirements
- tasks
- decisions
- assignments
- agent identity
- authorization
- reviews
- approvals
- project intent
- organizational history
- reasons for changes

The two systems should be linked explicitly.

------

# 19. Casting Drives the Workflow; Git Owns the Artifacts

The correct relationship is not:

```text
Git inside Casting
```

nor:

```text
Casting ignores Git
```

It is:

```text
                    CASTING
                       │
               Organizational layer
                       │
                 Git integration
                       │
                       ▼
                      GIT
               Artifact layer
```

Casting should orchestrate Git operations while Git remains the source of truth for the artifacts themselves.

A typical workflow becomes:

```text
Requirement
    ↓
Task
    ↓
Agent assignment
    ↓
Git branch
    ↓
Code changes
    ↓
Git commits
    ↓
Review
    ↓
Approval
    ↓
Merge
    ↓
Task completion
```

------

# 20. Code Changes Should Be Isolated

Autonomous agents should generally work on isolated branches.

For example:

```text
main
 │
 ├── casting/task-381-authentication
 ├── casting/task-382-billing
 └── casting/task-383-onboarding
```

Agents should not directly modify protected branches.

The general invariant is:

> Autonomous work must be isolated, inspectable, reversible, and reviewable before it affects protected project state.

This provides a powerful safety boundary.

------

# 21. Tasks Should Know About Their Code

A task should be able to reference its software work.

For example:

```text
Task #381
────────────────────────────

Implement authentication

Assigned:
Marcus Reed

Repository:
climbing-gym

Branch:
casting/task-381-authentication

ChangeSet:
change-set-73

Commits:
a83f91c
b18c220
d931e01

Tests:
Passing

Review:
Security review pending
```

Git remains authoritative for the branch and commits.

Casting owns the association between those artifacts and the organizational work.

------

# 22. Change Sets

Casting should eventually expose a higher-level `ChangeSet` concept.

A ChangeSet represents the software artifacts produced as part of a piece of organizational work.

Conceptually:

```text
ChangeSet
─────────

Task
Repository
Branch
Commits
Diff
Tests
Reviews
Status
```

This abstraction is useful because the organizational layer should not be tightly coupled to a specific Git hosting provider.

A ChangeSet might eventually correspond to:

```text
local Git branch
GitHub Pull Request
GitLab Merge Request
```

without changing the PM's mental model.

------

# 23. Git Events Should Be Semantic

Low-level Git operations should generally remain runtime telemetry:

```text
git status
git checkout
git fetch
git add
git object created
```

These should not normally become project-level events.

Instead, Casting should observe meaningful repository activity such as:

```text
BranchCreated
CommitObserved
ChangeSetUpdated
ReviewRequested
ReviewCompleted
MergeCompleted
MergeConflictDetected
TestsPassed
TestsFailed
BranchAbandoned
```

The principle is:

> **Semantic events, not plumbing events.**

------

# 24. Commit Provenance

Casting should maintain explicit relationships between code artifacts and project context.

A commit may be associated with:

```text
commit
task
requirement
decision
agent
agent_run
change_set
review
```

For example:

```text
Commit:
a83f91c

Task:
#381

Agent:
Marcus Reed

Agent run:
run-8421

Decision:
#184

ChangeSet:
#73
```

Git remains authoritative for the commit itself.

Casting owns the organizational relationship surrounding it.

Commit metadata may also contain Casting references, for example using Git trailers:

```text
Casting-Task: task-381
Casting-Agent-Run: run-8421
Casting-Project: project-123
```

However, commit-message conventions should supplement rather than replace Casting's own persisted associations.

------

# 25. The "Why Does This Code Exist?" Graph

One of Casting's most valuable long-term capabilities should be bidirectional provenance.

Starting from code:

```text
Code
 ↓
Commit
 ↓
ChangeSet
 ↓
Task
 ↓
Decision
 ↓
Requirement
 ↓
Owner intent
```

Or starting from a decision:

```text
Decision
 ↓
Tasks
 ↓
ChangeSets
 ↓
Commits
 ↓
Code
```

This allows the system to answer:

> Why is this code here?

and:

> What did this decision produce?

For example:

```text
Why is OAuth used here?

Decision #184
    ↓
Owner approved OAuth authentication.
    ↓
Requirement #12
    ↓
Task #381
    ↓
Marcus Reed implemented authentication.
    ↓
Commits a83f91c, b18c220
    ↓
Security review approved.
    ↓
Merged to main.
```

This is a major potential product differentiator.

------

# 26. Git Is Also an Observation Source for the PM

Git should feed the PM's control loop.

For example:

```text
TaskAssigned
    ↓
Agent creates branch
    ↓
Agent produces commits
    ↓
Casting observes meaningful repository activity
    ↓
PM evaluates progress
    ↓
Review requested
    ↓
ReviewCompleted
    ↓
PM decides whether to merge
    ↓
Merge executed
    ↓
MergeObserved
    ↓
TaskCompleted
```

The PM therefore manages real software work rather than merely managing task records.

------

# 27. Do Not Build GitHub First

The first real implementation should support local Git directly.

A project might look like:

```text
my-project/
├── .git/
├── .casting/
│   ├── casting.db
│   └── config.toml
├── src/
└── ...
```

Then:

```text
cast run
```

can discover and manage the repository.

External integrations can come later:

```text
Casting Repository
       │
       ├── LocalGit
       ├── GitHub
       ├── GitLab
       └── ...
```

Only implement the integrations that are actually needed.

The abstraction should exist because the boundary is useful, not because a large provider matrix is required on day one.

------

# 28. Version Control and the First Vertical Slice

The simulated first vertical slice does not need real Git.

It should prove:

```text
Owner
  ↓
PM
  ↓
Requirement
  ↓
Tasks
  ↓
Agents
  ↓
Observations
  ↓
Replanning
  ↓
Owner decisions
```

After that architecture is proven, the first real coding slice should introduce:

```text
Repository
Branch
Commit
Diff
ChangeSet
Review
Merge
```

Only then should the system evolve toward sophisticated autonomous coding.

This preserves the project's original principle:

> Prove the organizational model before building the swarm.

------

# 29. The Combined Mental Model

The most useful way to think about Casting is now:

```text
                         OWNER
                           │
                           │ intent
                           ▼
                    PROJECT MANAGER
                           │
                    control loop
                           │
          ┌────────────────┼────────────────┐
          │                │                │
          ▼                ▼                ▼
       Planning         Agents          Decisions
          │                │                │
          └────────────────┼────────────────┘
                           │
                         Tasks
                           │
                           ▼
                      ChangeSets
                           │
                           ▼
                          GIT
                           │
                           ▼
                         CODE
```

But information flows in both directions.

For example:

```text
Owner intent
    ↓
Requirement
    ↓
Decision
    ↓
Task
    ↓
Agent
    ↓
Git change
    ↓
Tests
    ↓
Review
    ↓
Observation
    ↓
PM
    ↓
Re-plan
```

The organization learns from the artifacts it creates.

------

# 30. The Core Architectural Boundary

The following distinction should be treated as an explicit architectural principle:

### Casting owns organizational truth.

```text
What are we trying to accomplish?
Why?
Who is responsible?
What decisions have been made?
What authority has been delegated?
What work exists?
What has been observed?
What should happen next?
```

### Git owns artifact truth.

```text
What code exists?
What changed?
What branch contains it?
What commits produced it?
What was merged?
What does the repository currently contain?
```

### The integration owns provenance.

```text
Why did this code change?
Which task produced it?
Which agent produced it?
Which decision authorized it?
Which review approved it?
Which requirement motivated it?
```

This three-way separation should guide the implementation.

------

# 31. What This Means for the Architecture

The system should therefore be thought of as three related layers:

```text
┌──────────────────────────────────────────┐
│              ORGANIZATION                │
│                                          │
│ Project / PM / Tasks / Decisions /       │
│ Requirements / Agents / Policies /       │
│ Budget / History                         │
│                                          │
│              Casting Domain              │
└───────────────────┬──────────────────────┘
                    │
             provenance links
                    │
┌───────────────────┴──────────────────────┐
│                ARTIFACTS                 │
│                                          │
│ Repository / Branch / Commit / Diff /    │
│ Merge / Working Tree                     │
│                                          │
│                    Git                   │
└──────────────────────────────────────────┘

                    +

┌──────────────────────────────────────────┐
│                EXECUTION                 │
│                                          │
│ LLM calls / tools / shell / containers / │
│ tests / agent runs / telemetry           │
│                                          │
│           Runtime Infrastructure         │
└──────────────────────────────────────────┘
```

These layers should interact through explicit interfaces rather than becoming one giant abstraction.

------

# 32. Final Principles

The implementation should preserve the following principles.

### The PM is a control loop, not a chatbot.

It wakes from meaningful project events, assembles relevant context, reasons about the current state, takes bounded actions, records those actions, and waits.

### The PM should maintain continuity.

It should have a durable cursor and a persistent plan so that every event does not cause a completely new interpretation of the project.

### Events should trigger reconsideration, not automatic action.

The PM should be able to conclude:

```text
Nothing needs to happen.
```

### Autonomy must be bounded.

Budget, capabilities, permissions, decision policies, action limits, and escalation rules are part of the control system.

### The PM manages coordination rather than performing every specialist task.

The PM's unique value is judgment, prioritization, delegation, reconciliation, and project coherence.

### Git remains Git.

Do not reinvent version control inside Casting.

### Casting remains Casting.

Do not make the Git repository the organizational source of truth.

### Casting drives the workflow; Git owns the artifacts.

This is the intended relationship.

### Provenance connects the two.

Every meaningful piece of software work should eventually be traceable to:

```text
code
 → commit
 → change set
 → task
 → decision
 → requirement
 → owner intent
```

### Semantic events matter more than low-level telemetry.

The PM should react to meaningful changes in project reality, not every operation performed by the underlying machinery.

### The ultimate goal is explainability.

The system should eventually be able to answer:

> What is happening?

> Why is it happening?

> Who decided this?

> What changed?

> What code resulted?

> What should happen next?

> What does the owner need to decide?

That is the difference between an autonomous software company and a swarm of agents.