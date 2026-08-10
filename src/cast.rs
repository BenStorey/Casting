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

/// The default cast a freshly onboarded company starts with: the PM plus a
/// small engineering/QA team. A CEO will extend this by hiring more agents.
pub const DEFAULT_CAST: &[CastMember] = &[
    CastMember {
        agent_id: "marcus-reed",
        role_id: "engineer",
    },
    CastMember {
        agent_id: "maya-patel",
        role_id: "qa",
    },
];
