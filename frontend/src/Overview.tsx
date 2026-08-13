// Overview — the owner's operating picture (/api/model) + provenance lookup.
//
// Renders the curated read-model the backend derives (what the PM and each
// agent are actually seeing), the external-request intake inbox, spend,
// worktrees, and drift signals — plus a provenance lookup ("why does this code
// exist") via /api/provenance/task/{id}. Pure read surface; no writes here.
import { useState, type ReactNode } from "react";
import { useCastStore } from "./store";
import {
  AgentContext,
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

export default function Overview({ model }: { model: OperatingModel | null }) {
  if (!model) return null;
  const { governance, knowledge, context, requests, spend, worktrees, drift_signals } = model;

  return (
    <div className="grid gap-4">
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

      <ProvenanceLookup />
    </div>
  );
}
