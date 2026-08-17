//! Provenance — the "why does this code exist?" graph (ADDENDUM §24–25).
//!
//! Starting from a commit, Casting can answer:
//!
//! ```text
//! commit → changeSet → task → requirement → decision → owner intent
//! ```
//!
//! Or starting from a decision:
//!
//! ```text
//! decision → tasks → changeSets → commits → code
//! ```
//!
//! The provenance chain is built from two sources already in the event log:
//!   1. **Event data fields** — `task_id` on CommitObserved/BranchCreated links
//!      a commit to a task.
//!   2. **Event metadata** — `correlation_id` groups events from the same PM
//!      run (so a task, its requirement, and its decision share a correlation
//!      id), and `causation_id` links an event to the event that directly
//!      caused it (typically the director's message).
//!
//! This module provides pure query functions over the event log — no new
//! events, no projection changes. It's the read-side of the provenance graph.

use crate::event::EventType;
use crate::store::EventStore;
use anyhow::Result;
use serde::Serialize;

/// One link in the provenance chain — an event and what it represents.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceLink {
    /// The event sequence (for ordering).
    pub sequence: i64,
    /// The event type as a human-readable string.
    pub event_type: String,
    /// The aggregate id (task id, requirement id, decision id, etc.).
    pub entity_id: String,
    /// The aggregate kind (task, requirement, decision, message, etc.).
    pub entity_kind: String,
    /// A human-readable description of what this event represents.
    pub description: String,
}

/// The full provenance chain from a commit back to owner intent.
#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceChain {
    /// The commit sha we started from.
    pub commit: String,
    /// The ChangeSet that contains this commit, if any.
    pub changeset_id: Option<String>,
    /// The task this work fulfills.
    pub task_id: Option<String>,
    /// The requirement that motivated the task.
    pub requirement_id: Option<String>,
    /// The decision associated with the task, if any.
    pub decision_id: Option<String>,
    /// the director message that initiated the chain.
    pub owner_message: Option<String>,
    /// The ordered chain of events from commit back to owner intent.
    pub chain: Vec<ProvenanceLink>,
}

/// Build the provenance chain for a commit sha: walk from the commit back to
/// the director's original message/requirement.
///
/// The chain is built by:
/// 1. Find the CommitObserved event for this sha → get task_id from its data.
/// 2. Find the TaskCreated event for that task → get correlation_id.
/// 3. Find RequirementCreated events with the same correlation_id.
/// 4. Find DecisionProposed events with the same correlation_id.
/// 5. Follow causation_id from TaskCreated to the director's MessageSent.
pub fn for_commit<S: EventStore>(store: &S, project: &str, sha: &str) -> Result<ProvenanceChain> {
    let events = store.read_since(project, 0)?;

    // 1. Find the CommitObserved event for this sha.
    let commit_ev = events
        .iter()
        .find(|e| e.event_type == EventType::CommitObserved && e.aggregate.id == sha);

    let Some(commit_ev) = commit_ev else {
        return Ok(ProvenanceChain {
            commit: sha.to_string(),
            changeset_id: None,
            task_id: None,
            requirement_id: None,
            decision_id: None,
            owner_message: None,
            chain: Vec::new(),
        });
    };

    let task_id = commit_ev
        .data
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut chain = vec![ProvenanceLink {
        sequence: commit_ev.sequence,
        event_type: "CommitObserved".into(),
        entity_id: sha.to_string(),
        entity_kind: "commit".into(),
        description: format!(
            "Commit {} on branch {}",
            &sha[..sha.len().min(8)],
            commit_ev
                .data
                .get("branch")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
        ),
    }];

    // Derive the changeset id from the task id (same convention as the projection).
    let changeset_id = task_id.as_ref().map(|tid| format!("changeset-{tid}"));

    if let Some(tid) = &task_id {
        // 2. Find the TaskCreated event for this task.
        let task_ev = events
            .iter()
            .find(|e| e.event_type == EventType::TaskCreated && e.aggregate.id == *tid);

        if let Some(task_ev) = task_ev {
            chain.push(ProvenanceLink {
                sequence: task_ev.sequence,
                event_type: "TaskCreated".into(),
                entity_id: tid.clone(),
                entity_kind: "task".into(),
                description: format!(
                    "Task {}: {}",
                    tid,
                    task_ev
                        .data
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ),
            });

            let correlation_id = task_ev.metadata.correlation_id.clone();

            // 3. Find RequirementCreated events with the same correlation_id.
            if let Some(corr) = &correlation_id {
                for e in &events {
                    if e.event_type == EventType::RequirementCreated
                        && e.metadata.correlation_id.as_deref() == Some(corr)
                    {
                        chain.push(ProvenanceLink {
                            sequence: e.sequence,
                            event_type: "RequirementCreated".into(),
                            entity_id: e.aggregate.id.clone(),
                            entity_kind: "requirement".into(),
                            description: format!(
                                "Requirement: {}",
                                e.data.get("title").and_then(|v| v.as_str()).unwrap_or("?")
                            ),
                        });
                    }
                }

                // 4. Find DecisionProposed events with the same correlation_id.
                for e in &events {
                    if e.event_type == EventType::DecisionProposed
                        && e.metadata.correlation_id.as_deref() == Some(corr)
                    {
                        chain.push(ProvenanceLink {
                            sequence: e.sequence,
                            event_type: "DecisionProposed".into(),
                            entity_id: e.aggregate.id.clone(),
                            entity_kind: "decision".into(),
                            description: format!(
                                "Decision: {}",
                                e.data
                                    .get("subject")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                            ),
                        });
                    }
                }
            }

            // 5. Follow causation_id to the director's message.
            if let Some(cause_id) = task_ev.metadata.causation_id {
                if let Some(cause_ev) = events.iter().find(|e| e.event_id == cause_id) {
                    if cause_ev.event_type == EventType::MessageSent {
                        chain.push(ProvenanceLink {
                            sequence: cause_ev.sequence,
                            event_type: "MessageSent".into(),
                            entity_id: cause_ev.aggregate.id.clone(),
                            entity_kind: "message".into(),
                            description: format!(
                                "Owner said: \"{}\"",
                                cause_ev
                                    .data
                                    .get("body")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("?")
                            ),
                        });
                    }
                }
            }
        }
    }

    // Extract the IDs for the summary fields.
    let requirement_id = chain
        .iter()
        .find(|l| l.entity_kind == "requirement")
        .map(|l| l.entity_id.clone());
    let decision_id = chain
        .iter()
        .find(|l| l.entity_kind == "decision")
        .map(|l| l.entity_id.clone());
    let owner_message = chain
        .iter()
        .find(|l| l.entity_kind == "message")
        .map(|l| l.description.clone());

    Ok(ProvenanceChain {
        commit: sha.to_string(),
        changeset_id,
        task_id,
        requirement_id,
        decision_id,
        owner_message,
        chain,
    })
}

/// Build the provenance chain starting from a task id (the reverse direction:
/// what work did this task produce?). Returns the commits and changesets
/// linked to this task.
#[derive(Debug, Clone, Serialize)]
pub struct TaskProvenance {
    pub task_id: String,
    pub changeset_id: Option<String>,
    pub branch: Option<String>,
    pub commits: Vec<String>,
    pub requirement_id: Option<String>,
    pub decision_id: Option<String>,
    pub owner_message: Option<String>,
}

/// Build the provenance for a task: what code, what requirement, what decision
/// produced it.
pub fn for_task<S: EventStore>(store: &S, project: &str, task_id: &str) -> Result<TaskProvenance> {
    let events = store.read_since(project, 0)?;

    // Find the TaskCreated event to get the correlation_id.
    let task_ev = events
        .iter()
        .find(|e| e.event_type == EventType::TaskCreated && e.aggregate.id == task_id);

    let mut requirement_id = None;
    let mut decision_id = None;
    let mut owner_message = None;

    if let Some(task_ev) = task_ev {
        let correlation_id = task_ev.metadata.correlation_id.as_deref();

        // Find RequirementCreated with the same correlation_id.
        if let Some(corr) = correlation_id {
            requirement_id = events
                .iter()
                .find(|e| {
                    e.event_type == EventType::RequirementCreated
                        && e.metadata.correlation_id.as_deref() == Some(corr)
                })
                .map(|e| e.aggregate.id.clone());

            decision_id = events
                .iter()
                .find(|e| {
                    e.event_type == EventType::DecisionProposed
                        && e.metadata.correlation_id.as_deref() == Some(corr)
                })
                .map(|e| e.aggregate.id.clone());
        }

        // Follow causation_id to owner message.
        if let Some(cause_id) = task_ev.metadata.causation_id {
            if let Some(cause_ev) = events.iter().find(|e| e.event_id == cause_id) {
                if cause_ev.event_type == EventType::MessageSent {
                    owner_message = cause_ev
                        .data
                        .get("body")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                }
            }
        }
    }

    // Find all CommitObserved events for this task.
    let commits: Vec<String> = events
        .iter()
        .filter(|e| {
            e.event_type == EventType::CommitObserved
                && e.data.get("task_id").and_then(|v| v.as_str()) == Some(task_id)
        })
        .map(|e| e.aggregate.id.clone())
        .collect();

    // Find the branch from the first commit or a BranchCreated event.
    let branch = events
        .iter()
        .find(|e| {
            e.event_type == EventType::BranchCreated
                && e.data.get("task_id").and_then(|v| v.as_str()) == Some(task_id)
        })
        .map(|e| e.aggregate.id.clone())
        .or_else(|| {
            events
                .iter()
                .find(|e| {
                    e.event_type == EventType::CommitObserved
                        && e.data.get("task_id").and_then(|v| v.as_str()) == Some(task_id)
                })
                .and_then(|e| {
                    e.data
                        .get("branch")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
        });

    Ok(TaskProvenance {
        task_id: task_id.to_string(),
        changeset_id: Some(format!("changeset-{task_id}")),
        branch,
        commits,
        requirement_id,
        decision_id,
        owner_message,
    })
}

/// The audit for a single decision: who proposed it, what class/involvement,
/// who decided it, the director's note, and why (back to the initiator message).
#[derive(Debug, Clone, Serialize)]
pub struct DecisionAudit {
    pub decision_id: String,
    pub subject: String,
    pub class: crate::pm::DecisionClass,
    pub involvement: crate::pm::OwnerInvolvement,
    pub status: String,
    pub proposed_by: String,
    /// The decider once `DecisionMade` is recorded (owner or agent id).
    pub decided_by: Option<String>,
    /// The note attached when the decision was made (often the director's verdict).
    pub note: Option<String>,
    /// the director's original message that motivated this decision, if traceable.
    pub owner_message: Option<String>,
    /// The ordered chain of link events: proposal → decision → owner message.
    pub chain: Vec<ProvenanceLink>,
}

/// Build the audit for a decision: walk the log for its proposal and any
/// resolution, then trace back to the initiating owner message.
pub fn for_decision<S: EventStore>(
    store: &S,
    project: &str,
    decision_id: &str,
) -> Result<DecisionAudit> {
    let events = store.read_since(project, 0)?;

    let Some(proposed) = events
        .iter()
        .find(|e| e.event_type == EventType::DecisionProposed && e.aggregate.id == decision_id)
    else {
        return Ok(DecisionAudit {
            decision_id: decision_id.to_string(),
            subject: String::new(),
            class: crate::pm::DecisionClass::InternalImplementation,
            involvement: crate::pm::OwnerInvolvement::Ask,
            status: "unknown".into(),
            proposed_by: String::new(),
            decided_by: None,
            note: None,
            owner_message: None,
            chain: Vec::new(),
        });
    };

    let subject = proposed
        .data
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();
    let class = proposed
        .data
        .get("class")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(crate::pm::DecisionClass::InternalImplementation);
    let involvement = proposed
        .data
        .get("involvement")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(crate::pm::OwnerInvolvement::Ask);
    let proposed_by = actor_label(&proposed.actor);

    // The resolution, if any.
    let made = events
        .iter()
        .find(|e| e.event_type == EventType::DecisionMade && e.aggregate.id == decision_id);
    let decided_by = made.map(|e| actor_label(&e.actor));
    let note = made.and_then(|e| {
        e.data
            .get("note")
            .and_then(|v| v.as_str())
            .map(String::from)
    });
    let status = match made {
        None => "proposed".to_string(),
        Some(e)
            if e.data
                .get("approved")
                .and_then(|v| v.as_bool())
                .unwrap_or(false) =>
        {
            "approved".to_string()
        }
        Some(_) => "rejected".to_string(),
    };

    let mut chain = vec![ProvenanceLink {
        sequence: proposed.sequence,
        event_type: "DecisionProposed".into(),
        entity_id: decision_id.to_string(),
        entity_kind: "decision".into(),
        description: format!("Decision proposed: {subject}"),
    }];
    if let Some(m) = made {
        chain.push(ProvenanceLink {
            sequence: m.sequence,
            event_type: "DecisionMade".into(),
            entity_id: decision_id.to_string(),
            entity_kind: "decision".into(),
            description: format!("Decision made by {:?}", decided_by),
        });
    }

    // Trace to the initiating owner message via the proposal's causation chain.
    let mut owner_message = None;
    let mut cause_id = proposed.metadata.causation_id;
    let mut guard = 0;
    while let Some(cid) = cause_id {
        guard += 1;
        if guard > 32 {
            break;
        }
        let Some(cause) = events.iter().find(|e| e.event_id == cid) else {
            break;
        };
        if cause.event_type == EventType::MessageSent
            && cause.actor
                == (crate::event::Actor::Director {
                    user_id: "ceo".into(),
                })
        {
            owner_message = cause
                .data
                .get("body")
                .and_then(|v| v.as_str())
                .map(String::from);
            chain.push(ProvenanceLink {
                sequence: cause.sequence,
                event_type: "MessageSent".into(),
                entity_id: cause.aggregate.id.clone(),
                entity_kind: "message".into(),
                description: format!(
                    "Owner said: \"{}\"",
                    cause
                        .data
                        .get("body")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ),
            });
            break;
        }
        cause_id = cause.metadata.causation_id;
    }

    Ok(DecisionAudit {
        decision_id: decision_id.to_string(),
        subject,
        class,
        involvement,
        status,
        proposed_by,
        decided_by,
        note,
        owner_message,
        chain,
    })
}

fn actor_label(a: &crate::event::Actor) -> String {
    match a {
        crate::event::Actor::Director { .. } => "owner".into(),
        crate::event::Actor::Agent { id } => id.clone(),
        crate::event::Actor::System => "system".into(),
    }
}
