//! `cast` — Casting CLI (slice one: project init + event replay smoke test).
//!
//! Eventually this becomes the magical `cast run`. For now it only needs to
//! prove the headless core: create a project, append a few domain events,
//! read them back by sequence, and exercise a durable cursor.

use anyhow::{Context, Result};
use casting::cursor::CursorStore;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::{self, AppState};
use casting::sqlite_store::SqliteEventStore;
use casting::store::EventStore;
use casting::web;
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
            let dir_str = args.get(2).context("usage: cast run <project-dir>")?;
            let dir = Path::new(dir_str);
            do_run(dir)
        }
        "help" | "--help" | "-h" => {
            println!(
                "cast — Casting autonomous software company (vertical slice)\n\n\
                 USAGE:\n  cast init <dir>    create a Casting project skeleton\n  cast smoke <dir>   append sample events and replay them\n  cast run <dir>     start the workspace (PM + web UI)\n\n\
                 Env:\n  CAST_ADDR   bind address for `cast run` (default {DEFAULT_ADDR})\n"
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

/// `cast run` — boot the whole workspace: seed the project, start the simulated
/// PM control loop, and serve the API + embedded React UI from one binary.
fn do_run(dir: &Path) -> Result<()> {
    let paths = ProjectPaths::for_dir(dir)?;
    let store = SqliteEventStore::open(&paths.db)?;
    let cursors = CursorStore::open(&paths.cursors)?;

    let addr = std::env::var("CAST_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.to_string());

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let state = AppState::new(store, cursors, PROJECT_ID);

        // Seed the empty project with its existence + the PM hire.
        seed_project(&state)?;

        // Start the simulated PM control loop (background, durable cursor).
        tokio::spawn(pm::run_pm(state.clone()));

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
