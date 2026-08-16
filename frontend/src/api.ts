// Types mirroring the Rust projection + event JSON shapes (see src/projection.rs).
export type TaskStatus = "backlog" | "working" | "blocked" | "in_review" | "done";

export type MergeAuthority = "self_merge" | "pm_merge";

export interface Agent {
  id: string;
  role: string;
}

export interface Requirement {
  id: string;
  title: string;
  description: string;
}

export interface TaskReview {
  reviewer: string;
  note: string;
  approved: boolean;
}

export interface TaskDependency {
  task: string;
  blocking_task: string;
  required_state: TaskStatus;
}

export interface Task {
  id: string;
  title: string;
  kind: string;
  status: TaskStatus;
  assignee: string | null;
  merge_authority: MergeAuthority;
  priority: Priority;
  review: TaskReview | null;
  parent_id: string | null;
}

export type DecisionStatus = "proposed" | "approved" | "rejected" | "superseded";

export type DecisionClass =
  | "internal_rename"
  | "internal_refactor"
  | "testing_library"
  | "add_consultant"
  | "internal_implementation"
  | "database"
  | "architecture"
  | "product_requirement"
  | "spending_threshold"
  | "production_deployment"
  | "security_critical"
  | "irreversible"
  | "governance_change";

export type OwnerInvolvement = "never" | "pm" | "notify" | "ask";

export interface Decision {
  id: string;
  subject: string;
  options: Record<string, string>;
  recommendation: string | null;
  status: DecisionStatus;
  class: DecisionClass;
  involvement: OwnerInvolvement;
  decided_by: string | null;
  superseded_by: string | null;
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
  pm_action_required: boolean;
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

// ---- First-class semantic objects (risks, assumptions, constraints, etc.) ----

export type RiskStatus = "open" | "materialized" | "resolved";

export interface Risk {
  id: string;
  subject: string;
  severity: string;
  status: RiskStatus;
  discovered_by: string;
}

export interface Assumption {
  id: string;
  body: string;
  recorded_by: string;
}

export interface Constraint {
  id: string;
  body: string;
  recorded_by: string;
}

export type OpinionStatus = "active" | "superseded";

export interface Opinion {
  id: string;
  subject: string;
  category: string;
  statement: string;
  recorded_by: string;
  status: OpinionStatus;
  supersedes: string | null;
}

export interface Fact {
  id: string;
  kind: string;
  statement: string;
  recorded_by: string;
  recorded_at: string;
}

export interface CostEntry {
  id: string;
  agent_id: string;
  task_id: string | null;
  cost_class: string;
  model_tier: string;
  model: string | null;
  provider: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  cache_read_input_tokens: number;
  cache_creation_input_tokens: number;
  latency_ms: number;
  input_price_per_mtok: number | null;
  output_price_per_mtok: number | null;
  estimated_usd: number;
  incurred_at: string;
}

export interface Budget {
  limit_usd: number;
  warn_at: number;
}

export type BriefingStatus = "active" | "superseded";

export interface BriefingAsset {
  caption: string;
  location: string;
}

export interface Briefing {
  id: string;
  source: string;
  subject: string;
  title: string;
  body: string;
  assets: BriefingAsset[];
  brought_in_by: string;
  status: BriefingStatus;
  supersedes: string | null;
  imported_at: string;
}

export type ExternalRequestStatus = "open" | "closed";

export interface ExternalRequest {
  id: string;
  source: string;
  external_id: string | null;
  title: string;
  body: string;
  reporter: string;
  labels: string[];
  url: string | null;
  classification: string;
  severity: string;
  status: ExternalRequestStatus;
  received_at: string;
}

export interface Diagram {
  id: string;
  title: string;
  data: string;
  saved_by: string;
  saved_at: string;
}

export type DirectiveKind = "policy" | "constraint" | "principle" | "practice" | "preference" | "objective";

export type DirectiveStrength = "recommended" | "strong" | "required";

export type DirectiveStatus = "active" | "suspended" | "superseded" | "expired";

export interface Directive {
  id: string;
  kind: DirectiveKind;
  statement: string;
  scope: string[];
  strength: DirectiveStrength;
  status: DirectiveStatus;
  created_by: string;
  supersedes: string | null;
}

export interface Worktree {
  consultant: string;
  slot: number;
  task_id: string | null;
  branch: string;
  path: string;
  cargo_target_dir: string;
  port: number;
}

export interface ArchivedRecord {
  entity_kind: string;
  entity_id: string;
  summary: string;
  result: string;
  archived_at: string;
  archived_by: string;
}

export interface DecisionPolicy {
  default_involvement: OwnerInvolvement;
}

export interface ProjectPlan {
  objective: string | null;
  priorities: PlannedItem[];
  deprioritized: PlannedItem[];
  open_risks: string[];
  active_directives: string[];
  open_decisions: string[];
}

export interface Projection {
  project_id: string;
  agents: Agent[];
  requirements: Requirement[];
  tasks: Task[];
  dependencies: TaskDependency[];
  decisions: Decision[];
  messages: Message[];
  advisor_thread: Message[];
  observations: Observation[];
  branches: Branch[];
  commits: Commit[];
  merges: Merge[];
  changesets: ChangeSet[];
  risks: Risk[];
  assumptions: Assumption[];
  constraints: Constraint[];
  opinions: Opinion[];
  facts: Fact[];
  spend: CostEntry[];
  budget: Budget | null;
  paused: PauseInfo | null;
  briefings: Briefing[];
  external_requests: ExternalRequest[];
  diagrams: Diagram[];
  directives: Directive[];
  repo_metrics: RepoMetrics[];
  worktrees: Worktree[];
  policy: DecisionPolicy;
  archived: ArchivedRecord[];
  plan: ProjectPlan;
  rejections: ActionRejection[];
  orchestration: OrchestrationRun[];
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
  const token = getAuthToken();
  const headers: Record<string, string> = {
    ...(init?.headers as Record<string, string> | undefined),
  };
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }
  const res = await fetch(url, { ...init, headers });
  if (res.status === 401 && token) {
    // Token rejected — clear it so the UI shows the login prompt.
    clearAuthToken();
    throw new AuthError("token rejected");
  }
  if (!res.ok) {
    throw new Error(`${res.status} ${await res.text()}`);
  }
  return res.json() as Promise<T>;
}

// ── Auth helpers ──────────────────────────────────────────────
const AUTH_STORAGE_KEY = "casting_auth_token";

export function getAuthToken(): string | null {
  return localStorage.getItem(AUTH_STORAGE_KEY);
}

export function setAuthToken(token: string): void {
  localStorage.setItem(AUTH_STORAGE_KEY, token);
}

export function clearAuthToken(): void {
  localStorage.removeItem(AUTH_STORAGE_KEY);
}

export function hasAuthToken(): boolean {
  return getAuthToken() !== null;
}

export class AuthError extends Error {
  constructor(msg: string) {
    super(msg);
    this.name = "AuthError";
  }
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

export function summarizeAdvisor(): Promise<{ summary: string }> {
  return j("/api/advisor/summarize", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
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

// ---- Consultants (/api/consultants) -----------------------------------------
// Mirrors src/consultants ConsultantConfig — the loadable, shareable team
// packages. Configuration, never authority (who's hired stays in /api/state).

export interface ConsultantConfig {
  id: string;
  name: string;
  title: string;
  role: string; // catalog role id
  role_title: string;
  scope: string;
  avatar: string | null;
  summary: string | null;
  system_prompt_file: string | null;
  system_prompt: string | null;
  routing: {
    specializations: string[];
    trigger_patterns: string[];
    auto_join: boolean;
  };
  model: {
    provider: string | null;
    model_id: string | null;
    cost_tier: string;
    temperature: number | null;
    max_tokens: number | null;
  };
  verification: { review_required: boolean };
}

export function fetchConsultants(): Promise<ConsultantConfig[]> {
  return j<ConsultantConfig[]>("/api/consultants");
}

// ---- Per-actor model routing (/api/routing) --------------------------------

export interface ActorRouting {
  actor: string;
  provider: string;
  model: string;
  base_url: string;
  temperature: number | null;
  max_tokens: number | null;
  input_price_per_mtok: number;
  output_price_per_mtok: number;
}

export function fetchRouting(): Promise<ActorRouting[]> {
  return j<ActorRouting[]>("/api/routing");
}

// ---- Diagnostics audit trail (/api/model) -----------------------------------

export interface ActionRejection {
  who: string;
  action: string;
  reason: string;
  correlation: string | null;
  at: string;
}

export interface OrchestrationRun {
  trigger: string;
  actor: string;
  correlation: string;
  context_summary: string;
  planned: string[];
  metered: boolean;
  metering_agent: string | null;
  provider: string | null;
  model: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  latency_ms: number;
  estimated_usd: number;
  at: string;
}

export interface DiagnosticsView {
  rejection_count: number;
  recent_rejections: ActionRejection[];
  orchestration_count: number;
  recent_orchestration: OrchestrationRun[];
}

export interface BudgetView {
  limit_usd: number;
  warn_at: number;
  status: "disabled" | "ok" | "warn" | "halted";
  spend_fraction: number;
}

export interface PauseInfo {
  reason: string;
  by: string;
  at: string;
}

export interface GuardsView {
  budget: BudgetView | null;
  paused: PauseInfo | null;
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
  ownerName?: string,
  experienceLevel?: string,
  apiKey?: string,
  ownerToken?: string
): Promise<SetupResult> {
  return j<SetupResult>("/api/setup", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      name,
      objective,
      cast,
      owner_name: ownerName || undefined,
      experience_level: experienceLevel || undefined,
      api_key: apiKey || undefined,
      owner_token: ownerToken || undefined,
    }),
  });
}

export function fetchInbox(): Promise<Inbox> {
  return j<Inbox>("/api/inbox");
}

// ---- Telegram owner channel (2026-08-14) -----------------------------------

export interface TelegramConfigureResult {
  bot_id: number;
  bot_name: string;
  bot_username: string;
  chat_id: number | null;
  chat_linked: boolean;
  loop_started: boolean;
}

export interface TelegramStatus {
  configured: boolean;
  chat_id: number | null;
  bot_name: string | null;
  bot_username: string | null;
}

export function fetchTelegramStatus(): Promise<TelegramStatus> {
  return j<TelegramStatus>("/api/telegram/status");
}

/** Paste a BotFather token: validate, brand the bot as the PM, learn the
 *  chat_id, persist, and start the loop. */
export function configureTelegram(token: string): Promise<TelegramConfigureResult> {
  return j<TelegramConfigureResult>("/api/telegram/configure", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ token }),
  });
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
    cache_read_input_tokens: number;
    cache_creation_input_tokens: number;
    cache_hit_ratio: number;
    avg_latency_ms: number | null;
    entries: number;
    by_agent: Record<string, number>;
  };
  actor_contexts: AgentContext[];
  worktrees: WorktreeInfo[];
  drift_signals: string[];
  /** Harness guard rails: budget phase + active pause. */
  guards: GuardsView;
  /** Diagnostics audit trail: refused actions + orchestrator runs. */
  diagnostics: DiagnosticsView;
  /** Owner engagement: is the owner answering escalations or muting? */
  engagement: OwnerEngagementView;
  /** Diff quality over time: language-agnostic git churn. */
  diff_quality: DiffQualityView;
  /** Repo metrics: per-PR snapshots (files, lines by language, coverage). */
  repo_metrics: RepoMetricsView;
}

export interface OwnerEngagementView {
  /** Open decisions still requiring the owner (blocked on them). */
  awaiting_owner: number;
  /** Decisions the owner has ruled on. */
  owner_decided: number;
  /** Decisions handled autonomously by the PM/agent. */
  delegated_decided: number;
  /** 1.0 = caught up; falling toward 0 = owner muting. */
  response_rate: number;
}

export interface CommitChurnView {
  sha: string;
  branch: string;
  task_id: string | null;
  message: string;
  additions: number;
  deletions: number;
  files: number;
}

export interface DiffQualityView {
  commit_count: number;
  total_additions: number;
  total_deletions: number;
  total_files: number;
  avg_churn_per_commit: number;
  avg_files_per_commit: number;
  large_rewrites: number;
  large_rewrite_threshold: number;
  recent: CommitChurnView[];
}

export interface CoverageInfo {
  percent: number | null;
  source: string;
}

export interface LanguageLines {
  language: string;
  code: number;
  comments: number;
  blanks: number;
  files: number;
}

export interface RepoMetrics {
  merge_sha: string | null;
  captured_at: string;
  file_count: number;
  lines_by_language: LanguageLines[];
  coverage: CoverageInfo | null;
}

export interface RepoMetricsView {
  snapshot_count: number;
  latest: RepoMetrics | null;
  trend: RepoMetrics[];
}

export function fetchModel(): Promise<OperatingModel> {
  return j<OperatingModel>("/api/model");
}

// ---- Full actor context (/api/debug/context/{actor}) -----------------------

export interface FullActorContext {
  actor: string;
  system_prompt: string;
  planning_instructions: string;
  agent_context: AgentContext;
  assembled_context: string;
}

export function fetchFullContext(actor: string): Promise<FullActorContext> {
  return j<FullActorContext>(`/api/debug/context/${encodeURIComponent(actor)}`);
}

// ---- Graph / transition spine (/api/graph) ----------------------------------
// Mirrors src/graph.rs GraphView (derived — the backend stays the authority).

export type GraphTaskState =
  | "queued"
  | "working"
  | "in_review"
  | "awaiting_human"
  | "rejected"
  | "done";

export interface GraphNode {
  task_id: string;
  title: string;
  kind: string;
  status: string;
  state: GraphTaskState;
  assignee: string | null;
  parent_id: string | null;
  children: string[];
  awaiting_human: boolean;
  /** Hard-dependency blockers still unsatisfied (nodes this one waits on). */
  blocked_by: string[];
  /** State-derived causal steps ("why in this order"). */
  chain: string[];
  /** Currently-available transition ids from this node. */
  transitions: string[];
}

export interface GraphGroup {
  parent_id: string;
  title: string;
  children: string[];
  done: string[];
  remaining: string[];
  /** True iff every child is done — the deterministic join rule. */
  resolved: boolean;
}

export interface GraphView {
  nodes: GraphNode[];
  groups: GraphGroup[];
  active: string[];
  blocked: string[];
  done: number;
  total: number;
}

export function fetchGraph(): Promise<GraphView> {
  return j<GraphView>("/api/graph");
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

/// Subscribe to the realtime event stream. Calls `onEvent(seqBump)` — `seqBump`
/// is true when a NEW event arrived (vs. a heartbeat), so the caller can track
/// "last event Ns ago". On reconnect, passes `?after=<lastSeq>` so the server
/// replays any events missed while disconnected (SSE catch-up). Report
/// disconnect/connect via `onStatus(connected)` so a silently-stale UI is
/// visible. Returns an unsubscribe function.
export function subscribe(
  onEvent: (seqBump: boolean) => void,
  onStatus?: (connected: boolean) => void
): () => void {
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
      onStatus?.(true);
      let bumped = false;
      try {
        const ev = JSON.parse(raw.data);
        if (typeof ev.sequence === "number" && ev.sequence > lastSeq) {
          lastSeq = ev.sequence;
          bumped = true;
        }
      } catch {
        // Malformed payload — ignore, the caller will refetch state anyway.
      }
      onEvent(bumped);
    });
    es.onerror = () => {
      // The connection dropped: reflect that immediately so the UI can show a
      // stale indicator while EventSource auto-reconnects.
      onStatus?.(false);
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
