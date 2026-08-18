import { Suspense, lazy, useEffect, useRef, useState } from "react";
import SetupWizard from "./SetupWizard";
import { useCastStore } from "./store";
import ActivityView from "./ActivityView";
import DebugView from "./DebugView";
import TaskDrawer from "./TaskDrawer";
import GraphView from "./GraphView";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  Decision,
  Inbox,
  Message,
  Observation,
  Projection,
  Task,
  sendMessage,
  decide,
} from "./api";
import { TASK_COLUMNS } from "./boardColumns";
import { identityForAgent } from "./identities";
import TelegramConnect from "./TelegramConnect";
import { useNavigate, useLocation } from "react-router-dom";
import { pathForTab, tabLabel, tabForPath, type Tab } from "./nav";
import { ShellSidebar } from "./ShellSidebar";
import { ShellHeader } from "./ShellHeader";
import { TooltipProvider } from "@/components/ui/tooltip";
import Home from "./pages/Home";
import Spend from "./pages/Spend";
import Routing from "./pages/Routing";
import Knowledge from "./pages/Knowledge";
import {
  decisionStatusVariant,
  severityVariant,
  taskStatusVariant,
} from "./lib/status";
import type {
  ConsultantConfig,
  GraphView as GraphViewData,
  OperatingModel,
} from "./api";

// Lazy: heavy feature surfaces — never load unless opened.
const Whiteboard = lazy(() => import("./Whiteboard"));
const Advisor = lazy(() => import("./Advisor"));

function agentLabel(id: string): string {
  const c = useCastStore.getState().consultants;
  return identityForAgent(id, "", c)?.name ?? id;
}

export default function App() {
  const navigate = useNavigate();
  const location = useLocation();
  const tab = tabForPath(location.pathname);
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  const [openTask, setOpenTask] = useState<Task | null>(null);
  const state = useCastStore((s) => s.state);
  const model = useCastStore((s) => s.model);
  const graph = useCastStore((s) => s.graph);
  const consultants = useCastStore((s) => s.consultants);
  const routing = useCastStore((s) => s.routing);
  const inbox = useCastStore((s) => s.inbox);
  const errors = useCastStore((s) => s.errors);
  const refresh = useCastStore((s) => s.refresh);
  const refreshLazy = useCastStore((s) => s.refreshLazy);
  const start = useCastStore((s) => s.start);

  useEffect(() => {
    const unsub = start();
    return unsub;
  }, [start]);

  // Lazy-refresh the data a surface needs when it's opened.
  useEffect(() => {
    if (tab === "home") refreshLazy("model");
    if (tab === "graph") refreshLazy("graph");
    if (tab === "inbox") refreshLazy("inbox");
    if (tab === "debug") refreshLazy("events");
  }, [tab, refreshLazy]);

  // Normalise unknown/deep URLs to a known tab (e.g. stray path → home).
  useEffect(() => {
    const resolved = pathForTab(tab);
    if (location.pathname !== resolved) {
      navigate(resolved, { replace: true });
    }
  }, [location.pathname, tab, navigate]);

  // First-run: no cast hired beyond the seed PM → show setup wizard full-screen.
  const needsSetup = state && state.agents.filter((a) => a.id !== "mei").length === 0;
  if (needsSetup) return <SetupWizard onDone={refresh} />;
  if (!state) return null;

  const go = (t: Tab) => navigate(pathForTab(t));

  return (
    <TooltipProvider delayDuration={200}>
      <div className="app-shell">
        <ShellSidebar
          active={tab}
          collapsed={collapsed}
          onToggle={() => setCollapsed((c) => !c)}
          unread={inbox?.unread ?? 0}
          mobileOpen={mobileOpen}
          onCloseMobile={() => setMobileOpen(false)}
        />
        <main className="app-main">
          <ShellHeader tab={tab} onOpenMobile={() => setMobileOpen(true)} />

        {errors.length > 0 && (
          <div className="banner">
            {errors.map((e) => (
              <div key={`${e.resource}-${e.at}`}>
                ⚠️ <strong>{e.resource}</strong>: {e.message.replace(/^[^:]*: /, "")}
              </div>
            ))}
          </div>
        )}

        <div className="app-content">
          <PageHead title={tabLabel(tab)} tab={tab} />
          <PageBody
            tab={tab}
            state={state}
            model={model}
            graph={graph}
            consultants={consultants}
            routing={routing}
            inbox={inbox}
            onOpenTask={setOpenTask}
            onDecide={refresh}
            onSent={refresh}
            onGoInbox={() => go("inbox")}
            onGoChat={() => go("chat")}
          />
        </div>
      </main>

      {openTask && <TaskDrawer task={openTask} onClose={() => setOpenTask(null)} />}
      </div>
    </TooltipProvider>
  );
}

function PageHead({ title, tab }: { title: string; tab: Tab }) {
  const subtitle = SUBTITLES[tab] ?? "";
  return (
    <div className="page-head">
      <h1 className="title">{title}</h1>
      {subtitle && <div className="subtitle">{subtitle}</div>}
    </div>
  );
}

const SUBTITLES: Partial<Record<Tab, string>> = {
  home: "Your operating picture — what needs you, and how the production is going.",
  inbox: "Everything awaiting your decision or attention.",
  chat: "Direct the cast — commands, questions, and updates in one thread.",
  board: "The work stream across all five stages.",
  graph: "How tasks and dependencies flow from one state to the next.",
  team: "The cast you've hired for this production.",
  activity: "The full event stream — the single source of truth.",
  decisions: "Every decision, its options, and who ruled on it.",
  knowledge: "Facts, opinions, risks, and briefings the company runs on.",
  spend: "Where the budget is going, call by call.",
  advisor: "Your thinking partner on product direction.",
  sketch: "A freeform whiteboard for ideas and diagrams.",
  settings: "Messaging, autonomy, and how your company connects.",
  routing: "Which model each actor runs on, and what that costs.",
  debug: "Under the hood — events, contexts, and diagnostics.",
};

interface PageBodyProps {
  tab: Tab;
  state: Projection;
  model: OperatingModel | null;
  graph: GraphViewData | null;
  consultants: ConsultantConfig[];
  routing: import("./api").ActorRouting[];
  inbox: Inbox | null;
  onOpenTask: (t: Task) => void;
  onDecide: () => void;
  onSent: () => void;
  onGoInbox: () => void;
  onGoChat: () => void;
}

function PageBody(p: PageBodyProps) {
  switch (p.tab) {
    case "home":
      return (
        <Home
          model={p.model}
          inbox={p.inbox}
          observations={p.state.observations}
          decisions={p.state.decisions}
          onGoInbox={p.onGoInbox}
          onGoChat={p.onGoChat}
        />
      );
    case "inbox":
      return <InboxView inbox={p.inbox} observations={p.state.observations} onDecide={p.onDecide} />;
    case "chat":
      return <Chat state={p.state} onSent={p.onSent} />;
    case "board":
      return <Board tasks={p.state.tasks} onOpenTask={p.onOpenTask} />;
    case "graph":
      return <GraphView graph={p.graph} />;
    case "team":
      return <Team agents={p.state.agents} consultants={p.consultants} />;
    case "activity":
      return <ActivityView />;
    case "decisions":
      return <Decisions decisions={p.state.decisions} onDecide={p.onDecide} />;
    case "knowledge":
      return <Knowledge model={p.model} />;
    case "spend":
      return <Spend model={p.model} spendEntries={p.state.spend} />;
    case "routing":
      return <Routing routing={p.routing} />;
    case "advisor":
      return (
        <Suspense fallback={<Card className="muted"><CardContent className="py-6">Loading advisor…</CardContent></Card>}>
          <Advisor thread={p.state.advisor_thread} onChanged={p.onSent} />
        </Suspense>
      );
    case "sketch":
      return (
        <Suspense fallback={<Card className="muted"><CardContent className="py-6">Loading sketchpad…</CardContent></Card>}>
          <Whiteboard onSaved={p.onSent} />
        </Suspense>
      );
    case "settings":
      return <SettingsView />;
    case "debug":
      return <DebugView />;
    default:
      return null;
  }
}

// ── Chat ────────────────────────────────────────────────────────────────────
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

  const kind = (from: string) =>
    from === "owner" ? "owner" : from === "mei" ? "pm" : "consultant";
  const label = (from: string) => (from === "owner" ? "You" : agentLabel(from));

  return (
    <Card className="max-w-3xl">
      <CardHeader className="pb-2">
        <CardTitle>Team chat</CardTitle>
        <CardDescription>
          Send commands to the PM; the cast surfaces findings and requests decisions here.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="thread">
          {state.messages.length === 0 && <div className="muted small">Tell the PM what you want to build.</div>}
          {state.messages.map((m: Message) => (
            <div key={m.id} className={`bubble ${kind(m.from)}`}>
              <div className="flex items-center gap-2 mb-1">
                <span className="text-xs font-semibold text-muted-foreground">{label(m.from)}</span>
              </div>
              {m.body}
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
        <div className="mt-3 flex gap-2">
          <Input
            value={draft}
            placeholder="e.g. “Build me a todo app”"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && send()}
          />
          <Button onClick={send} disabled={busy || !draft.trim()}>Send</Button>
        </div>
      </CardContent>
    </Card>
  );
}

// ── Board ───────────────────────────────────────────────────────────────────
function Board({ tasks, onOpenTask }: { tasks: Task[]; onOpenTask: (t: Task) => void }) {
  return (
    <div>
      {tasks.length === 0 && (
        <Card className="muted"><CardContent className="pt-6">No tasks yet — tell the PM what to build.</CardContent></Card>
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
                        <div className="mt-2 flex flex-wrap items-center gap-1.5">
                          <Badge variant={taskStatusVariant(t.status)}>{t.status}</Badge>
                          {t.assignee && (
                            <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                              <Avatar className="h-4 w-4">
                                <AvatarImage src="" alt="" />
                                <AvatarFallback className="text-[8px]">
                                  {agentLabel(t.assignee).slice(0, 2).toUpperCase()}
                                </AvatarFallback>
                              </Avatar>
                              {agentLabel(t.assignee)}
                            </span>
                          )}
                        </div>
                        <div className="mt-1 text-xs text-muted-foreground">{t.kind}</div>
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

// ── Team / Cast ─────────────────────────────────────────────────────────────
function Team({ agents, consultants }: { agents: Projection["agents"]; consultants: ConsultantConfig[] }) {
  return (
    <div>
      {agents.length === 0 && <Card className="muted"><CardContent className="pt-6">No one hired yet.</CardContent></Card>}
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {agents.map((a) => {
          const identity = identityForAgent(a.id, a.role, consultants);
          return (
            <Card key={a.id}>
              <CardContent className="pt-5">
                <div className="flex items-center gap-4">
                  <Avatar className="h-12 w-12">
                    {identity?.avatar && <AvatarImage src={identity.avatar} alt={identity.name} />}
                    <AvatarFallback>{a.id.slice(0, 2).toUpperCase()}</AvatarFallback>
                  </Avatar>
                  <div>
                    <div className="font-semibold">{identity?.name ?? a.id}</div>
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

// ── Decisions ───────────────────────────────────────────────────────────────
function Decisions({ decisions, onDecide }: { decisions: Decision[]; onDecide: () => void }) {
  return (
    <div className="flex flex-col gap-3">
      {decisions.length === 0 && <Card className="muted"><CardContent className="py-6">No decisions recorded yet.</CardContent></Card>}
      {decisions.map((d) => (
        <Card key={d.id}>
          <CardContent className="pt-5">
            <div className="flex items-center justify-between gap-2">
              <div className="font-semibold">{d.subject}</div>
              <Badge variant={decisionStatusVariant(d.status)}>{d.status}</Badge>
            </div>
            {Object.entries(d.options).length > 0 && (
              <ul className="mt-2 list-disc pl-5 text-sm">
                {Object.entries(d.options).map(([k, v]) => (
                  <li key={k}><strong>{k}:</strong> {v}</li>
                ))}
              </ul>
            )}
            {d.recommendation && <div className="text-xs text-muted-foreground mt-1">Pm recommends: {d.recommendation}</div>}
            {d.owner_verdict && <div className="text-xs text-muted-foreground mt-1">Owner: {d.owner_verdict}</div>}
            {d.status === "proposed" && (
              <div className="flex gap-2 mt-3">
                <Button size="sm" onClick={() => void decide(d.id, d.subject, true).then(onDecide)}>Approve</Button>
                <Button size="sm" variant="outline" onClick={() => void decide(d.id, d.subject, false).then(onDecide)}>Reject</Button>
              </div>
            )}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

// ── Inbox (action center) ───────────────────────────────────────────────────
function InboxView({ inbox, observations, onDecide }: { inbox: Inbox | null; observations: Observation[]; onDecide: () => void }) {
  const items = inbox?.items ?? [];
  const flagged = observations.filter((o) => o.pm_action_required);
  const empty = items.length === 0 && flagged.length === 0;

  return (
    <div className="flex flex-col gap-6">
      {empty && (
        <Card><CardContent className="pt-6">
          <div className="flex items-center gap-3">
            <span className="text-2xl">🎉</span>
            <div>
              <div className="font-semibold">You're all caught up</div>
              <div className="text-sm text-muted-foreground">Nothing needs your attention right now.</div>
            </div>
          </div>
        </CardContent></Card>
      )}

      {items.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Pending decisions</CardTitle>
            <CardDescription>Items awaiting your approval or rejection.</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex flex-col gap-3">
              {items.map((it) => (
                <Card key={it.id} className="border-primary/40">
                  <CardContent className="pt-5">
                    <Badge variant="warning" className="mb-2">awaiting your decision</Badge>
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
          </CardContent>
        </Card>
      )}

      {flagged.length > 0 && (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-base">Flagged observations</CardTitle>
            <CardDescription>Findings from the cast that need your attention.</CardDescription>
          </CardHeader>
          <CardContent>
            <div className="flex flex-col gap-2">
              {flagged.map((o) => (
                <Card key={o.id} className="border-l-4 border-l-destructive/60">
                  <CardContent className="py-3">
                    <div className="flex items-center justify-between gap-2">
                      <Badge variant={severityVariant(o.severity)} className="text-[10px]">{o.severity}</Badge>
                      <span className="text-xs text-muted-foreground shrink-0">from {o.from}</span>
                    </div>
                    <div className="font-medium text-sm mt-1">{o.subject}</div>
                    {o.body && <div className="text-sm text-muted-foreground mt-0.5">{o.body}</div>}
                  </CardContent>
                </Card>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

// ── Settings ────────────────────────────────────────────────────────────────
function SettingsView() {
  return (
    <Card className="max-w-2xl">
      <CardHeader className="pb-2">
        <CardTitle>Setup & Connect</CardTitle>
        <CardDescription>
          Connect or reconnect your messaging, and review your setup.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <TelegramConnect />
      </CardContent>
    </Card>
  );
}
