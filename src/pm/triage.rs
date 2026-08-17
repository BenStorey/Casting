//! Deterministic triage for external requests (the product's intake surface).
//!
//! This is the SINGLE source of truth for how an external request (a GitHub
//! issue/PR, an email, a web submission) is classified and severitied. It is
//! called from BOTH:
//!   - `Projection::triage_request` (the read-side verdict incl. duplicate
//!     detection), and
//!   - the `PmAction::ReceiveExternalRequest` event mapper (so the recorded
//!     event carries the same verdict).
//!
//! Keeping it here means the two can never drift out of sync (director refactor
//! 2026-08-10; prior review flagged the inlined copy as a latent bug).

/// Classify an external request: returns (classification, severity).
///
/// classification: "security" | "bug" | "feature" (security wins, then bug, else
/// feature — an unlabeled/ambiguous request is treated as a feature request).
/// severity: "high" | "medium" | "low" (security/crash/data-loss => high; a bug
/// => medium; else low).
pub fn classify(title: &str, body: &str, labels: &[String]) -> (String, String) {
    let haystack = format!("{} {}", title, body).to_lowercase();

    let classification = if labels
        .iter()
        .any(|l| l.to_lowercase().contains("security") || l.to_lowercase().contains("vuln"))
    {
        "security"
    } else if labels
        .iter()
        .any(|l| l.to_lowercase().contains("feature") || l.to_lowercase().contains("enhancement"))
    {
        "feature"
    } else if labels.iter().any(|l| l.to_lowercase().contains("bug"))
        || [
            "crash", "broken", "fail", "error", "can't", "cannot", "bug", "wrong",
        ]
        .iter()
        .any(|w| haystack.contains(w))
    {
        "bug"
    } else {
        "feature"
    };

    let severity = if classification == "security"
        || haystack.contains("crash")
        || haystack.contains("data loss")
    {
        "high"
    } else if classification == "bug" {
        "medium"
    } else {
        "low"
    };

    (classification.to_string(), severity.to_string())
}
