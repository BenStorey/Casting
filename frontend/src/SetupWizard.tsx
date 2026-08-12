import { useEffect, useState } from "react";
import { fetchSetupStatus, submitSetup, SetupRole, SetupStatus } from "./api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PM_IDENTITY, ROLE_IDENTITIES } from "./cast";

/// First-run onboarding — an in-character, stepped experience.
/// Sarah Chen (the Project Manager) introduces herself, explains the setup,
/// and walks the owner through: team -> objective -> security. On launch she
/// hires the cast and kicks off the build (the SAME engine as `cast init`).

type Step = 0 | 1 | 2 | 3;

export default function SetupWizard({ onDone }: { onDone: () => void }) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
  const [step, setStep] = useState<Step>(0);
  const [name, setName] = useState("");
  const [objective, setObjective] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set(["engineer", "qa"]));
  const [ownerToken, setOwnerToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    fetchSetupStatus().then(setStatus).catch((e) => setErr(String(e)));
  }, []);

  const toggle = (id: string) => {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    setSelected(next);
  };

  const launch = async () => {
    if (!objective.trim()) return;
    setBusy(true);
    setErr(null);
    try {
      await submitSetup(
        name.trim() || "Acme Inc",
        objective.trim(),
        Array.from(selected),
        ownerToken.trim() || undefined
      );
      onDone();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  const roles: SetupRole[] = status?.roles ?? [];
  const canContinue =
    step === 0 ? true : step === 1 ? selected.size > 0 : step === 2 ? objective.trim().length > 0 : true;

  return (
    <div className="app max-w-2xl">
      <div className="flex items-start gap-4">
        <img
          src={PM_IDENTITY.avatar}
          alt={PM_IDENTITY.name}
          className="h-16 w-16 rounded-2xl shrink-0"
        />
        <div>
          <h1 className="text-2xl font-bold leading-none">Hi — I'm {PM_IDENTITY.name}.</h1>
          <p className="text-muted-foreground mt-1">
            {PM_IDENTITY.role} · {PM_IDENTITY.persona}
          </p>
        </div>
      </div>

      {err && <div className="banner mt-4">⚠️ {err}</div>}

      <Card className="mt-6">
        <CardHeader className="pb-2">
          <CardTitle className="text-base">
            {step === 0
              ? "Let me introduce the team."
              : step === 1
              ? "Pick who starts on your team."
              : step === 2
              ? "What should we build first?"
              : "A note on security."}
          </CardTitle>
        </CardHeader>
        <CardContent>
          {step === 0 && (
            <div className="space-y-4">
              <p className="text-sm leading-relaxed text-muted-foreground">
                I keep the whole company moving toward one goal. You talk to me in plain language —
                I scope it into tasks, hand them to the right people, and come back to you only when
                a decision really needs an owner. Hello from everyone:
              </p>
              <div className="flex flex-col gap-2">
                {["engineer", "qa"].map((roleId) => {
                  const m = ROLE_IDENTITIES[roleId];
                  if (!m) return null;
                  return (
                    <div key={m.id} className="flex items-center gap-3 border rounded-lg p-2">
                      {/* eslint-disable-next-line @next/next/no-img-element */}
                      <img src={m.avatar} alt={m.name} className="h-10 w-10 rounded-lg" />
                      <div>
                        <div className="text-sm font-medium">{m.name}</div>
                        <div className="text-xs text-muted-foreground">{m.role}</div>
                      </div>
                    </div>
                  );
                })}
              </div>
              <p className="text-sm leading-relaxed text-muted-foreground">
                Together we turn an idea into working software. Setting up really only takes a few
                quick questions.
              </p>
            </div>
          )}

          {step === 1 && (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                You can always hire more people later. To get started:
              </p>
              <div className="flex flex-wrap gap-2">
                {roles.map((r) => {
                  const meta = ROLE_IDENTITIES[r.id];
                  const on = selected.has(r.id);
                  return (
                    <button
                      key={r.id}
                      type="button"
                      onClick={() => toggle(r.id)}
                      className={
                        "flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm transition-colors " +
                        (on
                          ? "border-primary bg-primary/15 text-foreground"
                          : "border-border bg-card text-muted-foreground hover:text-foreground")
                      }
                    >
                      <img src={meta?.avatar} alt="" className="h-6 w-6 rounded-full" />
                      {meta?.name ?? r.title}
                      <span className="text-xs opacity-70">{r.scope}</span>
                    </button>
                  );
                })}
              </div>
              <p className="text-xs text-muted-foreground">
                Each role has its own cartoon face and a short CV — scroll the Team tab later to meet
                them properly.
              </p>
            </div>
          )}

          {step === 2 && (
            <div className="space-y-3">
              <p className="text-sm text-muted-foreground">
                I'll scope whatever you tell me into a real plan. What are we building first?
              </p>
              <Input
                value={objective}
                onChange={(e) => setObjective(e.target.value)}
                placeholder='e.g. "Build me a todo app"'
                onKeyDown={(e) => e.key === "Enter" && canContinue && setStep(3)}
              />
              <label className="block">
                <span className="text-sm text-muted-foreground">Company / product name</span>
                <Input
                  className="mt-1"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Acme Inc"
                />
              </label>
            </div>
          )}

          {step === 3 && (
            <div className="space-y-3">
              <p className="text-sm leading-relaxed text-muted-foreground">
                One last thing: who's allowed to make decisions here? By default writes are open, but
                I recommend setting an <strong>owner token</strong> — a secret only you know. When it's
                set, the endpoints that change the company require it.
              </p>
              <label className="block">
                <span className="text-sm text-muted-foreground">Owner auth token (optional)</span>
                <Input
                  className="mt-1"
                  value={ownerToken}
                  onChange={(e) => setOwnerToken(e.target.value)}
                  placeholder="a long random secret"
                />
              </label>
              <div className="text-xs text-muted-foreground">
                Your team: {Array.from(selected)
                  .map((id) => ROLE_IDENTITIES[id]?.name ?? id)
                  .join(", ")}{" "}
                — Objective: <Badge variant="secondary">{objective.trim() || "…"}</Badge>
              </div>
            </div>
          )}

          <div className="flex items-center justify-between mt-6">
            <Button variant="ghost" disabled={step === 0} onClick={() => setStep((s) => (s - 1) as Step)}>
              Back
            </Button>
            {step < 3 ? (
              <Button onClick={() => setStep((s) => (s + 1) as Step)} disabled={!canContinue}>
                Continue
              </Button>
            ) : (
              <Button onClick={() => void launch()} disabled={busy}>
                {busy ? "Hiring your team…" : "🚀 Launch my company"}
              </Button>
            )}
          </div>
          <div className="mt-3 flex items-center justify-center gap-1.5">
            {[0, 1, 2, 3].map((i) => (
              <span
                key={i}
                className={"h-1.5 rounded-full transition-all " + (i <= step ? "w-6 bg-primary" : "w-3 bg-border")}
              />
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
