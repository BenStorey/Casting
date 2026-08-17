import { create } from "zustand";
import {
  ActorRouting,
  ConsultantConfig,
  EventEnvelope,
  GraphView,
  Inbox,
  OperatingModel,
  Projection,
  fetchConsultants,
  fetchEvents,
  fetchGraph,
  fetchInbox,
  fetchModel,
  fetchRouting,
  fetchState,
  subscribe,
} from "./api";

/** A per-endpoint fetch error, so a failing API is attributable (not a generic
 *  banner hiding which resource broke). */
export interface ResourceError {
  resource: string;
  message: string;
  at: number;
}

interface CastStore {
  state: Projection | null;
  model: OperatingModel | null;
  graph: GraphView | null;
  consultants: ConsultantConfig[];
  routing: ActorRouting[];
  inbox: Inbox | null;
  events: EventEnvelope[];
  errors: ResourceError[];
  streamConnected: boolean;
  reconnects: number;
  lastEventAt: number;
  lastRefreshAt: number;
  /** Fetch the full snapshot (state + model + graph + inbox + recent events). */
  refresh: () => Promise<void>;
  /** Fetch only the projection — called on every SSE event. Much cheaper. */
  refreshState: () => Promise<void>;
  /** Fetch a single resource lazily (e.g. when switching tabs). */
  refreshLazy: (resource: string) => Promise<void>;
  /** Hydrate once, then keep in sync with the live event stream. Returns an
   *  unsubscribe function. Safe to call multiple times (guarded). */
  start: () => () => void;
}

function actorName(a: EventEnvelope["actor"]): string {
  if (typeof a === "string") return a;
  if (!a || typeof a !== "object") return "system";
  // Rust's serde untagged enum: {"Agent":{"id":"diego"}}, {"Owner":null}, {"System":null}
  if ("Agent" in a && a.Agent && typeof a.Agent === "object")
    return ((a.Agent as Record<string, unknown>).id as string) ?? "system";
  if ("Owner" in a) return "owner";
  if ("System" in a) return "system";
  return ((a as Record<string, unknown>).id as string) ?? "system";
}

// Coalesce SSE-triggered projection refreshes. The backend can emit a burst of
// events (e.g. ~10 on startup); fetching the full projection once per event
// hammers /api/state. We cap it to at most one fetch per window.
const STATE_REFRESH_THROTTLE_MS = 250;
let lastStateFetchAt = 0;
let stateRefreshScheduled = false;

async function fetchWithError<T>(resource: string, fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    throw new Error(`${resource}: ${e instanceof Error ? e.message : String(e)}`);
  }
}

export const useCastStore = create<CastStore>((set, get) => ({
  state: null,
  model: null,
  graph: null,
  consultants: [],
  routing: [],
  inbox: null,
  events: [],
  errors: [],
  streamConnected: false,
  reconnects: 0,
  lastEventAt: 0,
  lastRefreshAt: 0,

  // Fast path: only re-fetch the projection on SSE events. The other endpoints
  // (model, graph, inbox) change less frequently and are fetched on initial load
  // and lazy refreshes.
  refreshState: async () => {
    // Coalesce bursts of SSE events into a single fetch. If events arrive
    // faster than the throttle window, schedule one trailing refresh instead of
    // slamming /api/state on every event.
    const now = Date.now();
    const sinceLast = now - lastStateFetchAt;
    if (sinceLast < STATE_REFRESH_THROTTLE_MS) {
      if (!stateRefreshScheduled) {
        stateRefreshScheduled = true;
        const wait = STATE_REFRESH_THROTTLE_MS - sinceLast;
        setTimeout(() => {
          stateRefreshScheduled = false;
          void get().refreshState();
        }, wait);
      }
      return;
    }
    lastStateFetchAt = now;
    const errors: ResourceError[] = [];
    try {
      const s = await fetchWithError("state", fetchState).catch((err) => {
        errors.push({ resource: "state", message: String(err), at: Date.now() });
        return null;
      });
      set((cur) => ({
        state: s ?? cur.state,
        errors: errors.length > 0 ? errors : [],
        lastRefreshAt: Date.now(),
      }));
    } catch (e) {
      set((cur) => ({
        errors: [{ resource: "refresh", message: String(e), at: Date.now() }, ...cur.errors.slice(0, 4)],
      }));
    }
  },

  refreshLazy: async (resource: string) => {
    try {
      switch (resource) {
        case "model": {
          const m = await fetchWithError("model", fetchModel);
          set({ model: m });
          break;
        }
        case "graph": {
          const g = await fetchWithError("graph", fetchGraph);
          set({ graph: g });
          break;
        }
        case "inbox": {
          const i = await fetchWithError("inbox", fetchInbox);
          set({ inbox: i });
          break;
        }
        case "events": {
          const e = await fetchWithError("events", fetchEvents);
          set({ events: e.map((x) => ({ ...x, actor: actorName(x.actor) })) });
          break;
        }
      }
    } catch {
      // Silent — individual errors are visible via the refresh() error mechanism.
    }
  },

  refresh: async () => {
    const errors: ResourceError[] = [];
    try {
      const [s, m, g, c, r, i, e] = await Promise.all([
        fetchWithError("state", fetchState).catch((err) => {
          errors.push({ resource: "state", message: String(err), at: Date.now() });
          return null;
        }),
        fetchWithError("model", fetchModel).catch((err) => {
          errors.push({ resource: "model", message: String(err), at: Date.now() });
          return null;
        }),
        fetchWithError("graph", fetchGraph).catch((err) => {
          errors.push({ resource: "graph", message: String(err), at: Date.now() });
          return null;
        }),
        fetchWithError("consultants", fetchConsultants).catch((err) => {
          errors.push({ resource: "consultants", message: String(err), at: Date.now() });
          return [];
        }),
        fetchWithError("routing", fetchRouting).catch((err) => {
          errors.push({ resource: "routing", message: String(err), at: Date.now() });
          return [];
        }),
        fetchWithError("inbox", fetchInbox).catch((err) => {
          errors.push({ resource: "inbox", message: String(err), at: Date.now() });
          return null;
        }),
        fetchWithError("events", fetchEvents).catch((err) => {
          errors.push({ resource: "events", message: String(err), at: Date.now() });
          return [];
        }),
      ]);
      set((cur) => ({
        state: s ?? cur.state,
        model: m ?? cur.model,
        graph: g ?? cur.graph,
        consultants: c,
        routing: r,
        inbox: i ?? cur.inbox,
        events: e.map((x) => ({ ...x, actor: actorName(x.actor) })),
        errors: errors.length > 0 ? errors : [],
        lastRefreshAt: Date.now(),
      }));
    } catch (e) {
      set((cur) => ({
        errors: [
          { resource: "refresh", message: String(e), at: Date.now() },
          ...cur.errors.slice(0, 4),
        ],
      }));
    }
  },

  start: () => {
    let wasConnected = false;
    const unsub = subscribe(
      (seqBump) => {
        // SSE event arrived: mark stream live, then do a FAST refresh
        // (only the projection, not the full 7-endpoint blast).
        if (seqBump) {
          set({ lastEventAt: Date.now(), streamConnected: true });
        }
        // Only re-fetch the projection — cheap and sufficient for live updates.
        void get().refreshState();
      },
      (connected) => {
        // Count only genuine reconnects (disconnected -> connected), not the
        // initial connect or a stray duplicate status.
        set((cur) => ({
          streamConnected: connected,
          reconnects: connected && !wasConnected ? cur.reconnects + 1 : cur.reconnects,
        }));
        wasConnected = connected;
      }
    );
    set({ streamConnected: true });
    // Initial full load: fetch everything once.
    void get().refresh();
    return () => {
      set({ streamConnected: false });
      unsub();
    };
  },
}));