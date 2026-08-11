import { useEffect, useState } from "react";
import { fetchSetupStatus, submitSetup, SetupRole, SetupStatus } from "./api";

/// First-run onboarding: name, objective, cast roles, optional owner token.
/// Drives the SAME setup engine as `cast init` (owner decision: one engine,
/// both CLI and UI). On submit, hires the cast and fires the objective so
/// plan_onboard kicks off the build.
export default function SetupWizard({ onDone }: { onDone: () => void }) {
  const [status, setStatus] = useState<SetupStatus | null>(null);
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

  return (
    <div className="app">
      <header className="top">
        <div className="logo">🎬</div>
        <div className="brand">
          <h1>Welcome to Casting</h1>
          <p>Spin up your autonomous software company</p>
        </div>
      </header>

      {err && <div className="banner">⚠️ {err}</div>}

      <div className="card" style={{ maxWidth: 620, margin: "0 auto" }}>
        <label style={{ display: "block", marginBottom: 12 }}>
          <div className="muted small" style={{ marginBottom: 4 }}>Company / product name</div>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Acme Inc"
            style={fieldStyle}
          />
        </label>

        <label style={{ display: "block", marginBottom: 12 }}>
          <div className="muted small" style={{ marginBottom: 4 }}>
            What should your team build first?
          </div>
          <input
            value={objective}
            onChange={(e) => setObjective(e.target.value)}
            placeholder='e.g. "Build me a todo app"'
            style={fieldStyle}
          />
        </label>

        <div className="muted small" style={{ marginBottom: 6 }}>Initial team</div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8, marginBottom: 14 }}>
          {roles.map((r) => (
            <button
              key={r.id}
              type="button"
              onClick={() => toggle(r.id)}
              style={{
                ...roleStyle,
                borderColor: selected.has(r.id) ? "var(--accent)" : "var(--border)",
                background: selected.has(r.id) ? "rgba(79,140,255,0.14)" : "var(--panel-2)",
              }}
            >
              {r.title}
              <span className="muted small" style={{ marginLeft: 6 }}>{r.scope}</span>
            </button>
          ))}
        </div>

        <label style={{ display: "block", marginBottom: 18 }}>
          <div className="muted small" style={{ marginBottom: 4 }}>
            Owner auth token{" "}
            <span className="muted small">(optional — blank leaves writes open)</span>
          </div>
          <input
            value={ownerToken}
            onChange={(e) => setOwnerToken(e.target.value)}
            placeholder="a long random secret"
            style={fieldStyle}
          />
        </label>

        <button
          className="primary"
          onClick={() => void launch()}
          disabled={busy || !objective.trim() || selected.size === 0}
          style={{ width: "100%", padding: "12px", fontSize: 15 }}
        >
          {busy ? "Launching…" : "🚀 Launch my company"}
        </button>
        <div className="muted small" style={{ marginTop: 10, textAlign: "center" }}>
          Your cast is hired from the catalog and the build kicks off right away.
        </div>
      </div>
    </div>
  );
}

const fieldStyle: React.CSSProperties = {
  width: "100%",
  background: "var(--panel-2)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  borderRadius: 8,
  padding: "11px 13px",
  fontSize: 14,
};

const roleStyle: React.CSSProperties = {
  cursor: "pointer",
  border: "1px solid var(--border)",
  background: "var(--panel-2)",
  color: "var(--text)",
  borderRadius: 999,
  padding: "8px 13px",
  fontSize: 13,
};