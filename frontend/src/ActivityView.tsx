// ActivityView — the live event log with full diagnostic payloads (G1 + G6).
//
// G1: renders each event's `data` payload — esp. the `error` field on
//   ActivityFailed / WorkPaused / PlanActionRejected — instead of throwing it
//   away. Failures are highlighted destructively.
// G6: timestamps, a type/actor filter, and an expandable raw payload per row.
//
// Still genuinely the event stream (store.events <- /api/events), never a
// reconstruction — the event-sourcing payoff showing up in the UI.
import { useMemo, useState } from "react";
import { useCastStore } from "./store";
import { EventEnvelope } from "./api";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

const FAILURE_TYPES = new Set([
  "ActivityFailed",
  "WorkPaused",
  "PlanActionRejected",
  "MergeConflictDetected",
]);

function timeLabel(ts: string): string {
  const d = new Date(ts);
  return isNaN(d.getTime()) ? "" : d.toLocaleTimeString();
}

/** The human-meaningful summary of an event's data payload (falls back to a
 *  compact JSON if no specific fields match). */
function dataLine(ev: EventEnvelope): { text: string; isError: boolean } | null {
  const d = ev.data ?? {};
  if (!d || Object.keys(d).length === 0) return null;
  switch (ev.event_type) {
    case "ActivityFailed":
      return { text: String(d.error ?? d.id ?? ""), isError: true };
    case "PlanActionRejected":
      return { text: `${String(d.who ?? "")} refused: ${String(d.action ?? "")} — because ${String(d.reason ?? "")}`, isError: true };
    case "WorkPaused":
      return { text: `paused: ${String(d.reason ?? "")}`, isError: true };
    case "OrchestrationRun":
      return { text: `${String(d.trigger ?? "")} — planned ${(d.planned as unknown[])?.length ?? 0} action(s)`, isError: false };
    case "CostIncurred":
      return { text: `${String(d.agent_id ?? "")} ${String(d.prompt_tokens ?? 0)}p/${String(d.completion_tokens ?? 0)}c $${Number(d.estimated_usd ?? 0).toFixed(4)}`, isError: false };
    case "MessageSent":
      return { text: String(d.body ?? ""), isError: false };
    default:
      return { text: JSON.stringify(d), isError: false };
  }
}

export default function ActivityView() {
  const events = useCastStore((s) => s.events);
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<"all" | "failures" | "model">("all");
  const [expanded, setExpanded] = useState<number | null>(null);

  const sorted = useMemo(() => [...events].reverse(), [events]);

  const filtered = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return sorted.filter((ev) => {
      const type = String(ev.event_type);
      const typeLower = type.toLowerCase();
      const actor = String(ev.actor).toLowerCase();
      if (filter === "failures" && !FAILURE_TYPES.has(type)) return false;
      if (filter === "model" && !["OrchestrationRun", "CostIncurred"].includes(type)) return false;
      if (needle && !typeLower.includes(needle) && !actor.includes(needle)) return false;
      return true;
    });
  }, [sorted, q, filter]);

  const counts = useMemo(() => {
    let failures = 0;
    for (const ev of events) if (FAILURE_TYPES.has(String(ev.event_type))) failures++;
    return { failures, total: events.length };
  }, [events]);

  return (
    <Card>
      <CardHeader className="pb-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle className="text-base">
            Company activity
            <span className="ml-2 text-sm font-normal text-muted-foreground">
              {counts.total} events
              {counts.failures > 0 && (
                <Badge variant="destructive" className="ml-2">{counts.failures} failure</Badge>
              )}
            </span>
          </CardTitle>
          <div className="flex gap-1.5">
            <Badge
              variant={filter === "all" ? "default" : "outline"}
              className="cursor-pointer"
              onClick={() => setFilter("all")}
            >
              all
            </Badge>
            <Badge
              variant={filter === "failures" ? "destructive" : "outline"}
              className="cursor-pointer"
              onClick={() => setFilter("failures")}
            >
              failures
            </Badge>
            <Badge
              variant={filter === "model" ? "secondary" : "outline"}
              className="cursor-pointer"
              onClick={() => setFilter("model")}
            >
              model
            </Badge>
          </div>
        </div>
        <CardDescription>The raw event stream — the single source of truth.</CardDescription>
        <Input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="filter by type or actor…"
          className="mt-2 max-w-xs"
        />
      </CardHeader>
      <CardContent>
        {filtered.length === 0 && (
          <div className="text-sm text-muted-foreground">
            {events.length === 0 ? "Nothing yet." : "No events match that filter."}
          </div>
        )}
        <div className="stream">
          {filtered.slice(0, 120).map((ev) => {
            const type = String(ev.event_type);
            const isFailure = FAILURE_TYPES.has(type.toLowerCase());
            const line = dataLine(ev);
            const isOpen = expanded === ev.sequence;
            return (
              <div
                key={ev.sequence}
                onClick={() => setExpanded(isOpen ? null : ev.sequence)}
                className={`row cursor-pointer rounded px-1.5 py-1 hover:bg-muted/50 ${
                  isFailure ? "!bg-destructive/5" : ""
                }`}
              >
                <div className="flex items-baseline gap-2">
                  <span className="seq">#{ev.sequence}</span>
                  <span className="who">{type}</span>
                  <span className="muted text-xs">{ev.actor as string}</span>
                  <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">
                    {timeLabel(ev.timestamp)}
                  </span>
                </div>
                {line && (
                  <div
                    className={`mt-0.5 break-words pl-6 text-xs ${
                      line.isError ? "text-destructive" : "text-muted-foreground"
                    }`}
                  >
                    {line.text}
                  </div>
                )}
                {isOpen && (
                  <pre className="mt-1 max-h-64 overflow-auto rounded bg-muted p-2 text-[10px]">
                    {JSON.stringify(ev.data, null, 2)}
                  </pre>
                )}
              </div>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}
