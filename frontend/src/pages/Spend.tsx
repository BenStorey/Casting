import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import { Separator } from "@/components/ui/separator";
import { usd, usdCompact, tokens, fmtTime } from "@/lib/status";
import type { CostEntry, OperatingModel } from "@/api";

interface SpendProps {
  model: OperatingModel | null;
  spendEntries: CostEntry[];
}

/** The cost dashboard — total + budget, breakdown by agent & cost class. */
export default function Spend({ model, spendEntries }: SpendProps) {
  const spend = model?.spend;
  const budget = model?.guards?.budget;
  const budgetFrac = budget?.spend_fraction ?? null;
  const budgetStatus = budget?.status ?? "disabled";

  // By-agent (from the live projection of cost entries).
  const byAgent = new Map<string, number>();
  const byClass = new Map<string, number>();
  for (const e of spendEntries) {
    const aid = e.agent_id || "unknown";
    byAgent.set(aid, (byAgent.get(aid) ?? 0) + e.estimated_usd);
    const cls = e.cost_class || "other";
    byClass.set(cls, (byClass.get(cls) ?? 0) + e.estimated_usd);
  }
  const byAgentSorted = [...byAgent.entries()].sort((a, b) => b[1] - a[1]);
  const byClassSorted = [...byClass.entries()].sort((a, b) => b[1] - a[1]);

  const recent = [...spendEntries].sort((a, b) =>
    b.incurred_at.localeCompare(a.incurred_at)
  );

  return (
    <div className="grid gap-4">
      {/* Top stats */}
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <div className="stat">
          <div className="label">Total spend</div>
          <div className="value">{spend ? usd(spend.total_estimated_usd) : "—"}</div>
          <div className="hint">{spend?.entries ?? 0} metered calls</div>
        </div>
        <div className="stat">
          <div className="label">Budget</div>
          <div className="value">{budget ? usd(budget.limit_usd) : "No limit"}</div>
          <div className="hint">
            {budget ? `${((budgetFrac ?? 0) * 100).toFixed(0)}% used` : "not configured"}
          </div>
        </div>
        <div className="stat">
          <div className="label">Prompt tokens</div>
          <div className="value">{spend ? tokens(spend.prompt_tokens) : "—"}</div>
          <div className="hint">+ {spend ? tokens(spend.completion_tokens) : "—"} completion</div>
        </div>
        <div className="stat">
          <div className="label">Cache hit</div>
          <div className="value">
            {spend?.cache_hit_ratio != null ? `${(spend.cache_hit_ratio * 100).toFixed(0)}%` : "—"}
          </div>
          <div className="hint">
            read {spend ? tokens(spend.cache_read_input_tokens) : "—"} / write{" "}
            {spend ? tokens(spend.cache_creation_input_tokens) : "—"}
          </div>
        </div>
      </div>

      {/* Budget progress + status */}
      <Card className={budgetStatus === "halted" ? "border-destructive/40" : budgetStatus === "warn" ? "border-warning/40" : ""}>
        <CardHeader className="pb-2">
          <CardTitle className="flex items-center gap-2">
            Budget
            <Badge
              variant={
                budgetStatus === "halted"
                  ? "destructive"
                  : budgetStatus === "warn"
                  ? "warning"
                  : budgetStatus === "disabled"
                  ? "outline"
                  : "success"
              }
            >
              {budgetStatus}
            </Badge>
          </CardTitle>
          <CardDescription>
            {budgetStatus === "halted"
              ? "The circuit breaker refuses all LLM calls until the limit is raised."
              : budgetStatus === "warn"
              ? "Approaching the hard limit."
              : "Spend is within budget."}
          </CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {budgetFrac != null && (
            <>
              <Progress
                value={Math.min(budgetFrac * 100, 100)}
                indicatorClassName={
                  budgetStatus === "halted"
                    ? "bg-destructive"
                    : budgetStatus === "warn"
                    ? "bg-warning"
                    : "bg-primary"
                }
              />
              <div className="mt-1.5 flex justify-between text-xs text-muted-foreground tabular">
                <span>{usd(spend?.total_estimated_usd ?? 0)} spent</span>
                <span>
                  warn at {(budget?.warn_at ?? 0) * 100}% · limit {usd(budget?.limit_usd ?? 0)}
                </span>
              </div>
            </>
          )}
          {budgetFrac == null && (
            <div className="empty">
              <div className="icon">💸</div>
              <div className="title">No budget configured</div>
              <div className="hint">Set a limit to guardrail spending.</div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Breakdowns */}
      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle>By agent</CardTitle>
            <CardDescription>Where the money goes across the cast.</CardDescription>
          </CardHeader>
          <CardContent className="pt-2">
            {byAgentSorted.length === 0 ? (
              <div className="empty">
                <div className="icon">🎭</div>
                <div className="title">No spend yet</div>
                <div className="hint">Costs appear here as the cast works.</div>
              </div>
            ) : (
              <ul className="space-y-2">
                {byAgentSorted.map(([agent, v]) => {
                  const total = spend?.total_estimated_usd || 1;
                  return (
                    <li key={agent} className="text-sm">
                      <div className="flex items-center justify-between gap-2">
                        <span className="font-medium">{agent}</span>
                        <span className="tabular text-muted-foreground">{usd(v)}</span>
                      </div>
                      <Progress className="mt-1" value={(v / total) * 100} />
                    </li>
                  );
                })}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle>By cost class</CardTitle>
            <CardDescription>What kind of work is being billed.</CardDescription>
          </CardHeader>
          <CardContent className="pt-2">
            {byClassSorted.length === 0 ? (
              <div className="empty">
                <div className="icon">🧾</div>
                <div className="title">No cost classes yet</div>
              </div>
            ) : (
              <ul className="space-y-1.5">
                {byClassSorted.map(([cls, v]) => (
                  <li key={cls} className="flex items-center justify-between gap-2 text-sm">
                    <span className="capitalize text-muted-foreground">{cls.replace(/_/g, " ")}</span>
                    <span className="tabular">{usd(v)}</span>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Recent entries */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Recent calls</CardTitle>
          <CardDescription>The latest metered LLM calls.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          {recent.length === 0 ? (
            <div className="empty">
              <div className="icon">📡</div>
              <div className="title">No calls yet</div>
            </div>
          ) : (
            <div className="divide-y">
              {recent.slice(0, 12).map((e) => (
                <div key={e.id} className="flex flex-wrap items-center gap-2 py-2 text-sm">
                  <Badge variant="secondary">{e.agent_id || "system"}</Badge>
                  <span className="flex-1 min-w-0 truncate text-muted-foreground">
                    {e.cost_class.replace(/_/g, " ")} · {e.model || e.provider || ""}
                  </span>
                  <span className="tabular text-muted-foreground">
                    {tokens(e.prompt_tokens)}/{tokens(e.completion_tokens)} tok
                  </span>
                  <span className="tabular font-medium">{usd(e.estimated_usd)}</span>
                  <span className="text-xs text-muted-foreground">{fmtTime(e.incurred_at)}</span>
                </div>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
      <Separator className="my-1" />
    </div>
  );
}
