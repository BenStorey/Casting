//! `cast` — Casting CLI (slice one: project init + event replay smoke test).
//!
//! Eventually this becomes the magical `cast run`. For now it only needs to
//! prove the headless core: create a project, append a few domain events,
//! read them back by sequence, and exercise a durable cursor.

use anyhow::{Context, Result};
use casting::cursor::CursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::git_observer as git;
use casting::pm::{self, AppState};
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;
use casting::web;
use casting::workspace::{Selfhost, Workspace};
use std::path::{Path, PathBuf};

/// Open a project's storage backend from the CLI (workspace + selector), then
/// build an integrity-enforcing, snapshot-aware AppState. Shared by the
/// command-line write paths (brief/request) so they can't drift (review
/// refactor 2026-08-10).
fn open_state(repo: &Path, db: Option<&str>) -> Result<AppState> {
    let ws = Workspace::open(repo, Selfhost::Disabled)?;
    let selector = db
        .map(str::to_string)
        .or_else(|| std::env::var("CAST_DB").ok())
        .unwrap_or_else(|| "sqlite".to_string());
    let backend = casting::backend::from_selector(&selector, ws.casting_dir())?;
    let store = backend.events();
    let cursors = backend.cursors();
    let snapshots = backend.snapshots();
    let mut state = setup_state(store, cursors, snapshots);
    // Harness guard (2026-08-13, secrets.rs): attach the per-project secret
    // store (gitignored, NEVER in the event log). The executor then refuses to
    // schedule/execute an activity that embeds a raw secret value. The runner
    // resolves `@secret:NAME@` at execution time, in memory.
    state = state.with_secrets(casting::secrets::SecretStore::load(ws.casting_dir())?);
    Ok(state)
}

fn setup_state(
    store: std::sync::Arc<dyn EventStore>,
    cursors: std::sync::Arc<dyn CursorStore>,
    snapshots: Option<std::sync::Arc<dyn casting::snapshot::SnapshotStore>>,
) -> AppState {
    let mut state = AppState::new(store, cursors, PROJECT_ID).with_integrity();
    if let Some(snaps) = snapshots {
        state = state.with_snapshots(snaps);
    }
    if std::env::var("CAST_DECOMPOSE").is_ok() {
        state = state.with_decompose();
    }
    state
}

const PROJECT_DIR: &str = ".casting";
const PROJECT_ID: &str = "project-demo";
const DEFAULT_ADDR: &str = "127.0.0.1:8080";

struct ProjectPaths {
    db: PathBuf,
    cursors: PathBuf,
}

impl ProjectPaths {
    fn for_dir(dir: &Path) -> Result<Self> {
        let db = dir.join(PROJECT_DIR).join("events.db");
        let cursors = dir.join(PROJECT_DIR).join("cursors.db");
        Ok(ProjectPaths { db, cursors })
    }
}

/// Load the consultant registry for a project: the curated embedded defaults
/// overlaid by any user-supplied packages in `<project>/.casting/consultants/`
/// (drop a `.toml` to add a consultant, reuse its `id` to override a default).
/// A malformed user package is surfaced (not silently dropped); a missing
/// directory is a no-op.
fn load_consultants(
    ws: &std::sync::Arc<casting::workspace::Workspace>,
) -> std::sync::Arc<casting::consultants::ConsultantRegistry> {
    let mut reg = casting::consultants::ConsultantRegistry::from_embedded().unwrap_or_default();
    let dir = ws.casting_dir().join("consultants");
    match reg.overlay_dir(&dir) {
        Ok(n) if n > 0 => {
            println!(
                "🧑‍💼 loaded {n} user consultant package(s) from {}",
                dir.display()
            );
        }
        Ok(_) => {}
        Err(e) => eprintln!(
            "⚠️  [consultants] ignoring overlay {}: {e:#}",
            dir.display()
        ),
    }
    std::sync::Arc::new(reg)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "init" => {
            let init = parse_init(&args[2..])?;
            do_init(init)
        }
        "smoke" => {
            let dir_str = args.get(2).context("usage: cast smoke <project-dir>")?;
            let dir = Path::new(dir_str);
            do_smoke(dir)
        }
        // `cast run <project-dir>` — boot the ONE project. Single-project model:
        // the binary only ever relates to one project; multi-project is removed
        // (owner decision 2026-08-12). `--db <selector>` selects the storage
        // backend (sqlite default); CAST_DB env also works.
        "run" => {
            let dir = args
                .get(2)
                .context("usage: cast run <project-dir> [--db <selector>]")?;
            let db = args
                .windows(2)
                .find(|w| w[0] == "--db")
                .and_then(|w| w.get(1))
                .cloned();
            do_run(PathBuf::from(dir), db)
        }
        // `cast brief <project-dir> [--subject S] [--source SRC] [--title T] <file|->`
        "brief" => {
            let dir = args.get(2).context(
                "usage: cast brief <project-dir> [--subject S] [--source SRC] [--title T] <file|->",
            )?;
            do_brief(&args[3..], Path::new(dir))
        }
        // `cast request <project-dir> [--source SRC] [--reporter R] [--label L] <title>`
        "request" => {
            let dir = args.get(2).context(
                "usage: cast request <project-dir> [--source SRC] [--reporter R] [--label L] <title>",
            )?;
            do_request(&args[3..], Path::new(dir))
        }
        "log" => {
            let log = parse_log(&args[2..])?;
            do_log(log)
        }
        "help" | "--help" | "-h" => {
            println!(
                "cast — Casting autonomous software company\n\n\
                 USAGE:\n  cast init <project-dir> [--interactive] [--name=..] [--objective=..] [--cast=a,b] [--owner-token=..] [--directive=stmt|scope]\n                                create + configure a project\n  cast run <project-dir> [--db <selector>] [--selfhost]\n                                start the workspace (PM + web UI) for the one project\n  cast smoke <dir>              append sample events and replay them\n  cast brief <project-dir> [--subject S] [--source SRC] [--title T] <file|->\n                                import EXTERNAL advisor content as an advisory briefing\n  cast request <project-dir> [--source SRC] [--reporter R] [--label L] <title>\n                                receive an EXTERNAL request (issue/PR) into the intake\n  cast log --db <events.db> [--project <id>] [--verify]\n                                dump / verify the raw event stream\n\n                 Single-project:\n  Casting is SINGLE-PROJECT. The binary relates to exactly one project (the\n  dir you pass). Multi-project is deliberately NOT supported — the cloud\n  service later will be the multi-project-in-one-window differentiator.\n  State lives collocated in <project-dir>/.casting/ (gitignored).\n\n                 Env:\n  CAST_ADDR       bind address for `cast run` (default {DEFAULT_ADDR})\n  CAST_DB         storage backend selector ('sqlite' or a libpq Postgres string)\n  CAST_OWNER_TOKEN owner auth token (or set via `cast init --owner-token`)\n  CAST_SELFHOST   1 to enable self-hosting instead of --selfhost\n"
            );
            Ok(())
        }
        other => anyhow::bail!("unknown command: {other} (try `cast help`)"),
    }
}

/// `cast brief` — import EXTERNAL advisor content (a text file, or stdin via
/// `-`) as an ADVISORY briefing: it can inform context but never sets rules.
/// Usage: `cast brief <project-name> [--subject S] [--source SRC] [--title T] <file|->`
fn do_brief(args: &[String], repo: &std::path::Path) -> Result<()> {
    let (mut subject, mut source, mut title) = (
        "general".to_string(),
        "advisor".to_string(),
        "advisor briefing".to_string(),
    );
    let mut file: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--subject" => {
                subject = args.get(i + 1).context("--subject needs a value")?.clone();
                i += 2;
            }
            "--source" => {
                source = args.get(i + 1).context("--source needs a value")?.clone();
                i += 2;
            }
            "--title" => {
                title = args.get(i + 1).context("--title needs a value")?.clone();
                i += 2;
            }
            other => {
                file = Some(other);
                i += 1;
            }
        }
    }
    let body = match file {
        Some("-") | None => {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        Some(p) => std::fs::read_to_string(p).with_context(|| format!("read {p}"))?,
    };
    if body.trim().is_empty() {
        anyhow::bail!("briefing body is empty");
    }

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let state = open_state(repo, None)?;

        let source_clone = source.clone();
        let body_len = body.len();

        let action = casting::actions::PmAction::ImportBriefing {
            id: format!("brief-{}", uuid::Uuid::new_v4()),
            source,
            subject,
            title,
            body,
            assets: Vec::new(),
        };
        let proj = state.projection()?;
        casting::actions::validate(&action, "owner", &proj)?;

        // Build the event through the action, then append it (advisory, never
        // authoritative — recorded with its `source` so provenance is explicit).
        let stored = {
            use casting::event::{Actor, Aggregate, Event, EventType};
            let previous = state.store.latest_sequence(&state.project)?;
            let cause = state
                .store
                .read_since(&state.project, previous.saturating_sub(1))?
                .pop()
                .unwrap_or_else(|| {
                    Event::new(
                        &state.project,
                        Actor::Owner,
                        EventType::MessageSent,
                        Aggregate {
                            kind: "message".into(),
                            id: "bootstrap".into(),
                        },
                        serde_json::json!({}),
                    )
                });
            let ev = action
                .to_events(&state.project, "owner", &cause, "brief")
                .into_iter()
                .next()
                .expect("ImportBriefing produces one event");
            state.append(ev.clone())?;
            ev
        };

        println!(
            "imported advisory briefing {} (from {source_clone}, {body_len} bytes)",
            stored.aggregate.id
        );
        Ok(())
    })
}

/// `cast request` — receive an EXTERNAL request (a GitHub issue/PR, an email,
/// a form submission) into the product's intake surface. Recorded with
/// provenance + deterministic triage; NOT the owner's own intent.
fn do_request(args: &[String], repo: &std::path::Path) -> Result<()> {
    let (mut source, mut reporter) = ("external".to_string(), "external".to_string());
    let mut labels: Vec<String> = Vec::new();
    let mut title: Option<&str> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => {
                source = args.get(i + 1).context("--source needs a value")?.clone();
                i += 2;
            }
            "--reporter" => {
                reporter = args.get(i + 1).context("--reporter needs a value")?.clone();
                i += 2;
            }
            "--label" => {
                let l = args.get(i + 1).context("--label needs a value")?.clone();
                labels.push(l);
                i += 2;
            }
            other => {
                title = Some(other);
                i += 1;
            }
        }
    }
    let title = title.context("usage: cast request <project> [flags] <title>")?;
    if title.trim().is_empty() {
        anyhow::bail!("request title is empty");
    }

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let state = open_state(repo, None)?;

        let proj = state.projection()?;
        let action = casting::actions::PmAction::ReceiveExternalRequest {
            id: format!("req-{}", uuid::Uuid::new_v4()),
            source,
            external_id: None,
            title: title.to_string(),
            body: String::new(),
            reporter,
            labels,
            url: None,
        };
        casting::actions::validate(&action, "pm", &proj)?;

        {
            use casting::event::{Actor, Aggregate, Event, EventType};
            let previous = state.store.latest_sequence(&state.project)?;
            let cause = state
                .store
                .read_since(&state.project, previous.saturating_sub(1))?
                .pop()
                .unwrap_or_else(|| {
                    Event::new(
                        &state.project,
                        Actor::System,
                        EventType::ExternalRequestReceived,
                        Aggregate {
                            kind: "external_request".into(),
                            id: "bootstrap".into(),
                        },
                        serde_json::json!({}),
                    )
                });
            let ev = action
                .to_events(&state.project, "pm", &cause, "request")
                .into_iter()
                .next()
                .expect("ReceiveExternalRequest produces one event");
            state.append(ev)?;
        }

        // Rebuild projection to report the recorded triage (classification/severity).
        let proj = state.projection()?;
        let r = proj
            .external_requests
            .last()
            .expect("request just appended");
        println!(
            "received {} request {} — {} ({} / {}) from {}",
            r.source, r.id, r.title, r.classification, r.severity, r.reporter
        );
        Ok(())
    })
}

/// Flags for `cast init`, parsed by [`parse_init`].
struct InitArgs {
    dir: PathBuf,
    interactive: bool,
    name: Option<String>,
    objective: Option<String>,
    cast: Vec<String>,
    owner_token: Option<String>,
    directives: Vec<(String, String)>, // (statement, scope)
}

fn parse_init(args: &[String]) -> Result<InitArgs> {
    let mut dir = None;
    let mut interactive = false;
    let mut name = None;
    let mut objective = None;
    let mut cast = Vec::new();
    let mut owner_token = None;
    let mut directives = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--interactive" | "-i" => interactive = true,
            "--name" => {
                name = Some(args.get(i + 1).context("--name requires a value")?.clone());
                i += 1;
            }
            "--objective" => {
                objective = Some(
                    args.get(i + 1)
                        .context("--objective requires a value")?
                        .clone(),
                );
                i += 1;
            }
            "--cast" => {
                cast = args
                    .get(i + 1)
                    .context("--cast requires a comma-separated role list")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                i += 1;
            }
            "--owner-token" => {
                owner_token = Some(
                    args.get(i + 1)
                        .context("--owner-token requires a value")?
                        .clone(),
                );
                i += 1;
            }
            "--directive" => {
                let spec = args
                    .get(i + 1)
                    .context("--directive requires 'statement|scope'")?
                    .clone();
                let (statement, scope) = spec.split_once('|').unwrap_or((&spec, ""));
                directives.push((statement.to_string(), scope.to_string()));
                i += 1;
            }
            other if !other.starts_with('-') => dir = Some(PathBuf::from(other)),
            other => anyhow::bail!("unknown init flag: {other}"),
        }
        i += 1;
    }

    Ok(InitArgs {
        dir: dir.context("usage: cast init <state-dir> [--interactive] [--name=..] [--objective=..] [--cast=a,b] [--owner-token=..]")?,
        interactive,
        name,
        objective,
        cast,
        owner_token,
        directives,
    })
}

/// Prompt for a single line on stdin.
fn prompt(prompt: &str, default: Option<&str>) -> std::io::Result<String> {
    use std::io::Write;
    match default {
        Some(d) => print!("{prompt} [{d}] "),
        None => print!("{prompt} "),
    }
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn do_init(mut args: InitArgs) -> Result<()> {
    if args.interactive {
        args.name = Some(args.name.unwrap_or_else(|| {
            prompt("Company / product name?", Some("Acme Inc"))
                .unwrap_or_else(|_| "Acme Inc".into())
        }));
        args.objective = Some(args.objective.unwrap_or_else(|| {
            prompt("What should your team build first? (the objective)", None).unwrap_or_default()
        }));
        if args.cast.is_empty() {
            let answer = prompt(
                "Initial roles (comma-sep from catalog: engineer,qa,security,devops)",
                Some("engineer,qa"),
            )
            .unwrap_or_else(|_| "engineer,qa".into());
            args.cast = answer
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if args.owner_token.is_none() {
            let tok = prompt("Owner auth token? (blank = auth off)", None).unwrap_or_default();
            args.owner_token = if tok.is_empty() { None } else { Some(tok) };
        }
    }

    let spec = casting::setup::SetupSpec {
        name: args.name.clone().unwrap_or_else(|| "Casting demo".into()),
        roles: args.cast.clone(),
        owner_token: args.owner_token.clone(),
        directives: args
            .directives
            .into_iter()
            .map(|(statement, scope)| casting::setup::StartDirective {
                id: format!(
                    "setup-{}",
                    statement
                        .to_lowercase()
                        .replace(' ', "-")
                        .chars()
                        .take(20)
                        .collect::<String>()
                ),
                kind: casting::directive::DirectiveKind::Policy,
                statement,
                scope: scope
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                strength: casting::directive::DirectiveStrength::Required,
            })
            .collect(),
    };

    let plan = casting::setup::SetupPlan::build(spec)?;

    // State lives collocated in <project>/.casting/ (gitignored).
    std::fs::create_dir_all(&args.dir).context("create project dir")?;
    let casting_dir = args.dir.join(".casting");
    let ws = Workspace::open(&args.dir, Selfhost::Disabled)?;
    ws.ensure_self_ignored()
        .context("ensure .casting self-ignored")?;

    let written = plan.apply(&casting_dir)?;
    // Write a no-secrets config template to the repo root (like .env.example).
    casting::setup::write_template(&args.dir, &plan.spec.name)?;
    println!(
        "   project ready at {} (run `cast run {}`)",
        args.dir.display(),
        args.dir.display()
    );
    if written == 0 {
        println!(
            "Company already set up at {} — no changes.",
            args.dir.display()
        );
    } else {
        println!(
            "🎬 {} is live at {}\n   Team: {}",
            plan.spec.name,
            args.dir.display(),
            plan.hires
                .iter()
                .map(|(id, role)| format!("{id} ({role})"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if let Some(obj) = args.objective {
        println!("   📣 To kick off the build, send \u{201c}{obj}\u{201d} as your first message in the UI.");
    }
    if args.interactive && args.owner_token.is_none() {
        println!("   ⚠️  no owner token set — owner-mutating writes are OPEN (set one via --owner-token / CAST_OWNER_TOKEN)");
    }
    Ok(())
}

/// Print the preflight banner: the canonical target + detected repo HEAD, so
/// the operator *sees* what Casting is about to touch before anything mutates.
fn preflight(ws: &Workspace, repo_created: bool) {
    println!("🎬 Casting workspace");
    println!("   project:       {}", ws.repo.display());
    println!("   state:         {}", ws.casting_dir().display());
    if repo_created {
        println!("   git:           initialized (repo had no .git)");
    }
    match ws.head() {
        Some(sha) => println!("   repo HEAD:     {sha}"),
        None => println!("   repo HEAD:     (no commits yet)"),
    }
    if let Some(branch) = ws.current_branch() {
        println!("   branch:        {branch}");
    }
    if ws.selfhost() == Selfhost::Enabled {
        println!("   self-hosting:  enabled (operating on the Casting source repo)");
    }
}

/// `cast run` — boot the whole workspace: enforce the ownership boundary, seed
/// the project, start the simulated PM control loop, and serve the API +
/// embedded React UI from one binary.
fn do_run(project: std::path::PathBuf, db: Option<String>) -> Result<()> {
    let ws = Workspace::open(&project, Selfhost::Disabled)?;

    // Ensure the artifact repo is a real git repo (git-init if missing). This
    // wires Git into the workspace at startup (Git slice increment 1).
    let created = ws.ensure_repo().context("ensure git repo")?;

    // Casting's internal state lives collocated in <repo>/.casting/, which is
    // self-ignored by git (so it never shows as pending changes).
    ws.ensure_self_ignored()
        .context("ensure .casting self-ignored")?;

    preflight(&ws, created);

    // Open the storage backend: --db flag, else the CAST_DB env var, else
    // SQLite (the default). Postgres is swappable behind
    // the same traits.
    let backend = {
        let selector = db
            .or_else(|| std::env::var("CAST_DB").ok())
            .unwrap_or_else(|| "sqlite".to_string());
        casting::backend::from_selector(&selector, ws.casting_dir())?
    };
    let store = backend.events();
    let cursors = backend.cursors();
    let snapshots = backend.snapshots();

    let addr = std::env::var("CAST_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    let ws = std::sync::Arc::new(ws);
    let ws_for_pm = ws.clone();
    rt.block_on(async move {
        let mut state = AppState::new(store, cursors, PROJECT_ID).with_integrity();
        if let Some(snaps) = snapshots {
            state = state.with_snapshots(snaps);
        }
        if std::env::var("CAST_DECOMPOSE").is_ok() {
            state = state.with_decompose();
        }
        let state = state
            .with_state_dir(ws.casting_dir().to_path_buf())
            // Attach the workspace so the PM can physically provision isolated
            // worktrees when a consultant is summoned (2026-08-12).
            .with_workspace(ws.clone())
            // The consultant registry: curated embedded defaults overlaid by
            // any user-supplied packages in <project>/.casting/consultants/
            // (drop a .toml to add a consultant, reuse an id to override one).
            .with_consultants(load_consultants(&ws))
            // D2 LLM wiring: when CAST_LLM_API_KEY is set, attach the real
            // OpenAI-compatible orchestrator (OpenRouter day-one; provider is
            // config — base_url+key+model — so a local LiteLLM swaps in without
            // code). Unconfigured → the deterministic scripted PM stays the
            // default (off by default, no spend, backwards compatible).
            .pipe_llm_orchestrator();
        // Owner auth: the token comes from the persisted setup config.json
        // first (set via `cast init --owner-token`), else CAST_OWNER_TOKEN env.
        // Off by default. Owner-mutating endpoints require Authorization:
        // Bearer <token>.
        let persisted_token = casting::setup::read_config(ws.casting_dir())
            .and_then(|c| c.owner_token)
            .filter(|t| !t.is_empty());
        let token = persisted_token.or_else(|| std::env::var("CAST_OWNER_TOKEN").ok());
        let state = match token {
            Some(tok) if !tok.is_empty() => {
                println!("🔐 owner auth enabled (send 'Authorization: Bearer <token>' to mutate)");
                state.with_owner_auth(tok)
            }
            _ => state,
        };

        // Seed the empty project with its existence + the PM hire.
        seed_project(&state)?;

        // Run the git observer once at boot so the event log reflects the
        // current repo state before the PM starts reasoning (Git slice
        // increment 2). Subsequent observations happen on each PM drain.
        git::observe_once(&state, &ws).await;

        // Durable-execution recovery: re-dispatch any activity that was
        // scheduled but never completed/failed (i.e. the server died mid-
        // activity). The executor's idempotency guard makes the re-run safe.
        // The NoopRunner is the safe default until D2 (LLM) / git / shell
        // runners are wired — it can only do inline work and fails loudly on
        // external kinds rather than silently fake-completing them.
        match casting::executor::redispatch_inflight(
            &state,
            &casting::executor::NoopRunner,
            casting::event::Actor::System,
        ) {
            Ok(ids) if !ids.is_empty() => {
                println!(
                    "🔁 crash-recovery: re-dispatched {} in-flight activities",
                    ids.len()
                );
            }
            _ => {}
        }

        // Start the simulated PM control loop (background, durable cursor).
        // The loop also triggers the git observer on each drain pass.
        tokio::spawn(pm::run_pm(state.clone(), (*ws_for_pm).clone()));

        // Liveness watchdog (2026-08-13): a wall-clock "dead man's switch" that
        // auto-pauses the cast on a stall (repeated errors / silent in-flight
        // work). Self-actuating; enabled via CAST_WATCHDOG=1 (+ tuning env).
        if let Some(cfg) = casting::watchdog::WatchConfig::from_env() {
            println!(
                "🛡️ liveness watchdog enabled (poll {}s, stall {}h, retry>{}x)",
                cfg.poll_secs, cfg.stall_hours, cfg.max_repeat_errors
            );
            tokio::spawn(casting::watchdog::monitor(state.clone(), cfg));
        }

        // Serve the workspace.
        let app = web::router(state);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        println!("🎬 Casting workspace ready: http://{addr}");
        println!("   Tell the PM what you want from the chat — the team will kick off.");
        axum::serve(listener, app)
            .await
            .context("axum server error")?;
        Ok(())
    })
}

/// Flags for `cast log`, parsed by [`parse_log`].
struct LogArgs {
    db: PathBuf,
    project: Option<String>,
    verify: bool,
}

fn parse_log(args: &[String]) -> Result<LogArgs> {
    let mut db = None;
    let mut project = None;
    let mut verify = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => {
                db = Some(
                    args.get(i + 1)
                        .context("--db requires a path to events.db")?
                        .into(),
                );
                i += 2;
            }
            "--project" => {
                project = Some(
                    args.get(i + 1)
                        .context("--project requires an id")?
                        .clone(),
                );
                i += 2;
            }
            "--verify" => {
                verify = true;
                i += 1;
            }
            other => anyhow::bail!(
                "unknown argument {other:?} (tip: cast log --db <events.db> [--project <id>] [--verify])"
            ),
        }
    }
    Ok(LogArgs {
        db: db.context("cast log requires --db <events.db>")?,
        project,
        verify,
    })
}

/// `cast log` — dump the raw event stream and/or verify its invariants.
fn do_log(log: LogArgs) -> Result<()> {
    let store = SqliteEventStore::open(&log.db)?;

    // Resolve the project(s): explicit id, or every project in the store.
    let projects = match &log.project {
        Some(p) => vec![p.clone()],
        None => {
            let all = store.list_projects()?;
            if all.is_empty() {
                println!("(no projects in {})", log.db.display());
                return Ok(());
            }
            all
        }
    };

    for project in &projects {
        if projects.len() > 1 {
            println!("== project: {project} ==");
        }
        if log.verify {
            let problems = casting::replay::verify(&store, project)?;
            if problems.is_empty() {
                println!("{}: OK (event stream invariants hold)", project);
            } else {
                println!("{}: {} problem(s):", project, problems.len());
                for p in problems {
                    println!("  - {p}");
                }
            }
        } else {
            for line in casting::replay::dump(&store, project)? {
                println!("{line}");
            }
        }
    }
    Ok(())
}

/// Idempotently seed a fresh project: ProjectCreated + hire the PM, so the
/// board/team starts with the management layer visible. Safe to re-run (the
/// PM's cursor starts at 0 and these aren't owner-input events, so the loop
/// won't react to them).
fn seed_project(state: &AppState) -> Result<()> {
    let project = state.project.clone();
    if state.store.latest_sequence(&project)? > 0 {
        return Ok(()); // already seeded
    }
    state.append(Event::new(
        &project,
        Actor::System,
        EventType::ProjectCreated,
        Aggregate {
            kind: "project".into(),
            id: project.clone(),
        },
        serde_json::json!({"name": "Casting demo"}),
    ))?;
    state.append(Event::new(
        &project,
        Actor::System,
        EventType::AgentHired,
        Aggregate {
            kind: "agent".into(),
            id: "pm".into(),
        },
        serde_json::json!({"role": "Project Manager"}),
    ))?;

    // Hire the default cast at seed so EVERY fresh open — scripted OR LLM-driven
    // — always has a working team (at least one developer) to assign work to.
    // The scripted `plan_onboard` also hires the default cast on the first owner
    // message, but its already-hired filter makes this idempotent (no dupes); the
    // LLM path never runs plan_onboard, so seeding here is what guarantees the
    // developer exists before the model tries to assign. A custom cast chosen via
    // setup is a separate store (apply_to_store), so `seed_project` only fires on
    // a genuinely bare first open.
    for m in casting::cast::DEFAULT_CAST {
        let role = casting::cast::role_by_id(m.role_id)
            .map(|r| r.title.to_string())
            .unwrap_or_else(|| m.role_id.to_string());
        state.append(Event::new(
            &project,
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: m.agent_id.into(),
            },
            serde_json::json!({ "role": role }),
        ))?;
    }
    Ok(())
}

/// Append a representative slice of domain events and prove append->read_since
/// and cursor resume. Purely a harness; real agents fill this in later.
fn do_smoke(dir: &Path) -> Result<()> {
    let paths = ProjectPaths::for_dir(dir)?;
    let store = SqliteEventStore::open(&paths.db)?;
    let cursors = casting::cursor::SqliteCursorStore::open(&paths.cursors)?;

    let project = "project-demo";

    // ProjectCreated by the system.
    store.append(Event::new(
        project,
        Actor::System,
        EventType::ProjectCreated,
        Aggregate {
            kind: "project".into(),
            id: project.into(),
        },
        serde_json::json!({"name": "Demo project"}),
    ))?;

    // Owner asks for a thing.
    store.append(Event::new(
        project,
        Actor::Owner,
        EventType::MessageSent,
        Aggregate {
            kind: "project".into(),
            id: project.into(),
        },
        serde_json::json!({"body": "Build me a todo app"}),
    ))?;

    // PM + engineer hired.
    for (id, role) in [
        ("pm", "Project Manager"),
        ("marcus-reed", "Principal Engineer"),
    ] {
        store.append(Event::new(
            project,
            Actor::System,
            EventType::AgentHired,
            Aggregate {
                kind: "agent".into(),
                id: id.into(),
            },
            serde_json::json!({"role": role}),
        ))?;
    }

    // PM creates a task and assigns it.
    let task_created = store.append(Event::new(
        project,
        Actor::Agent { id: "pm".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({"title": "Implement authentication", "kind": "feature"}),
    ))?;
    store.append(Event::new(
        project,
        Actor::Agent { id: "pm".into() },
        EventType::TaskAssigned,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({"assignee": "marcus-reed"}),
    ))?;

    println!(
        "Appended {} events (latest seq {})",
        store.latest_sequence(project)?,
        store.latest_sequence(project)?
    );

    // PM's cursor: replay everything fresh, then persist position.
    let pm_cursor = cursors.get(project, "pm")?;
    println!(
        "PM cursor before: seq {} (resume point)",
        pm_cursor.last_seen
    );
    let events = store.read_since(project, pm_cursor.last_seen)?;
    for e in &events {
        println!(
            "  #{:<4} {:?} :: {:?} ({})",
            e.sequence, e.event_type, e.actor, e.aggregate.id
        );
    }
    let new_last = store.latest_sequence(project)?;
    cursors.advance(project, "pm", new_last)?;
    println!("PM cursor advanced to seq {new_last}");

    // Prove idempotence / durability: a second read_since returns nothing.
    let again = store.read_since(project, new_last)?;
    println!(
        "Replay from committed cursor: {} new events (expected 0)",
        again.len()
    );

    // Prove cursor survived: reopen a fresh store object on the same file.
    let _task_created_seq = task_created.sequence;
    println!(
        "TaskCreated assigned sequence {} (monotonic ordering)",
        task_created.sequence
    );
    Ok(())
}
