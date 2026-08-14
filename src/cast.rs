//! The cast — the company's team, as configuration (not script logic).
//!
//! A different CEO building a different product ends up with a different cast.
//! This module makes that configuration explicit: a curated **role catalog**
//! (each role carries its governance scope — and, later, its model + base
//! setup prompt for D2) plus a **default cast** seed the onboarded company
//! starts with. The owner can add agents (pick a role) and authorize the PM to
//! do so via the TeamChange policy class.
//!
//! This is **configuration/data**, never authoritative state — the projection
//! (agents hired via `AgentHired` events) is the truth. The catalog and default
//! cast are just the seed.

use serde::Serialize;

/// A curated role: a named archetype with a default governance scope. The role
/// is the atom — there is no separate "specialization" axis. Later the role
/// will also carry the model + base setup prompt the orchestrator reads (D2).
#[derive(Debug, Clone, Serialize)]
pub struct Role {
    pub id: &'static str,
    pub title: &'static str,
    /// The governance area this role operates in (drives directive filtering).
    pub scope: &'static str,
}

/// The curated role catalog. Hardcoded for now; owner-creating *new role types*
/// is a later, more consequential capability (it invents a new model/prompt).
pub const ROLE_CATALOG: &[Role] = &[
    Role {
        id: "engineer",
        title: "Engineer",
        scope: "engineering",
    },
    Role {
        id: "qa",
        title: "QA",
        scope: "qa",
    },
    Role {
        id: "security",
        title: "Security Engineer",
        scope: "architecture",
    },
    Role {
        id: "devops",
        title: "DevOps / SRE",
        scope: "engineering",
    },
    // The curated default assignable cast (the "Default Cast" from the
    // consultant plan 2026-08-14). Each is a real catalog role the PM can
    // assign implementation work to. The special (non-assignable) roles —
    // PM, Advisor — are NOT catalog roles: they are fixed co-ordinator /
    // adviser actors, never hireable agents.
    Role {
        id: "lead-programmer",
        title: "Lead Programmer",
        scope: "engineering",
    },
    Role {
        id: "test-engineer",
        title: "Test Engineer",
        scope: "qa",
    },
    Role {
        id: "systems-architect",
        title: "Systems Architect",
        scope: "architecture",
    },
    Role {
        id: "stage-manager",
        title: "Stage Manager",
        scope: "engineering",
    },
    Role {
        id: "critic",
        title: "The Critic",
        scope: "qa",
    },
];

/// Look up a role by id from the catalog.
pub fn role_by_id(id: &str) -> Option<&'static Role> {
    ROLE_CATALOG.iter().find(|r| r.id == id)
}

/// Look up a role by its title (used to map an agent's stored role title back
/// to a catalog role and its scope).
pub fn role_by_title(title: &str) -> Option<&'static Role> {
    ROLE_CATALOG.iter().find(|r| r.title == title)
}

/// A member of the default cast: an agent id bound to a role.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CastMember {
    pub agent_id: &'static str,
    pub role_id: &'static str,
}

/// The default cast a freshly onboarded company starts with: the five
/// ASSIGNABLE consultants the PM can route implementation work to. The two
/// SPECIAL roles — the PM and the Advisor — are NOT seeded as hireable agents:
/// they are fixed co-ordinator / adviser actors with their own personas and
/// can never be assigned tasks (enforced by the policy gate). A CEO extends
/// the assignable cast by hiring more agents.
pub const DEFAULT_CAST: &[CastMember] = &[
    CastMember {
        agent_id: "lead-programmer",
        role_id: "lead-programmer",
    },
    CastMember {
        agent_id: "test-engineer",
        role_id: "test-engineer",
    },
    CastMember {
        agent_id: "systems-architect",
        role_id: "systems-architect",
    },
    CastMember {
        agent_id: "stage-manager",
        role_id: "stage-manager",
    },
    CastMember {
        agent_id: "critic",
        role_id: "critic",
    },
];
