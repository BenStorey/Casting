//! The 7 CastRole variants — the only consultant roles the system knows.
//!
//! Every consultant TOML file in `active-cast/` declares which `cast_role`
//! it fills. At startup the loader validates exactly one file per role.
//! Code looks up by `CastRole` enum, never by agent-id string.
//!
//! Three of these are **special** (non-assignable): the PM, the Advisor, and
//! the Stage Manager orchestrate / advise rather than taking implementation
//! tasks. The remaining four are assignable.

use serde::{Deserialize, Serialize};

/// The 7 consultant roles. Serialised as snake_case for both TOML and JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CastRole {
    /// Project Manager — special coordinator, never assignable.
    ProjectManager,
    /// Advisor — strategic thinking partner, never assignable.
    Advisor,
    /// Lead Developer — primary implementation workhorse.
    LeadDeveloper,
    /// Testing Engineer — owns coverage, edge cases, test strategy.
    TestingEngineer,
    /// Systems Architect — large structural work + health reviews.
    SystemsArchitect,
    /// Stage Manager — build pipeline health, environment, CI.
    StageManager,
    /// Critic — adversarial review and stress scenarios.
    Critic,
}

/// All 7 roles in a canonical order.
pub const ALL_CAST_ROLES: &[CastRole] = &[
    CastRole::ProjectManager,
    CastRole::Advisor,
    CastRole::LeadDeveloper,
    CastRole::TestingEngineer,
    CastRole::SystemsArchitect,
    CastRole::StageManager,
    CastRole::Critic,
];

impl CastRole {
    /// The stable role id used in events and the projection
    /// (replaces the old catalog role id).
    pub fn role_id(self) -> &'static str {
        match self {
            Self::ProjectManager => "pm",
            Self::Advisor => "advisor",
            Self::LeadDeveloper => "lead-developer",
            Self::TestingEngineer => "testing-engineer",
            Self::SystemsArchitect => "systems-architect",
            Self::StageManager => "stage-manager",
            Self::Critic => "critic",
        }
    }

    /// Human-readable display title.
    pub fn title(self) -> &'static str {
        match self {
            Self::ProjectManager => "Project Manager",
            Self::Advisor => "Advisor",
            Self::LeadDeveloper => "Lead Developer",
            Self::TestingEngineer => "Testing Engineer",
            Self::SystemsArchitect => "Systems Architect",
            Self::StageManager => "Stage Manager",
            Self::Critic => "The Critic",
        }
    }

    /// Governance scope (drives directive filtering).
    pub fn scope(self) -> &'static str {
        match self {
            Self::ProjectManager => "governance",
            Self::Advisor => "strategy",
            Self::LeadDeveloper => "engineering",
            Self::TestingEngineer => "qa",
            Self::SystemsArchitect => "architecture",
            Self::StageManager => "engineering",
            Self::Critic => "qa",
        }
    }

    /// Whether this role may be assigned implementation work.
    /// PM and Advisor are special non-assignable roles.
    pub fn is_assignable(self) -> bool {
        matches!(
            self,
            Self::LeadDeveloper
                | Self::TestingEngineer
                | Self::SystemsArchitect
                | Self::StageManager
                | Self::Critic
        )
    }

    /// The two special (non-assignable) actors: PM and Advisor.
    pub fn is_special(self) -> bool {
        !self.is_assignable()
    }

    /// Parse from a snake_case string as used in TOML.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "project_manager" => Some(Self::ProjectManager),
            "advisor" => Some(Self::Advisor),
            "lead_developer" => Some(Self::LeadDeveloper),
            "testing_engineer" => Some(Self::TestingEngineer),
            "systems_architect" => Some(Self::SystemsArchitect),
            "stage_manager" => Some(Self::StageManager),
            "critic" => Some(Self::Critic),
            _ => None,
        }
    }
}
