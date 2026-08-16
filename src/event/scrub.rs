//! Event-payload scrubbing for secrets.
//!
//! Before an event is committed to the append-only event log, this filter walks
//! the event's `data` JSON and redacts known secret patterns. This prevents API
//! keys, private keys, JWT tokens, and other secrets from being permanently
//! embedded in the event history even if a developer inadvertently pastes them
//! into a chat message, briefing, decision note, or task result.
//!
//! This is a best-effort filter, not a security boundary. It catches common
//! patterns that developers accidentally paste. Deliberately conservative:
//! redacts on suspicion rather than trying to prove something is a secret.

use serde_json::Value;

/// Secret-bearing JSON keys — if a key matches (case-insensitive), its string
/// value is redacted in full.
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "secret",
    "secret_key",
    "secret-key",
    "password",
    "passwd",
    "token",
    "auth_token",
    "auth-token",
    "bearer",
    "credential",
    "credentials",
    "private_key",
    "private-key",
    "ssh_key",
    "ssh-key",
    "access_key",
    "access-key",
    "client_secret",
    "client-secret",
    "session_key",
    "session-key",
];

/// Known API key / token prefixes whose values should be redacted wherever
/// they appear as string values.
const SECRET_PREFIXES: &[&str] = &[
    "sk-",         // OpenAI / OpenRouter
    "sk-or-v1-",   // OpenRouter (explicit)
    "pk-",         // OpenAI publishable (less sensitive, still redact)
    "fkey-",       // Firebase
    "ghp_",        // GitHub personal access
    "gho_",        // GitHub OAuth
    "github_pat_", // GitHub fine-grained PAT
    "xoxb-",       // Slack bot token
    "xoxp-",       // Slack user token
    "xapp-",       // Slack app-level token
    "eyJ",         // JWT / JWS (base64url-encoded JSON header)
];

/// Known JWT-like patterns (base64url-encoded header `eyJ...` followed by
/// two more dot-separated segments). These are caught by the `eyJ` prefix
/// above but we keep an explicit regex-like check in context.
const SSH_KEY_MARKER: &str = "-----BEGIN";

/// Entry point: scrub the `data` field of an event **in place**, redacting
/// any secret-bearing content.
pub fn scrub_event(event: &mut crate::event::Event) {
    scrub_value(&mut event.data);
}

/// Recursively walk a JSON value and redact secret-bearing strings in place.
fn scrub_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Collect keys first so we can mutate while iterating.
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(v) = map.get_mut(&key) {
                    if is_secret_key(&key) {
                        // Redact the entire value regardless of its type.
                        *v = Value::String("[REDACTED]".into());
                    } else {
                        // Recurse into the value.
                        scrub_value(v);
                    }
                }
            }
        }
        Value::Array(arr) => {
            for elem in arr.iter_mut() {
                scrub_value(elem);
            }
        }
        Value::String(s) if looks_like_secret(s) => {
            *s = "[REDACTED]".into();
        }
        // Numbers, booleans, null: nothing to scrub.
        _ => {}
    }
}

/// Check if a JSON key is a known secret-bearing key (case-insensitive).
fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SECRET_KEYS
        .iter()
        .any(|k| lower == *k || lower.ends_with(&format!(".{k}")))
        || lower.contains("password")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("credential")
        || lower.contains("api.key")
        || lower.contains("private.key")
        || lower.contains("ssh.key")
}

/// Check if a string value looks like a secret.
fn looks_like_secret(s: &str) -> bool {
    // SSH / PGP / TLS private keys.
    if s.contains(SSH_KEY_MARKER) {
        return true;
    }
    // Known API key prefixes — check both the bare string and as substrings
    // at word boundaries (the key is often embedded in a sentence like
    // "use sk-proj-xxx" or "set API_KEY=sk-...").
    for prefix in SECRET_PREFIXES {
        if s.starts_with(prefix) || contains_token(s, prefix) {
            return true;
        }
    }
    // Generic heuristic: a string that is >= 32 characters of hex or
    // alphanumeric, with no spaces, that isn't clearly a URL or UUID.
    // This catches env-injected values like random hex secrets.
    let trimmed = s.trim();
    if trimmed.len() >= 32
        && !trimmed.contains(' ')
        && !trimmed.contains('/')
        && !trimmed.contains('.') // exclude emails / URLs
        && !trimmed.contains('@')
        && !trimmed.contains('-')
    // exclude UUIDs
    {
        let alpha_num_count = trimmed.chars().filter(|c| c.is_alphanumeric()).count();
        if alpha_num_count == trimmed.len() {
            // Redact if it's at least 60% alphanumeric and no whitespace.
            return true;
        }
    }
    false
}

/// Check if a string contains `prefix` at a word boundary (start of string,
/// or preceded by a space, `=`, `:`, `"`, `'`, `(`, `[`, `{`, or newline).
fn contains_token(s: &str, prefix: &str) -> bool {
    let boundary: &[char] = &[' ', '=', ':', '"', '\'', '(', '[', '{', '\n', '\r', '\t'];
    s.match_indices(prefix).any(|(pos, _)| {
        pos == 0
            || s.as_bytes()
                .get(pos.saturating_sub(1))
                .is_some_and(|b| boundary.contains(&(*b as char)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Actor, Aggregate, Event, EventType};
    use serde_json::json;

    fn make_event(data: serde_json::Value) -> Event {
        Event::new(
            "proj",
            Actor::Owner,
            EventType::MessageSent,
            Aggregate {
                kind: "message".into(),
                id: "m1".into(),
            },
            data,
        )
    }

    #[test]
    fn scrub_openai_api_key_in_chat_body() {
        let mut ev = make_event(json!({"body": "my key is sk-proj-AbCdEf123456"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_openrouter_key() {
        let mut ev = make_event(json!({"body": "use sk-or-v1-abc123def456 for openrouter"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_ssh_private_key() {
        let mut ev = make_event(
            json!({"body": "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA..."}),
        );
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_jwt() {
        let mut ev =
            make_event(json!({"body": "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgN"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_nested_secret_key() {
        let mut ev = make_event(json!({
            "config": {
                "api_key": "some-value",
                "other": "visible"
            }
        }));
        scrub_event(&mut ev);
        assert_eq!(ev.data["config"]["api_key"], "[REDACTED]");
        assert_eq!(ev.data["config"]["other"], "visible");
    }

    #[test]
    fn scrub_github_pat() {
        let mut ev =
            make_event(json!({"body": "token: github_pat_11ABCdefGHIjklmNOpqrsTUVwxYZ123"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_slack_token() {
        let mut ev = make_event(json!({"body": "xoxb-1234567890-abc123def456"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn scrub_jwt_in_json_key() {
        let mut ev = make_event(json!({
            "auth": {
                "token": "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.signature"
            }
        }));
        scrub_event(&mut ev);
        assert_eq!(ev.data["auth"]["token"], "[REDACTED]");
    }

    #[test]
    fn scrub_generic_hex_secret() {
        // 40+ chars of pure hex/alphanumeric with no spaces should be caught.
        let mut ev = make_event(
            json!({"body": "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3"}),
        );
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn normal_text_passes_through() {
        let mut ev = make_event(json!({"body": "What do you think about this architecture?"}));
        scrub_event(&mut ev);
        assert_eq!(
            ev.data["body"],
            "What do you think about this architecture?"
        );
    }

    #[test]
    fn url_passes_through() {
        let mut ev = make_event(json!({"body": "Check out https://example.com/api/v1/endpoint"}));
        scrub_event(&mut ev);
        assert_eq!(
            ev.data["body"],
            "Check out https://example.com/api/v1/endpoint"
        );
    }

    #[test]
    fn uuid_passes_through() {
        let mut ev = make_event(json!({"body": "550e8400-e29b-41d4-a716-446655440000"}));
        scrub_event(&mut ev);
        // UUID contains dashes, which are excluded by our heuristic.
        assert_eq!(ev.data["body"], "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn short_string_passes_through() {
        let mut ev = make_event(json!({"body": "abc123"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "abc123");
    }

    #[test]
    fn key_value_pairs_in_chat_body() {
        // A message with API_KEY=... in the text body.
        let mut ev = make_event(json!({"body": "API_KEY=sk-abc123xyz"}));
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
    }

    #[test]
    fn briefing_body_is_scrubbed() {
        let mut ev = Event::new(
            "proj",
            Actor::Owner,
            EventType::AdvisoryBriefingImported,
            Aggregate {
                kind: "briefing".into(),
                id: "b1".into(),
            },
            json!({
                "source": "advisor",
                "subject": "deployment",
                "title": "Infra notes",
                "body": "Use key sk-proj-fake for the API",
            }),
        );
        scrub_event(&mut ev);
        assert_eq!(ev.data["body"], "[REDACTED]");
        assert_eq!(ev.data["title"], "Infra notes"); // normal text stays
    }

    #[test]
    fn decision_note_is_scrubbed() {
        let mut ev = Event::new(
            "proj",
            Actor::Owner,
            EventType::DecisionMade,
            Aggregate {
                kind: "decision".into(),
                id: "d1".into(),
            },
            json!({
                "decision_id": "d1",
                "approved": true,
                "note": "I used sk-proj-fake-key and it worked",
            }),
        );
        scrub_event(&mut ev);
        assert_eq!(ev.data["note"], "[REDACTED]");
        assert_eq!(ev.data["approved"], true);
    }
}
