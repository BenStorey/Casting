import { useEffect, useState } from "react";
import { fetchSetupStatus, submitSetup, type SetupRole, type SetupStatus, type ConsultantConfig } from "./api";
import { useCastStore } from "./store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PM_IDENTITY } from "./identities";

type ExpLevel = "novice" | "somewhat" | "confident";

const EXP_LEVELS: { value: ExpLevel; label: string; desc: string }[] = [
  {
    value: "novice",
    label: "No experience",
    desc: "I'm new to software development — explain things simply.",
  },
  {
    value: "somewhat",
    label: "Somewhat familiar",
    desc: "I've dabbled or worked with dev teams before.",
  },
  {
    value: "confident",
    label: "Confident with technology",
    desc: "I'm technical — give me the details.",
  },
];

/// Step enumeration. Using numbers so we can add/subtract easily.
type Step =
  | { kind: "welcome" }
  | { kind: "name" }
  | { kind: "experience" }
  | { kind: "cast-intro"; index: number }
  | { kind: "existing-project" }
  | { kind: "project-details" }
  | { kind: "policies" }
  | { kind: "api-key" }
  | { kind: "launch" };

const STEPS: { kind: Step["kind"]; label: string }[] = [
  { kind: "welcome", label: "Welcome" },
  { kind: "name", label: "Name" },
  { kind: "experience", label: "Experience" },
  { kind: "cast-intro", label: "Meet the team" },
  { kind: "existing-project", label: "Project" },
  { kind: "project-details", label: "Details" },
  { kind: "policies", label: "Policies" },
  { kind: "api-key", label: "API Key" },
  { kind: "launch", label: "Launch" },
];

// Decision classes and their human-readable labels
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

// Policy presets: which classes are "ask owner" vs "pm can decide"
type PolicyPreset = "autonomous" | "balanced" | "supervised";
const POLICY_PRESETS: { id: PolicyPreset; label: string; desc: string }[] = [
  { id: "autonomous", label: "Do everything autonomously", desc: "I trust you — only flag security issues to me." },
  { id: "balanced", label: "Only high-impact changes by me", desc: "Run the day-to-day; escalate architecture, database, and spending decisions." },
  { id: "supervised", label: "Run everything past me", desc: "I want to review every decision before it's made." },
];

export default function SetupWizard({ onDone }: { onDone: () => void }) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Wizard data
  const [step, setStep] = useState<Step>({ kind: "welcome" });
  const [ownerName, setOwnerName] = useState("");
  const [expLevel, setExpLevel] = useState<ExpLevel | null>(null);
  const [existingProject, setExistingProject] = useState<boolean | null>(null);
  const [projectPath, setProjectPath] = useState("");
  const [projectName, setProjectName] = useState("");
  const [objective, setObjective] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [policyPreset, setPolicyPreset] = useState<PolicyPreset>("balanced");
  // Which classes should ask the owner (true = ask, false = pm can decide)
  const [policyOverrides, setPolicyOverrides] = useState<Record<string, boolean>>(() => {
    const overrides: Record<string, boolean> = {};
    for (const dc of DECISION_CLASSES) {
      // Default: Database, Architecture, ProductRequirement, SpendingThreshold,
      // ProductionDeployment, Irreversible, GovernanceChange = ask
      // SecurityCritical = notify (still needs owner)
      // Everything else = pm can decide
      const askByDefault = ["Database", "Architecture", "ProductRequirement",
        "SpendingThreshold", "ProductionDeployment", "Irreversible",
        "GovernanceChange", "SecurityCritical"];
      overrides[dc.id] = askByDefault.includes(dc.id);
    }
    return overrides;
  });

  const consultants = useCastStore((s) => s.consultants);

  useEffect(() => {
    fetchSetupStatus().then(setStatus).catch((e) => setErr(String(e)));
  }, []);

  // The cast members to introduce — from the consultant registry, excluding the PM and Advisor
  const castMembers = consultants.filter(
    (c) => c.id !== "pm" && c.id !== "advisor" && c.role !== "advisor"
  );

  const stepIndex = STEPS.findIndex((s) => s.kind === step.kind);

  const canContinue = (() => {
    switch (step.kind) {
      case "welcome":
        return true;
      case "name":
        return ownerName.trim().length > 0;
      case "experience":
        return expLevel !== null;
      case "cast-intro":
        return true; // always can continue past a cast intro card
      case "existing-project":
        return existingProject !== null;
      case "project-details":
        return projectName.trim().length > 0 && objective.trim().length > 0;
      case "policies":
        return true;
      case "api-key":
        return apiKey.trim().length > 0;
      case "launch":
        return true;
    }
  })();

  const nextStep = () => {
    switch (step.kind) {
      case "welcome":
        setStep({ kind: "name" });
        break;
      case "name":
        setStep({ kind: "experience" });
        break;
      case "experience":
        setStep({ kind: "cast-intro", index: 0 });
        break;
      case "cast-intro":
        if (step.index + 1 < castMembers.length) {
          setStep({ kind: "cast-intro", index: step.index + 1 });
        } else {
          setStep({ kind: "existing-project" });
        }
        break;
      case "existing-project":
        setStep({ kind: "project-details" });
        break;
      case "project-details":
        setStep({ kind: "policies" });
        break;
      case "policies":
        setStep({ kind: "api-key" });
        break;
      case "api-key":
        setStep({ kind: "launch" });
        break;
      case "launch":
        break;
    }
  };

  const prevStep = () => {
    switch (step.kind) {
      case "welcome":
        break;
      case "name":
        setStep({ kind: "welcome" });
        break;
      case "experience":
        setStep({ kind: "name" });
        break;
      case "cast-intro":
        if (step.index > 0) {
          setStep({ kind: "cast-intro", index: step.index - 1 });
        } else {
          setStep({ kind: "experience" });
        }
        break;
      case "existing-project":
        setStep({ kind: "cast-intro", index: castMembers.length - 1 });
        break;
      case "project-details":
        setStep({ kind: "existing-project" });
        break;
      case "policies":
        setStep({ kind: "project-details" });
        break;
      case "api-key":
        setStep({ kind: "policies" });
        break;
      case "launch":
        setStep({ kind: "api-key" });
        break;
    }
  };

  const applyPreset = (preset: PolicyPreset) => {
    setPolicyPreset(preset);
    const updated: Record<string, boolean> = {};
    for (const dc of DECISION_CLASSES) {
      if (preset === "supervised") {
        updated[dc.id] = true; // everything asks owner
      } else if (preset === "autonomous") {
        // only security-critical asks
        updated[dc.id] = dc.id === "SecurityCritical";
      } else {
        // balanced: sensible defaults
        const askByDefault = ["Database", "Architecture", "ProductRequirement",
          "SpendingThreshold", "ProductionDeployment", "Irreversible",
          "GovernanceChange", "SecurityCritical"];
        updated[dc.id] = askByDefault.includes(dc.id);
      }
    }
    setPolicyOverrides(updated);
  };

  const launch = async () => {
    setBusy(true);
    setErr(null);
    try {
      await submitSetup(
        projectName.trim(),
        objective.trim(),
        status?.roles?.map((r) => r.id) || ["engineer", "qa", "devops", "security"],
        ownerName.trim() || undefined,
        expLevel ?? undefined,
        apiKey.trim() || undefined
      );
      // Apply policy overrides: for classes where the owner chose a non-default
      // involvement, POST to /api/policy. The backend fires DecisionPolicyChanged.
      for (const dc of DECISION_CLASSES) {
        const ask = policyOverrides[dc.id];
        // Map boolean to OwnerInvolvement: true → "ask", false → "pm"
        const involvement = ask ? "ask" : "pm";
        await fetch("/api/policy", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ class: dc.id, involvement }),
        });
      }
      onDone();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  const current = castMembers[step.kind === "cast-intro" ? step.index : -1];

  return (
    <div className="min-h-screen flex flex-col items-center justify-center px-4 py-12 bg-gradient-to-b from-background to-muted/30">
      <div className="w-full max-w-xl space-y-8">
        {/* Step indicator */}
        <div className="flex items-center justify-center gap-2">
          {STEPS.slice(0, 7).map((s, i) => (
            <span
              key={s.kind}
              className={
                "h-2 rounded-full transition-all duration-300 " +
                (i < stepIndex
                  ? "w-6 bg-primary"
                  : i === stepIndex
                  ? "w-8 bg-primary shadow-md"
                  : "w-2 bg-border")
              }
            />
          ))}
        </div>

        {err && (
          <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive">
            ⚠️ {err}
          </div>
        )}

        {/* WELCOME */}
        {step.kind === "welcome" && (
          <div className="space-y-8 text-center">
            <div className="mx-auto flex h-24 w-24 items-center justify-center rounded-3xl bg-primary/10 text-5xl">
              🎬
            </div>
            <div className="space-y-4">
              <h1 className="text-4xl font-bold tracking-tight">
                Welcome to Casting
              </h1>
              <p className="text-lg text-muted-foreground max-w-sm mx-auto leading-relaxed">
                Every great production starts with a great cast. Casting is your
                autonomous software company — a team of AI specialists who plan,
                build, test, and ship software while you direct.
              </p>
            </div>
            <div className="flex items-start gap-4 bg-card border rounded-2xl p-5 text-left">
              <img
                src={PM_IDENTITY.avatar ?? ""}
                alt={PM_IDENTITY.name}
                className="h-14 w-14 rounded-xl shrink-0"
              />
              <div>
                <div className="font-semibold text-base">
                  {PM_IDENTITY.name}
                </div>
                <div className="text-sm text-muted-foreground">
                  {PM_IDENTITY.role} · {PM_IDENTITY.persona}
                </div>
                <p className="text-sm text-muted-foreground mt-2 leading-relaxed">
                  I'm your Project Manager. You tell me what you want in plain
                  language — I scope it into tasks, hand them to the right
                  people, and come back to you only when a decision really needs
                  an owner. Let's get you set up.
                </p>
              </div>
            </div>
            <Button size="lg" className="px-10" onClick={nextStep}>
              Get started
            </Button>
          </div>
        )}

        {/* NAME */}
        {step.kind === "name" && (
          <div className="space-y-6 text-center">
            <div className="flex items-center justify-center gap-3">
              <img
                src={PM_IDENTITY.avatar ?? ""}
                alt={PM_IDENTITY.name}
                className="h-12 w-12 rounded-xl"
              />
              <div className="text-left">
                <div className="font-semibold">{PM_IDENTITY.name}</div>
                <div className="text-sm text-muted-foreground">
                  Your Project Manager
                </div>
              </div>
            </div>
            <h2 className="text-2xl font-bold">What should I call you?</h2>
            <p className="text-muted-foreground">
              I'll use your name throughout our conversations.
            </p>
            <Input
              className="text-center text-lg max-w-xs mx-auto"
              value={ownerName}
              onChange={(e) => setOwnerName(e.target.value)}
              placeholder="e.g. Ben"
              onKeyDown={(e) => e.key === "Enter" && canContinue && nextStep()}
              autoFocus
            />
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep} disabled={!canContinue}>
                Continue
              </Button>
            </div>
          </div>
        )}

        {/* EXPERIENCE */}
        {step.kind === "experience" && (
          <div className="space-y-6 text-center">
            <h2 className="text-2xl font-bold">
              How familiar are you with software development?
            </h2>
            <p className="text-muted-foreground">
              This helps me calibrate how technically I explain things.
            </p>
            <div className="space-y-3">
              {EXP_LEVELS.map((el) => (
                <button
                  key={el.value}
                  type="button"
                  onClick={() => setExpLevel(el.value)}
                  className={
                    "w-full text-left rounded-xl border p-4 transition-all " +
                    (expLevel === el.value
                      ? "border-primary bg-primary/10 shadow-sm"
                      : "border-border bg-card hover:border-primary/40")
                  }
                >
                  <div className="font-semibold">{el.label}</div>
                  <div className="text-sm text-muted-foreground mt-1">
                    {el.desc}
                  </div>
                </button>
              ))}
            </div>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep} disabled={!canContinue}>
                Continue
              </Button>
            </div>
          </div>
        )}

        {/* CAST INTRO — one by one */}
        {step.kind === "cast-intro" && current && (
          <div className="space-y-6 text-center">
            <div className="text-sm text-muted-foreground">
              Meet your team ({step.index + 1} of {castMembers.length})
            </div>
            <div className="bg-card border rounded-2xl p-8 space-y-5">
              <img
                src={current.avatar ?? ""}
                alt={current.name}
                className="mx-auto h-24 w-24 rounded-2xl"
              />
              <div>
                <h2 className="text-2xl font-bold">{current.name}</h2>
                <div className="text-muted-foreground">{current.title}</div>
              </div>
              {current.summary && (
                <p className="text-sm text-muted-foreground leading-relaxed max-w-sm mx-auto">
                  {current.summary}
                </p>
              )}
              {current.routing?.specializations &&
                current.routing.specializations.length > 0 && (
                  <div className="flex flex-wrap justify-center gap-2">
                    {current.routing.specializations.map((s) => (
                      <Badge key={s} variant="secondary">
                        {s}
                      </Badge>
                    ))}
                  </div>
                )}
            </div>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep}>
                {step.index + 1 < castMembers.length
                  ? "Meet the next member"
                  : "All set — continue"}
              </Button>
            </div>
          </div>
        )}

        {/* EXISTING PROJECT */}
        {step.kind === "existing-project" && (
          <div className="space-y-6 text-center">
            <h2 className="text-2xl font-bold">Do you have an existing project?</h2>
            <p className="text-muted-foreground">
              If you already have a codebase, I can point Casting at it. Otherwise
              I'll create a new project for you.
            </p>
            <div className="flex justify-center gap-4">
              <button
                type="button"
                onClick={() => {
                  setExistingProject(true);
                  setProjectPath("");
                }}
                className={
                  "rounded-xl border p-5 w-44 text-center transition-all " +
                  (existingProject === true
                    ? "border-primary bg-primary/10 shadow-sm"
                    : "border-border bg-card hover:border-primary/40")
                }
              >
                <div className="text-2xl mb-2">📁</div>
                <div className="font-semibold">Yes, I have one</div>
              </button>
              <button
                type="button"
                onClick={() => setExistingProject(false)}
                className={
                  "rounded-xl border p-5 w-44 text-center transition-all " +
                  (existingProject === false
                    ? "border-primary bg-primary/10 shadow-sm"
                    : "border-border bg-card hover:border-primary/40")
                }
              >
                <div className="text-2xl mb-2">✨</div>
                <div className="font-semibold">Start something new</div>
              </button>
            </div>
            {existingProject === true && (
              <Input
                value={projectPath}
                onChange={(e) => setProjectPath(e.target.value)}
                placeholder="/path/to/your/project"
                className="max-w-sm mx-auto"
              />
            )}
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep} disabled={!canContinue}>
                Continue
              </Button>
            </div>
          </div>
        )}

        {/* PROJECT DETAILS */}
        {step.kind === "project-details" && (
          <div className="space-y-6 text-center">
            <h2 className="text-2xl font-bold">Tell me about your project</h2>
            <div className="space-y-4 text-left">
              <div>
                <label className="text-sm font-medium text-muted-foreground block mb-1">
                  Project name
                </label>
                <Input
                  value={projectName}
                  onChange={(e) => setProjectName(e.target.value)}
                  placeholder="e.g. MyTodo"
                  onKeyDown={(e) => e.key === "Enter" && canContinue && nextStep()}
                  autoFocus
                />
              </div>
              <div>
                <label className="text-sm font-medium text-muted-foreground block mb-1">
                  What are you building? (short description)
                </label>
                <textarea
                  value={objective}
                  onChange={(e) => setObjective(e.target.value)}
                  placeholder="e.g. A todo app with user accounts, shared lists, and real-time sync"
                  className="flex min-h-[100px] w-full rounded-lg border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onKeyDown={(e) => e.key === "Enter" && e.metaKey && canContinue && nextStep()}
                />
              </div>
            </div>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep} disabled={!canContinue}>
                Continue
              </Button>
            </div>
          </div>
        )}

        {/* POLICIES */}
        {step.kind === "policies" && (
          <div className="space-y-6">
            <h2 className="text-2xl font-bold text-center">
              How much autonomy should the PM have?
            </h2>
            <p className="text-muted-foreground text-center max-w-sm mx-auto leading-relaxed">
              You can always change these later. Pick a style that feels right.
            </p>
            <div className="space-y-3">
              {POLICY_PRESETS.map((p) => (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => applyPreset(p.id)}
                  className={
                    "w-full text-left rounded-xl border p-4 transition-all " +
                    (policyPreset === p.id
                      ? "border-primary bg-primary/10 shadow-sm"
                      : "border-border bg-card hover:border-primary/40")
                  }
                >
                  <div className="font-semibold">{p.label}</div>
                  <div className="text-sm text-muted-foreground mt-1">{p.desc}</div>
                </button>
              ))}
            </div>
            <details className="text-sm">
              <summary className="cursor-pointer text-muted-foreground hover:text-foreground font-medium">
                Tweak individual decisions
              </summary>
              <div className="mt-3 space-y-2 max-h-64 overflow-y-auto">
                {DECISION_CLASSES.map((dc) => (
                  <label
                    key={dc.id}
                    className="flex items-center gap-3 rounded-lg border border-border p-3 cursor-pointer hover:bg-muted/30"
                  >
                    <input
                      type="checkbox"
                      checked={policyOverrides[dc.id] ?? false}
                      onChange={() => {
                        const next = { ...policyOverrides };
                        next[dc.id] = !next[dc.id];
                        setPolicyOverrides(next);
                      }}
                      className="h-4 w-4 rounded border-gray-300"
                    />
                    <div>
                      <div className="text-sm font-medium">{dc.label}</div>
                      <div className="text-xs text-muted-foreground">{dc.desc}</div>
                    </div>
                  </label>
                ))}
              </div>
            </details>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>Back</Button>
              <Button onClick={nextStep}>Continue</Button>
            </div>
          </div>
        )}

        {/* API KEY */}
        {step.kind === "api-key" && (
          <div className="space-y-6 text-center">
            <h2 className="text-2xl font-bold">
              Connect your AI provider
            </h2>
            <p className="text-muted-foreground max-w-sm mx-auto leading-relaxed">
              Casting uses an AI model to power the team. You'll need an API key
              from your provider of choice. We default to OpenRouter — it gives
              you access to many models through one key.
            </p>
            <div className="bg-card border rounded-xl p-5 text-left space-y-3">
              <div className="flex items-center gap-2 text-sm font-medium">
                <span className="text-primary">1.</span>
                <span>
                  Go to{" "}
                  <a
                    href="https://openrouter.ai/keys"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="underline underline-offset-2 hover:text-primary"
                  >
                    openrouter.ai/keys
                  </a>
                </span>
              </div>
              <div className="flex items-center gap-2 text-sm font-medium">
                <span className="text-primary">2.</span>
                <span>Create a key (add a small credit balance)</span>
              </div>
              <div className="flex items-center gap-2 text-sm font-medium">
                <span className="text-primary">3.</span>
                <span>Paste it below</span>
              </div>
            </div>
            <Input
              className="text-center font-mono text-sm max-w-sm mx-auto"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder="sk-or-v1-..."
              onKeyDown={(e) => e.key === "Enter" && canContinue && nextStep()}
              autoFocus
            />
            <p className="text-xs text-muted-foreground">
              Your key is stored in .casting/config.json (gitignored) and never
              leaves your machine.
            </p>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={nextStep} disabled={!canContinue}>
                Continue
              </Button>
            </div>
          </div>
        )}

        {/* LAUNCH */}
        {step.kind === "launch" && (
          <div className="space-y-6 text-center">
            <div className="mx-auto flex h-16 w-16 items-center justify-center rounded-2xl bg-green-500/10 text-3xl">
              🚀
            </div>
            <h2 className="text-2xl font-bold">Ready to launch</h2>
            <p className="text-muted-foreground">
              Here's what we've got. Everything can be changed later.
            </p>
            <Card>
              <CardContent className="pt-6 space-y-3 text-left">
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">Owner</span>
                  <span className="font-medium">{ownerName}</span>
                </div>
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">Experience</span>
                  <span className="font-medium capitalize">{expLevel}</span>
                </div>
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">Project</span>
                  <span className="font-medium">{projectName}</span>
                </div>
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">Objective</span>
                  <span className="font-medium text-right max-w-[60%]">
                    {objective}
                  </span>
                </div>
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">Team size</span>
                  <span className="font-medium">
                    {castMembers.length} specialists
                  </span>
                </div>
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">API key</span>
                  <span className="font-medium">
                    {apiKey
                      ? `sk-or-...${apiKey.slice(-4)}`
                      : "Not set"}
                  </span>
                </div>
                {existingProject && projectPath && (
                  <div className="flex justify-between text-sm">
                    <span className="text-muted-foreground">Existing project</span>
                    <span className="font-medium font-mono text-xs">
                      {projectPath}
                    </span>
                  </div>
                )}
              </CardContent>
            </Card>
            <div className="flex justify-center gap-3">
              <Button variant="ghost" onClick={prevStep}>
                Back
              </Button>
              <Button onClick={launch} disabled={busy} size="lg">
                {busy ? "Setting up your company…" : "🚀 Launch my company"}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}