//! Agent execution runtime — orchestrator, executor, mental model, wake logic.
//!
//! These modules drive the agent lifecycle: the orchestrator dispatches work
//! to agents, the executor runs activities (scripts, git operations), the
//! mental model provides the PM's view of project state, wake logic decides
//! when to act, directives encode governance overrides, context assembles
//! agent prompts, and persona provides agent identity profiles.

pub mod channel;
pub mod context;
pub mod directive;
pub mod executor;
pub mod mental;
pub mod orchestrator;
pub mod persona;
pub mod telegram;
pub mod wake;
pub mod watchdog;

pub use channel::{NoopChannel, OwnerChannel};
pub use context::{AgentContext, WorktreeInfo};
pub use directive::{Directive, DirectiveKind, DirectiveStatus, DirectiveStrength};
pub use executor::{Activity, ActivityKind, ActivityResult, ActivityRunner, WorkspaceRunner};
pub use mental::OperatingModel;
pub use orchestrator::{CostMetering, MockOrchestrator, Orchestrator, PlanOutput};
pub use persona::Persona;
pub use wake::WakeTier;
