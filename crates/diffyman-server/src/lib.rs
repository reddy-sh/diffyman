//! `diffyman-server`: the local REST service over the diffyman engine
//! ([ADR 0012](../../../docs/adr/0012-local-service-architecture.md),
//! API reference in `docs/api/openapi.json`, guide in `docs/server.md`).
//!
//! - [`api`]: the routes, their request and response types and the OpenAPI
//!   document.
//! - [`guard`]: request IDs and time limits, `Host` and `Origin` checks,
//!   API-key authentication, scopes, rate limits and usage records.
//! - [`worker`]: the job queue's worker; [`sweeper`]: retention.
//! - [`serve`]: start-up and graceful shutdown; [`conn`]: the header-read
//!   and idle time limits of connections; [`rejected`]: typed answers for
//!   requests hyper refuses before routing; [`keys`]: the `keys` commands.
//! - [`config`], [`admin_token`], [`logging`] (a field allowlist).
//! - [`alloc`]: the global allocator of the binary, which returns large
//!   blocks to the system when they are freed.

pub mod admin_token;
pub mod alloc;
pub mod api;
pub mod config;
pub mod conn;
pub mod content;
pub mod engine;
pub mod error;
pub mod guard;
pub mod keys;
pub mod logging;
pub mod rejected;
pub mod serve;
pub mod state;
pub mod sweeper;
pub mod worker;

pub use api::router;
pub use state::{AppState, Tuning};

use diffyman_db::records::Actor;

/// The audit actor of a caller: its key, or the admin when authentication
/// is off (the local machine's owner).
pub(crate) fn actor(caller: &guard::Caller) -> Actor {
    caller.key_id().map_or(Actor::Admin, Actor::Key)
}
