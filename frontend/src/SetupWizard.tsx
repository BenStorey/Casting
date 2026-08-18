import { useEffect, useState } from "react";
import {
  fetchSetupStatus,
  submitSetup,
  type SetupStatus,
  type SetupResult,
  type ConsultantConfig,
} from "./api";
import { useCastStore } from "./store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PM_IDENTITY } from "./identities";
import { cn } from "@/lib/utils";

type ExpLevel = "novice" | "somewhat" | "confident";

const EXP_LEVELS: { value: ExpLevel; label: string; desc: string }[] = [
  { value: "novice", label: "No experience", desc: "I'm new — explain things simply." },
  { value: "somewhat", label: "Somewhat familiar", desc: "I've dabbled with dev teams before." },
  { value: "confident", label: "Confident with technology", desc: "I'm technical — give me the details." },
];

type Provider = "openrouter" | "openai" | "anthropic";

const PROVIDERS: {
  id: Provider;
  label: string;
  keysHint: string;
  defaultModel: string;
  keyPlaceholder: string;
  blurb: string;
}[] = [
  { id: "openrouter", label: "OpenRouter", keysHint: "openrouter.ai/keys", defaultModel: "deepseek/deepseek-v4-flash-0731", keyPlaceholder: "sk-or-v1-...", blurb: "one key, many models" },
  { id: "openai", label: "OpenAI", keysHint: "platform.openai.com/api-keys", defaultModel: "gpt-4o-mini", keyPlaceholder: "sk-...", blurb: "your OpenAI subscription" },
  { id: "anthropic", label: "Anthropic", keysHint: "console.anthropic.com/settings/keys", defaultModel: "claude-sonnet-4-5", keyPlaceholder: "sk-ant-...", blurb: "your Anthropic subscription" },
];

/// Regrouped wizard steps — 6 meaningful ones (about you, cast, project,
/// autonomy, AI, launch) instead of the old 9 micro-steps. Fewer clicks, same
/// backend contract.
const STEPS = [
  { id: "about", title: "About you" },
  { id: "cast", title: "Meet your cast" },
  { id: "project", title: "Your project" },
  { id: "autonomy", title: "Autonomy" },
  { id: "ai", title: "AI provider" },
  { id: "launch", title: "Launch" },
] as const;
type StepId = (typeof STEPS)[number]["id"];

const DECISION_CLASSES: { id: string; label: string; desc: string }[] = [
  { id: "internal_rename", label: "Internal renames", desc: "Renaming variables or symbols" },
  { id: "internal_refactor", label: "Internal refactors", desc: "Code changes with no product-facing effect" },
  { id: "testing_library", label: "Testing choices", desc: "Choosing test frameworks or tools" },
  { id: "add_consultant", label: "Hiring new team members", desc: "Bringing new specialists onto the cast" },
  { id: "internal_implementation", label: "Implementation approach", desc: "How to build something internally" },
  { id: "database", label: "Database changes", desc: "Choosing or changing the database" },
  { id: "architecture", label: "Architecture decisions", desc: "System-level design choices" },
  { id: "product_requirement", label: "Product requirements", desc: "Changes to product scope or specs" },
  { id: "spending_threshold", label: "Spending decisions", desc: "Exceeding configured budget thresholds" },
  { id: "production_deployment", label: "Production deploys", desc: "When and how to deploy to production" },
  { id: "security_critical", label: "Security issues", desc: "Security-critical actions" },
  { id: "irreversible", label: "Irreversible actions", desc: "Actions that can't be undone" },
  { id: "governance_change", label: "Governance changes", desc: "Changing project rules or policies" },
];

type PolicyPreset = "autonomous" | "balanced" | "supervised";
const POLICY_PRESETS: { id: PolicyPreset; label: string; desc: string }[] = [
  { id: "autonomous", label: "Do everything autonomously", desc: "Only flag security issues to me." },
  { id: "balanced", label: "Only high-impact changes by me", desc: "Run the day-to-day; escalate architecture, database, and spending." },
  { id: "supervised", label: "Run everything past me", desc: "Review every decision before it's made." },
];

const ASK_BY_DEFAULT = [
  "Database", "Architecture", "ProductRequirement", "SpendingThreshold",
  "ProductionDeployment", "Irreversible", "GovernanceChange", "SecurityCritical",
];

export default function SetupWizard({ onDone }: { onDone: () => void }) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [created, setCreated] = useState<SetupResult | null>(null);

  // Wizard data
  const [stepIdx, setStepIdx] = useState(0);
  const [ownerName, setOwnerName] = useState("");
  const [expLevel, setExpLevel] = useState<ExpLevel | null>(null);
  const [existingProject, setExistingProject] = useState<boolean | null>(null);
  const [projectPath, setProjectPath] = useState("");
  const [projectName, setProjectName] = useState("");
  const [objective, setObjective] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [provider, setProvider] = useState<Provider>("openrouter");
  const [model, setModel] = useState(PROVIDERS[0].defaultModel);
  const [policyPreset, setPolicyPreset] = useState<PolicyPreset>("balanced");
  const [castIndex, setCastIndex] = useState(0);
  const [policyOverrides, setPolicyOverrides] = useState<Record<string, boolean>>(() => {
    const o: Record<string, boolean> = {};
    for (const dc of DECISION_CLASSES) o[dc.id] = ASK_BY_DEFAULT.includes(dc.id);
    return o;
  });

  const consultants = useCastStore((s) => s.consultants);
  const castMembers = consultants.filter(
    (c) => c.id !== "mei" && c.id !== "jeeves" && c.role !== "advisor"
  );

  useEffect(() => {
    fetchSetupStatus().then(setStatus).catch((e) => setErr(String(e)));
  }, []);

  const activeProvider = PROVIDERS.find((p) => p.id === provider) ?? PROVIDERS[0];
  const step = STEPS[stepIdx];

  const canContinue = (() => {
    switch (step.id) {
      case "about": return ownerName.trim().length > 0;
      case "cast": return true;
      case "project":
        return (
          projectName.trim().length > 0 &&
          objective.trim().length > 0 &&
          (status?.project_exists !== false || projectPath.trim().length > 0)
        );
      case "autonomy": return true;
      case "ai": return apiKey.trim().length > 0;
      case "launch": return true;
    }
  })();

  const applyPreset = (preset: PolicyPreset) => {
    setPolicyPreset(preset);
    const updated: Record<string, boolean> = {};
    for (const dc of DECISION_CLASSES) {
      if (preset === "supervised") updated[dc.id] = true;
      else if (preset === "autonomous") updated[dc.id] = dc.id === "SecurityCritical";
      else updated[dc.id] = ASK_BY_DEFAULT.includes(dc.id);
    }
    setPolicyOverrides(updated);
  };

  const launch = async () => {
    setBusy(true);
    setErr(null);
    try {
      const res = await submitSetup(
        projectName.trim(), objective.trim(), status?.roles?.map((r) => r.id) ?? [],
        ownerName.trim() || undefined, expLevel ?? undefined, apiKey.trim() || undefined,
        undefined, provider, model.trim() || undefined,
        status?.project_exists === false ? projectPath.trim() : undefined
      );
      if (res.created) {
        setCreated(res);
        setBusy(false);
        return;
      }
      for (const dc of DECISION_CLASSES) {
        const ask = policyOverrides[dc.id];
        await fetch("/api/policy", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ class: dc.id, involvement: ask ? "ask" : "pm" }),
        });
      }
      onDone();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  if (created) {
    return <SuccessScreen name={created.name ?? projectName} slug={created.slug} port={created.port} />;
  }

  return (
    <div className="min-h-screen bg-background">
      <div className="mx-auto flex min-h-screen w-full max-w-5xl">
        {/* Progress rail */}
        <aside className="hidden w-72 shrink-0 flex-col border-r border-border bg-card px-6 py-10 md:flex">
          <div className="mb-8 flex items-center gap-3">
            <span className="text-2xl">🎬</span>
            <div>
              <div className="font-display text-lg font-bold leading-none">Casting</div>
              <div className="text-xs text-muted-foreground mt-0.5">set up your company</div>
            </div>
          </div>
          <nav className="space-y-1">
            {STEPS.map((s, i) => {
              const done = i < stepIdx;
              const active = i === stepIdx;
              return (
                <div key={s.id}>
                  <button
                    onClick={() => setStepIdx(i)}
                    className={cn(
                      "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors",
                      active ? "bg-primary-soft font-medium text-primary" : "text-muted-foreground hover:bg-accent"
                    )}
                  >
                    <span
                      className={cn(
                        "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold",
                        done ? "bg-success/15 text-success-foreground" : active ? "bg-primary text-primary-foreground" : "bg-secondary text-muted-foreground"
                      )}
                    >
                      {done ? "✓" : i + 1}
                    </span>
                    <span>{s.title}</span>
                  </button>
                </div>
              );
            })}
          </nav>
        </aside>

        {/* Content */}
        <main className="flex-1 px-6 py-10 md:px-12">
          {/* Mobile stepper */}
          <div className="mb-6 flex items-center gap-2 md:hidden">
            {STEPS.map((s, i) => (
              <div
                key={s.id}
                className={cn(
                  "h-1.5 flex-1 rounded-full",
                  i <= stepIdx ? "bg-primary" : "bg-border"
                )}
              />
            ))}
          </div>

          {err && (
            <div className="mb-4 rounded-lg border border-destructive/40 bg-destructive/5 px-4 py-3 text-sm text-destructive">
              ⚠️ {err}
            </div>
          )}

          <div className="mx-auto max-w-xl">
            {step.id === "about" && <AboutStep {...{ ownerName, setOwnerName, expLevel, setExpLevel }} />}
            {step.id === "cast" && (
              <CastStep
                members={castMembers}
                index={castIndex}
                onNext={() => setCastIndex((i) => i + 1)}
                onPrev={() => setCastIndex((i) => i - 1)}
                done={castIndex >= castMembers.length - 1}
              />
            )}
            {step.id === "project" && <ProjectStep {...{ existingProject, setExistingProject, projectPath, setProjectPath, projectName, setProjectName, objective, setObjective, needsRepo: status?.project_exists === false }} />}
            {step.id === "autonomy" && <AutonomyStep {...{ policyPreset, applyPreset, policyOverrides, setPolicyOverrides }} />}
            {step.id === "ai" && <AiStep {...{ provider, setProvider, activeProvider, model, setModel, apiKey, setApiKey }} />}
            {step.id === "launch" && (
              <LaunchStep {...{ ownerName, expLevel, projectName, objective, castMembers, activeProvider, model, apiKey, existingProject, projectPath, status, launch, busy }} />
            )}

            <div className="mt-8 flex items-center justify-between gap-3">
              <Button
                variant="ghost"
                onClick={() => {
                  if (step.id === "cast" && castIndex > 0) setCastIndex((i) => i - 1);
                  else setStepIdx((i) => Math.max(0, i - 1));
                }}
                disabled={stepIdx === 0 && !(step.id === "cast" && castIndex > 0)}
              >
                Back
              </Button>
              {step.id !== "launch" ? (
                <Button
                  onClick={() => {
                    if (step.id === "cast" && castIndex < castMembers.length - 1) {
                      setCastIndex((i) => i + 1);
                    } else {
                      setStepIdx((i) => i + 1);
                    }
                  }}
                  disabled={!canContinue}
                >
                  {step.id === "cast" && castIndex < castMembers.length - 1 ? "Next member" : "Continue"}
                </Button>
              ) : (
                <Button onClick={launch} disabled={busy} size="lg">
                  {busy ? "Setting up your company…" : "🚀 Launch my company"}
                </Button>
              )}
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}

// ── Steps ───────────────────────────────────────────────────────────────────

function AboutStep({ ownerName, setOwnerName, expLevel, setExpLevel }: any) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-3xl font-bold tracking-tight">Welcome to Casting</h2>
        <p className="mt-2 text-muted-foreground leading-relaxed">
          Your autonomous software company — a cast of AI specialists who plan,
          build, test, and ship software while you direct.
        </p>
      </div>
      <div className="flex items-start gap-4 rounded-xl border bg-card p-4">
        <img src={PM_IDENTITY.avatar ?? ""} alt={PM_IDENTITY.name} className="h-14 w-14 rounded-xl shrink-0" />
        <div>
          <div className="font-semibold">{PM_IDENTITY.name}</div>
          <div className="text-sm text-muted-foreground">{PM_IDENTITY.role} · {PM_IDENTITY.persona}</div>
          <p className="mt-1 text-sm text-muted-foreground leading-relaxed">
            I'm your Project Manager. You tell me what you want in plain language — I scope it,
            hand it to the right people, and come back only when a decision needs an owner.
          </p>
        </div>
      </div>
      <div className="space-y-4">
        <div>
          <label className="text-sm font-medium text-muted-foreground block mb-1">What should I call you?</label>
          <Input value={ownerName} onChange={(e) => setOwnerName(e.target.value)} placeholder="e.g. Ben" autoFocus />
        </div>
        <div>
          <label className="text-sm font-medium text-muted-foreground block mb-1">How familiar are you with software?</label>
          <div className="space-y-2">
            {EXP_LEVELS.map((el) => (
              <button key={el.value} type="button" onClick={() => setExpLevel(el.value)}
                className={cn("w-full text-left rounded-lg border p-3 transition-all", expLevel === el.value ? "border-primary bg-primary-soft/50" : "border-border bg-card hover:border-primary/40")}>
                <div className="font-medium text-sm">{el.label}</div>
                <div className="text-xs text-muted-foreground mt-0.5">{el.desc}</div>
              </button>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

function CastStep({ members, index, onNext, done }: any) {
  const m = members[index];
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-3xl font-bold tracking-tight">Meet your cast</h2>
        <p className="mt-2 text-muted-foreground">A specialist team, ready to work. You can hire more later.</p>
      </div>
      {!m ? (
        <div className="empty">
          <div className="icon">🎭</div>
          <div className="title">No cast packages loaded</div>
        </div>
      ) : (
        <div className="rounded-xl border bg-card p-6 text-center">
          {m.avatar && <img src={m.avatar} alt={m.name} className="mx-auto h-24 w-24 rounded-2xl" />}
          <h3 className="mt-3 text-xl font-bold">{m.name}</h3>
          <div className="text-muted-foreground">{m.title}</div>
          {m.summary && <p className="mx-auto mt-2 max-w-sm text-sm text-muted-foreground leading-relaxed">{m.summary}</p>}
          {m.routing?.specializations?.length > 0 && (
            <div className="mt-3 flex flex-wrap justify-center gap-2">
              {m.routing.specializations.map((s: string) => <Badge key={s} variant="secondary">{s}</Badge>)}
            </div>
          )}
        </div>
      )}
      <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
        {members.length > 0 && <span>{index + 1} of {members.length}</span>}
      </div>
    </div>
  );
}

function ProjectStep({ existingProject, setExistingProject, projectPath, setProjectPath, projectName, setProjectName, objective, setObjective, needsRepo }: any) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-3xl font-bold tracking-tight">Your project</h2>
        <p className="mt-2 text-muted-foreground">Point Casting at an existing codebase, or start fresh.</p>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <button type="button" onClick={() => setExistingProject(true)}
          className={cn("rounded-xl border p-5 text-center transition-all", existingProject === true ? "border-primary bg-primary-soft/50" : "border-border bg-card hover:border-primary/40")}>
          <div className="text-2xl mb-2">📁</div>
          <div className="font-semibold text-sm">Yes, I have one</div>
        </button>
        <button type="button" onClick={() => setExistingProject(false)}
          className={cn("rounded-xl border p-5 text-center transition-all", existingProject === false ? "border-primary bg-primary-soft/50" : "border-border bg-card hover:border-primary/40")}>
          <div className="text-2xl mb-2">✨</div>
          <div className="font-semibold text-sm">Start something new</div>
        </button>
      </div>
      {existingProject === true && (
        <Input value={projectPath} onChange={(e) => setProjectPath(e.target.value)} placeholder="/path/to/your/project" />
      )}
      <div className="space-y-4">
        <div>
          <label className="text-sm font-medium text-muted-foreground block mb-1">Project name</label>
          <Input value={projectName} onChange={(e) => setProjectName(e.target.value)} placeholder="e.g. MyTodo" />
        </div>
        <div>
          <label className="text-sm font-medium text-muted-foreground block mb-1">What are you building?</label>
          <textarea value={objective} onChange={(e) => setObjective(e.target.value)}
            placeholder="e.g. A todo app with user accounts, shared lists, and real-time sync"
            className="min-h-[100px] w-full rounded-lg border border-input bg-card px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60" />
        </div>
        {needsRepo && (
          <div>
            <label className="text-sm font-medium text-muted-foreground block mb-1">Artifact repo path</label>
            <Input value={projectPath} onChange={(e) => setProjectPath(e.target.value)} placeholder="/path/to/your/repo (must exist)" />
          </div>
        )}
      </div>
    </div>
  );
}

function AutonomyStep({ policyPreset, applyPreset, policyOverrides, setPolicyOverrides }: any) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-3xl font-bold tracking-tight">How much autonomy?</h2>
        <p className="mt-2 text-muted-foreground">You can always change these later.</p>
      </div>
      <div className="space-y-2">
        {POLICY_PRESETS.map((p) => (
          <button key={p.id} type="button" onClick={() => applyPreset(p.id)}
            className={cn("w-full text-left rounded-lg border p-4 transition-all", policyPreset === p.id ? "border-primary bg-primary-soft/50" : "border-border bg-card hover:border-primary/40")}>
            <div className="font-semibold text-sm">{p.label}</div>
            <div className="mt-0.5 text-xs text-muted-foreground">{p.desc}</div>
          </button>
        ))}
      </div>
      <details className="text-sm">
        <summary className="cursor-pointer font-medium text-muted-foreground hover:text-foreground">Tweak individual decisions</summary>
        <div className="mt-3 max-h-64 space-y-2 overflow-y-auto">
          {DECISION_CLASSES.map((dc) => (
            <label key={dc.id} className="flex cursor-pointer items-center gap-3 rounded-lg border border-border p-3 hover:bg-muted/30">
              <input type="checkbox" checked={policyOverrides[dc.id] ?? false}
                onChange={() => { const n = { ...policyOverrides }; n[dc.id] = !n[dc.id]; setPolicyOverrides(n); }}
                className="h-4 w-4 rounded border-gray-300" />
              <div>
                <div className="text-sm font-medium">{dc.label}</div>
                <div className="text-xs text-muted-foreground">{dc.desc}</div>
              </div>
            </label>
          ))}
        </div>
      </details>
    </div>
  );
}

function AiStep({ provider, setProvider, activeProvider, model, setModel, apiKey, setApiKey }: any) {
  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-display text-3xl font-bold tracking-tight">Connect your AI provider</h2>
        <p className="mt-2 text-muted-foreground">OpenRouter is the easiest default — one key, many models.</p>
      </div>
      <div className="grid grid-cols-3 gap-2">
        {PROVIDERS.map((p) => (
          <button key={p.id} type="button" onClick={() => { setProvider(p.id); setModel(p.defaultModel); }}
            className={cn("rounded-lg border p-3 text-sm transition-all", provider === p.id ? "border-primary bg-primary-soft/50" : "border-border bg-card hover:border-primary/40")}>
            <div className="font-semibold">{p.label}</div>
            <div className="mt-1 text-xs text-muted-foreground leading-tight">{p.blurb}</div>
          </button>
        ))}
      </div>
      <div className="rounded-xl border bg-card p-5 text-left text-sm space-y-2">
        <div className="flex items-center gap-2 font-medium"><span className="text-primary">1.</span>Create an API key at{" "}<a href={`https://${activeProvider.keysHint}`} target="_blank" rel="noopener noreferrer" className="underline underline-offset-2 hover:text-primary">{activeProvider.keysHint}</a></div>
        <div className="flex items-center gap-2"><span className="text-primary">2.</span><span>Model &amp; key below</span></div>
      </div>
      <Input className="font-mono text-sm" value={model} onChange={(e) => setModel(e.target.value)} placeholder={activeProvider.defaultModel} />
      <Input className="font-mono text-sm" value={apiKey} onChange={(e) => setApiKey(e.target.value)} placeholder={activeProvider.keyPlaceholder} />
      <p className="text-xs text-muted-foreground">Your key is stored in .casting/config.json (gitignored) and never leaves your machine.</p>
    </div>
  );
}

function LaunchStep(props: any) {
  const { ownerName, expLevel, projectName, objective, castMembers, activeProvider, model, apiKey, projectPath, existingProject, launch, busy } = props;
  const rows: [string, React.ReactNode][] = [
    ["Owner", ownerName || "—"],
    ["Experience", expLevel || "—"],
    ["Project", projectName],
    ["Objective", <span className="text-right">{objective}</span>],
    ["Team size", `${castMembers.length} specialists`],
    ["Provider", activeProvider.label],
    ["Model", <span className="font-mono text-xs break-all text-right">{model}</span>],
    ["API key", apiKey ? `...${apiKey.slice(-4)}` : "Not set"],
  ];
  if (existingProject && projectPath) rows.push(["Repo", <span className="font-mono text-xs">{projectPath}</span>]);
  return (
    <div className="space-y-6">
      <div className="text-center">
        <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-success/15 text-3xl">🚀</div>
        <h2 className="mt-4 font-display text-3xl font-bold tracking-tight">Ready to launch</h2>
        <p className="mt-1 text-muted-foreground">Here's what we've got — everything can be changed later.</p>
      </div>
      <Card>
        <CardContent className="pt-5 space-y-3 text-sm">
          {rows.map(([label, val], i) => (
            <div key={i} className="flex items-start justify-between gap-3">
              <span className="text-muted-foreground shrink-0">{label}</span>
              <span className="font-medium text-right">{val}</span>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

function SuccessScreen({ name, slug, port }: { name: string; slug?: string; port?: number }) {
  return (
    <div className="min-h-screen flex flex-col items-center justify-center px-4 py-12 bg-background">
      <div className="w-full max-w-lg space-y-6 text-center">
        <div className="mx-auto flex h-20 w-20 items-center justify-center rounded-3xl bg-success/15 text-5xl">🎉</div>
        <h2 className="font-display text-3xl font-bold">Project “{name}” created</h2>
        <p className="text-muted-foreground leading-relaxed">
          State is at <code>~/.casting/{slug}</code> {port ? `(port ${port})` : ""}. Stop this{" "}
          <code>cast run</code> and start it again — it will now auto-select this project and launch the workspace.
        </p>
      </div>
    </div>
  );
}
