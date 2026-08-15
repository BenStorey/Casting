import { lazy, Suspense, useEffect, useRef, useState } from "react";
import SetupWizard from "./SetupWizard";
import { useCastStore } from "./store";
import Health from "./Health";
import ActivityView from "./ActivityView";
import DebugView from "./DebugView";
import TaskDrawer from "./TaskDrawer";
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
import { identityForAgent } from "./identities";
import Overview from "./Overview";
import GraphView from "./GraphView";
import {
  Decision,
  Inbox,
  Message,
  Observation,
  Projection,
  Task,
  TaskStatus,
  decide,
  sendMessage,
} from "./api";
import { TASK_COLUMNS } from "./boardColumns";
import TelegramConnect from "./TelegramConnect";

type Tab = "overview" | "graph" | "chat" | "board" | "team" | "decisions" | "inbox" | "activity" | "debug" | "advisor" | "sketch" | "settings";

// Lazy: Excalidraw is ~1MB — never load it unless the owner opens Sketch.
const Whiteboard = lazy(() => import("./Whiteboard"));
const Advisor = lazy(() => import("./Advisor"));

const AGENT_NAMES: Record<string, string> = {
  pm: "Sarah Chen · PM",
};

function agentLabel(id: string): string {
  const c = useCastStore.getState().consultants;
  return identityForAgent(id, "", c)?.name ?? AGENT_NAMES[id] ?? id;
}

function agentName(id: string): string {
  const c = useCastStore.getState().consultants;
  return identityForAgent(id, "", c)?.name ?? AGENT_NAMES[id]?.split(" · ")[0] ?? id;
}

function agentAvatar(id: string): string | undefined {
  const c = useCastStore.getState().consultants;
  return identityForAgent(id, "", c)?.avatar ?? undefined;
}

export default function App() {
  const [tab, setTab] = useState<Tab>("chat");
  const [openTask, setOpenTask] = useState<Task | null>(null);
  const state = useCastStore((s) => s.state);
  const model = useCastStore((s) => s.model);
  const graph = useCastStore((s) => s.graph);
  const consultants = useCastStore((s) => s.consultants);
  const inbox = useCastStore((s) => s.inbox);
  const errors = useCastStore((s) => s.errors);
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
        <div className="ml-auto">
          <Health />
        </div>
      </header>

      {/* G8: per-resource errors — which endpoint broke, so a partial snapshot
          isn't mistaken for "all quiet". Auto-clears on successful refetch. */}
      {errors.length > 0 && (
        <div className="banner">
          {errors.map((e) => (
            <div key={`${e.resource}-${e.at}`}>
              ⚠️ <strong>{e.resource}</strong>: {e.message.replace(/^[^:]*: /, "")}
            </div>
          ))}
        </div>
      )}

      {/* First-run: no company cast yet (only the seed PM) -> show the setup
          wizard. Once engaged it drives the same engine as `cast init`. */}
      {state && state.agents.filter((a) => a.id !== "pm").length === 0 && (
        <SetupWizard onDone={refresh} />
      )}
      {state && state.agents.filter((a) => a.id !== "pm").length > 0 && (
        <>
          <Tabs value={tab} onValueChange={(v) => setTab(v as Tab)}>
            <TabsList className="grid w-full max-w-3xl grid-cols-3 sm:grid-cols-5 md:grid-cols-10">
              <TabsTrigger value="overview">Overview</TabsTrigger>
              <TabsTrigger value="graph">Graph</TabsTrigger>
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
              <TabsTrigger value="debug">Debug</TabsTrigger>
              <TabsTrigger value="advisor">Advisor</TabsTrigger>
              <TabsTrigger value="sketch">Sketch</TabsTrigger>
              <TabsTrigger value="settings">Settings</TabsTrigger>
            </TabsList>
            {tab === "overview" && <Overview model={model} />}
            {tab === "graph" && <GraphView graph={graph} />}
            {tab === "chat" && <Chat state={state} onSent={refresh} />}
            {tab === "board" && <Board tasks={state.tasks} onOpenTask={setOpenTask} />}
            {tab === "team" && <Team agents={state.agents} consultants={consultants} />}
            {tab === "decisions" && (
              <Decisions decisions={state.decisions} onDecide={refresh} />
            )}
            {tab === "inbox" && <InboxView inbox={inbox} observations={state.observations} onDecide={refresh} />}
            {tab === "activity" && <ActivityView />}
            {tab === "debug" && <DebugView />}
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
            {tab === "settings" && <SettingsView />}
          </Tabs>
        </>
      )}

      {/* G7: per-task drill-down drawer (opened from the board). */}
      {openTask && <TaskDrawer task={openTask} onClose={() => setOpenTask(null)} />}
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

  const agentLabel = (id: string) => {
    if (id === "owner") return "Owner (You)";
    if (id === "pm") return "Project Manager";
    // A consultant id from the assignable cast, or unknown actor
    return id.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
  };

  const agentBadge = (id: string) => {
    if (id === "owner") return <Badge variant="default">You</Badge>;
    if (id === "pm") return <Badge variant="secondary">PM</Badge>;
    return <Badge variant="outline">Consultant</Badge>;
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Team Chat</CardTitle>
        <CardDescription>
          Messages between the owner, PM, and the cast. Owner sends commands;
          consultants surface findings, flag concerns, and request decisions.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="thread">
          {state.messages.length === 0 && (
            <div className="muted small">Tell the PM what you want to build.</div>
          )}
          {state.messages.map((m: Message) => (
            <div
              key={m.id}
              className={`bubble ${
                m.from === "owner"
                  ? "owner"
                  : m.from === "pm"
                  ? "pm"
                  : "consultant"
              }`}
            >
              <div className="flex items-center gap-2 mb-1">
                {agentBadge(m.from)}
                <span className="text-xs font-medium text-muted-foreground">
                  {agentLabel(m.from)}
                </span>
              </div>
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

function Board({
  tasks,
  onOpenTask,
}: {
  tasks: Projection["tasks"];
  onOpenTask: (t: Task) => void;
}) {
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
                    <Card
                      key={t.id}
                      className="cursor-pointer border-border/60 transition-colors hover:border-primary/50"
                      onClick={() => onOpenTask(t)}
                    >
                      <CardContent className="p-3">
                        <div className="text-sm font-medium leading-snug">{t.title}</div>
                        <Badge variant={t.status === "blocked" ? "destructive" : "secondary"} className="mt-2">
                          {t.status}
                        </Badge>
                        <div className="text-xs text-muted-foreground mt-1">
                          {t.kind}
                          {t.assignee ? ` · ${agentLabel(t.assignee)}` : ""}
                        </div>
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

function Team({
  agents,
  consultants,
}: {
  agents: Projection["agents"];
  consultants: import("./api").ConsultantConfig[];
}) {
  return (
    <div>
      {agents.length === 0 && <Card className="muted"><CardContent className="pt-6">No one hired yet.</CardContent></Card>}
      <div className="grid gap-3 sm:grid-cols-2">
        {agents.map((a) => {
          const identity = identityForAgent(a.id, a.role, consultants);
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

/// Settings — owner-configurable surface (2026-08-14). Hosts the Telegram
/// owner-channel connect (reusable, also in the setup wizard) so messaging can
/// be set up / reconnected any time, not just first-run.
function SettingsView() {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Settings</CardTitle>
        <CardDescription>
          Connect or reconnect your messaging. Each Casting install uses its own
          Telegram bot — the PM you talk to on your phone.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <TelegramConnect />
      </CardContent>
    </Card>
  );
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
  observations,
  onDecide,
}: {
  inbox: Inbox | null;
  observations: Observation[];
  onDecide: () => void;
}) {
  const items = inbox?.items ?? [];
  const flagged = observations.filter((o) => o.pm_action_required);
  return (
    <div className="flex flex-col gap-6">
      {/* ---- Pending decisions ---- */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base">Pending decisions</CardTitle>
          <CardDescription>Items awaiting your approval or rejection.</CardDescription>
        </CardHeader>
        <CardContent>
          {items.length === 0 ? (
            <div className="muted text-sm">🟢 Nothing needs your attention right now.</div>
          ) : (
            <div className="flex flex-col gap-3">
              {items.map((it) => (
                <Card key={it.id} className="border-primary/40">
                  <CardContent className="pt-6">
                    <Badge variant="outline" className="mb-2">awaiting your decision</Badge>
                    <div className="font-semibold">{it.subject}</div>
                    {Object.entries(it.options).length > 0 && (
                      <ul className="mt-2 list-disc pl-5 text-sm">
                        {Object.entries(it.options).map(([k, v]) => (
                          <li key={k}><strong>{k}:</strong> {v}</li>
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
          )}
        </CardContent>
      </Card>

      {/* ---- Observations from the cast ---- */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-base flex items-center gap-2">
            Observations from the cast
            {flagged.length > 0 && (
              <Badge variant="destructive" className="text-xs">{flagged.length} need action</Badge>
            )}
          </CardTitle>
          <CardDescription>
            Findings and flags raised by consultants. Items marked "needs action" require your attention.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {observations.length === 0 ? (
            <div className="muted text-sm">No observations yet.</div>
          ) : (
            <div className="flex flex-col gap-2">
              {observations.map((o) => (
                <Card key={o.id} className={`border-l-4 ${o.pm_action_required ? "border-l-destructive/60" : "border-l-muted"}`}>
                  <CardContent className="py-3">
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <Badge variant={o.pm_action_required ? "destructive" : "secondary"} className="text-[10px]">
                          {o.severity}
                        </Badge>
                        {o.pm_action_required && (
                          <Badge variant="outline" className="text-[10px] border-destructive/40 text-destructive">
                            Needs action
                          </Badge>
                        )}
                      </div>
                      <span className="text-xs text-muted-foreground shrink-0">from {o.from}</span>
                    </div>
                    <div className="font-medium text-sm mt-1">{o.subject}</div>
                    {o.body && <div className="text-sm text-muted-foreground mt-0.5">{o.body}</div>}
                  </CardContent>
                </Card>
              ))}
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
