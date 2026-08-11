//! Postgres append-only event store + cursors + snapshots.
//!
//! Implements the same storage traits as the SQLite backend
//! (`EventStore`, `CursorStore`, `SnapshotStore`), so a deployment can run its
//! event log on Postgres behind the abstraction — the storage seam the owner
//! wanted. Schema mirrors SQLite (`events` / `cursors` / `projections` tables),
//! with Postgres-flavored DDL.
//!
//! Threading: the async tokio-postgres driver runs on a DEDICATED background
//! thread that owns its own current-thread runtime and connection. Each store
//! method sends a `Job` (boxed, typed) to that thread — carrying an async
//! closure that owns a cheap `Client` clone — and blocks on the result over a
//! std channel. This keeps the store traits synchronous and lets Postgres work
//! from any caller thread (incl. our tokio server), with no nested-runtime
//! problem.

use crate::cursor::Cursor;
use crate::event::{Actor, Aggregate, Event, Metadata};
use crate::projection::Projection;
use crate::snapshot::SnapshotStore;
use crate::store::EventStore;
use anyhow::{anyhow, Result};
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use tokio_postgres::{Client, NoTls};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A job closure: owns a `Client` handle, returns a pinned async result.
type JobFn<T> = Box<dyn FnOnce(Arc<Client>) -> BoxFuture<'static, Result<T>> + Send + 'static>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS events (
    event_id      TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL,
    sequence      BIGINT NOT NULL,
    timestamp     TIMESTAMPTZ NOT NULL,
    actor_type    TEXT NOT NULL,
    actor_id      TEXT,
    event_type    TEXT NOT NULL,
    aggregate_kind TEXT NOT NULL,
    aggregate_id  TEXT NOT NULL,
    data          TEXT NOT NULL,
    correlation_id TEXT,
    causation_id  TEXT,
    agent_run_id  TEXT,
    UNIQUE (project_id, sequence)
);
CREATE INDEX IF NOT EXISTS idx_events_project_sequence
    ON events (project_id, sequence);

CREATE TABLE IF NOT EXISTS cursors (
    project_id  TEXT NOT NULL,
    consumer    TEXT NOT NULL,
    last_seen   BIGINT NOT NULL,
    PRIMARY KEY (project_id, consumer)
);

CREATE TABLE IF NOT EXISTS projections (
    project_id   TEXT NOT NULL PRIMARY KEY,
    sequence     BIGINT NOT NULL,
    projection   TEXT NOT NULL
);
"#;

/// A boxed, type-erased unit of work run on the background connection thread.
/// `run` drives the async closure on that thread's runtime; the closure owns a
/// `Client` handle via its concrete `JobInner` receiver.
trait Job: Send {
    fn run(self: Box<Self>, client: std::sync::Arc<tokio_postgres::Client>);
}

/// Concrete `Job`: holds the typed reply channel and an async closure that
/// borrows the passed `Client`.
struct JobInner<T: Send + 'static> {
    f: JobFn<T>,
    reply: Sender<Result<T>>,
}

impl<T: Send + 'static> Job for JobInner<T> {
    fn run(self: Box<Self>, client: Arc<Client>) {
        let fut = (self.f)(client);
        let res = tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut));
        let _ = self.reply.send(res);
    }
}

/// Postgres backend behind the storage traits, driven on a dedicated thread.
#[derive(Clone)]
pub struct PostgresBackend {
    tx: Arc<Sender<Box<dyn Job>>>,
}

impl PostgresBackend {
    /// Connect to Postgres at `config` (a libpq connection string), applying
    /// the schema, and spawn the dedicated connection thread.
    pub fn connect(config: &str) -> Result<Self> {
        let (tx, rx) = channel::<Box<dyn Job>>();
        let config = config.to_string();

        std::thread::Builder::new()
            .name("casting-postgres".to_string())
            .spawn(move || -> Result<()> {
                // A multi-thread runtime lets block_in_place park the caller
                // thread while the connection task runs on another worker.
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|e| anyhow!(e.to_string()))?;

                rt.block_on(async move {
                    let (client, conn) = tokio_postgres::connect(&config, NoTls)
                        .await
                        .map_err(|e| anyhow!(e.to_string()))?;
                    let client = std::sync::Arc::new(client);
                    let conn_client = client.clone();
                    tokio::spawn(async move {
                        if let Err(e) = conn.await {
                            eprintln!("[casting-postgres] connection error: {e}");
                        }
                        // keep conn_client alive for the connection's lifetime? no-op
                        drop(conn_client);
                    });
                    client
                        .batch_execute(SCHEMA)
                        .await
                        .map_err(|e| anyhow!(e.to_string()))?;

                    // Serve jobs forever.
                    while let Ok(job) = rx.recv() {
                        job.run(client.clone());
                    }
                    Ok(())
                })
            })
            .map_err(|e| anyhow!("spawn postgres thread: {e}"))?;

        Ok(PostgresBackend { tx: Arc::new(tx) })
    }

    /// Submit a typed job and block for its result.
    fn call<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Arc<Client>) -> BoxFuture<'static, Result<T>> + Send + 'static,
    {
        let (reply, rx) = channel::<Result<T>>();
        let job = JobInner {
            f: Box::new(f),
            reply,
        };
        self.tx
            .send(Box::new(job))
            .map_err(|_| anyhow!("postgres connection thread gone"))?;
        rx.recv()
            .map_err(|_| anyhow!("postgres reply channel closed"))?
    }
}

impl EventStore for PostgresBackend {
    fn append(&self, mut event: Event) -> Result<Event> {
        self.call(move |client| {
            Box::pin(async move {
                let max: Option<i64> = client
                    .query_one(
                        "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE project_id = $1",
                        &[&event.project_id],
                    )
                    .await
                    .ok()
                    .map(|r| r.get(0));
                event.sequence = max.unwrap_or(0) + 1;

                let (actor_type, actor_id) = match &event.actor {
                    Actor::Owner => ("owner", None),
                    Actor::Agent { id } => ("agent", Some(id.clone())),
                    Actor::System => ("system", None),
                };

                client
                    .execute(
                        "INSERT INTO events (
                        event_id, project_id, sequence, timestamp,
                        actor_type, actor_id, event_type,
                        aggregate_kind, aggregate_id, data,
                        correlation_id, causation_id, agent_run_id
                    ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                        &[
                            &event.event_id.to_string(),
                            &event.project_id,
                            &event.sequence,
                            &event.timestamp,
                            &actor_type,
                            &actor_id,
                            &serde_json::to_string(&event.event_type)?,
                            &event.aggregate.kind,
                            &event.aggregate.id,
                            &serde_json::to_string(&event.data)?,
                            &event.metadata.correlation_id,
                            &event.metadata.causation_id.map(|id| id.to_string()),
                            &event.metadata.agent_run_id,
                        ],
                    )
                    .await
                    .map_err(|e| anyhow!("insert event seq {}: {e}", event.sequence))?;

                Ok(event)
            })
        })
    }

    fn read_since(&self, project_id: &str, after: i64) -> Result<Vec<Event>> {
        let project_id = project_id.to_string();
        self.call(move |client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        "SELECT event_id, project_id, sequence, timestamp,
                            actor_type, actor_id, event_type,
                            aggregate_kind, aggregate_id, data,
                            correlation_id, causation_id, agent_run_id
                     FROM events
                     WHERE project_id = $1 AND sequence > $2
                     ORDER BY sequence ASC",
                        &[&project_id, &after],
                    )
                    .await
                    .map_err(|e| anyhow!("read events: {e}"))?;

                let mut out = Vec::with_capacity(rows.len());
                for r in rows {
                    let actor_type: String = r.get(4);
                    let actor_id: Option<String> = r.get(5);
                    let actor = match actor_type.as_str() {
                        "owner" => Actor::Owner,
                        "system" => Actor::System,
                        _ => Actor::Agent {
                            id: actor_id.unwrap_or_default(),
                        },
                    };
                    out.push(Event {
                        event_id: uuid::Uuid::parse_str(&r.get::<_, String>(0)).unwrap(),
                        project_id: r.get(1),
                        sequence: r.get(2),
                        timestamp: r.get(3),
                        actor,
                        event_type: serde_json::from_str(&r.get::<_, String>(6)).unwrap(),
                        aggregate: Aggregate {
                            kind: r.get(7),
                            id: r.get(8),
                        },
                        data: serde_json::from_str(&r.get::<_, String>(9)).unwrap(),
                        metadata: Metadata {
                            correlation_id: r.get(10),
                            causation_id: r
                                .get::<_, Option<String>>(11)
                                .and_then(|s| uuid::Uuid::parse_str(&s).ok()),
                            agent_run_id: r.get(12),
                        },
                    });
                }
                Ok(out)
            })
        })
    }

    fn latest_sequence(&self, project_id: &str) -> Result<i64> {
        let project_id = project_id.to_string();
        self.call(move |client| {
            Box::pin(async move {
                let row = client
                    .query_one(
                        "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE project_id = $1",
                        &[&project_id],
                    )
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
                Ok(row.get(0))
            })
        })
    }

    fn list_projects(&self) -> Result<Vec<String>> {
        self.call(|client| {
            Box::pin(async move {
                let rows = client
                    .query(
                        "SELECT DISTINCT project_id FROM events ORDER BY project_id ASC",
                        &[],
                    )
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
                Ok(rows.iter().map(|r| r.get(0)).collect())
            })
        })
    }
}

impl crate::cursor::CursorStore for PostgresBackend {
    fn get(&self, project_id: &str, consumer: &str) -> Result<Cursor> {
        let project_id = project_id.to_string();
        let consumer = consumer.to_string();
        self.call(move |client| {
            Box::pin(async move {
                let row = client
                    .query_opt(
                        "SELECT last_seen FROM cursors WHERE project_id = $1 AND consumer = $2",
                        &[&project_id, &consumer],
                    )
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
                Ok(Cursor {
                    project_id,
                    consumer,
                    last_seen: row.map(|r| r.get(0)).unwrap_or(0),
                })
            })
        })
    }

    fn advance(&self, project_id: &str, consumer: &str, to: i64) -> Result<()> {
        let project_id = project_id.to_string();
        let consumer = consumer.to_string();
        self.call(move |client| {
            Box::pin(async move {
                client
                    .execute(
                        "INSERT INTO cursors (project_id, consumer, last_seen)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (project_id, consumer)
                     DO UPDATE SET last_seen = EXCLUDED.last_seen",
                        &[&project_id, &consumer, &to],
                    )
                    .await
                    .map_err(|e| anyhow!("advance cursor: {e}"))?;
                Ok(())
            })
        })
    }
}

impl SnapshotStore for PostgresBackend {
    fn save(&self, project_id: &str, sequence: i64, projection: &Projection) -> Result<()> {
        let project_id = project_id.to_string();
        let json = serde_json::to_string(projection)?;
        self.call(move |client| {
            Box::pin(async move {
                client
                    .execute(
                        "INSERT INTO projections (project_id, sequence, projection)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (project_id) DO UPDATE SET sequence = EXCLUDED.sequence,
                                                            projection = EXCLUDED.projection",
                        &[&project_id, &sequence, &json],
                    )
                    .await
                    .map_err(|e| anyhow!("save snapshot: {e}"))?;
                Ok(())
            })
        })
    }

    fn load(&self, project_id: &str) -> Option<(i64, Projection)> {
        let project_id = project_id.to_string();
        let res = self.call(move |client| {
            Box::pin(async move {
                let row = client
                    .query_opt(
                        "SELECT sequence, projection FROM projections WHERE project_id = $1",
                        &[&project_id],
                    )
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
                let loaded = match row {
                    Some(r) => {
                        let seq: i64 = r.get(0);
                        let json: String = r.get(1);
                        match serde_json::from_str(&json) {
                            Ok(p) => Some((seq, p)),
                            Err(_) => None, // corrupt snapshot -> disposable
                        }
                    }
                    None => None,
                };
                Ok(loaded)
            })
        });
        res.ok().flatten()
    }

    fn clear(&self, project_id: &str) -> Result<()> {
        let project_id = project_id.to_string();
        self.call(move |client| {
            Box::pin(async move {
                client
                    .execute(
                        "DELETE FROM projections WHERE project_id = $1",
                        &[&project_id],
                    )
                    .await
                    .map_err(|e| anyhow!("clear snapshot: {e}"))?;
                Ok(())
            })
        })
    }
}
