//! Out-of-band prompt/response archival (2026-08-19).
//!
//! Every LLM call's fully-assembled prompt and raw response are written to
//! disk under the per-project state dir (`~/.casting/<slug>/prompts/`), NOT
//! into the artifact repo the agents work in (the [`crate::workspace::Workspace`]
//! boundary guarantees Casting's own state never lives inside `repo`).
//!
//! Why out-of-band instead of inline in the event log: a prompt can be tens of
//! KB of serialized `AgentContext`, so writing it into every `OrchestrationRun`
//! event would bloat the append-only log. Instead the event carries a small
//! `prompt_ref` / `response_ref` (a filename) and the blobs live here. The log
//! stays self-sufficient (it keeps `context_summary` + full metering + parsed
//! actions inline); the blobs are best-effort enrichment for bit-exact audit
//! and replay, and may dangle if the state dir is ever pruned.
//!
//! Archival is always best-effort: a disk write failure is logged and yields
//! `None` refs — it never fails or blocks an LLM call.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The on-disk archive for one project's LLM prompts/responses.
#[derive(Debug, Clone)]
pub struct PromptArchive {
    /// The `prompts/` directory under the project's state dir.
    pub dir: PathBuf,
}

impl PromptArchive {
    /// Open the archive for a project, rooted at its state dir
    /// (`~/.casting/<slug>/`). The archive lives at `<state_dir>/prompts/`.
    pub fn open(state_dir: &Path) -> Self {
        PromptArchive {
            dir: state_dir.join("prompts"),
        }
    }

    /// Persist one LLM call's assembled prompt + raw response. Returns the
    /// refs to embed in the audit event (`<correlation>.prompt.txt`,
    /// `<correlation>.response.txt`, relative to the archive dir), or `None`
    /// for each part that couldn't be written. `response` is optional (some
    /// calls have no text reply to archive). Never fails the caller — a write
    /// error is logged and surfaced as `None`.
    pub fn persist(
        &self,
        correlation: &str,
        prompt: &str,
        response: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let base = sanitize(correlation);
        let created = match std::fs::create_dir_all(&self.dir) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("[prompt_archive] cannot create {}: {e}", self.dir.display());
                false
            }
        };
        if !created {
            return (None, None);
        }

        let prompt_ref = write(
            &self.dir.join(format!("{base}.prompt.txt")),
            prompt.as_bytes(),
        )
        .ok()
        .map(|_| format!("{base}.prompt.txt"));

        let response_ref = match response {
            Some(r) => write(&self.dir.join(format!("{base}.response.txt")), r.as_bytes())
                .ok()
                .map(|_| format!("{base}.response.txt")),
            None => None,
        };

        (prompt_ref, response_ref)
    }

    /// The absolute path a stored ref resolves to (for debugging / serving the
    /// blob back). Returns `None` if the ref isn't a plain filename.
    pub fn resolve(&self, ref_: &str) -> Option<PathBuf> {
        let name = Path::new(ref_).file_name()?.to_str()?;
        Some(self.dir.join(name))
    }
}

/// Write `bytes` to `path`, best-effort. `Ok` means the file landed; `Err` is
/// logged (the archive is never load-bearing).
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).with_context(|| format!("write prompt archive {}", path.display()))
}

/// Make a correlation id safe to use as a filename (no path separators /
/// surprising characters). Correlations are like `run-123` / `actor-mei-42`,
/// but this keeps the archive robust if one ever contains a slash.
fn sanitize(correlation: &str) -> String {
    let mut out = String::with_capacity(correlation.len());
    for c in correlation.chars() {
        out.push(match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_prompt_and_response_and_resolves_refs() {
        let dir =
            std::env::temp_dir().join(format!("casting-prompt-archive-{}", uuid::Uuid::new_v4()));
        let archive = PromptArchive {
            dir: dir.join("prompts"),
        };

        let (pr, rr) = archive.persist("run-42", "SYSTEM hi USER you", Some("{\"ok\":true}"));
        assert_eq!(pr.as_deref(), Some("run-42.prompt.txt"));
        assert_eq!(rr.as_deref(), Some("run-42.response.txt"));

        // Files landed and content matches.
        assert_eq!(
            std::fs::read_to_string(archive.resolve(pr.as_deref().unwrap()).unwrap()).unwrap(),
            "SYSTEM hi USER you"
        );
        assert_eq!(
            std::fs::read_to_string(archive.resolve(rr.as_deref().unwrap()).unwrap()).unwrap(),
            "{\"ok\":true}"
        );

        // Cleanup.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitizes_correlations_for_filenames() {
        assert_eq!(sanitize("run-123"), "run-123");
        assert_eq!(sanitize("actor/mei:1"), "actor_mei_1");
    }

    #[test]
    fn none_response_produces_no_response_ref() {
        let dir = std::env::temp_dir().join(format!(
            "casting-prompt-archive-none-{}",
            uuid::Uuid::new_v4()
        ));
        let archive = PromptArchive {
            dir: dir.join("prompts"),
        };
        let (pr, rr) = archive.persist("run-1", "hi", None);
        assert!(pr.is_some());
        assert!(rr.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
