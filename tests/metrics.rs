//! Tests for the two week-1 metrics: director engagement (response rate / muting
//! signal) and code diff quality (language-agnostic git churn). Both are pure
//! derivations off the projection exposed through `/api/model`, so they're
//! tested by folding a hand-built projection through `operating_model()`.

use casting::pm::policy::{DecisionClass, OwnerInvolvement};
use casting::projection::{Commit, Decision, DecisionStatus, Projection};
use casting::runtime::mental::OperatingModel;

fn owner_decision(id: &str, involvement: OwnerInvolvement, decided_by: Option<&str>) -> Decision {
    Decision {
        id: id.to_string(),
        subject: format!("subject-{id}"),
        options: serde_json::json!({}),
        recommendation: None,
        status: if decided_by.is_some() {
            DecisionStatus::Approved
        } else {
            DecisionStatus::Proposed
        },
        class: DecisionClass::Architecture,
        involvement,
        decided_by: decided_by.map(|s| s.to_string()),
        superseded_by: None,
        owner_verdict: None,
    }
}

#[test]
fn owner_engagement_counts_backlog_and_rate() {
    let proj = Projection {
        // One escalation the director hasn't answered -> awaiting.
        // One the director ruled on -> owner_decided.
        // One the PM handled itself -> delegated.
        decisions: vec![
            owner_decision("a", OwnerInvolvement::Ask, None),
            owner_decision("b", OwnerInvolvement::Ask, Some("director")),
            owner_decision("c", OwnerInvolvement::Pm, Some("mei")),
            // A Pm-tier decision still proposed by the PM is NOT an escalation.
            owner_decision("d", OwnerInvolvement::Pm, None),
        ],
        ..Default::default()
    };
    let m: OperatingModel = proj.operating_model();
    assert_eq!(m.engagement.awaiting_owner, 1);
    assert_eq!(m.engagement.owner_decided, 1);
    assert_eq!(m.engagement.delegated_decided, 1);
    // director(1) / (director(1) + awaiting(1)) = 0.5
    assert!((m.engagement.response_rate - 0.5).abs() < 1e-9);
}

#[test]
fn owner_engagement_is_caught_up_when_nothing_awaits() {
    let proj = Projection {
        decisions: vec![owner_decision("a", OwnerInvolvement::Ask, Some("director"))],
        ..Default::default()
    };
    let m: OperatingModel = proj.operating_model();
    assert_eq!(m.engagement.awaiting_owner, 0);
    assert_eq!(m.engagement.response_rate, 1.0);
}

fn commit(sha: &str, add: u64, del: u64, files: u64) -> Commit {
    Commit {
        sha: sha.to_string(),
        branch: "main".to_string(),
        message: format!("commit {sha}"),
        author: "agent".to_string(),
        task_id: Some("task-1".to_string()),
        additions: add,
        deletions: del,
        files,
    }
}

#[test]
fn diff_quality_aggregates_churn_and_flags_large_rewrites() {
    let proj = Projection {
        // One normal commit + one big rewrite (> eigenvalue threshold 500).
        commits: vec![commit("aaa", 10, 2, 1), commit("bbb", 600, 10, 3)],
        ..Default::default()
    };
    let m: OperatingModel = proj.operating_model();
    let dq = &m.diff_quality;
    assert_eq!(dq.commit_count, 2);
    assert_eq!(dq.total_additions, 610);
    assert_eq!(dq.total_deletions, 12);
    assert_eq!(dq.total_files, 4);
    // churn = (10+2 + 600+10) / 2 = 311
    assert!((dq.avg_churn_per_commit - 311.0).abs() < 1e-9);
    assert_eq!(dq.large_rewrites, 1);
    assert_eq!(dq.large_rewrite_threshold, 500);
    assert_eq!(dq.recent.len(), 2);
}

#[test]
fn metrics_are_zeroed_on_empty_project() {
    let m: OperatingModel = Projection::default().operating_model();
    assert_eq!(m.engagement.awaiting_owner, 0);
    assert_eq!(m.engagement.response_rate, 1.0);
    assert_eq!(m.diff_quality.commit_count, 0);
    assert_eq!(m.diff_quality.avg_churn_per_commit, 0.0);
    assert_eq!(m.diff_quality.large_rewrites, 0);
    assert!(m.diff_quality.recent.is_empty());
}
