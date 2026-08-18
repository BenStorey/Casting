import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import {
  usd,
  usdCompact,
  tokens,
  priorityVariant,
} from "@/lib/status";
import type {
  Decision,
  Inbox,
  Observation,
  OperatingModel,
} from "@/api";

interface HomeProps {
  model: OperatingModel | null;
  inbox: Inbox | null;
  observations: Observation[];
  decisions: Decision[];
  onGoInbox: () => void;
  onGoChat: () => void;
}

/** The owner's landing surface — "what needs me + is it healthy". */
export default function Home({
  model,
  inbox,
  observations,
  decisions,
  onGoInbox,
  onGoChat,
}: HomeProps) {
  const pendingDecisions = (inbox?.items ?? []).length;
  const flagged = observations.filter((o) => o.pm_action_required);
  const attentionItems = pendingDecisions + flagged.length;

  const guards = model?.guards;
  const spend = model?.spend;
  const engagement = model?.engagement;
  const diffQuality = model?.diff_quality;
  const context = model?.context;

  const paused = guards?.paused != null;
  const budgetHalted = guards?.budget?.status === "halted";
  const budgetWarn = guards?.budget?.status === "warn";
  const needsAttention = attentionItems > 0 || paused || budgetHalted || budgetWarn;

  const budgetFrac = guards?.budget?.spend_fraction ?? null;

  return (
    <div className="grid gap-4">
      {/* Greeting + objective */}
      <Card className="border-primary/20 bg-gradient-to-br from-primary-soft/60 to-card">
        <CardContent className="p-5">
          <div className="text-xs font-medium uppercase tracking-[0.08em] text-primary">
            The production
          </div>
          <div className="mt-1 font-display text-2xl font-bold leading-tight">
            {model?.objective || "No objective set yet"}
          </div>
          <div className="mt-2 text-sm text-muted-foreground">
            {needsAttention
              ? "There's something waiting for your call."
              : "All quiet — the cast is working autonomously."}
          </div>
        </CardContent>
      </Card>

      {/* Attention strip */}
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <button
          onClick={onGoInbox}
          className="stat text-left transition-shadow hover:shadow-md"
        >
          <div className="label">Needs your decision</div>
          <div className="value">{pendingDecisions}</div>
          <div className="hint">
            {pendingDecisions > 0
              ? "Waiting in your inbox"
              : "Nothing pending — good"}
          </div>
        </button>
        <div className="stat">
          <div className="label">Flagged observations</div>
          <div className="value">{flagged.length}</div>
          <div className="hint">
            {flagged.length > 0 ? "Need your attention" : "No flags raised"}
          </div>
        </div>
        <div className="stat">
          <div className="label">Spend (est. USD)</div>
          <div className="value">{spend ? usdCompact(spend.total_estimated_usd) : "—"}</div>
          <div className="hint">
            {budgetFrac != null && (
              <span className={budgetWarn || budgetHalted ? "text-destructive" : ""}>
                {(budgetFrac * 100).toFixed(0)}% of limit · {tokens(spend?.prompt_tokens ?? 0)} prompt tokens
              </span>
            )}
            {budgetFrac == null && "No budget limit set"}
          </div>
        </div>
        <div className="stat">
          <div className="label">Owner engagement</div>
          <div className="value">
            {engagement ? `${(engagement.response_rate * 100).toFixed(0)}%` : "—"}
          </div>
          <div className="hint">
            {engagement && engagement.awaiting_owner > 0
              ? `${engagement.awaiting_owner} awaiting you`
              : "You're caught up"}
          </div>
        </div>
      </div>

      {/* Guard rail / attention status */}
      {(paused || budgetHalted || budgetWarn) && (
        <Card className={paused || budgetHalted ? "border-destructive/40" : "border-warning/40"}>
          <CardContent className="p-5">
            <div className="flex flex-wrap items-center gap-3">
              {paused && (
                <Badge variant="destructive">⏸ Work paused</Badge>
              )}
              {budgetHalted && (
                <Badge variant="destructive">🛑 Budget halt</Badge>
              )}
              {budgetWarn && !budgetHalted && (
                <Badge variant="warning">⚠ Budget nearing limit</Badge>
              )}
              <span className="text-sm text-muted-foreground">
                {paused
                  ? `Reason: ${guards?.paused?.reason || "none given"} · by ${guards?.paused?.by || "?"}`
                  : budgetFrac != null
                  ? `Spend ${usd(spend?.total_estimated_usd ?? 0)} of $${(guards?.budget?.limit_usd ?? 0).toFixed(2)} limit`
                  : ""}
              </span>
            </div>
            {budgetFrac != null && (
              <Progress
                className="mt-3"
                value={Math.min(budgetFrac * 100, 100)}
                indicatorClassName={
                  budgetHalted
                    ? "bg-destructive"
                    : budgetWarn
                    ? "bg-warning"
                    : "bg-primary"
                }
              />
            )}
          </CardContent>
        </Card>
      )}

      {/* Two-column: priorities + decisions + activity */}
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle>Current priorities</CardTitle>
            <CardDescription>Ranked by what matters most right now.</CardDescription>
          </CardHeader>
          <CardContent className="pt-2">
            {(model?.priorities ?? []).length === 0 ? (
              <div className="empty">
                <div className="icon">🎯</div>
                <div className="title">No plan yet</div>
                <div className="hint">Tell the PM what you want to build to get things moving.</div>
              </div>
            ) : (
              <ol className="space-y-2">
                {(model?.priorities ?? []).slice(0, 5).map((p, i) => (
                  <li key={p.task_id} className="flex items-center gap-3 text-sm">
                    <span className="w-5 shrink-0 text-right tabular text-muted-foreground">{i + 1}.</span>
                    <span className="flex-1 break-words">{p.title}</span>
                    <Badge variant={priorityVariant(p.priority)}>{p.priority}</Badge>
                  </li>
                ))}
              </ol>
            )}
            <ButtonLink onClick={onGoChat} className="mt-3">
              → Give the PM direction
            </ButtonLink>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle>Open decisions</CardTitle>
            <CardDescription>Proposals that are waiting for you or the PM.</CardDescription>
          </CardHeader>
          <CardContent className="pt-2">
            {decisions.filter((d) => d.status === "proposed").length === 0 ? (
              <div className="empty">
                <div className="icon">⚖️</div>
                <div className="title">No open decisions</div>
                <div className="hint">Nothing needs a ruling right now.</div>
              </div>
            ) : (
              <ul className="space-y-2">
                {decisions
                  .filter((d) => d.status === "proposed")
                  .slice(0, 5)
                  .map((d) => (
                    <li key={d.id} className="flex items-center justify-between gap-2 text-sm">
                      <span className="flex-1 break-words">{d.subject}</span>
                      <Badge variant="warning">proposed</Badge>
                    </li>
                  ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Health strip */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Production health</CardTitle>
        </CardHeader>
        <CardContent className="pt-2 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <HealthStat label="Tasks" value={String(context?.task_counts.total ?? 0)}
            hint={`${context?.task_counts.open ?? 0} open · ${context?.task_counts.in_review ?? 0} review · ${context?.task_counts.done ?? 0} done`} />
          <HealthStat label="Live worktrees" value={model?.worktrees?.length != null ? String(model.worktrees.length) : "—"}
            hint="isolated work areas for the cast" />
          <HealthStat
            label="Diff quality"
            value={diffQuality ? `${diffQuality.commit_count} commits` : "—"}
            hint={
              diffQuality
                ? `+${diffQuality.total_additions} / −${diffQuality.total_deletions} lines`
                : ""
            }
          />
          <HealthStat
            label="Avg latency"
            value={spend?.avg_latency_ms != null ? `${spend.avg_latency_ms.toFixed(0)} ms` : "—"}
            hint={`cache hit ${spend?.cache_hit_ratio != null ? (spend.cache_hit_ratio * 100).toFixed(0) : 0}% across ${spend?.entries ?? 0} calls`}
          />
        </CardContent>
      </Card>
    </div>
  );
}

function HealthStat({ label, value, hint }: { label: string; value: string; hint: string }) {
  return (
    <div>
      <div className="overline-label">{label}</div>
      <div className="mt-1 text-lg font-semibold tabular">{value || "—"}</div>
      <div className="mt-0.5 text-xs text-muted-foreground">{hint}</div>
    </div>
  );
}

function ButtonLink({ onClick, children, className }: { onClick: () => void; children: React.ReactNode; className?: string }) {
  return (
    <button
      onClick={onClick}
      className={`text-sm font-medium text-primary hover:underline ${className ?? ""}`}
    >
      {children}
    </button>
  );
}
