import { lazy, Suspense, useEffect, useRef, useState } from "react";
import SetupWizard from "./SetupWizard";
import { useCastStore } from "./store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { identityForAgent } from "./cast";
import {
  Decision,
  Inbox,
  Message,
  Projection,
  TaskStatus,
  decide,
  sendMessage,
} from "./api";

type Tab = "chat" | "board" | "team" | "decisions" | "inbox" | "activity" | "sketch" | "advisor";

// Lazy: tldraw/excalidraw is ~1MB — never load it unless the owner opens Sketch.
const Whiteboard = lazy(() => import("./Whiteboard"));
const Advisor = lazy(() => import("./Advisor"));

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

function agentName(id: string): string {
  const identity = identityForAgent(id, "");
  return identity?.stable_name ?? AGENT_NAMES[id]?.split(" · ")[0] ?? id;
}

function agentAvatar(id: string): string | undefined {
  return identityForAgent(id, "")?.avatar;
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
            <TabsList className="grid w-full max-w-3xl grid-cols-8">
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
              <TabsTrigger value="advisor">Advisor</TabsTrigger>
              <TabsTrigger value="sketch">Sketch</TabsTrigger>
            </TabsList>
            {tab === "chat" && <Chat state={state} onSent={refresh} />}
            {tab === "board" && <Board tasks={state.tasks} />}
            {tab === "team" && <Team agents={state.agents} />}
            {tab === "decisions" && (
              <Decisions decisions={state.decisions} onDecide={refresh} />
            )}
            {tab === "inbox" && <InboxView inbox={inbox} onDecide={refresh} />}
            {tab === "activity" && <Activity />}
            {tab === "sketch" && (
              <Suspense fallback={<Card className="muted"><CardContent className="py-6">Loading sketchpad…</CardContent></Card>}>
                <Whiteboard onSaved={refresh} />
              </Suspense>
            )}
            {tab === "advisor" && (
              <Suspense fallback={<Card className="muted"><CardContent className="py-6">Loading advisor…</CardContent></Card>}>
                <Advisor thread={state.advisor_thread} onChanged={refresh} />
              </Suspense>
            )}
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
    <Card>
      <CardHeader>
        <CardTitle>Chat with your Project Manager</CardTitle>
        <CardDescription>The owner ↔ PM channel. Tell it what to build.</CardDescription>
      </CardHeader>
      <CardContent>
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
          <Button onClick={send} disabled={busy || !draft.trim()}>
            Send
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function Board({ tasks }: { tasks: Projection["tasks"] }) {
  return (
    <div>
      {tasks.length === 0 && (
        <Card className="muted">
          <CardContent className="pt-6">No tasks yet — tell the PM what to build.</CardContent>
        </Card>
      )}
      <div className="board">
        {TASK_COLUMNS.map((col) => (
          <Card key={col.key} className="col">
            <CardHeader className="pb-2">
              <CardTitle className="text-sm">{col.label}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="flex flex-col gap-2">
                {tasks
                  .filter((t) => t.status === col.key)
                  .map((t) => (
                    <Card key={t.id} className="border-border/60">
                      <CardContent className="p-3">
                        <div className="text-sm font-medium leading-snug">{t.title}</div>
                        <Badge variant={t.status === "blocked" ? "destructive" : "secondary"} className="mt-2">
                          {t.status}
                        </Badge>
                        <div className="text-xs text-muted-foreground mt-1">{t.kind}</div>
                      </CardContent>
                    </Card>
                  ))}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}

function Team({ agents }: { agents: Projection["agents"] }) {
  return (
    <div>
      {agents.length === 0 && <Card className="muted"><CardContent className="pt-6">No one hired yet.</CardContent></Card>}
      <div className="grid gap-3 sm:grid-cols-2">
        {agents.map((a) => {
          const identity = identityForAgent(a.id, a.role);
          const avatar = identity?.avatar ?? agentAvatar(a.id);
          return (
            <Card key={a.id}>
              <CardContent className="pt-6">
                <div className="flex items-center gap-4">
                  {avatar ? (
                    <img src={avatar} alt={identity?.name ?? a.id} className="h-12 w-12 rounded-xl shrink-0" />
                  ) : (
                    <div
                      className="flex h-11 w-11 items-center justify-center rounded-full font-bold"
                      style={{ background: "linear-gradient(135deg,#4f8cff,#7a5cff)", color: "#fff" }}
                    >
                      {a.id === "pm" ? "SC" : initials(a.id)}
                    </div>
                  )}
                  <div>
                    <div className="font-semibold">{identity?.name ?? agentName(a.id) ?? agentLabel(a.id).split(" · ")[0]}</div>
                    <div className="text-sm text-muted-foreground">{a.role}</div>
                  </div>
                </div>
                {identity && identity.cv.length > 0 && (
                  <ul className="mt-3 space-y-1 text-xs text-muted-foreground">
                    {identity.cv.map((line, i) => (
                      <li key={i} className="flex gap-1.5">
                        <span className="text-primary">•</span>
                        <span>{line}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          );
        })}
      </div>
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
    <div className="flex flex-col gap-3">
      {decisions.length === 0 && <Card className="muted"><CardContent className="py-6">No decisions recorded yet.</CardContent></Card>}
      {decisions.map((d) => (
        <Card key={d.id}>
          <CardContent className="pt-6">
            <div className="flex items-center justify-between gap-2">
              <div className="font-semibold">{d.subject}</div>
              <Badge variant={d.status === "proposed" ? "default" : d.status === "approved" ? "secondary" : "destructive"}>
                {d.status}
              </Badge>
            </div>
            {Object.entries(d.options).length > 0 && (
              <ul className="mt-2 list-disc pl-5 text-sm">
                {Object.entries(d.options).map(([k, v]) => (
                  <li key={k}>
                    <strong>{k}:</strong> {v}
                  </li>
                ))}
              </ul>
            )}
            {d.recommendation && <div className="text-xs text-muted-foreground mt-1">Pm recommends: {d.recommendation}</div>}
            {d.owner_verdict && <div className="text-xs text-muted-foreground mt-1">Owner: {d.owner_verdict}</div>}
            {d.status === "proposed" && (
              <div className="flex gap-2 mt-3">
                <Button size="sm" onClick={() => void decide(d.id, d.subject, true).then(onDecide)}>
                  Approve
                </Button>
                <Button size="sm" variant="outline" onClick={() => void decide(d.id, d.subject, false).then(onDecide)}>
                  Reject
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
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
    <div className="flex flex-col gap-3">
      {items.length === 0 && <Card className="muted"><CardContent className="py-6">🟢 Nothing needs your attention right now.</CardContent></Card>}
      {items.map((it) => (
        <Card key={it.id} className="border-primary/40">
          <CardContent className="pt-6">
            <Badge variant="outline" className="mb-2">awaiting your decision</Badge>
            <div className="font-semibold">{it.subject}</div>
            {Object.entries(it.options).length > 0 && (
              <ul className="mt-2 list-disc pl-5 text-sm">
                {Object.entries(it.options).map(([k, v]) => (
                  <li key={k}>
                    <strong>{k}:</strong> {v}
                  </li>
                ))}
              </ul>
            )}
            <div className="flex gap-2 mt-3">
              <Button size="sm" onClick={() => void decide(it.id, it.subject, true).then(onDecide)}>Approve</Button>
              <Button size="sm" variant="outline" onClick={() => void decide(it.id, it.subject, false).then(onDecide)}>Reject</Button>
            </div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

function Activity() {
  // The Activity view is powered by the REAL event stream from the store (each
  // event is a durable log entry), not a reconstruction from the projection.
  const events = useCastStore((s) => s.events);
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-base">Company activity</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="stream">
          {events.length === 0 && <div className="text-sm text-muted-foreground">Nothing yet.</div>}
          {[...events].reverse().slice(0, 60).map((ev) => (
            <div className="row" key={ev.event_id}>
              <span className="seq">#{ev.sequence}</span>
              <span className="who">{ev.event_type}</span>
              <span className="muted text-xs">{ev.actor as string}</span>
            </div>
          ))}
        </div>
      </CardContent>
    </Card>
  );
}
