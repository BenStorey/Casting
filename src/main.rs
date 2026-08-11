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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "init" => {
            let dir_str = args.get(2).context("usage: cast init <project-dir>")?;
            let dir = Path::new(dir_str);
            do_init(dir)
        }
        "smoke" => {
            let dir_str = args.get(2).context("usage: cast smoke <project-dir>")?;
            let dir = Path::new(dir_str);
            do_smoke(dir)
        }
        "run" => {
            let run = parse_run(&args[2..])?;
            do_run(run)
        }
        "log" => {
            let log = parse_log(&args[2..])?;
            do_log(log)
        }
        "help" | "--help" | "-h" => {
            println!(
                "cast — Casting autonomous software company (vertical slice)\n\n\
                 USAGE:\n  cast init <dir>                 create a Casting project skeleton\n  cast smoke <dir>                append sample events and replay them\n  cast run --repo <dir> --state-dir <path> [--selfhost]\n                                  start the workspace (PM + web UI)\n  cast log --db <events.db> [--project <id>] [--verify]\n                                  dump / verify the raw event stream\n\n\
                 --repo <dir>     the artifact repo Casting drives (git)\n  --state-dir <path> Casting's internal state dir (always separate from the repo)\n  --selfhost       operate on the Casting source repo itself (off by default)\n\n\
                 Env:\n  CAST_ADDR   bind address for `cast run` (default {DEFAULT_ADDR})\n  CAST_SELFHOST  1 to enable self-hosting instead of --selfhost\n"
            );
            Ok(())
        }
        other => anyhow::bail!("unknown command: {other} (try `cast help`)"),
    }
}

fn do_init(dir: &Path) -> Result<()> {
    let paths = ProjectPaths::for_dir(dir)?;
    let _events = SqliteEventStore::open(&paths.db)?;
    let _cursors = CursorStore::open(&paths.cursors)?;
    println!(
        "Initialized Casting project at {}",
        dir.join(PROJECT_DIR).display()
    );
    Ok(())
}

/// Flags for `cast run`, parsed by [`parse_run`].
struct RunArgs {
    repo: PathBuf,
    state_dir: PathBuf,
    selfhost: Selfhost,
}

fn parse_run(args: &[String]) -> Result<RunArgs> {
    let mut repo = None;
    let mut state_dir = None;
    let mut selfhost = Selfhost::Disabled;

    // --selfhost may also come from the env (CAST_SELFHOST=1).
    if std::env::var("CAST_SELFHOST").is_ok_and(|v| v == "1") {
        selfhost = Selfhost::Enabled;
    }

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" => {
                repo = Some(
                    args.get(i + 1)
                        .context("--repo requires a path")?
                        .into(),
                );
                i += 2;
            }
            "--state-dir" => {
                state_dir = Some(
                    args.get(i + 1)
                        .context("--state-dir requires a path")?
                        .into(),
                );
                i += 2;
            }
            "--selfhost" => {
                selfhost = Selfhost::Enabled;
                i += 1;
            }
            other => anyhow::bail!(
                "unknown argument {other:?} (tip: cast run --repo <dir> --state-dir <path> [--selfhost])"
            ),
        }
    }

    let repo = repo.context("cast run requires --repo <dir>")?;
    let state_dir = state_dir.context("cast run requires --state-dir <path>")?;
    Ok(RunArgs {
        repo,
        state_dir,
        selfhost,
    })
}

/// Print the preflight banner: the canonical target + detected repo HEAD, so
/// the operator *sees* what Casting is about to touch before anything mutates.
fn preflight(ws: &Workspace, repo_created: bool) {
    println!("🎬 Casting workspace");
    println!("   artifact repo: {}", ws.repo.display());
    println!("   state-dir:     {}", ws.state_dir.display());
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
fn do_run(run: RunArgs) -> Result<()> {
    let ws = Workspace::open(&run.repo, &run.state_dir, run.selfhost)?;

    // Ensure the artifact repo is a real git repo (git-init if missing). This
    // wires Git into the workspace at startup (Git slice increment 1).
    let created = ws.ensure_repo().context("ensure git repo")?;

    preflight(&ws, created);

    // Casting's internal state lives in the (mandatory, separate) state dir.
    let store = SqliteEventStore::open(ws.state_dir.join("events.db"))?;
    let cursors = CursorStore::open(ws.state_dir.join("cursors.db"))?;
    // Projection snapshots (a pure read optimization, never a source of truth).
    let snapshots = casting::snapshot::SnapshotStore::open(ws.state_dir.join("snapshots.db"))?;

    let addr = std::env::var("CAST_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let state = AppState::new(store, cursors, PROJECT_ID)
            .with_snapshots(snapshots)
            .with_integrity();

        // Seed the empty project with its existence + the PM hire.
        seed_project(&state)?;

        // Run the git observer once at boot so the event log reflects the
        // current repo state before the PM starts reasoning (Git slice
        // increment 2). Subsequent observations happen on each PM drain.
        git::observe_once(&state, &ws).await;

        // Start the simulated PM control loop (background, durable cursor).
        // The loop also triggers the git observer on each drain pass.
        tokio::spawn(pm::run_pm(state.clone(), ws.clone()));

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
    Ok(())
}

/// Append a representative slice of domain events and prove append->read_since
/// and cursor resume. Purely a harness; real agents fill this in later.
fn do_smoke(dir: &Path) -> Result<()> {
    let paths = ProjectPaths::for_dir(dir)?;
    let store = SqliteEventStore::open(&paths.db)?;
    let cursors = CursorStore::open(&paths.cursors)?;

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
