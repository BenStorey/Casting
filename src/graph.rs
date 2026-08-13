//! The Graph / Transition spine — the explicit, derived view of the task
//! lifecycle (2026-08-13).
//!
//! Every structure here is a **deterministic read-side projection** over the
//! event-sourced `Projection`. There is NO stored authoritative state, no
//! second authority, and no LLM in the loop: the event log stays the only
//! source of truth and `graph()` merely *makes explicit* what the lifecycle
//! already encodes (task states, transitions, decomposition/joins, causal
//! order). This is the coherence spine the rest of the product attaches to.
//!
//! Two jobs:
//!
//! 1. A **single written transition contract** (the `TABLE`): consumed by the PM
//!    prompt ("valid exits from state X: …"), a validation/debug check, and the
//!    dashboard. One definition, many consumers, no drift.
//! 2. A **GraphView** — nodes + groups (join points) + active/blocked tokens +
//!    per-node provenance chain — for parallel-work visibility and "why in this
//!    order".

use crate::projection::{Projection, Task, TaskStatus};
use serde::{Deserialize, Serialize};

/// The semantic state of a task, DERIVED from its lifecycle events. This is a
/// richer, reader-facing classification layered *over* the low-level
/// `TaskStatus` (which stays the canonical board position). Fully deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// In the backlog, not yet assigned/started.
    Queued,
    /// In progress (assigned and/or started), not yet submitted.
    Working,
    /// Submitted, awaiting review.
    InReview,
    /// Blocked — waiting on the human owner (a pause node in the graph).
    AwaitingHuman,
    /// Review rejected — rework due.
    Rejected,
    /// Terminal: complete.
    Done,
}

impl TaskState {
    /// Human-readable label for dashboards/prompts.
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Working => "working",
            TaskState::InReview => "in review",
            TaskState::AwaitingHuman => "awaiting human",
            TaskState::Rejected => "rejected — rework due",
            TaskState::Done => "done",
        }
    }
}

/// One legal transition between two `TaskState`s — the atomic unit of the
/// contract. The `gate` decides whether it is *currently* available for a
/// specific task (e.g. "submit" needs an assignee).
pub struct Transition {
    /// Stable id (snake_case) used by prompts, the API and the SPA.
    pub id: &'static str,
    /// Human label for the PM prompt / dashboard.
    pub label: &'static str,
    pub from: TaskState,
    pub to: TaskState,
    /// The `PmAction` variant that realizes this transition (snake_case id).
    pub action: &'static str,
    /// Availability: `Some(reason)` = unavailable right now; `None` = allowed.
    pub gate: fn(&Projection, &Task) -> Option<&'static str>,
}

fn avail(state: &Projection, task: &Task) -> Option<&'static str> {
    let _ = (state, task);
    None
}
fn needs_assignee(_state: &Projection, task: &Task) -> Option<&'static str> {
    if task.assignee.is_some() {
        None
    } else {
        Some("no consultant assigned")
    }
}
fn no_children(state: &Projection, task: &Task) -> Option<&'static str> {
    if state.children_of(&task.id).is_empty() {
        None
    } else {
        Some("already decomposed")
    }
}

/// The single transition contract. Written once, read by the PM prompt, the
/// validation/debug surface and the dashboard.
pub static TABLE: &[Transition] = &[
    Transition {
        id: "assign",
        label: "Assign to consultant",
        from: TaskState::Queued,
        to: TaskState::Working,
        action: "assign_task",
        gate: avail,
    },
    Transition {
        id: "start",
        label: "Start",
        from: TaskState::Working,
        to: TaskState::Working,
        action: "start_task",
        gate: needs_assignee,
    },
    Transition {
        id: "submit",
        label: "Submit for review",
        from: TaskState::Working,
        to: TaskState::InReview,
        action: "request_review",
        gate: needs_assignee,
    },
    Transition {
        id: "block",
        label: "Escalate — need owner",
        from: TaskState::Working,
        to: TaskState::AwaitingHuman,
        action: "block_task",
        gate: avail,
    },
    Transition {
        id: "approve",
        label: "Approve",
        from: TaskState::InReview,
        to: TaskState::Done,
        action: "review_task",
        gate: avail,
    },
    Transition {
        id: "reject",
        label: "Request changes",
        from: TaskState::InReview,
        to: TaskState::Rejected,
        action: "review_task",
        gate: avail,
    },
    Transition {
        id: "block",
        label: "Escalate — need owner",
        from: TaskState::InReview,
        to: TaskState::AwaitingHuman,
        action: "block_task",
        gate: avail,
    },
    Transition {
        id: "decompose",
        label: "Decompose into parallel tasks",
        from: TaskState::Queued,
        to: TaskState::Queued,
        action: "decompose_task",
        gate: no_children,
    },
    Transition {
        id: "decompose",
        label: "Decompose into parallel tasks",
        from: TaskState::Working,
        to: TaskState::Working,
        action: "decompose_task",
        gate: no_children,
    },
    Transition {
        id: "resume",
        label: "Resolve & resume",
        from: TaskState::AwaitingHuman,
        to: TaskState::Working,
        action: "start_task",
        gate: avail,
    },
    Transition {
        id: "rework",
        label: "Rework",
        from: TaskState::Rejected,
        to: TaskState::Working,
        action: "start_task",
        gate: needs_assignee,
    },
];

/// The currently-available transitions from `state` for `task`.
pub fn transitions_for(
    state: TaskState,
    projection: &Projection,
    task: &Task,
) -> Vec<&'static Transition> {
    TABLE
        .iter()
        .filter(|t| t.from == state)
        .filter(|t| (t.gate)(projection, task).is_none())
        .collect()
}

/// A node in the graph — one task with its derived state + causal chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub task_id: String,
    pub title: String,
    pub kind: String,
    pub status: TaskStatus,
    pub state: TaskState,
    pub assignee: Option<String>,
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub awaiting_human: bool,
    /// State-derived causal steps ("why in this order").
    pub chain: Vec<String>,
    /// Currently-available transition ids from this node (owned).
    pub transitions: Vec<String>,
}

/// A parallel-work group: a parent task + its children. The parent is the
/// JOIN point — `resolved` iff every child is terminal (Done). The join rule
/// is a structural, deterministic aggregation, NOT a policy/LLM judgment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphGroup {
    pub parent_id: String,
    pub title: String,
    pub children: Vec<String>,
    pub done: Vec<String>,
    /// Children still not done (blocking the join).
    pub remaining: Vec<String>,
    pub resolved: bool,
}

/// The derived graph view of the whole project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub groups: Vec<GraphGroup>,
    /// Tokens currently moving (Working / InReview).
    pub active: Vec<String>,
    /// Tokens waiting on the human (AwaitingHuman).
    pub blocked: Vec<String>,
    pub done: usize,
    pub total: usize,
}

/// The narrow PM planning context for ONE task — the "which transition and
/// why?" seam that replaces "here's the whole snapshot" once D2 lands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PmTaskContext {
    pub task_id: String,
    pub title: String,
    pub state: TaskState,
    pub assignee: Option<String>,
    pub report: String,
    pub valid_transitions: Vec<TransitionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TransitionInfo {
    pub id: String,
    pub label: String,
    pub to: TaskState,
}

impl Projection {
    /// Derive the semantic state of a task from its lifecycle facts.
    pub fn task_state(&self, task: &Task) -> TaskState {
        match task.status {
            TaskStatus::Done => TaskState::Done,
            TaskStatus::Backlog => TaskState::Queued,
            TaskStatus::Working => TaskState::Working,
            // A blocked task is the graph's pause node — awaiting the human.
            TaskStatus::Blocked => TaskState::AwaitingHuman,
            TaskStatus::InReview => match &task.review {
                Some(r) if !r.approved => TaskState::Rejected,
                _ => TaskState::InReview,
            },
        }
    }

    /// The ids of all tasks whose parent is `parent_id` (derived; not stored).
    pub fn children_of(&self, parent_id: &str) -> Vec<String> {
        self.tasks
            .iter()
            .filter(|t| t.parent_id.as_deref() == Some(parent_id))
            .map(|t| t.id.clone())
            .collect()
    }

    /// Whether a task is waiting on the human.
    pub fn awaiting_human(&self, task: &Task) -> bool {
        self.task_state(task) == TaskState::AwaitingHuman
    }

    /// State-derived causal chain for a task ("why in this order").
    pub fn task_chain(&self, task: &Task) -> Vec<String> {
        let mut chain = vec!["created".to_string()];
        if let Some(a) = &task.assignee {
            chain.push(format!("assigned to {a}"));
        }
        let started = matches!(
            task.status,
            TaskStatus::Working | TaskStatus::InReview | TaskStatus::Done
        );
        if started {
            chain.push("started".into());
        }
        if matches!(task.status, TaskStatus::InReview | TaskStatus::Blocked)
            || task.review.is_some()
        {
            chain.push("submitted for review".into());
        }
        if let Some(r) = &task.review {
            chain.push(if r.approved {
                format!("approved by {}", r.reviewer)
            } else {
                format!("changes requested by {}", r.reviewer)
            });
        }
        if task.status == TaskStatus::Blocked {
            chain.push("blocked".into());
        }
        if task.status == TaskStatus::Done {
            chain.push("completed".into());
        }
        chain
    }

    /// Build the full derived graph view.
    pub fn graph(&self) -> GraphView {
        let mut nodes = Vec::with_capacity(self.tasks.len());
        let mut active = Vec::new();
        let mut blocked = Vec::new();
        let mut done = 0usize;

        for task in &self.tasks {
            let state = self.task_state(task);
            match state {
                TaskState::Working | TaskState::InReview => active.push(task.id.clone()),
                TaskState::AwaitingHuman => blocked.push(task.id.clone()),
                TaskState::Done => done += 1,
                _ => {}
            }
            nodes.push(GraphNode {
                task_id: task.id.clone(),
                title: task.title.clone(),
                kind: task.kind.clone(),
                status: task.status,
                state,
                assignee: task.assignee.clone(),
                parent_id: task.parent_id.clone(),
                children: self.children_of(&task.id),
                awaiting_human: state == TaskState::AwaitingHuman,
                chain: self.task_chain(task),
                transitions: transitions_for(state, self, task)
                    .iter()
                    .map(|t| t.id.to_string())
                    .collect(),
            });
        }

        // Groups = tasks that have children (join points).
        let groups = self
            .tasks
            .iter()
            .filter(|t| !self.children_of(&t.id).is_empty())
            .map(|t| {
                let children = self.children_of(&t.id);
                let done_children: Vec<String> = children
                    .iter()
                    .filter(|id| {
                        self.tasks
                            .iter()
                            .any(|c| &c.id == *id && c.status == TaskStatus::Done)
                    })
                    .cloned()
                    .collect();
                let remaining: Vec<String> = children
                    .iter()
                    .filter(|id| !done_children.contains(id))
                    .cloned()
                    .collect();
                GraphGroup {
                    parent_id: t.id.clone(),
                    title: t.title.clone(),
                    children,
                    done: done_children,
                    remaining: remaining.clone(),
                    resolved: remaining.is_empty(),
                }
            })
            .collect();

        let total = self.tasks.len();
        GraphView {
            nodes,
            groups,
            active,
            blocked,
            done,
            total,
        }
    }

    /// The narrow planning context for one task — the D2 prompt seam.
    pub fn pm_task_context(&self, task_id: &str) -> Option<PmTaskContext> {
        let task = self.tasks.iter().find(|t| t.id == task_id)?;
        let state = self.task_state(task);
        let children = self.children_of(task_id);
        let done_children = children
            .iter()
            .filter(|id| {
                self.tasks
                    .iter()
                    .any(|c| &c.id == *id && c.status == TaskStatus::Done)
            })
            .count();
        let report = if children.is_empty() {
            task.review
                .as_ref()
                .map(|r| {
                    if r.approved {
                        format!("reviewed & approved by {}", r.reviewer)
                    } else {
                        format!("changes requested by {}", r.reviewer)
                    }
                })
                .unwrap_or_else(|| match task.assignee {
                    Some(_) => "work in progress".to_string(),
                    None => "not yet assigned".to_string(),
                })
        } else {
            format!("{done_children}/{} parallel subtasks done", children.len())
        };
        let valid_transitions = transitions_for(state, self, task)
            .iter()
            .map(|t| TransitionInfo {
                id: t.id.to_string(),
                label: t.label.to_string(),
                to: t.to,
            })
            .collect();
        Some(PmTaskContext {
            task_id: task.id.clone(),
            title: task.title.clone(),
            state,
            assignee: task.assignee.clone(),
            report,
            valid_transitions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::TaskReview;

    fn proj_with(tasks: Vec<Task>) -> Projection {
        Projection {
            project_id: "p".into(),
            tasks,
            ..Default::default()
        }
    }

    fn task(id: &str, status: TaskStatus) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            kind: "feature".into(),
            status,
            assignee: None,
            priority: crate::plan::Priority::default(),
            review: None,
            parent_id: None,
        }
    }

    #[test]
    fn task_state_maps_lifecycle() {
        let p = proj_with(vec![]);
        assert_eq!(
            p.task_state(&task("a", TaskStatus::Backlog)),
            TaskState::Queued
        );
        assert_eq!(
            p.task_state(&task("a", TaskStatus::Working)),
            TaskState::Working
        );
        assert_eq!(
            p.task_state(&task("a", TaskStatus::Blocked)),
            TaskState::AwaitingHuman
        );
        assert_eq!(p.task_state(&task("a", TaskStatus::Done)), TaskState::Done);
        // Rejected review -> Rejected.
        let mut t = task("a", TaskStatus::InReview);
        t.review = Some(TaskReview {
            reviewer: "maya-patel".into(),
            note: "nits".into(),
            approved: false,
        });
        assert_eq!(p.task_state(&t), TaskState::Rejected);
    }

    #[test]
    fn children_are_derived_from_parent_id() {
        let mut child = task("c1", TaskStatus::Working);
        child.parent_id = Some("parent".into());
        let p = proj_with(vec![task("parent", TaskStatus::Backlog), child]);
        assert_eq!(p.children_of("parent"), vec!["c1".to_string()]);
    }

    #[test]
    fn group_join_resolves_when_children_done() {
        let mut c1 = task("c1", TaskStatus::Done);
        c1.parent_id = Some("parent".into());
        let mut c2 = task("c2", TaskStatus::Working);
        c2.parent_id = Some("parent".into());
        let p = proj_with(vec![task("parent", TaskStatus::Backlog), c1, c2]);
        let g = p.graph();
        assert_eq!(g.groups.len(), 1);
        let group = &g.groups[0];
        assert!(!group.resolved, "one child still working -> not resolved");
        assert_eq!(group.done, vec!["c1".to_string()]);
        assert_eq!(group.remaining, vec!["c2".to_string()]);
        assert_eq!(group.remaining[0], "c2");
    }

    #[test]
    fn block_transition_avail_via_gate() {
        let mut w = task("t", TaskStatus::Working);
        w.assignee = Some("marcus-reed".into());
        let p = proj_with(vec![w]);
        let avail = transitions_for(TaskState::Working, &p, &p.tasks[0]);
        let ids: Vec<_> = avail.iter().map(|t| t.id).collect();
        assert!(ids.contains(&"submit"));
    }

    #[test]
    fn pm_context_narrows_to_valid_exits() {
        let mut w = task("t", TaskStatus::InReview);
        w.assignee = Some("marcus-reed".into());
        let p = proj_with(vec![w]);
        let ctx = p.pm_task_context("t").unwrap();
        assert_eq!(ctx.state, TaskState::InReview);
        let ids: Vec<_> = ctx
            .valid_transitions
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert!(ids.contains(&"approve"));
        assert!(ids.contains(&"reject"));
    }

    #[test]
    fn decompose_shows_for_undivided_task_only() {
        let mut child = task("c", TaskStatus::Backlog);
        child.parent_id = Some("parent".into());
        let p = proj_with(vec![task("parent", TaskStatus::Backlog), child]);
        let ids: Vec<_> = transitions_for(TaskState::Queued, &p, &p.tasks[0])
            .iter()
            .map(|t| t.id)
            .collect();
        assert!(!ids.contains(&"decompose"), "parent already has children");
    }

    #[test]
    fn decompose_folds_children_with_parent_link_end_to_end() {
        use crate::actions::PmAction;
        use crate::event::{Actor, Aggregate, Event, EventType};

        let cause = Event::new(
            "p".to_string(),
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            serde_json::json!({ "body": "ship auth" }),
        );
        let mut p = Projection::default();
        // Parent task exists first.
        let parent_ev = PmAction::CreateTask {
            id: "auth".into(),
            title: "Auth".into(),
            kind: "feature".into(),
        }
        .to_events("p", "pm", &cause, "corr");
        for e in &parent_ev {
            p.apply(e);
        }
        // Fan out two parallel subtasks.
        let evs = PmAction::DecomposeTask {
            parent: "auth".into(),
            children: vec![
                crate::actions::TaskSpec {
                    id: "oauth".into(),
                    title: "OAuth".into(),
                    kind: "feature".into(),
                },
                crate::actions::TaskSpec {
                    id: "ui".into(),
                    title: "Login UI".into(),
                    kind: "feature".into(),
                },
            ],
        }
        .to_events("p", "pm", &cause, "corr");
        assert_eq!(evs.len(), 3, "TaskDecomposed + 2 TaskCreated");
        assert_eq!(evs[0].event_type, EventType::TaskDecomposed);
        assert!(evs[1..]
            .iter()
            .all(|e| e.event_type == EventType::TaskCreated));
        for e in &evs {
            p.apply(e);
        }
        // parent_id links are folded and the graph sees a group at the join.
        assert_eq!(p.children_of("auth").len(), 2);
        assert!(p
            .tasks
            .iter()
            .all(|t| t.id == "auth" || t.parent_id.as_deref() == Some("auth")));
        let g = p.graph();
        assert_eq!(g.groups.len(), 1);
        assert!(!g.groups[0].resolved, "both children still queued");
        assert_eq!(g.total, 3);
        assert_eq!(g.nodes.len(), 3);
    }
}
