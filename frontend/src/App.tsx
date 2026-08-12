import { useEffect, useRef, useState } from "react";
import SetupWizard from "./SetupWizard";
import { useCastStore } from "./store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Decision,
  Inbox,
  Message,
  Projection,
  TaskStatus,
  decide,
  sendMessage,
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
  const state = useCastStore((s) => s.state);
  const inbox = useCastStore((s) => s.inbox);
  const error = useCastStore((s) => s.error);
  const refresh = useCastStore((s) => s.refresh);
  const start = useCastStore((s) => s.start);

  useEffect(() => {
    // Hydrate the snapshot and subscribe to the live stream (once).
    const unsub = start();
    return unsub;
  }, [start]);

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

      {/* First-run: no company cast yet (only the seed PM) -> show the setup
          wizard. Once engaged it drives the same engine as `cast init`. */}
      {state && state.agents.filter((a) => a.id !== "pm").length === 0 && (
        <SetupWizard onDone={refresh} />
      )}
      {state && state.agents.filter((a) => a.id !== "pm").length > 0 && (
        <>
          <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)}>
            <TabsList className="grid w-full max-w-xl grid-cols-6">
              <TabsTrigger value="chat">Chat</TabsTrigger>
              <TabsTrigger value="board">Board</TabsTrigger>
              <TabsTrigger value="team">Team</TabsTrigger>
              <TabsTrigger value="decisions">Decisions</TabsTrigger>
              <TabsTrigger value="inbox" className="relative">
                Inbox
                {inbox && inbox.unread > 0 && (
                  <span className="absolute -right-1 -top-1">
                    <Badge className="bg-primary text-primary-foreground">
                      {inbox.unread}
                    </Badge>
                  </span>
                )}
              </TabsTrigger>
              <TabsTrigger value="activity">Activity</TabsTrigger>
            </TabsList>
            {tab === "chat" && <Chat state={state} onSent={refresh} />}
            {tab === "board" && <Board tasks={state.tasks} />}
            {tab === "team" && <Team agents={state.agents} />}
            {tab === "decisions" && (
              <Decisions decisions={state.decisions} onDecide={refresh} />
            )}
            {tab === "inbox" && <InboxView inbox={inbox} onDecide={refresh} />}
            {tab === "activity" && <Activity state={state} />}
          </Tabs>
        </>
      )}
    </div>
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
      <div className="composer" style={{ display: "flex", gap: 10, marginTop: 12 }}>
        <Input
          value={draft}
          placeholder='e.g. "Build me a todo app"'
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && send()}
        />
        <Button className="primary" onClick={send} disabled={busy || !draft.trim()}>
          Send
        </Button>
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
            <div className="actions" style={{ display: "flex", gap: 8, marginTop: 10 }}>
              <Button
                size="sm"
                className="bg-emerald-500 text-white hover:bg-emerald-600"
                onClick={() => void decide(d.id, d.subject, true).then(onDecide)}
              >
                Approve
              </Button>
              <Button
                size="sm"
                variant="outline"
                className="text-destructive"
                onClick={() => void decide(d.id, d.subject, false).then(onDecide)}
              >
                Reject
              </Button>
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
          <div className="actions" style={{ display: "flex", gap: 8, marginTop: 10 }}>
            <Button
              size="sm"
              className="bg-emerald-500 text-white hover:bg-emerald-600"
              onClick={() => void decide(it.id, it.subject, true).then(onDecide)}
            >
              Approve
            </Button>
            <Button
              size="sm"
              variant="outline"
              className="text-destructive"
              onClick={() => void decide(it.id, it.subject, false).then(onDecide)}
            >
              Reject
            </Button>
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
