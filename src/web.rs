//! Axum web server for the vertical slice: JSON API over the projections,
//! director inbox endpoints, SSE realtime, and the embedded React SPA.
//!
//! Serves everything from ONE binary (brief §26/§29/§31): the API and the
//! compiled frontend are both handled here, so `cast run` stays a single
//! self-contained native executable whose only output is a local workspace.
//!
//! This module is a thin facade: the handlers + request DTOs now live split by
//! concern under `src/web/routes/`, and the public `casting::web::router(AppState)`
//! entry point is re-exported unchanged.

mod routes;

pub use routes::router;
