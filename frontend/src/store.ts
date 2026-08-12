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
  Inbox,
  Projection,
  fetchEvents,
  fetchInbox,
  fetchState,
  subscribe,
} from "./api";

interface CastStore {
  state: Projection | null;
  inbox: Inbox | null;
  events: EventEnvelope[];
  error: string | null;
  streamReady: boolean;
  /** Fetch the current snapshot (state + inbox + recent events). Idempotent,
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
  inbox: null,
  events: [],
  error: null,
  streamReady: false,

  refresh: async () => {
    try {
      const [s, i, e] = await Promise.all([fetchState(), fetchInbox(), fetchEvents()]);
      set({
        state: s,
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
