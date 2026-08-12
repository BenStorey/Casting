// Cast identities — names, titles, brief CVs, and cartoon avatars for the PM
// and each catalog role. This is PRESENTATION/config data (frontend-only); the
// authoritative cast (who's hired, bound to which role) lives in the event log.
// Avatars are role-based cartoon headshots served from /avatars/*.png.
//
// The default cast seeds the PM + Engineer(Marcus) + QA(Maya); the catalog also
// offers Security + DevOps the owner can add via the role picker.

export interface CastIdentity {
  id: string; // the agent id or role id used to look this up
  name: string;
  stable_name: string; // "Marcus Reed" style — never changes
  role: string; // display role / title
  persona: string; // short in-character descriptor
  cv: string[]; // brief CV bullets
  avatar: string; // path served by the embedded SPA (/avatars/x.png)
}

const AV = "/avatars/";

export const PM_IDENTITY: CastIdentity = {
  id: "pm",
  name: "Sarah Chen",
  stable_name: "Sarah Chen",
  role: "Project Manager",
  persona: "Your company's conductor",
  cv: [
    "15+ years shepherding products from idea to production",
    "Keeps the whole cast moving on one clear objective",
    "Escalates to you only when a decision really needs an owner",
  ],
  avatar: `${AV}pm.svg`,
};

export const ROLE_IDENTITIES: Record<string, CastIdentity> = {
  engineer: {
    id: "marcus-reed",
    name: "Marcus Reed",
    stable_name: "Marcus Reed",
    role: "Senior Engineer",
    persona: "Ships clean, working code",
    cv: [
      "Full-stack builder — Rust, TypeScript, React",
      "Prefers boring, battle-tested tech that ships",
      "Leaves the codebase tidier than he found it",
    ],
    avatar: `${AV}engineer.svg`,
  },
  qa: {
    id: "maya-patel",
    name: "Maya Patel",
    stable_name: "Maya Patel",
    role: "QA Lead",
    persona: "Your safety net, in the best way",
    cv: [
      "Automated + exploratory testing specialist",
      "Catches the bug you didn't know you had",
      "Signs off only on work that actually holds up",
    ],
    avatar: `${AV}qa.svg`,
  },
  security: {
    id: "devon-carter",
    name: "Devon Carter",
    stable_name: "Devon Carter",
    role: "Security Engineer",
    persona: "Keeps the doors locked and the lights on",
    cv: [
      "Offensive + defensive security background",
      "Threat-models before we build, not after",
      "Treats every dependency like a possible entry point",
    ],
    avatar: `${AV}security.svg`,
  },
  devops: {
    id: "priya-sharma",
    name: "Priya Sharma",
    stable_name: "Priya Sharma",
    role: "DevOps / SRE",
    persona: "Makes the machinery disappear",
    cv: [
      "CIs, deploys, and infrastructure that run themselves",
      "Obsessive about repeatable, zero-drama releases",
      "Monitors so a problem never reaches your users",
    ],
    avatar: `${AV}devops.svg`,
  },
};

/** Resolve identity for any agent id or role id we might render. */
export function identityFor(key: string): CastIdentity | undefined {
  if (key === "pm" || key === "Sarah Chen" || key === "Project Manager") {
    return PM_IDENTITY;
  }
  return ROLE_IDENTITIES[key];
}

/** Given a hired agent (id + role title), resolve the best identity. */
export function identityForAgent(agentId: string, roleTitle: string): CastIdentity {
  // Match by agent id first (default cast ids), then by role title.
  for (const r of Object.values(ROLE_IDENTITIES)) {
    if (r.id === agentId) return r;
  }
  const byRole = Object.values(ROLE_IDENTITIES).find((r) =>
    roleTitle.toLowerCase().includes(r.role.toLowerCase().split(" ")[0].toLowerCase())
  );
  if (byRole) return byRole;
  return { ...PM_IDENTITY, id: agentId };
}
