// Central client-side state (Zustand).
//
// Design: the RUST backend is the single source of truth (it owns the event
// log + projection). This store holds the *snapshot* it serves (`/api/state`)
// and treats the SSE stream as "something changed → refresh". We deliberately
// do NOT re-derive the projection in TypeScript — that would create two
// authorities. Components just subscribe to the slices they need.
//
// Diagnostics (2026-08): the store also tracks connection/liveness health
// (stream up/down, last event + last refresh age) and per-resource fetch
// errors, so a silently-stale UI (backend wedge, stream drop) is visible
// instead of masquerading as "all quiet".
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
  /** The operating picture (`/api/model`) — the owner's curated dashboard. */
  model: OperatingModel | null;
  /** The derived graph view (`/api/graph`) — nodes + groups + tokens. */
  graph: GraphView | null;
  /** The consultant registry (`/api/consultants`) — identity/meta for every
   *  available (default + user-added) consultant. Configuration, not authority. */
  consultants: ConsultantConfig[];
  /** Per-actor model routing (`/api/routing`) — what each actor runs on. */
  routing: ActorRouting[];
  inbox: Inbox | null;
  events: EventEnvelope[];
  /** Per-resource fetch errors from the last refresh (empty = all good). */
  errors: ResourceError[];
  /** Whether the SSE stream is currently connected (healthy). */
  streamConnected: boolean;
  /** Monotonic counter of stream reconnects — bump = a drop happened. */
  reconnects: number;
  /** Epoch-ms of the last SSE event received (0 = none yet). */
  lastEventAt: number;
  /** Epoch-ms of the last successful refresh. */
  lastRefreshAt: number;
  /** Fetch the current snapshot (state + model + graph + inbox + recent events). */
  refresh: () => Promise<void>;
  /** Hydrate once, then keep in sync with the live event stream. Returns an
   *  unsubscribe function. Safe to call multiple times (guarded). */
  start: () => () => void;
}

function actorName(a: EventEnvelope["actor"]): string {
  if (typeof a === "string") return a;
  return a?.id ?? "system";
}

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

  refresh: async () => {
    // Per-resource errors, so a single broken endpoint doesn't take down the
    // whole snapshot silently and isn't reported as an opaque "Error".
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
        // Auto-clear errors that recovered; keep only still-failing resources.
        errors: errors.length > 0 ? errors : [],
        lastRefreshAt: Date.now(),
      }));
    } catch (e) {
      // Promise.all only rejects if a fetchWithError threw outside its own
      // catch — treat as a global error but keep the last known snapshot.
      set((cur) => ({
        errors: [
          { resource: "refresh", message: String(e), at: Date.now() },
          ...cur.errors.slice(0, 4),
        ],
      }));
    }
  },

  start: () => {
    const unsub = subscribe(
      (seqBump) => {
        // An SSE event arrived: mark the stream live + note event recency, then
        // refetch (idempotent snapshot). seqBump true = a NEW event (not just a
        // heartbeat), so we can show "last event Ns ago".
        if (seqBump) {
          set({ lastEventAt: Date.now(), streamConnected: true });
        }
        void get().refresh();
      },
      (connected) => {
        // Reflect stream health immediately (a drop shows as stale in the UI).
        set((cur) => ({
          streamConnected: connected,
          reconnects: connected ? cur.reconnects + 1 : cur.reconnects,
        }));
      }
    );
    set({ streamConnected: true });
    void get().refresh();
    return () => {
      set({ streamConnected: false });
      unsub();
    };
  },
}));
