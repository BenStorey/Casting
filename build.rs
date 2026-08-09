//! Build script: guarantee the embedded-SPA folder exists before rust-embed
//! reads it at compile time, and emit the runtime identity constants that power
//! the ownership boundary (docs/OWNERSHIP_BOUNDARY.md, D5).
//!
//! The SPA part: `src/web.rs` embeds `frontend/dist/` via rust-embed, which
//! requires the folder (and an `index.html`) to be present when the crate
//! compiles. That folder is gitignored (it's the `npm run build` output), so on
//! a fresh checkout it doesn't exist and `cargo build` would fail. This script
//! writes a minimal placeholder only when the real build is absent, so `cast
//! run` always builds and serves *something*. A developer running `npm run
//! build` in `frontend/` overwrites it with the real SPA, which is then
//! embedded.
//!
//! The identity part: emit `CASTING_SOURCE_ROOT` (the git toplevel that built
//! this binary) and `CASTING_HEAD` (its commit) as compile-time env vars. These
//! let `Workspace::open` refuse to operate on the very repo that built the
//! binary (the self-identity guard), so Casting can never accidentally conduct
//! on its own source. See docs/OWNERSHIP_BOUNDARY.md §3/§7.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let dist = PathBuf::from("frontend/dist");
    std::fs::create_dir_all(&dist).expect("create frontend/dist for embedded SPA");

    let index = dist.join("index.html");
    if !index.exists() {
        std::fs::write(&index, PLACEHOLDER).expect("write embedded-SPA placeholder");
    }

    emit_identity();

    // Deliberately NO `cargo:rerun-if-changed`: this script is tiny and cheap,
    // and it MUST run before every compile so the embed folder is never allowed
    // to go missing (e.g. after a `git clean` or fresh checkout), and so the
    // identity env vars track the latest HEAD. Without a rerun-if-changed
    // directive cargo reruns the script on every build.
}

/// Locate the git toplevel containing `from`, walking up for a `.git` entry.
/// Pure filesystem walk — no git binary required.
fn git_toplevel(from: &Path) -> Option<PathBuf> {
    let mut cur = from;
    loop {
        if cur.join(".git").exists() {
            return Some(cur.to_path_buf());
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p,
            _ => return None,
        }
    }
}

/// Emit the source-root + HEAD identity as compile-time env vars.
fn emit_identity() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    if let Some(root) = git_toplevel(&manifest) {
        println!("cargo:rustc-env=CASTING_SOURCE_ROOT={}", root.display());
        // Best-effort HEAD sha (git may be absent in restricted build envs).
        if let Ok(out) = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()
        {
            if out.status.success() {
                let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !sha.is_empty() {
                    println!("cargo:rustc-env=CASTING_HEAD={sha}");
                }
            }
        }
    }
}

/// Self-contained page shown when the real SPA hasn't been built yet.
const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Casting</title>
    <style>
      body{font-family:ui-sans-serif,system-ui,sans-serif;background:#0e1116;color:#e6eaf0;
        display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0}
      .box{max-width:520px;text-align:center;padding:40px;background:#161b24;border:1px solid #2a3342;border-radius:12px}
      h1{margin:0 0 10px;font-size:22px}b{color:#4f8cff}
      p{color:#93a0b4;line-height:1.6;font-size:14px}
      code{background:#1e2530;padding:2px 7px;border-radius:5px;color:#e6eaf0}
    </style>
  </head>
  <body>
    <div class="box">
      <h1>🎬 Casting</h1>
      <p>The API is running, but the web UI hasn't been built yet.</p>
      <p>Run <code>npm run build</code> inside <code>frontend/</code>, then
        rebuild with <code>cargo build</code>, and this page becomes the real
        Casting workspace.</p>
    </div>
  </body>
</html>
"#;
