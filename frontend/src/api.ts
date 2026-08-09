// Types mirroring the Rust projection + event JSON shapes (see src/projection.rs).
export type TaskStatus = "backlog" | "working" | "blocked" | "done";

export interface Agent {
  id: string;
  role: string;
}

export interface Requirement {
  id: string;
  title: string;
  description: string;
}

export interface Task {
  id: string;
  title: string;
  kind: string;
  status: TaskStatus;
  assignee: string | null;
}

export type DecisionStatus = "proposed" | "approved" | "rejected";

export interface Decision {
  id: string;
  subject: string;
  options: Record<string, string>;
  recommendation: string | null;
  status: DecisionStatus;
  owner_verdict: string | null;
}

export interface Message {
  id: string;
  from: string;
  to: string;
  body: string;
}

export interface Observation {
  id: string;
  from: string;
  severity: string;
  subject: string;
  body: string;
}

export interface Branch {
  name: string;
  task_id: string | null;
}

export interface Commit {
  sha: string;
  branch: string;
  message: string;
  author: string;
  task_id: string | null;
}

export interface Merge {
  sha: string;
  from_branch: string;
  to_branch: string;
}

export interface Projection {
  project_id: string;
  agents: Agent[];
  requirements: Requirement[];
  tasks: Task[];
  decisions: Decision[];
  messages: Message[];
  observations: Observation[];
  branches: Branch[];
  commits: Commit[];
  merges: Merge[];
}

export interface InboxItem {
  id: string;
  subject: string;
  recommendation: string | null;
  options: Record<string, string>;
}

export interface Inbox {
  items: InboxItem[];
  unread: number;
}

async function j<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  if (!res.ok) {
    throw new Error(`${res.status} ${await res.text()}`);
  }
  return res.json() as Promise<T>;
}

export function fetchState(): Promise<Projection> {
  return j<Projection>("/api/state");
}

export function fetchInbox(): Promise<Inbox> {
  return j<Inbox>("/api/inbox");
}

export async function sendMessage(body: string): Promise<void> {
  await j("/api/message", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ body }),
  });
}

export async function decide(
  decision_id: string,
  subject: string,
  approved: boolean,
  note?: string
): Promise<void> {
  await j("/api/decision", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ decision_id, subject, approved, note: note ?? "" }),
  });
}

/// Subscribe to the realtime event stream. Calls `onEvent` for each event; the
/// caller decides what to refetch. On reconnect, passes `?after=<lastSeq>` so
/// the server replays any events missed while disconnected (SSE catch-up).
/// Returns an unsubscribe function.
export function subscribe(onEvent: () => void): () => void {
  let lastSeq = 0;
  let closed = false;

  const connect = () => {
    if (closed) return;
    const url =
      lastSeq > 0
        ? `/api/events/stream?after=${lastSeq}`
        : "/api/events/stream";
    const es = new EventSource(url);
    es.addEventListener("event", (raw: MessageEvent) => {
      try {
        const ev = JSON.parse(raw.data);
        if (typeof ev.sequence === "number" && ev.sequence > lastSeq) {
          lastSeq = ev.sequence;
        }
      } catch {
        // Malformed payload — ignore, the caller will refetch state anyway.
      }
      onEvent();
    });
    es.onerror = () => {
      // EventSource auto-reconnects, but the browser may not re-add the query
      // param. Close and reconnect explicitly so catch-up `?after=N` is sent.
      es.close();
      if (!closed) {
        setTimeout(connect, 1000);
      }
    };
  };

  connect();
  return () => {
    closed = true;
  };
}
