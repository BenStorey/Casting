import { useCallback, useEffect, useRef, useState } from "react";
import {
  Decision,
  Inbox,
  Message,
  Projection,
  TaskStatus,
  decide,
  fetchInbox,
  fetchState,
  sendMessage,
  subscribe,
} from "./api";

type Tab = "chat" | "board" | "team" | "decisions" | "inbox" | "activity";

const TASK_COLUMNS: { key: TaskStatus; label: string }[] = [
  { key: "backlog", label: "Backlog" },
  { key: "working", label: "Working" },
  { key: "blocked", label: "Blocked" },
  { key: "done", label: "Done" },
];

const AGENT_NAMES: Record<string, string> = {
  pm: "Sarah Chen · PM",
  "marcus-reed": "Marcus Reed · Engineering",
  "maya-patel": "Maya Patel · QA",
};

function agentLabel(id: string): string {
  return AGENT_NAMES[id] ?? id;
}

export default function App() {
  const [tab, setTab] = useState<Tab>("chat");
  const [state, setState] = useState<Projection | null>(null);
  const [inbox, setInbox] = useState<Inbox | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [s, i] = await Promise.all([fetchState(), fetchInbox()]);
      setState(s);
      setInbox(i);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    const unsub = subscribe(refresh);
    return unsub;
  }, [refresh]);

  return (
    <div className="app">
      <header className="top">
        <div className="logo">🎬</div>
        <div className="brand">
          <h1>Casting</h1>
          <p>Your autonomous software company</p>
        </div>
      </header>

      {error && <div className="banner">⚠️ {error}</div>}

      <nav className="tabs">
        <TabButton active={tab === "chat"} onClick={() => setTab("chat")}>Chat</TabButton>
        <TabButton active={tab === "board"} onClick={() => setTab("board")}>Board</TabButton>
        <TabButton active={tab === "team"} onClick={() => setTab("team")}>Team</TabButton>
        <TabButton active={tab === "decisions"} onClick={() => setTab("decisions")}>Decisions</TabButton>
        <TabButton
          active={tab === "inbox"}
          onClick={() => setTab("inbox")}
          badge={inbox?.unread}
        >
          Inbox
        </TabButton>
        <TabButton active={tab === "activity"} onClick={() => setTab("activity")}>Activity</TabButton>
      </nav>

      {state && (
        <>
          {tab === "chat" && <Chat state={state} onSent={refresh} />}
          {tab === "board" && <Board tasks={state.tasks} />}
          {tab === "team" && <Team agents={state.agents} />}
          {tab === "decisions" && (
            <Decisions decisions={state.decisions} onDecide={refresh} />
          )}
          {tab === "inbox" && (
            <InboxView inbox={inbox} onDecide={refresh} />
          )}
          {tab === "activity" && <Activity state={state} />}
        </>
      )}
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
  badge,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  badge?: number;
}) {
  return (
    <button className={active ? "active" : ""} onClick={onClick}>
      {children}
      {badge != null && badge > 0 && <span className="badge">{badge}</span>}
    </button>
  );
}

function Chat({ state, onSent }: { state: Projection; onSent: () => void }) {
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [state.messages.length]);

  const send = async () => {
    const body = draft.trim();
    if (!body || busy) return;
    setBusy(true);
    try {
      await sendMessage(body);
      setDraft("");
      onSent();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="card">
      <h3 style={{ marginTop: 0 }}>Chat with your Project Manager</h3>
      <div className="thread">
        {state.messages.length === 0 && (
          <div className="muted small">Say hello and tell the PM what you want to build.</div>
        )}
        {state.messages.map((m: Message) => (
          <div key={m.id} className={`bubble ${m.from === "owner" ? "owner" : "pm"}`}>
            <div className="from">{m.from === "owner" ? "You" : agentLabel(m.from)}</div>
            {m.body}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
      <div className="composer">
        <input
          value={draft}
          placeholder='e.g. "Build me a todo app"'
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && send()}
        />
        <button className="primary" onClick={send} disabled={busy || !draft.trim()}>
          Send
        </button>
      </div>
    </div>
  );
}

function Board({ tasks }: { tasks: Projection["tasks"] }) {
  return (
    <div>
      {tasks.length === 0 && (
        <div className="card muted">No tasks yet — tell the PM what to build.</div>
      )}
      <div className="board">
        {TASK_COLUMNS.map((col) => (
          <div className="col" key={col.key}>
            <h3>{col.label}</h3>
            {tasks
              .filter((t) => t.status === col.key)
              .map((t) => (
                <div className="tcard" key={t.id}>
                  <div className="title">{t.title}</div>
                  <div className="meta">
                    {t.assignee ? agentLabel(t.assignee) : "unassigned"} · {t.kind}
                  </div>
                </div>
              ))}
          </div>
        ))}
      </div>
    </div>
  );
}

function Team({ agents }: { agents: Projection["agents"] }) {
  return (
    <div>
      {agents.length === 0 && <div className="card muted">No one hired yet.</div>}
      {agents.map((a) => (
        <div className="card" key={a.id} style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <div
            style={{
              width: 44,
              height: 44,
              borderRadius: 22,
              background: "linear-gradient(135deg,#4f8cff,#7a5cff)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontWeight: 700,
              color: "#fff",
            }}
          >
            {a.id === "pm" ? "SC" : initials(a.id)}
          </div>
          <div>
            <div style={{ fontWeight: 600 }}>{agentLabel(a.id).split(" · ")[0]}</div>
            <div className="muted small">{a.role}</div>
          </div>
        </div>
      ))}
    </div>
  );
}

function initials(id: string): string {
  return id
    .split("-")
    .map((p) => p[0]?.toUpperCase() ?? "")
    .slice(0, 2)
    .join("");
}

function Decisions({
  decisions,
  onDecide,
}: {
  decisions: Decision[];
  onDecide: () => void;
}) {
  return (
    <div>
      {decisions.length === 0 && <div className="card muted">No decisions recorded yet.</div>}
      {decisions.map((d) => (
        <div className="decision" key={d.id}>
          <div className={`status ${d.status}`}>{d.status}</div>
          <div style={{ fontWeight: 600, margin: "4px 0" }}>{d.subject}</div>
          {Object.entries(d.options).length > 0 && (
            <ul>
              {Object.entries(d.options).map(([k, v]) => (
                <li key={k}>
                  <strong>{k}:</strong> {v}
                </li>
              ))}
            </ul>
          )}
          {d.recommendation && <div className="small muted">Pm recommends: {d.recommendation}</div>}
          {d.owner_verdict && <div className="small muted">Owner: {d.owner_verdict}</div>}
          {d.status === "proposed" && (
            <div className="actions">
              <button className="approve" onClick={() => void decide(d.id, d.subject, true).then(onDecide)}>
                Approve
              </button>
              <button className="reject" onClick={() => void decide(d.id, d.subject, false).then(onDecide)}>
                Reject
              </button>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function InboxView({
  inbox,
  onDecide,
}: {
  inbox: Inbox | null;
  onDecide: () => void;
}) {
  const items = inbox?.items ?? [];
  return (
    <div>
      {items.length === 0 && <div className="card muted">🟢 Nothing needs your attention right now.</div>}
      {items.map((it) => (
        <div className="decision" key={it.id}>
          <div className="status proposed">awaiting your decision</div>
          <div style={{ fontWeight: 600, margin: "4px 0" }}>{it.subject}</div>
          {Object.entries(it.options).length > 0 && (
            <ul>
              {Object.entries(it.options).map(([k, v]) => (
                <li key={k}>
                  <strong>{k}:</strong> {v}
                </li>
              ))}
            </ul>
          )}
          <div className="actions">
            <button className="approve" onClick={() => void decide(it.id, it.subject, true).then(onDecide)}>
              Approve
            </button>
            <button className="reject" onClick={() => void decide(it.id, it.subject, false).then(onDecide)}>
              Reject
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}

function Activity({ state }: { state: Projection }) {
  // We only have the projection here; the event stream is served at /api/events.
  // Render a summary composed from the projection as a lightweight activity view.
  const rows: { seq: number; text: string }[] = [];
  state.requirements.forEach((r, i) => rows.push({ seq: i + 1, text: `Requirement created: ${r.title}` }));
  state.tasks.forEach((t, i) => rows.push({ seq: 100 + i, text: `Task ${t.id}: ${t.title} → ${t.status}` }));
  state.decisions.forEach((d, i) => rows.push({ seq: 200 + i, text: `Decision ${d.id}: ${d.subject} (${d.status})` }));
  state.observations.forEach((o, i) => rows.push({ seq: 300 + i, text: `Observation from ${agentLabel(o.from)}: ${o.subject}` }));
  rows.sort((a, b) => a.seq - b.seq);

  return (
    <div className="card">
      <h3 style={{ marginTop: 0 }}>Company activity</h3>
      <div className="stream">
        {rows.map((r, i) => (
          <div className="row" key={i}>
            <span className="seq">#{r.seq}</span>
            <span className="who">{r.text}</span>
          </div>
        ))}
        {rows.length === 0 && <div className="muted">Nothing yet.</div>}
      </div>
    </div>
  );
}
