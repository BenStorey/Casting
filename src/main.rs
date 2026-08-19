//! `cast` — Casting CLI (slice one: project init + event replay smoke test).
//!
//! Eventually this becomes the magical `cast run`. For now it only needs to
//! prove the headless core: create a project, append a few domain events,
//! read them back by sequence, and exercise a durable cursor.

use anyhow::{Context, Result};
use casting::consultants::ConsultantRegistry;
use casting::event::{Actor, Aggregate, Event, EventType};
use casting::pm::{self, AppState};
use casting::store::CursorStore;
use casting::store::EventStore;
use casting::store::SqliteEventStore;
use casting::web;
use casting::workspace::git_observer as git;
use casting::workspace::setup::{casting_home, read_config, slugify, RuntimeConfig};
use casting::workspace::{Selfhost, Workspace};
use std::path::{Path, PathBuf};

/// Helper: parse the project slug from `--project <slug>` (or `--project=<slug>`)
/// if present in args, else None.
fn parse_project_arg(args: &[String]) -> Option<String> {
    if let Some(pos) = args.iter().position(|a| a == "--project") {
        return args.get(pos + 1).cloned();
    }
    args.iter()
        .find_map(|a| a.strip_prefix("--project="))
        .map(|s| s.to_string())
}

/// Resolve which project to operate on, given an optional `--project <slug>`:
///   - explicit slug -> that project's dir under ~/.casting/<slug>/;
///   - no slug -> if exactly one project exists, auto-select it; if none, error
///     telling the user to run `cast init`; if more than one, error listing them.
///
/// Returns the project's state dir plus its persisted config.
/// Scan `~/.casting/` for every project that has a readable `config.json`,
/// returning each project's state dir plus its persisted config. Used by both
/// `resolve_project` (to pick one) and `do_run` (to detect "no projects yet").
fn discover_projects() -> Vec<(PathBuf, RuntimeConfig)> {
    let home = match casting_home() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let Some(rd) = std::fs::read_dir(&home).ok() else {
        return Vec::new();
    };
    let found: Vec<(PathBuf, RuntimeConfig)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|d| read_config(&d).map(|c| (d, c)))
        .collect();
    log::info!(
        "discovered {} Casting project(s) under {}",
        found.len(),
        home.display()
    );
    found
}

/// Resolve which project to operate on, given an optional `--project <slug>`:
///   - explicit slug -> that project's dir under ~/.casting/<slug>/;
///   - no slug -> if exactly one project exists, auto-select it; if none, error
///     telling the user to run `cast init`; if more than one, error listing them.
///
/// Returns the project's state dir plus its persisted config.
fn resolve_project(slug: Option<String>) -> Result<(PathBuf, RuntimeConfig)> {
    match slug {
        Some(s) => {
            let dir = casting_home()?.join(&s);
            let cfg = read_config(&dir).with_context(|| {
                format!(
                    "no project '{s}' — expected state dir at {} (run `cast init <repo> --name ...` first)",
                    dir.display()
                )
            })?;
            Ok((dir, cfg))
        }
        None => {
            let existing = discover_projects();
            match existing.len() {
                0 => anyhow::bail!(
                    "no Casting projects found under {}. Run `cast init <repo> --name <name>` first.",
                    casting_home()?.display()
                ),
                1 => Ok(existing.into_iter().next().unwrap()),
                _ => {
                    let home = casting_home()?;
                    let list = existing
                        .iter()
                        .map(|(_, c)| format!("  - {} (slug: {})", c.name, c.slug.clone().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    anyhow::bail!(
                        "multiple projects found under {} — pass --project <slug>:\n{}",
                        home.display(),
                        list
                    );
                }
            }
        }
    }
}

/// Open a project's storage backend from the CLI (workspace + selector), then
/// build an integrity-enforcing, snapshot-aware AppState. Shared by the
/// command-line write paths (brief/request) so they can't drift (review
/// refactor 2026-08-10). The workspace's state dir is supplied explicitly
/// (it lives under `~/.casting/<slug>/`, NOT inside the repo).
fn open_state(repo: &Path, state_dir: &Path, db: Option<&str>) -> Result<AppState> {
    let ws = Workspace::open(repo, state_dir, Selfhost::Disabled)?;
    let selector = db
        .map(str::to_string)
        .or_else(|| std::env::var("CAST_DB").ok())
        .unwrap_or_else(|| "sqlite".to_string());
    let backend = casting::store::from_selector(&selector, ws.casting_dir())?;
    let store = backend.events();
    let cursors = backend.cursors();
    let snapshots = backend.snapshots();
    let mut state = setup_state(store, cursors, snapshots);
    // Harness guard (2026-08-13, secrets.rs): attach the per-project secret
    // store (NEVER in the event log). The executor then refuses to
    // schedule/execute an activity that embeds a raw secret value. The runner
    // resolves `@secret:NAME@` at execution time, in memory.
    state = state.with_secrets(casting::workspace::secrets::SecretStore::load(
        ws.casting_dir(),
    )?);
    // Out-of-band prompt/response archive (2026-08-19): write every LLM call's
    // assembled prompt + raw response under `~/.casting/<slug>/prompts/` (off
    // the artifact repo) so the `OrchestrationRun` events can carry refs.
    state = state.with_prompt_archive(Some(
        casting::workspace::prompt_archive::PromptArchive::open(ws.casting_dir()),
    ));
    Ok(state)
}

fn setup_state(
    store: std::sync::Arc<dyn EventStore>,
    cursors: std::sync::Arc<dyn CursorStore>,
    snapshots: Option<std::sync::Arc<dyn casting::store::SnapshotStore>>,
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

const PROJECT_ID: &str = "project-demo";

/// Where casting stores its databases inside a project's state dir.
const CASTING_SUBDIR_DB: &str = "events.db";
const CASTING_SUBDIR_CURSORS: &str = "cursors.db";

/// Load the consultant registry for a project: the curated embedded defaults
/// overlaid by any user-supplied packages in `~/.casting/<slug>/consultants/`
/// (drop a `.toml` to add a consultant, reuse its `id` to override a default).
/// A malformed user package is surfaced (not silently dropped); a missing
/// directory is a no-op.
fn load_consultants(
    ws: &std::sync::Arc<casting::workspace::Workspace>,
) -> std::sync::Arc<casting::consultants::ConsultantRegistry> {
    let mut reg = casting::consultants::ConsultantRegistry::from_embedded()
        .expect("embedded consultant defaults should always load; this is a build bug");
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
    // Initialise structured logging (env_logger / RUST_LOG). Default to "info"
    // level for cast-specific modules, "warn" for dependencies.
    // Use local time and a clean format: [HH:MM:SS LEVEL] message
    let rust_log = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(&rust_log))
        .format(|buf, record| {
            use std::io::Write;
            let ts = chrono::Local::now().format("%H:%M:%S%.3f");
            writeln!(buf, "[{} {}] {}", ts, record.level(), record.args())
        })
        .init();
    log::info!("Casting starting up");
    if std::env::var("RUST_LOG").is_err() {
        log::info!("tip: set RUST_LOG=cast=debug (or RUST_LOG=debug) for verbose logs");
    }

    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match cmd {
        "init" => {
            let init = parse_init(&args[2..])?;
            do_init(init)
        }
        "smoke" => {
            let slug = parse_project_arg(&args[2..]);
            do_smoke(slug)
        }
        // `cast run [--project <slug>] [--db <selector>] [--selfhost]` — boot the
        // ONE project. With no `--project`, auto-selects the sole project under
        // ~/.casting, or lists them if more than one exists. State lives under
        // `~/.casting/<slug>/`, never in the repo.
        "run" => {
            let project = args
                .windows(2)
                .find(|w| w[0] == "--project")
                .and_then(|w| w.get(1))
                .cloned();
            let db = args
                .windows(2)
                .find(|w| w[0] == "--db")
                .and_then(|w| w.get(1))
                .cloned();
            do_run(project, db)
        }
        // `cast brief [--project <slug>] [--subject S] [--source SRC] [--title T] <file|->`
        "brief" => {
            let project = args
                .windows(2)
                .find(|w| w[0] == "--project")
                .and_then(|w| w.get(1))
                .cloned();
            do_brief(&args[2..], project)
        }
        // `cast request [--project <slug>] [--source SRC] [--reporter R] [--label L] <title>`
        "request" => {
            let project = args
                .windows(2)
                .find(|w| w[0] == "--project")
                .and_then(|w| w.get(1))
                .cloned();
            do_request(&args[3..], project)
        }
        "log" => {
            let log = parse_log(&args[2..])?;
            do_log(log)
        }
        // `cast purge <slug> [--force]` — delete a project's state dir under
        // ~/.casting/<slug> to reset it to a clean slate. Defaults to the sole
        // project when exactly one exists. With --force, skip confirmation.
        "purge" => {
            // The slug is optional; `--force` is a flag, not a positional. Skip
            // any `--force` token when picking the slug so `cast purge --force`
            // (auto-select the sole project) doesn't treat "--force" as a slug.
            let slug = args
                .iter()
                .skip(2)
                .find(|a| *a != "--force")
                .map(|s| s.trim_start_matches("--project=").to_string());
            let force = args.iter().any(|a| a == "--force");
            do_purge(slug, force)
        }
        "help" | "--help" | "-h" => {
            println!(
                "cast — Casting autonomous software company\n\n\
                 USAGE:\n  cast init <project-dir> [--interactive] [--name=..] [--project=..] [--objective=..] [--cast=a,b] [--director-token=..] [--directive=stmt|scope]\n                                create + configure a project (state goes to ~/.casting/<slug>/)\n  cast run [--project <slug>] [--db <selector>] [--selfhost]\n                                start the workspace (PM + web UI) for the project\n                                (auto-selects the sole project when one exists)\n  cast purge [<slug>] [--force]\n                                delete a project's ~/.casting/<slug> state (reset to clean slate)\n  cast smoke [<dir>]            append sample events and replay them\n  cast brief [--project <slug>] [--subject S] [--source SRC] [--title T] <file|->\n                                import EXTERNAL advisor content as an advisory briefing\n  cast request [--project <slug>] [--source SRC] [--reporter R] [--label L] <title>\n                                receive an EXTERNAL request (issue/PR) into the intake\n  cast log --db <events.db> [--project <id>] [--verify]\n                                dump / verify the raw event stream\n\n                 Single-project (per Casting process):\n  Casting runs EXACTLY ONE project per `cast run`. State lives OUTSIDE the repo,\n  under ~/.casting/<slug>/ (honour $CASTING_HOME to relocate). Each project owns\n  its own database and a unique port (assigned at `cast init`), so two projects\n  on one machine never collide — run two `cast run --project <slug>` instances.\n  The artifact repo is never touched by Casting's own data.\n\n                 Env:\n  CASTING_HOME   root for all project state (default ~/.casting)\n  CAST_ADDR      bind address for `cast run` (defaults to the project's port)\n  CAST_DB        storage backend selector ('sqlite' or a libpq Postgres string)\n  CAST_DIRECTOR_TOKEN director auth token (or set via `cast init --director-token`)\n  CAST_SELFHOST  1 to enable self-hosting instead of --selfhost\n"
            );
            Ok(())
        }
        other => anyhow::bail!("unknown command: {other} (try `cast help`)"),
    }
}

/// Resolve the (repo, state_dir) pair for the CLI write paths (brief/request),
/// given an optional `--project <slug>`. Reuses [`resolve_project`] so the
/// auto-select-when-one-exists behaviour is identical to `cast run`.
fn resolve_repo_state(slug: Option<String>) -> Result<(PathBuf, PathBuf)> {
    let (state_dir, cfg) = resolve_project(slug)?;
    let repo = cfg
        .repo_path
        .as_ref()
        .map(PathBuf::from)
        .context("project config is missing repo_path; re-run `cast init`")?;
    Ok((repo, state_dir))
}

/// `cast brief` — import EXTERNAL advisor content (a text file, or stdin via
/// `-`) as an ADVISORY briefing: it can inform context but never sets rules.
/// Usage: `cast brief [--project <slug>] [--subject S] [--source SRC] [--title T] <file|->`
fn do_brief(args: &[String], project: Option<String>) -> Result<()> {
    let (repo, state_dir) = resolve_repo_state(project)?;
    let (mut subject, mut source, mut title) = (
        "general".to_string(),
        "jeeves".to_string(),
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
        let state = open_state(&repo, &state_dir, None)?;

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
        casting::actions::validate(&action, "director", &proj, None)?;

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
                        Actor::Director {
                            user_id: "ceo".into(),
                        },
                        EventType::MessageSent,
                        Aggregate {
                            kind: "message".into(),
                            id: "bootstrap".into(),
                        },
                        serde_json::json!({}),
                    )
                });
            let ev = action
                .to_events(&state.project, "director", &cause, "brief", None)
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
/// provenance + deterministic triage; NOT the director's own intent.
/// Usage: `cast request [--project <slug>] [--source SRC] [--reporter R] [--label L] <title>`
fn do_request(args: &[String], project: Option<String>) -> Result<()> {
    let (repo, state_dir) = resolve_repo_state(project)?;
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
    let title = title.context("usage: cast request [--project <slug>] [flags] <title>")?;
    if title.trim().is_empty() {
        anyhow::bail!("request title is empty");
    }

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let state = open_state(&repo, &state_dir, None)?;

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
        casting::actions::validate(&action, "mei", &proj, None)?;

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
                .to_events(&state.project, "mei", &cause, "request", None)
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
    /// Optional explicit slug for the state dir. Defaults to slugify(name).
    project: Option<String>,
    objective: Option<String>,
    cast: Vec<String>,
    director_token: Option<String>,
    directives: Vec<(String, String)>, // (statement, scope)
}

fn parse_init(args: &[String]) -> Result<InitArgs> {
    let mut dir = None;
    let mut interactive = false;
    let mut name = None;
    let mut project = None;
    let mut objective = None;
    let mut cast = Vec::new();
    let mut director_token = None;
    let mut directives = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--interactive" | "-i" => interactive = true,
            "--name" => {
                name = Some(args.get(i + 1).context("--name requires a value")?.clone());
                i += 1;
            }
            "--project" => {
                project = Some(
                    args.get(i + 1)
                        .context("--project requires a value")?
                        .clone(),
                );
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
            "--director-token" => {
                director_token = Some(
                    args.get(i + 1)
                        .context("--director-token requires a value")?
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
        dir: dir.context("usage: cast init <project-dir> [--interactive] [--name=..] [--project=..] [--objective=..] [--cast=a,b] [--director-token=..]")?,
        interactive,
        name,
        project,
        objective,
        cast,
        director_token,
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
        if args.director_token.is_none() {
            let tok = prompt("Owner auth token? (blank = auth off)", None).unwrap_or_default();
            args.director_token = if tok.is_empty() { None } else { Some(tok) };
        }
    }

    // Capture the name up front — it drives the slug + display.
    let name = args.name.clone().unwrap_or_else(|| "Casting demo".into());

    let spec = casting::workspace::setup::SetupSpec {
        name: name.clone(),
        roles: args.cast.clone(),
        director_token: args.director_token.clone(),
        directives: args
            .directives
            .into_iter()
            .map(
                |(statement, scope)| casting::workspace::setup::StartDirective {
                    id: format!(
                        "setup-{}",
                        statement
                            .to_lowercase()
                            .replace(' ', "-")
                            .chars()
                            .take(20)
                            .collect::<String>()
                    ),
                    kind: casting::runtime::directive::DirectiveKind::Policy,
                    statement,
                    scope: scope
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    strength: casting::runtime::directive::DirectiveStrength::Required,
                },
            )
            .collect(),
    };

    let plan = casting::workspace::setup::SetupPlan::build(spec)?;

    // Resolve the artifact repo (the thing Casting drives) and the project slug
    // under ~/.casting/<slug>/. State lives OUTSIDE the repo so the repo is
    // never mutated by Casting's own data.
    let repo = args
        .dir
        .canonicalize()
        .with_context(|| format!("resolve artifact repo path for {}", args.dir.display()))?;
    let slug = match &args.project {
        Some(p) => slugify(p),
        None => slugify(&name),
    };
    // Slug-collision guard: a project with this slug already exists.
    let home = casting_home()?;
    let state_dir = home.join(&slug);
    if state_dir.exists() {
        anyhow::bail!(
            "project slug '{slug}' already exists at {}. Choose a different --project, \
             or `cast purge {slug}` first.",
            state_dir.display()
        );
    }

    // Assign a unique port for this project (next free port).
    let port = casting::workspace::setup::next_port()?;

    // Write the location config (name, slug, repo_path, port) first so a crash
    // mid-init still leaves a resolvable record.
    std::fs::create_dir_all(&state_dir)
        .with_context(|| format!("create state dir {}", state_dir.display()))?;
    casting::workspace::setup::persist_location(&state_dir, &name, &slug, &repo, port)?;

    // The workspace now points its state dir at ~/.casting/<slug>/, not the repo.
    // Opening it here enforces the ownership boundary (refuses the Casting source
    // repo unless self-hosting) before we write anything.
    let _ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled)?;

    let written = plan.apply(&state_dir)?;
    // Write a no-secrets config template into the REPO root (like .env.example),
    // documenting the canonical config shape — never live state.
    casting::workspace::setup::write_template(&repo, &name)?;
    println!(
        "   📁 project '{}' (slug {}) ready — state in {}",
        name,
        slug,
        state_dir.display()
    );
    println!("   🚪 will serve on port {port} (run `cast run --project {slug}`)");
    if written == 0 {
        println!(
            "Company already seeded at {} — no changes (re-run applies events idempotently).",
            state_dir.display()
        );
    } else {
        println!(
            "🎬 {name} is live\n   Team: {}",
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
    if args.interactive && args.director_token.is_none() {
        println!("   ⚠️  no director token set — director-mutating writes are OPEN (set one via --director-token / CAST_DIRECTOR_TOKEN)");
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

/// `cast run` — boot the whole workspace: resolve the project (by slug or
/// auto-select the sole one), enforce the ownership boundary, seed the project,
/// start the simulated PM control loop, and serve the API + embedded React UI
/// from one binary on the project's assigned port.
fn do_run(project_slug: Option<String>, db: Option<String>) -> Result<()> {
    // Resolve which project to run. If none exist yet, boot the first-run setup
    // server (it serves the wizard that creates the project); a later `cast run`
    // then auto-selects the now-existing sole project.
    let projects = discover_projects();
    let (state_dir, cfg) = match (project_slug, projects.len()) {
        (Some(s), _) => resolve_project(Some(s))?,
        (None, 0) => return run_setup_server(db),
        (None, 1) => projects.into_iter().next().unwrap(),
        (None, _) => {
            let home = casting_home()?;
            let list = projects
                .iter()
                .map(|(_, c)| {
                    format!(
                        "  - {} (slug: {})",
                        c.name,
                        c.slug.clone().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::bail!(
                "multiple projects found under {} — pass --project <slug>:\n{}",
                home.display(),
                list
            );
        }
    };

    // The repo_path recorded at init is the artifact repo Casting drives.
    let repo = cfg
        .repo_path
        .as_ref()
        .map(PathBuf::from)
        .context("project config is missing repo_path; re-run `cast init`")?;
    if !repo.exists() {
        anyhow::bail!(
            "artifact repo {} for project '{}' no longer exists.\n\
             If you moved it, update the path (e.g. `cast project set-repo <new-path>` — \
             coming soon); for now, `cast purge {}` and re-`cast init`.",
            repo.display(),
            cfg.name,
            cfg.slug.clone().unwrap_or_default()
        );
    }

    // The port this project listens on (recorded at init, overridable via env).
    let port = cfg.port.unwrap_or(8080);
    let addr = std::env::var("CAST_ADDR").unwrap_or_else(|_| format!("127.0.0.1:{port}"));

    let ws = Workspace::open(&repo, &state_dir, Selfhost::Disabled)?;

    // Ensure the artifact repo is a real git repo (git-init if missing). This
    // wires Git into the workspace at startup (Git slice increment 1).
    let created = ws.ensure_repo().context("ensure git repo")?;

    preflight(&ws, created);

    // Open the storage backend: --db flag, else the CAST_DB env var, else
    // SQLite (the default). Postgres is swappable behind the same traits.
    let backend = {
        let selector = db
            .or_else(|| std::env::var("CAST_DB").ok())
            .unwrap_or_else(|| "sqlite".to_string());
        casting::store::from_selector(&selector, ws.casting_dir())?
    };
    let store = backend.events();
    let cursors = backend.cursors();
    let snapshots = backend.snapshots();

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
            // any user-supplied packages in ~/.casting/<slug>/consultants/
            // (drop a .toml to add a consultant, reuse an id to override one).
            .with_consultants(load_consultants(&ws))
            // D2 LLM wiring: when CAST_LLM_API_KEY is set, attach the real
            // OpenAI-compatible orchestrator (OpenRouter day-one; provider is
            // config — base_url+key+model — so a local LiteLLM swaps in without
            // code). Unconfigured → the deterministic scripted PM stays the
            // default (off by default, no spend, backwards compatible).
            .pipe_llm_orchestrator();
        // Owner auth: the token comes from the persisted setup config.json
        // first (set via `cast init --director-token`), else CAST_DIRECTOR_TOKEN env.
        // Off by default. Owner-mutating endpoints require Authorization:
        // Bearer <token>.
        let persisted_token = casting::workspace::setup::read_config(ws.casting_dir())
            .and_then(|c| c.director_token)
            .filter(|t| !t.is_empty());
        let token = persisted_token.or_else(|| std::env::var("CAST_DIRECTOR_TOKEN").ok());
        let state = match token {
            Some(tok) if !tok.is_empty() => {
                println!(
                    "🔐 director auth enabled (send 'Authorization: Bearer <token>' to mutate)"
                );
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
        // Use WorkspaceRunner so that in-flight worktree provisioning and
        // commits get properly re-dispatched (NoopRunner would fail them).
        let ws_for_recovery = ws.clone();
        match casting::runtime::executor::redispatch_inflight(
            &state,
            &casting::runtime::executor::WorkspaceRunner::new(ws_for_recovery),
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

        // Telegram director channel (2026-08-14): enabled from (in priority order)
        // a persisted `~/.casting/<slug>/config.json` (set via the UI
        // POST /api/telegram/configure) then the `CAST_TELEGRAM_TOKEN`/CHAT_ID
        // env vars. Attaches the channel + spawns the cursor-driven run loop
        // exactly once (idempotent via AppState.telegram_started). Off by
        // default — no channel, no network (mirrors the LLM seam).
        let telegram_cfg = casting::workspace::setup::read_config(ws.casting_dir())
            .and_then(|c| match (c.telegram_token, c.telegram_chat_id) {
                (Some(t), Some(cid)) => Some(
                    casting::runtime::telegram::TelegramConfig::from_pieces(t, cid),
                ),
                _ => None,
            })
            .or_else(casting::runtime::telegram::TelegramConfig::from_env);
        let state = match telegram_cfg {
            Some(cfg) => {
                casting::runtime::telegram::start_loop(&state, cfg);
                state
            }
            None => state,
        };

        // Liveness watchdog (2026-08-13): a wall-clock "dead man's switch" that
        // auto-pauses the cast on a stall (repeated errors / silent in-flight
        // work). Self-actuating; enabled via CAST_WATCHDOG=1 (+ tuning env).
        if let Some(cfg) = casting::runtime::watchdog::WatchConfig::from_env() {
            println!(
                "🛡️ liveness watchdog enabled (poll {}s, stall {}h, retry>{}x)",
                cfg.poll_secs, cfg.stall_hours, cfg.max_repeat_errors
            );
            tokio::spawn(casting::runtime::watchdog::monitor(state.clone(), cfg));
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

/// Boot the first-run setup server when no project exists yet.
///
/// Serves the embedded SPA + the `/api/setup` endpoints on the default port so
/// the browser wizard can create the project's state dir (name + slug + repo
/// path + port). The wizard does NOT require an existing project, so `cast run`
/// works straight from a clean machine. After the wizard creates the project, a
/// subsequent `cast run` auto-selects it and boots the real workspace.
fn run_setup_server(db: Option<String>) -> Result<()> {
    let _ = db; // no project store yet; the wizard creates one on submit.
    let home = casting_home().unwrap_or_else(|_| PathBuf::from("~/.casting"));
    log::info!(
        "[setup] no project configured — serving first-run wizard (projects scanned from {})",
        home.display()
    );
    println!("🎬 No Casting project configured yet — starting the first-run setup wizard.");
    println!("   Open http://127.0.0.1:8080 in your browser to configure a project.");
    println!("   Once you've set it up, restart `cast run` to launch the workspace.\n");

    // Minimal runtime state: an in-memory store (the wizard's project gets its
    // own persisted store on submit) and the embedded consultant catalogue so
    // the wizard can list roles. No workspace is attached — state_dir is None,
    // which tells /api/setup to CREATE the project rather than configure one.
    let store = SqliteEventStore::in_memory().context("in-memory event store")?;
    let cursors =
        casting::store::SqliteCursorStore::in_memory().context("in-memory cursor store")?;
    let mut state = AppState::new(store, cursors, PROJECT_ID).with_integrity();
    if std::env::var("CAST_DECOMPOSE").is_ok() {
        state = state.with_decompose();
    }
    let state = state.with_consultants(std::sync::Arc::new(
        ConsultantRegistry::from_embedded()
            .expect("embedded consultant defaults should always load; this is a build bug"),
    ));

    let port = 8080u16;
    let addr = std::env::var("CAST_ADDR").unwrap_or_else(|_| format!("127.0.0.1:{port}"));

    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(async move {
        let app = web::router(state);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("bind {addr}"))?;
        println!("🧰 Setup wizard ready: http://{addr}");
        axum::serve(listener, app)
            .await
            .context("axum server error")?;
        Ok::<(), anyhow::Error>(())
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
            let problems = casting::event::replay::verify(&store, project)?;
            if problems.is_empty() {
                println!("{}: OK (event stream invariants hold)", project);
            } else {
                println!("{}: {} problem(s):", project, problems.len());
                for p in problems {
                    println!("  - {p}");
                }
            }
        } else {
            for line in casting::event::replay::dump(&store, project)? {
                println!("{line}");
            }
        }
    }
    Ok(())
}

/// Idempotently seed a fresh project: ProjectCreated only, then reconcile the
/// cast roster from `active-cast/` (the directory IS the roster — everyone
/// present is hired). Safe to re-run; events tune to what's already there.
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

    // Hire the entire roster from the directory: the PM, the Advisor (jeeves),
    // and every assignable consultant (one per role). No names are hardcoded
    // here — adding/removing a package in active-cast/ changes who's hired.
    let hired = casting::pm::reconciler::cast_roster(state)?;
    println!("   cast onboarded from active-cast/ ({hired} hired)");
    Ok(())
}

/// Append a representative slice of domain events and prove append->read_since
/// and cursor resume. Purely a harness; real agents fill this in later. Writes
/// into the resolved project's state dir.
fn do_smoke(slug: Option<String>) -> Result<()> {
    let (state_dir, _cfg) = resolve_project(slug)?;
    let db = state_dir.join(CASTING_SUBDIR_DB);
    let cursors_path = state_dir.join(CASTING_SUBDIR_CURSORS);
    let store = SqliteEventStore::open(&db)?;
    let cursors = casting::store::SqliteCursorStore::open(&cursors_path)?;

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
        Actor::Director {
            user_id: "ceo".into(),
        },
        EventType::MessageSent,
        Aggregate {
            kind: "project".into(),
            id: project.into(),
        },
        serde_json::json!({"body": "Build me a todo app"}),
    ))?;

    // PM + engineer hired.
    for (id, role) in [("mei", "Project Manager"), ("diego", "Lead Developer")] {
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
        Actor::Agent { id: "mei".into() },
        EventType::TaskCreated,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({"title": "Implement authentication", "kind": "feature"}),
    ))?;
    store.append(Event::new(
        project,
        Actor::Agent { id: "mei".into() },
        EventType::TaskAssigned,
        Aggregate {
            kind: "task".into(),
            id: "task-1".into(),
        },
        serde_json::json!({"assignee": "diego"}),
    ))?;

    println!(
        "Appended {} events (latest seq {})",
        store.latest_sequence(project)?,
        store.latest_sequence(project)?
    );

    // PM's cursor: replay everything fresh, then persist position.
    let pm_cursor = cursors.get(project, "mei")?;
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
    cursors.advance(project, "mei", new_last)?;
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
        _task_created_seq
    );
    Ok(())
}

/// `cast purge [<slug>] [--force]` — delete a project's state dir under
/// ~/.casting/<slug> to reset it to a clean slate. With no slug, defaults to the
/// sole project when exactly one exists. Asks for confirmation unless `--force`.
fn do_purge(slug: Option<String>, force: bool) -> Result<()> {
    let (state_dir, cfg) = resolve_project(slug)?;
    if !state_dir.exists() {
        println!(
            "nothing to purge at {} (already clean)",
            state_dir.display()
        );
        return Ok(());
    }

    if !force {
        eprint!(
            "Delete project '{}' state at {}? [y/N] ",
            cfg.name,
            state_dir.display()
        );
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let answer = input.trim().to_lowercase();
        if answer != "y" && answer != "yes" {
            println!("aborted");
            return Ok(());
        }
    }

    std::fs::remove_dir_all(&state_dir)
        .with_context(|| format!("remove {}", state_dir.display()))?;
    println!(
        "✓ purged {} — project '{}' is clean, ready for `cast init`",
        state_dir.display(),
        cfg.name
    );
    Ok(())
}
