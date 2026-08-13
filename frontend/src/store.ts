// Central client-side state (Zustand).
//
// Design: the RUST backend is the single source of truth (it owns the event
// log + projection). This store holds the *snapshot* it serves (`/api/state`)
// and treats the SSE stream as "something changed → refresh". We deliberately
// do NOT re-derive the projection in TypeScript — that would create two
// authorities. Components just subscribe to the slices they need.
import { create } from "zustand";
import {
  EventEnvelope,
  GraphView,
  Inbox,
  OperatingModel,
  Projection,
  fetchEvents,
  fetchGraph,
  fetchInbox,
  fetchModel,
  fetchState,
  subscribe,
} from "./api";

interface CastStore {
  state: Projection | null;
  /** The operating picture (`/api/model`) — the owner's curated dashboard. */
  model: OperatingModel | null;
  /** The derived graph view (`/api/graph`) — nodes + groups + tokens. */
  graph: GraphView | null;
  inbox: Inbox | null;
  events: EventEnvelope[];
  error: string | null;
  streamReady: boolean;
  /** Fetch the current snapshot (state + model + graph + inbox + recent events). Idempotent,
   *  safe to call on any event from the stream or after any mutation. */
  refresh: () => Promise<void>;
  /** Hydrate once, then keep in sync with the live event stream. Returns an
   *  unsubscribe function. Safe to call multiple times (guarded). */
  start: () => () => void;
}

function actorName(a: EventEnvelope["actor"]): string {
  if (typeof a === "string") return a;
  return a?.id ?? "system";
}

export const useCastStore = create<CastStore>((set, get) => ({
  state: null,
  model: null,
  graph: null,
  inbox: null,
  events: [],
  error: null,
  streamReady: false,

  refresh: async () => {
    try {
      const [s, m, g, i, e] = await Promise.all([
        fetchState(),
        fetchModel(),
        fetchGraph(),
        fetchInbox(),
        fetchEvents(),
      ]);
      set({
        state: s,
        model: m,
        graph: g,
        inbox: i,
        events: e.map((x) => ({ ...x, actor: actorName(x.actor) })),
        error: null,
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  start: () => {
    const unsub = subscribe(() => {
      void get().refresh();
    });
    void get().refresh();
    set({ streamReady: true });
    return unsub;
  },
}));
