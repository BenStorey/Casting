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

export interface Projection {
  project_id: string;
  agents: Agent[];
  requirements: Requirement[];
  tasks: Task[];
  decisions: Decision[];
  messages: Message[];
  observations: Observation[];
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
/// caller decides what to refetch. Returns an unsubscribe function.
export function subscribe(onEvent: () => void): () => void {
  const es = new EventSource("/api/events/stream");
  es.addEventListener("event", () => onEvent());
  es.onerror = () => {
    /* EventSource auto-reconnects */
  };
  return () => es.close();
}
