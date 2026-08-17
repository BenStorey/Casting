//! The cast — the company's team, as configuration (not script logic).
//!
//! A different CEO building a different product ends up with a different cast.
//! This module makes that configuration explicit: a curated **role catalog**
//! (each role carries its governance scope — and, later, its model + base
//! setup prompt for D2) plus a **default cast** seed the onboarded company
//! starts with. the director can add agents (pick a role) and authorize the PM to
//! do so via the TeamChange policy class.
//!
//! This is **configuration/data**, never authoritative state — the projection
//! (agents hired via `AgentHired` events) is the truth. The catalog and default
//! cast are just the seed.
//!
//! The role catalog is derived from two sources:
//! 1. Legacy roles (engineer, qa, security, devops) — predate the 7-role enum
//! 2. The 7 `CastRole` variants — the authoritative source for role metadata

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

/// Legacy role ids that exist in the catalog but do NOT map to a CastRole
/// (they predate the 7-role enum). Kept for backward compat in the setup
/// wizard and test data. The 7 CastRole roles are the authoritative source.
const LEGACY_ROLES: &[Role] = &[
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
];

/// Build the complete role catalog: legacy roles + every CastRole-derived role.
/// The 7 CastRole roles are the authoritative source (the legacy 4 are
/// maintained for compatibility).
pub fn role_catalog() -> Vec<Role> {
    let mut roles: Vec<Role> = LEGACY_ROLES
        .iter()
        .map(|r| Role {
            id: r.id,
            title: r.title,
            scope: r.scope,
        })
        .collect();
    for cr in crate::consultants::cast_role::ALL_CAST_ROLES {
        // Only include assignable roles in the catalog (PM and Advisor are
        // special co-ordinator roles, not hireable agents).
        if !cr.is_assignable() {
            continue;
        }
        roles.push(Role {
            id: cr.role_id(),
            title: cr.title(),
            scope: cr.scope(),
        });
    }
    roles
}

/// Look up a role by id from the catalog.
pub fn role_by_id(id: &str) -> Option<Role> {
    role_catalog().into_iter().find(|r| r.id == id)
}

/// Look up a role by its title (used to map an agent's stored role title back
/// to a catalog role and its scope).
pub fn role_by_title(title: &str) -> Option<Role> {
    role_catalog().into_iter().find(|r| r.title == title)
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
        agent_id: "diego",
        role_id: "lead-developer",
    },
    CastMember {
        agent_id: "tess",
        role_id: "testing-engineer",
    },
    CastMember {
        agent_id: "nina",
        role_id: "systems-architect",
    },
    CastMember {
        agent_id: "ali",
        role_id: "stage-manager",
    },
    CastMember {
        agent_id: "julien",
        role_id: "critic",
    },
];
