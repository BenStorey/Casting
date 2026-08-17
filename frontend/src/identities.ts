// Agent identities for the UI.
//
// The SINGLE source for regular-cast identity is the consultant registry
// served by `/api/consultants` (identity + avatar + summary per role) — the
// backend owns it, and the SPA reads it via the store. This module only keeps
// the two SPECIAL identities that are NOT catalog consultants (the PM and the
// Direction Advisor, both structurally distinct from hire-able roles), plus
// the pure resolver that maps a hired agent (id + role title) onto a
// consultant config. No hardcoded role roster here anymore.

import type { ConsultantConfig } from "./api";

export interface CastIdentity {
  id: string;
  name: string;
  stable_name: string; // "Marcus Reed" style — never changes
  role: string; // display role / title
  persona: string; // short in-character descriptor
  cv: string[]; // brief shifting of what they do (from the consultant summary)
  avatar: string | null; // path served by the embedded SPA (/avatars/x.svg)
}

export const PM_IDENTITY: CastIdentity = {
  id: "mei",
  name: "Mei",
  stable_name: "Mei",
  role: "Project Manager",
  persona: "Your company's conductor",
  cv: [
    "15+ years shepherding products from idea to production",
    "Keeps the whole cast moving on one clear objective",
    "Escalates to you only when a decision really needs an owner",
  ],
  avatar: "/avatars/mei.jpeg",
};

export const ADVISOR_IDENTITY: CastIdentity = {
  id: "jeeves",
  name: "Jeeves",
  stable_name: "Jeeves",
  role: "Strategic Advisor",
  persona: "Your thinking partner on product direction",
  cv: [
    "Sees the whole product, not the task list",
    "Questions assumptions and surfaces what you haven't considered",
    "Stays out of day-to-day priorities — you decide when to bring it in",
  ],
  avatar: "/avatars/jeeves.jpeg",
};

/// Map a consultant config onto the UI identity shape. The summary becomes the
/// CV; the packaged title becomes the display role.
function fromConsultant(c: ConsultantConfig): CastIdentity {
  return {
    id: c.id,
    name: c.name,
    stable_name: c.name,
    role: c.title,
    persona: c.role_title,
    cv: c.summary ? [c.summary] : [],
    avatar: c.avatar,
  };
}

/// Resolve the best identity for a hired agent, using the loaded consultant
/// registry. Prefers an exact consultant `id` match, then a role-title match,
/// then a loose role keyword match; the PM falls back to its own identity.
/// Returns undefined only for an unknown non-PM agent (callers fall back to
/// initials / the raw id).
export function identityForAgent(
  agentId: string,
  roleTitle: string,
  consultants: ConsultantConfig[] = []
): CastIdentity | undefined {
  if (agentId === "mei") return PM_IDENTITY;
  if (agentId === "jeeves") return ADVISOR_IDENTITY;

  // 1) Exact consultant id (e.g. "marcus-reed").
  const byId = consultants.find((c) => c.id === agentId);
  if (byId) return fromConsultant(byId);

  // 2) Exact role-title match (e.g. agent.role "Security Engineer").
  const t = (roleTitle || "").toLowerCase();
  const byRole = consultants.find((c) => !!roleTitle && c.title.toLowerCase() === t);
  if (byRole) return fromConsultant(byRole);

  // 3) Loose match on the first role word (e.g. agent.role "Engineering" vs
  //    a consultant titled "Senior Engineer").
  if (t.length > 0) {
    const first = t.split(" ")[0];
    const loose = consultants.find((c) => c.title.toLowerCase().startsWith(first));
    if (loose) return fromConsultant(loose);
  }

  return undefined;
}