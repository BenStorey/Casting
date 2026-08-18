import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { usd } from "@/lib/status";
import type { ActorRouting } from "@/api";

interface RoutingProps {
  routing: ActorRouting[];
}

/** Read-mostly table of which model each actor runs on, and what that costs. */
export default function Routing({ routing }: RoutingProps) {
  if (!routing || routing.length === 0) {
    return (
      <div className="empty">
        <div className="icon">🧭</div>
        <div className="title">No model routing configured</div>
        <div className="hint">
          Set an API key to enable the model layer — the cast defaults to no LLM
          until one is configured.
        </div>
      </div>
    );
  }

  const totalInput = routing.reduce((s, r) => s + r.input_price_per_mtok, 0);
  const totalOutput = routing.reduce((s, r) => s + r.output_price_per_mtok, 0);

  return (
    <div>
      <div className="mb-4 grid gap-3 md:grid-cols-3">
        <div className="stat">
          <div className="label">Actors routed</div>
          <div className="value">{routing.length}</div>
          <div className="hint">cast members bound to a model</div>
        </div>
        <div className="stat">
          <div className="label">Avg input cost</div>
          <div className="value">${(totalInput / routing.length).toFixed(2)}/M</div>
          <div className="hint">per million input tokens</div>
        </div>
        <div className="stat">
          <div className="label">Avg output cost</div>
          <div className="value">${(totalOutput / routing.length).toFixed(2)}/M</div>
          <div className="hint">per million output tokens</div>
        </div>
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle>Actor → model</CardTitle>
          <CardDescription>Which provider/model each actor uses, with cost per million tokens.</CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          <div className="divide-y">
            {routing.map((r) => (
              <div key={r.actor} className="flex flex-wrap items-center gap-2 py-2.5 text-sm">
                <span className="w-40 shrink-0 font-medium">{r.actor}</span>
                <Badge variant="soft">{r.provider}</Badge>
                <span className="flex-1 min-w-0 truncate font-mono text-xs text-muted-foreground">
                  {r.model}
                </span>
                <span className="shrink-0 tabular text-muted-foreground">
                  ${r.input_price_per_mtok.toFixed(2)} in · ${r.output_price_per_mtok.toFixed(2)} out
                </span>
                {r.temperature != null && (
                  <span className="shrink-0 text-xs text-muted-foreground">t={r.temperature}</span>
                )}
              </div>
            ))}
          </div>
        </CardContent>
      </Card>

      <p className="mt-3 text-xs text-muted-foreground">
        Costs are list prices per million tokens — actual spend is metered in the{" "}
        <strong>Spend</strong> view. Editing routing is done via configuration, not here.
      </p>
    </div>
  );
}
