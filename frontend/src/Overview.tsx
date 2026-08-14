// Overview — the owner's operating picture (/api/model) + provenance lookup.
//
// Renders the curated read-model the backend derives (what the PM and each
// agent are actually seeing), the external-request intake inbox, spend,
// worktrees, and drift signals — plus a provenance lookup ("why does this code
// exist") via /api/provenance/task/{id}. Pure read surface; no writes here.
import { useState, type ReactNode } from "react";
import { useCastStore } from "./store";
import {
  ActorRouting,
  AgentContext,
  DiagnosticsView,
  fetchTaskProvenance,
  OperatingModel,
  TaskProvenance,
} from "./api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-base">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="pt-2">{children}</CardContent>
    </Card>
  );
}

function Bullets({ items, empty }: { items: string[]; empty: string }) {
  if (items.length === 0) {
    return <div className="text-sm text-muted-foreground">{empty}</div>;
  }
  return (
    <ul className="space-y-1 text-sm">
      {items.map((it, i) => (
        <li key={i} className="flex gap-2">
          <span className="text-primary shrink-0">•</span>
          <span className="break-words">{it}</span>
        </li>
      ))}
    </ul>
  );
}

const PRIORITY_VARIANT: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
  critical: "destructive",
  high: "default",
  medium: "secondary",
  low: "outline",
};

function PriorityView({ model }: { model: OperatingModel }) {
  const priorities = model.priorities;
  if (priorities.length === 0) {
    return <div className="text-sm text-muted-foreground">No ranked plan yet.</div>;
  }
  return (
    <ol className="space-y-1.5">
      {priorities.map((p, i) => (
        <li key={p.task_id} className="flex items-center gap-2 text-sm">
          <span className="w-5 shrink-0 text-right text-muted-foreground">{i + 1}.</span>
          <span className="flex-1 break-words">{p.title}</span>
          <Badge variant={PRIORITY_VARIANT[p.priority] ?? "secondary"}>{p.priority}</Badge>
        </li>
      ))}
    </ol>
  );
}

function ActorContextCard({ ctx }: { ctx: AgentContext }) {
  const isOwner = ctx.actor === "owner";
  return (
    <Card className="border-border/60">
      <CardContent className="pt-4">
        <div className="mb-1 flex items-center justify-between">
          <span className="font-semibold text-sm">{ctx.actor}</span>
          {isOwner && <Badge variant="outline">owner</Badge>}
        </div>
        <div className="space-y-2 text-sm">
          <div>
            <div className="text-xs text-muted-foreground mb-0.5">My tasks</div>
            <Bullets items={ctx.my_tasks} empty="none" />
          </div>
          {ctx.scored_priorities.length > 0 && (
            <div>
              <div className="text-xs text-muted-foreground mb-0.5">Scored priorities</div>
              <ul className="space-y-0.5">
                {ctx.scored_priorities.map((sp) => (
                  <li key={sp.task_id} className="flex justify-between gap-2">
                    <span className="truncate">{sp.title}</span>
                    <span className="text-muted-foreground shrink-0">{sp.relevance.toFixed(1)}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          {ctx.worktree && (
            <div>
              <div className="text-xs text-muted-foreground mb-0.5">Worktree</div>
              <div className="text-xs">
                {ctx.worktree.branch} · :{ctx.worktree.port}
              </div>
            </div>
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function ProvenanceLookup() {
  const [taskId, setTaskId] = useState("");
  const [result, setResult] = useState<TaskProvenance | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const lookup = async () => {
    const id = taskId.trim();
    if (!id || busy) return;
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      setResult(await fetchTaskProvenance(id));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section title="Provenance" description='Look up a task — "why does this code exist?"'>
      <div className="flex gap-2">
        <Input
          value={taskId}
          placeholder="task id, e.g. task-1"
          onChange={(e) => setTaskId(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && lookup()}
        />
        <Button onClick={lookup} disabled={busy || !taskId.trim()}>
          Trace
        </Button>
      </div>
      {error && <div className="mt-2 text-sm text-destructive">{error}</div>}
      {result && (
        <pre className="mt-3 overflow-auto rounded bg-muted p-3 text-xs">
          {JSON.stringify(result, null, 2)}
        </pre>
      )}
    </Section>
  );
}

function RoutingView({ routing }: { routing: ActorRouting[] }) {
  if (routing.length === 0) {
    return (
      <div className="text-sm text-muted-foreground">
        No LLM routing configured (set CAST_LLM_API_KEY to enable the model layer).
      </div>
    );
  }
  return (
    <div className="space-y-1 text-sm">
      {routing.map((r) => (
        <div key={r.actor} className="flex items-center justify-between gap-2">
          <span className="font-medium">{r.actor}</span>
          <span className="text-muted-foreground">
            {r.provider}/{r.model}
            {r.temperature != null && <span className="ml-1">· t={r.temperature}</span>}
          </span>
          <span className="text-xs text-muted-foreground">
            ${r.input_price_per_mtok.toFixed(2)}/M in · ${r.output_price_per_mtok.toFixed(2)}/M out
          </span>
        </div>
      ))}
    </div>
  );
}

export default function Overview({ model }: { model: OperatingModel | null }) {
  const routing = useCastStore((s) => s.routing);
  if (!model) return null;
  const { governance, knowledge, context, requests, spend, worktrees, drift_signals } = model;
  const { guards, diagnostics, engagement, diff_quality } = model;

  return (
    <div className="grid gap-4">
      {/* G4: guard rail health — halted / paused / budget-warn must be unmissable.
          This answers "why did it stop going" at a glance. */}
      {guards.paused && (
        <Card className="border-destructive/50">
          <CardHeader className="pb-2">
            <CardTitle className="text-base text-destructive">⏸ Work paused</CardTitle>
            <CardDescription>All side-effecting work is halted until resumed.</CardDescription>
          </CardHeader>
          <CardContent className="pt-2 text-sm">
            <div>
              <span className="text-muted-foreground">Reason: </span>
              {guards.paused.reason || <em className="text-muted-foreground">(none given)</em>}
            </div>
            <div className="text-xs text-muted-foreground mt-1">
              by {guards.paused.by || "?"} · {new Date(guards.paused.at).toLocaleString()}
            </div>
          </CardContent>
        </Card>
      )}
      {guards.budget && guards.budget.status === "halted" && (
        <Card className="border-destructive/50">
          <CardHeader className="pb-2">
            <CardTitle className="text-base text-destructive">🛑 Budget halt</CardTitle>
            <CardDescription>
              Spend reached the hard limit — the circuit breaker refuses all LLM calls.
              Only a higher limit un-halts it.
            </CardDescription>
          </CardHeader>
          <CardContent className="pt-2 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Spend / limit</span>
              <span>
                ${spend.total_estimated_usd.toFixed(4)} / ${guards.budget.limit_usd.toFixed(2)}
              </span>
            </div>
          </CardContent>
        </Card>
      )}
      {guards.budget && guards.budget.status === "warn" && (
        <Card className="border-amber-500/50">
          <CardHeader className="pb-2">
            <CardTitle className="text-base text-amber-600">⚠ Budget nearing limit</CardTitle>
            <CardDescription>
              {((guards.budget.spend_fraction ?? 0) * 100).toFixed(0)}% of the hard limit used
              (warn at {(guards.budget.warn_at * 100).toFixed(0)}%).
            </CardDescription>
          </CardHeader>
        </Card>
      )}

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Operating picture</CardTitle>
          <CardDescription>What the PM and agents are actually working from.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          <div className="text-sm">
            <span className="text-muted-foreground">Objective: </span>
            {model.objective ?? <em className="text-muted-foreground">none set</em>}
          </div>
        </CardContent>
      </Card>

      <Section title="Model routing" description="Which model each actor runs on.">
        <RoutingView routing={routing} />
      </Section>

      {drift_signals.length > 0 && (
        <Card className="border-destructive/40">
          <CardHeader className="pb-2">
            <CardTitle className="text-base text-destructive">⚠ Drift signals</CardTitle>
          </CardHeader>
          <CardContent className="pt-2">
            <Bullets items={drift_signals} empty="" />
          </CardContent>
        </Card>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <Section title="Priorities" description="Ranked by relevance.">
          <PriorityView model={model} />
        </Section>

        <Section title="Request inbox" description="External issues / PRs reported in.">
          <div className="mb-2 text-sm">
            <Badge variant={requests.open_count > 0 ? "destructive" : "secondary"}>
              {requests.open_count} open
            </Badge>
          </div>
          <Bullets items={requests.open} empty="No open external requests." />
        </Section>

        <Section title="Governance">
          <div className="space-y-3 text-sm">
            <div>
              <div className="text-xs text-muted-foreground mb-1">Active directives</div>
              <Bullets items={governance.active_directives} empty="none" />
            </div>
            <div>
              <div className="text-xs text-muted-foreground mb-1">Open decisions</div>
              <Bullets items={governance.open_decisions} empty="none" />
            </div>
            {Object.keys(governance.decision_policy).length > 0 && (
              <div>
                <div className="text-xs text-muted-foreground mb-1">Decision policy</div>
                <ul className="space-y-0.5">
                  {Object.entries(governance.decision_policy).map(([k, v]) => (
                    <li key={k} className="flex gap-2">
                      <span className="text-muted-foreground">{k}:</span>
                      <span>{typeof v === "string" ? v : JSON.stringify(v)}</span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        </Section>

        <Section title="Knowledge">
          <div className="space-y-3 text-sm">
            <div>
              <div className="text-xs text-muted-foreground mb-1">Opinions</div>
              <Bullets items={knowledge.opinions} empty="none" />
            </div>
            {knowledge.superseded_opinions.length > 0 && (
              <div>
                <div className="text-xs text-muted-foreground mb-1">Superseded opinions</div>
                <Bullets items={knowledge.superseded_opinions} empty="" />
              </div>
            )}
            <div>
              <div className="text-xs text-muted-foreground mb-1">Facts</div>
              <Bullets items={knowledge.facts} empty="none" />
            </div>
            <div>
              <div className="text-xs text-muted-foreground mb-1">Advisor briefings</div>
              <Bullets items={knowledge.briefings.active} empty="none" />
            </div>
          </div>
        </Section>

        <Section title="Context">
          <div className="space-y-3 text-sm">
            <div>
              <div className="text-xs text-muted-foreground mb-1">Risks</div>
              <Bullets items={context.open_risks} empty="none" />
            </div>
            <div>
              <div className="text-xs text-muted-foreground mb-1">Requirements</div>
              <Bullets items={context.open_requirements} empty="none" />
            </div>
            <div>
              <div className="text-xs text-muted-foreground mb-1">Tasks</div>
              <div className="flex gap-2">
                <Badge variant="outline">{context.task_counts.total} total</Badge>
                <Badge variant="secondary">{context.task_counts.open} open</Badge>
                <Badge variant="default">{context.task_counts.in_review} review</Badge>
                <Badge variant="outline">{context.task_counts.done} done</Badge>
              </div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground mb-1">Active agents</div>
              <Bullets items={context.active_agents} empty="none" />
            </div>
          </div>
        </Section>

        <Section title="Spend">
          <div className="space-y-1 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Total (est. USD)</span>
              <span>${spend.total_estimated_usd.toFixed(4)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Tokens</span>
              <span>
                {spend.prompt_tokens} prompt / {spend.completion_tokens} completion
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Cache read / write / hit</span>
              <span>
                {spend.cache_read_input_tokens} / {spend.cache_creation_input_tokens} /{" "}
                {(spend.cache_hit_ratio * 100).toFixed(1)}%
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Avg latency</span>
              <span>{spend.avg_latency_ms != null ? `${spend.avg_latency_ms.toFixed(0)} ms` : "n/a"}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Entries</span>
              <span>{spend.entries}</span>
            </div>
            {Object.keys(spend.by_agent).length > 0 && (
              <div className="pt-1">
                {Object.entries(spend.by_agent).map(([agent, usd]) => (
                  <div key={agent} className="flex justify-between text-xs">
                    <span className="text-muted-foreground">{agent}</span>
                    <span>${usd.toFixed(4)}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </Section>

        <Section
          title={`Owner engagement · ${(engagement.response_rate * 100).toFixed(0)}%`}
          description="Is the owner answering escalations or muting? 1.0 = caught up."
        >
          <div className="space-y-1 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Awaiting owner (blocked)</span>
              <span>{engagement.awaiting_owner}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Owner decided</span>
              <span>{engagement.owner_decided}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Handled autonomously</span>
              <span>{engagement.delegated_decided}</span>
            </div>
          </div>
          {engagement.awaiting_owner > 0 && engagement.response_rate < 0.5 && (
            <p className="text-destructive mt-2 text-xs">
              Escalation backlog growing — the owner may be muting.
            </p>
          )}
        </Section>

        <Section
          title={`Code diff quality · ${diff_quality.commit_count} commits`}
          description="Language-agnostic git churn — is the code trending toward soup?"
        >
          <div className="space-y-1 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Lines added / deleted</span>
              <span>
                +{diff_quality.total_additions} / −{diff_quality.total_deletions}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Avg churn per commit</span>
              <span>{diff_quality.avg_churn_per_commit.toFixed(0)} lines</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Avg files per commit</span>
              <span>{diff_quality.avg_files_per_commit.toFixed(1)}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Large rewrites (&gt;{diff_quality.large_rewrite_threshold} lines)</span>
              <span className={diff_quality.large_rewrites > 0 ? "text-destructive" : undefined}>
                {diff_quality.large_rewrites}
              </span>
            </div>
            {diff_quality.recent.length > 0 && (
              <div className="border-t pt-2">
                {diff_quality.recent.map((c) => (
                  <div key={c.sha} className="flex justify-between text-xs">
                    <span className="text-muted-foreground truncate">
                      {c.message} · {c.sha.slice(0, 7)}
                    </span>
                    <span>
                      +{c.additions}/−{c.deletions} · {c.files}f
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </Section>
      </div>

      {(worktrees.length > 0 || model.actor_contexts.length > 0) && (
        <Section title="Per-actor operating context" description="Exactly what each model is handed when it plans.">
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {model.actor_contexts.map((ctx) => (
              <ActorContextCard key={ctx.actor} ctx={ctx} />
            ))}
          </div>
        </Section>
      )}

      <DiagnosticsSection diagnostics={diagnostics} />

      <ProvenanceLookup />
    </div>
  );
}

// G2/G3: the "what did the model try / what failed" surface. Renders the
// refused-action audit trail + recorded orchestrator planning passes.
function DiagnosticsSection({ diagnostics }: { diagnostics: DiagnosticsView }) {
  const rejectionCount = diagnostics.rejection_count;
  const runs = diagnostics.recent_orchestration;

  return (
    <Section
      title={`Diagnostics${rejectionCount > 0 ? ` · ${rejectionCount} refused action(s)` : ""}`}
      description="Audit trail for refused plans + orchestrator planning passes (what the model saw & decided)."
    >
      {rejectionCount > 0 && (
        <div className="mb-3">
          <div className="mb-1 text-xs text-muted-foreground">Refused actions (newest first)</div>
          <div className="flex flex-col gap-2">
            {diagnostics.recent_rejections.map((r, i) => (
              <div key={i} className="rounded border border-destructive/40 bg-destructive/5 p-2 text-xs">
                <div className="font-medium text-destructive">
                  {r.who} — refused: {r.action}
                </div>
                <div className="text-muted-foreground mt-0.5">because: {r.reason}</div>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="mb-1 text-xs text-muted-foreground">
        Orchestrator runs ({diagnostics.orchestration_count})
      </div>
      {runs.length === 0 ? (
        <div className="text-sm text-muted-foreground">
          No orchestrator planning passes recorded yet (the real LLM / mock isn't wired into this
          run).
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {runs.map((run, i) => (
            <div key={i} className="rounded border border-border/60 p-2 text-xs">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">on {run.trigger.toLowerCase().replace(/_/g, " ")}</span>
                {run.metered ? (
                  <span className="text-muted-foreground">
                    {run.provider ?? "?"} · {run.model ?? "?"} · {run.prompt_tokens}p/
                    {run.completion_tokens}c · ${run.estimated_usd.toFixed(4)}
                  </span>
                ) : (
                  <span className="text-muted-foreground">no cost (deterministic)</span>
                )}
              </div>
              <div className="text-muted-foreground mt-1 break-words">{run.context_summary}</div>
              {run.planned.length > 0 && (
                <ul className="mt-1 space-y-0.5">
                  {run.planned.map((p, j) => (
                    <li key={j} className="truncate">→ {p}</li>
                  ))}
                </ul>
              )}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}
