// Central client-side state (Zustand).
//
// Design: the RUST backend is the single source of truth (it owns the event
// log + projection). This store holds the *snapshot* it serves (`/api/state`)
// and treats the SSE stream as "something changed → refresh". We deliberately
// do NOT re-derive the projection in TypeScript — that would create two
// authorities. Components just subscribe to the slices they need.
import { create } from "zustand";
import {
  Inbox,
  Projection,
  fetchInbox,
  fetchState,
  subscribe,
} from "./api";

interface CastStore {
  state: Projection | null;
  inbox: Inbox | null;
  error: string | null;
  streamReady: boolean;
  /** Fetch the current snapshot (state + inbox). Idempotent, safe to call on
   *  any event from the stream or after any mutation. */
  refresh: () => Promise<void>;
  /** Hydrate once, then keep in sync with the live event stream. Returns an
   *  unsubscribe function. Safe to call multiple times (guarded). */
  start: () => () => void;
}

export const useCastStore = create<CastStore>((set, get) => ({
  state: null,
  inbox: null,
  error: null,
  streamReady: false,

  refresh: async () => {
    try {
      const [s, i] = await Promise.all([fetchState(), fetchInbox()]);
      set({ state: s, inbox: i, error: null });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  start: () => {
    const unsub = subscribe(() => {
      void get().refresh();
    });
    void get().refresh();
    // Mark the stream live; the first refresh eats the 400ms SSE handshake.
    set({ streamReady: true });
    return unsub;
  },
}));
