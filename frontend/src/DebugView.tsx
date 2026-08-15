// DebugView — raw projection + model + graph as collapsible JSON for debugging.
// Shows every field the backend returns that isn't surfaced by the main tabs.
// Nothing polished — just "what's in there" for inspecting state quickly.
import { useState } from "react";
import { useCastStore } from "./store";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { FullActorContext, fetchFullContext } from "./api";

function CollapsibleJson({
  label,
  data,
  defaultOpen,
}: {
  label: string;
  data: unknown;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen ?? false);
  return (
    <div className="border rounded">
      <button
        className="flex w-full items-center justify-between px-3 py-2 text-sm font-medium hover:bg-muted/50"
        onClick={() => setOpen(!open)}
      >
        <span>{label}</span>
        <Badge variant="outline" className="text-[10px]">
          {open ? "collapse" : "expand"}
        </Badge>
      </button>
      {open && (
        <pre className="max-h-[600px] overflow-auto border-t bg-muted p-3 text-[11px] leading-snug">
          {JSON.stringify(data, null, 2)}
        </pre>
      )}
    </div>
  );
}

function FullContextPanel() {
  const [actor, setActor] = useState("pm");
  const [ctx, setCtx] = useState<FullActorContext | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const lookup = async () => {
    const a = actor.trim();
    if (!a || busy) return;
    setBusy(true);
    setError(null);
    try {
      setCtx(await fetchFullContext(a));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="border rounded p-3">
      <div className="mb-2">
        <div className="font-medium text-sm">Full actor context</div>
        <div className="text-xs text-muted-foreground">
          The exact prompt the model would receive — persona + action vocabulary + operating context. Watch this grow to spot PM context bloat.
        </div>
      </div>
      <div className="flex gap-2">
        <Input
          value={actor}
          placeholder="actor id, e.g. pm, lead-programmer"
          onChange={(e) => setActor(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && lookup()}
        />
        <Button onClick={lookup} disabled={busy || !actor.trim()}>
          {busy ? "Loading…" : "Fetch"}
        </Button>
      </div>
      {error && <div className="mt-2 text-sm text-destructive">{error}</div>}
      {ctx && (
        <div className="mt-3 space-y-2">
          <div className="flex flex-wrap gap-2">
            <Badge variant="outline">{ctx.actor}</Badge>
            <Badge variant="secondary">
              context {ctx.assembled_context.length.toLocaleString()} chars
            </Badge>
            <Badge variant="outline">
              system {ctx.system_prompt.length} · vocab {ctx.planning_instructions.length}
            </Badge>
          </div>
          <pre className="max-h-[600px] overflow-auto rounded border bg-muted p-3 text-[11px] leading-snug whitespace-pre-wrap">
            {ctx.assembled_context}
          </pre>
        </div>
      )}
    </div>
  );
}

export default function DebugView() {
  const state = useCastStore((s) => s.state);
  const model = useCastStore((s) => s.model);
  const graph = useCastStore((s) => s.graph);
  const consultants = useCastStore((s) => s.consultants);
  const routing = useCastStore((s) => s.routing);
  const inbox = useCastStore((s) => s.inbox);
  const events = useCastStore((s) => s.events);
  const errors = useCastStore((s) => s.errors);
  const streamConnected = useCastStore((s) => s.streamConnected);
  return (
    <Card>
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-base">Debug</CardTitle>
          <div className="flex items-center gap-2">
            <Badge variant={streamConnected ? "default" : "destructive"}>
              SSE {streamConnected ? "live" : "disconnected"}
            </Badge>
            <Badge variant="outline">{errors.length} error(s)</Badge>
          </div>
        </div>
        <CardDescription>
          Raw responses from the backend — every field that exists, regardless of
          whether the UI exposes it.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {errors.length > 0 && (
          <div className="space-y-1">
            {errors.map((e, i) => (
              <div
                key={i}
                className="rounded border border-destructive/40 bg-destructive/5 p-2 text-xs"
              >
                <span className="font-medium text-destructive">{e.resource}</span>
                : {e.message}
              </div>
            ))}
          </div>
        )}

        <CollapsibleJson
          label={`/api/state (projection) — ${state ? Object.keys(state).length : "—"} fields`}
          data={state}
          defaultOpen={true}
        />

        <CollapsibleJson
          label={`/api/model (operating picture) — ${model ? Object.keys(model).length : "—"} fields`}
          data={model}
        />

        <CollapsibleJson
          label={`/api/graph (task graph) — ${graph ? Object.keys(graph).length : "—"} fields`}
          data={graph}
        />

        <CollapsibleJson
          label={`/api/consultants — ${consultants.length} consultant(s)`}
          data={consultants}
        />

        <CollapsibleJson
          label={`/api/routing — ${routing.length} actor(s)`}
          data={routing}
        />

        <CollapsibleJson
          label={`/api/inbox — ${inbox ? inbox.items.length : "—"} item(s)`}
          data={inbox}
        />

        <CollapsibleJson
          label={`/api/events — ${events.length} event(s)`}
          data={events.slice(-20)}
        />

        <FullContextPanel />
      </CardContent>
    </Card>
  );
}