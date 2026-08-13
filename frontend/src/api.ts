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

export type ChangeSetStatus = "open" | "ready" | "merged";

export interface ChangeSet {
  id: string;
  task_id: string;
  branch: string;
  commits: string[];
  agent: string | null;
  status: ChangeSetStatus;
}

export interface Projection {
  project_id: string;
  agents: Agent[];
  requirements: Requirement[];
  tasks: Task[];
  decisions: Decision[];
  messages: Message[];
  advisor_thread: Message[];
  observations: Observation[];
  branches: Branch[];
  commits: Commit[];
  merges: Merge[];
  changesets: ChangeSet[];
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

export function saveDiagram(title: string, data: string): Promise<unknown> {
  return j("/api/diagram", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ title, data }),
  });
}

export function sendToAdvisor(body: string): Promise<unknown> {
  return j("/api/advisor/message", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ body }),
  });
}

export function handoffAdvisor(summary: string, title?: string): Promise<unknown> {
  return j("/api/advisor/handoff", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ summary, title: title || "Advisor handoff" }),
  });
}

export interface EventEnvelope {
  event_id: string;
  project_id: string;
  sequence: number;
  timestamp: string;
  event_type: string;
  actor: string | { id: string };
  data: Record<string, unknown>;
}

export function fetchEvents(after = 0): Promise<EventEnvelope[]> {
  return j<EventEnvelope[]>(`/api/events?after=${after}`);
}

export interface SetupRole {
  id: string;
  title: string;
  scope: string;
}

export interface SetupStatus {
  configured: boolean;
  roles: SetupRole[];
}

export interface SetupResult {
  ok: boolean;
  hires: [string, string][];
  objective: string;
}

export function fetchSetupStatus(): Promise<SetupStatus> {
  return j<SetupStatus>("/api/setup/status");
}

export async function submitSetup(
  name: string,
  objective: string,
  cast: string[],
  ownerToken?: string
): Promise<SetupResult> {
  return j<SetupResult>("/api/setup", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name, objective, cast, owner_token: ownerToken || undefined }),
  });
}

export function fetchInbox(): Promise<Inbox> {
  return j<Inbox>("/api/inbox");
}

// ---- Operating picture (/api/model) -----------------------------------------
// Mirrors src/mental.rs OperatingModel + the context/plan shapes it embeds.

export type Priority = "low" | "medium" | "high" | "critical";

export interface PlannedItem {
  task_id: string;
  title: string;
  priority: Priority;
}

export interface ScoredItem {
  task_id: string;
  title: string;
  priority: string;
  status: string;
  is_mine: boolean;
  relevance: number;
}

export interface AgentContext {
  actor: string;
  objective: string | null;
  priorities: PlannedItem[];
  scored_priorities: ScoredItem[];
  my_tasks: string[];
  active_directives: string[];
  open_risks: string[];
  assumptions: string[];
  constraints: string[];
  open_decisions: string[];
  worktree: WorktreeInfo | null;
}

export interface WorktreeInfo {
  task_id: string;
  branch: string;
  path: string;
  cargo_target_dir: string;
  port: number;
}

export interface OperatingModel {
  project_id: string;
  objective: string | null;
  priorities: PlannedItem[];
  governance: {
    active_directives: string[];
    decision_policy: Record<string, string>;
    open_decisions: string[];
  };
  knowledge: {
    opinions: string[];
    superseded_opinions: string[];
    facts: string[];
    assumptions: string[];
    constraints: string[];
    briefings: { active: string[]; superseded: string[]; active_count: number };
  };
  context: {
    open_risks: string[];
    open_requirements: string[];
    task_counts: { total: number; open: number; in_review: number; done: number };
    active_agents: string[];
  };
  requests: { open_count: number; open: string[] };
  diagrams: { count: number; diagrams: string[] };
  spend: {
    total_estimated_usd: number;
    prompt_tokens: number;
    completion_tokens: number;
    entries: number;
    by_agent: Record<string, number>;
  };
  actor_contexts: AgentContext[];
  worktrees: WorktreeInfo[];
  drift_signals: string[];
}

export function fetchModel(): Promise<OperatingModel> {
  return j<OperatingModel>("/api/model");
}

// ---- Provenance --------------------------------------------------------------
export interface TaskProvenance {
  task_id: string;
  // Mirror whatever provenance::for_task returns; the SPA renders it generically.
  [key: string]: unknown;
}

export function fetchTaskProvenance(taskId: string): Promise<TaskProvenance> {
  return j<TaskProvenance>(`/api/provenance/task/${encodeURIComponent(taskId)}`);
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
