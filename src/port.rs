//! Deterministic, collision-free API port allocation for consultant worktrees.
//!
//! Each provisioned worktree gets a distinct network port so every consultant's
//! dev server can run in parallel without colliding (owner requirement,
//! 2026-08-12). Allocation is DETERMINISTIC and driven by the projection: pick
//! the lowest free port in a configured range that no existing worktree uses.
//!
//! The projection (`Projection.worktrees`) is the authority on which ports are
//! taken, so a re-provision is stable and the same worktree keeps its port.

use crate::projection::Projection;

/// The default base of the worktree port range (ports base..base+1024 are
/// reserved for consultants). Overridable via `CAST_WORKTREE_BASE_PORT`.
pub const DEFAULT_WORKTREE_BASE_PORT: u16 = 8081;

/// The size of the worktree port pool.
pub const WORKTREE_PORT_POOL: u16 = 1024;

/// The configured base port for the consultant worktree pool (env override, or
/// the default).
pub fn worktree_base_port() -> u16 {
    std::env::var("CAST_WORKTREE_BASE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(DEFAULT_WORKTREE_BASE_PORT)
}

/// Allocate the lowest port in the pool [base, base+pool) that is NOT already
/// used by an existing provisioned worktree in `projection`. Deterministic:
/// same projection → same answer. Returns `None` if the pool is exhausted.
pub fn allocate_port(projection: &Projection) -> Option<u16> {
    let base = worktree_base_port();
    let used: std::collections::HashSet<u16> =
        projection.worktrees.iter().map(|w| w.port).collect();
    (base..base.saturating_add(WORKTREE_PORT_POOL)).find(|p| !used.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::{Projection, Worktree};

    fn proj_with(ports: &[u16]) -> Projection {
        Projection {
            worktrees: ports
                .iter()
                .enumerate()
                .map(|(i, p)| Worktree {
                    consultant: format!("consultant-{i}"),
                    slot: 0,
                    task_id: Some(format!("task-{i}")),
                    branch: format!("casting/task-{i}-x"),
                    path: format!("/x/{i}"),
                    cargo_target_dir: format!("/x/{i}/target"),
                    port: *p,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn allocates_lowest_free_port_in_range() {
        let empty = proj_with(&[]);
        assert_eq!(allocate_port(&empty), Some(DEFAULT_WORKTREE_BASE_PORT));

        // Occupying 8081..8084 pushes the next allocation to 8085.
        let used = proj_with(&[8081, 8082, 8083, 8084]);
        assert_eq!(allocate_port(&used), Some(8085));
    }

    #[test]
    fn skips_occupied_ports_deterministically() {
        let proj = proj_with(&[8081, 8083, 8085]);
        assert_eq!(allocate_port(&proj), Some(8082));
    }

    #[test]
    fn preserves_an_agents_existing_port() {
        // If a worktree already has a port, the allocator skips it — the same
        // worktree keeps its port on re-provision.
        let proj = proj_with(&[8090]);
        assert_eq!(allocate_port(&proj), Some(DEFAULT_WORKTREE_BASE_PORT));
    }
}
