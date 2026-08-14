//! Repository metrics — a point-in-time snapshot of the repo (file count,
//! lines by language, best-effort test coverage), captured when a PR lands
//! (a `MergeCompleted`). 2026-08-14.
//!
//! Design constraints (from the owner):
//! - **Helpers zero-config** — no config file, no `cargo init`/plugin, nothing
//!   checked into the user's project. We only ever READ the repo.
//! - **Language-agnostic** — the platform never names a language. Line counts
//!   come from `tokei` (a generic extension/shebang counter); file count from
//!   `git ls-files` (tracked, `.gitignore`-aware by construction).
//! - **Coverage is best-effort and honest** — coverage only exists once
//!   someone runs the tests, so there is no universal "give me coverage"
//!   (tokei is to lines what coverage is NOT to tests). We (a) read a CI
//!   coverage artifact if one exists in the repo (lcov, cobertura, jacoco,
//!   python .coverage) or (b) run a configured `CAST_COVERAGE_CMD` and parse a
//!   percentage; if neither, coverage is `None` — an honest "no data", never a
//!   fabricated number.
//!
//! `capture()` is pure (takes a repo path) so it is unit-testable; the
//! observer appends the snapshot as a `RepoMetricsCaptured` event (the payload
//! IS the event data), folded into `proj.repo_metrics`.

use crate::workspace::Workspace;
use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Capture a repo-metrics snapshot for `repo`. Never writes into the repo.
pub fn capture(repo: &Path) -> crate::types::RepoMetrics {
    crate::types::RepoMetrics {
        merge_sha: None,
        captured_at: chrono::Utc::now().to_rfc3339(),
        file_count: ls_files_count(repo),
        lines_by_language: tokei_lines(repo),
        coverage: coverage(repo),
    }
}

/// Capture a snapshot and append it as a `RepoMetricsCaptured` event through
/// the write-time integrity rail (mirrors how the git observer appends).
/// Caller supplies `merge_sha` (the PR landing that triggered this) and a
/// live `proj` to keep the check_append chain consistent.
pub fn capture_and_emit<S: crate::store::EventStore>(
    ws: &Workspace,
    store: &S,
    project: &str,
    proj: &mut crate::projection::Projection,
    merge_sha: Option<&str>,
) -> Result<()> {
    let mut rm = capture(&ws.repo);
    rm.merge_sha = merge_sha.map(|s| s.to_string());

    let ev = crate::event::Event::new(
        project,
        crate::event::Actor::System,
        crate::event::EventType::RepoMetricsCaptured,
        crate::event::Aggregate {
            kind: "repo-metrics".into(),
            id: format!("rm-{}", rm.captured_at),
        },
        serde_json::to_value(&rm)?,
    );
    crate::integrity::check_append(proj, &ev)?;
    proj.apply(&ev);
    store.append(ev)?;
    Ok(())
}

/// Number of TRACKED files: `git ls-files | wc -l`. `.gitignore`-aware by
/// construction — untracked/vendored files never count.
fn ls_files_count(repo: &Path) -> u64 {
    let Ok(out) = Command::new("git")
        .arg("ls-files")
        .current_dir(repo)
        .output()
    else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u64
}

/// Lines by language via `tokei --output=json`. Returns empty if the `tokei`
/// binary isn't on PATH (graceful degradation — file_count + coverage still
/// work). Configure a custom binary with `TOKEI_BIN`.
fn tokei_lines(repo: &Path) -> Vec<crate::types::LanguageLines> {
    use crate::types::LanguageLines;
    let bin = std::env::var("TOKEI_BIN").unwrap_or_else(|_| "tokei".to_string());
    let Ok(out) = Command::new(&bin).arg("--output=json").arg(repo).output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for (name, node) in obj {
        if name == "Total" {
            continue;
        }
        rows.push(LanguageLines {
            language: name.clone(),
            code: node.get("code").and_then(|x| x.as_u64()).unwrap_or(0),
            comments: node.get("comments").and_then(|x| x.as_u64()).unwrap_or(0),
            blanks: node.get("blanks").and_then(|x| x.as_u64()).unwrap_or(0),
            files: count_files(node),
        });
    }
    // Stable-ish order: most code first (most informative at a glance).
    rows.sort_by_key(|r| std::cmp::Reverse(r.code));
    rows
}

/// Count files under a tokei language node: `reports` at this level + recurse
/// into `children`.
fn count_files(node: &serde_json::Value) -> u64 {
    let here = node
        .get("reports")
        .and_then(|r| r.as_array())
        .map(|a| a.len() as u64)
        .unwrap_or(0);
    let children = node
        .get("children")
        .and_then(|c| c.as_object())
        .map(|o| o.values().map(count_files).sum())
        .unwrap_or(0);
    here + children
}

/// Directories we never descend into when scanning for coverage artifacts
/// (they're vendored/build noise, not sources).
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    ".venv",
    ".tox",
];

/// Best-effort test coverage: an explicit `CAST_COVERAGE_CMD` wins, else we
/// detect a CI coverage artifact in the repo. Returns `Some` only when a real
/// source produced a figure; `None` = honest "no coverage data".
fn coverage(repo: &Path) -> Option<crate::types::CoverageInfo> {
    if let Some(cmd) = std::env::var_os("CAST_COVERAGE_CMD") {
        if let Some(pct) = run_coverage_cmd(repo, &cmd.to_string_lossy()) {
            return Some(crate::types::CoverageInfo {
                percent: Some(pct),
                source: "command".to_string(),
            });
        }
    }
    find_coverage_artifact(repo, 0)
}

/// Recursively scan for a known coverage artifact; parse the first one found.
fn find_coverage_artifact(dir: &Path, depth: u32) -> Option<crate::types::CoverageInfo> {
    if depth > 4 {
        return None;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            if let Some(info) = find_coverage_artifact(&path, depth + 1) {
                return Some(info);
            }
            continue;
        }
        if let Some(info) = parse_coverage_file(&path) {
            return Some(info);
        }
    }
    None
}

/// Parse a known coverage artifact's percentage out of a file, if it matches
/// a recognized format. Source name = the file's path relative to the repo.
fn parse_coverage_file(path: &Path) -> Option<crate::types::CoverageInfo> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let content = std::fs::read_to_string(path).ok()?;
    let pct = match name {
        "lcov.info" => parse_lcov(&content),
        "coverage.xml" | "cobertura.xml" | "cobertura-coverage.xml" => parse_cobertura(&content),
        "jacoco.xml" | "jacocoTestReport.xml" => parse_jacoco(&content),
        ".coverage" => parse_python_coverage(&content),
        _ => parse_generic_percent(&content),
    };
    Some(crate::types::CoverageInfo {
        percent: pct,
        source: name.to_string(),
    })
}

/// lcov: lines are `LF:<found>` (total) and `LH:<hit>` (coverable); pct = hit/found.
fn parse_lcov(content: &str) -> Option<f64> {
    let (mut found, mut hit) = (0u64, 0u64);
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("LF:") {
            found += v.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("LH:") {
            hit += v.trim().parse::<u64>().unwrap_or(0);
        }
    }
    if found > 0 {
        Some(hit as f64 / found as f64 * 100.0)
    } else {
        None
    }
}

/// Cobertura: `<coverage ... line-rate="0.65">` — the root element carries the
/// aggregate. Take the highest line-rate attr (the root is highest).
fn parse_cobertura(content: &str) -> Option<f64> {
    content
        .split("line-rate=\"")
        .skip(1)
        .filter_map(|rest| {
            rest.split('"')
                .next()
                .and_then(|s| s.parse::<f64>().ok())
                .map(|v| v * 100.0)
        })
        .reduce(f64::max)
}

/// JaCoCo: `<counter type="LINE" covered="X" missed="Y"/>`.
fn parse_jacoco(content: &str) -> Option<f64> {
    let mut covered = 0u64;
    let mut missed = 0u64;
    for seg in content.split("<counter") {
        if !seg.contains("type=\"LINE\"") {
            continue;
        }
        covered += attr_number(seg, "covered");
        missed += attr_number(seg, "missed");
    }
    if covered + missed > 0 {
        Some(covered as f64 / (covered + missed) as f64 * 100.0)
    } else {
        None
    }
}

/// Python coverage JSON: `{"totals": {"percent_covered": 55.2, ...}}`.
fn parse_python_coverage(content: &str) -> Option<f64> {
    let v: serde_json::Value = serde_json::from_str(content).ok()?;
    v.pointer("/totals/percent_covered")?
        .as_f64()
        .map(|n| n.min(100.0))
}

/// Generic fallback: the first `NN.N%` in the text.
fn parse_generic_percent(content: &str) -> Option<f64> {
    content.bytes().enumerate().find_map(|(i, b)| {
        if b != b'%' {
            return None;
        }
        let before = &content[..i];
        let start = before
            .rfind(|c: char| !c.is_ascii_digit() && c != '.')
            .map(|p| p + 1)
            .unwrap_or(0);
        before[start..]
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v.min(100.0))
    })
}

/// Pull `<counter ... name="X" value="Y"/>`: read a numeric attr from a
/// `<counter` segment.
fn attr_number(segment: &str, attr: &str) -> u64 {
    let key = format!("{attr}=\"");
    segment
        .split(&key)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Run a configured coverage command in the repo and parse a `NN.N%` from
/// stdout/stderr. Best-effort; returns None on any failure or no parse.
fn run_coverage_cmd(repo: &Path, cmd: &str) -> Option<f64> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(repo)
        .output()
        .ok()?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    parse_generic_percent(&combined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lcov_lh_over_lf() {
        let lc = "SF:src/x.rs\nLF:100\nLH:60\nSF:src/y.rs\nLF:50\nLH:50\n";
        assert!((parse_lcov(lc).unwrap() - 73.333333).abs() < 1e-3);
    }

    #[test]
    fn parse_cobertura_root_line_rate() {
        let xml = r#"<coverage line-rate="0.85">text</coverage>"#;
        assert!((parse_cobertura(xml).unwrap() - 85.0).abs() < 1e-9);
    }

    #[test]
    fn parse_jacoco_line_counter() {
        let xml = r#"<counter type="LINE" missed="25" covered="75"/>"#;
        assert!((parse_jacoco(xml).unwrap() - 75.0).abs() < 1e-9);
    }

    #[test]
    fn parse_python_coverage_json() {
        let json = r#"{"totals": {"percent_covered": 42.1, "lines": 100}}"#;
        assert!((parse_python_coverage(json).unwrap() - 42.1).abs() < 1e-9);
    }

    #[test]
    fn parse_generic_percent_takes_last_percentage() {
        assert!((parse_generic_percent("Coverage 12.5% computed").unwrap() - 12.5).abs() < 1e-9);
    }
}
