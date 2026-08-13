// Health — live connection/liveness indicator (G5).
//
// Renders whether the SSE stream is connected, how long since the last event
// arrived, and how long since the last snapshot refresh. The whole point: a
// "silently stale" UI (backend wedge, stream drop) is visible instead of
// masquerading as idleness.
import { useEffect, useState } from "react";
import { useCastStore } from "./store";
import { Badge } from "@/components/ui/badge";

function ago(ms: number): string {
  const s = Math.floor((Date.now() - ms) / 1000);
  if (s < 0) return "just now";
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h ${m % 60}m ago`;
  return `${Math.floor(h / 24)}d ${h % 24}h ago`;
}

export default function Health() {
  const streamConnected = useCastStore((s) => s.streamConnected);
  const reconnects = useCastStore((s) => s.reconnects);
  const lastEventAt = useCastStore((s) => s.lastEventAt);
  const lastRefreshAt = useCastStore((s) => s.lastRefreshAt);

  // Re-render every few seconds so the "Ns ago" ages tick over.
  const [, setTick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setTick((t) => t + 1), 5000);
    return () => clearInterval(id);
  }, []);

  const hasEvent = lastEventAt > 0;
  const eventStale = hasEvent && Date.now() - lastEventAt > 60000;
  const refreshReflectsLive = lastRefreshAt > 0 && Date.now() - lastRefreshAt < 15000;

  return (
    <div className="flex items-center gap-1.5">
      <Badge
        variant={streamConnected ? "outline" : "destructive"}
        className={streamConnected ? "gap-1" : "gap-1"}
        title={
          reconnects > 0
            ? `stream reconnected ${reconnects} time${reconnects > 1 ? "s" : ""}`
            : undefined
        }
      >
        <span
          className={`inline-block h-2 w-2 rounded-full ${
            streamConnected ? "bg-emerald-500" : "bg-destructive"
          }`}
        />
        {streamConnected
          ? eventStale
            ? "stream up · idle"
            : "live"
          : "stream down"}
      </Badge>
      {hasEvent && (
        <span className={`text-xs ${eventStale ? "text-amber-600" : "text-muted-foreground"}`}>
          last event {ago(lastEventAt)}
        </span>
      )}
      {!refreshReflectsLive && lastRefreshAt > 0 && (
        <span className="text-xs text-muted-foreground">last refresh {ago(lastRefreshAt)}</span>
      )}
    </div>
  );
}
