//! Shared server state.

use std::num::NonZeroU32;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use diffyman_db::Db;
use diffyman_storage::BlobStore;
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::config::Config;
use crate::engine::Admission;

/// Timing of background work. The defaults suit a real server; tests make
/// them short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tuning {
    /// How often an idle worker looks for queued jobs (a job created by this
    /// process wakes it at once).
    pub poll_interval: Duration,
    /// Delay before the first retry of a job after an infrastructure error;
    /// later retries wait this times the attempt number.
    pub retry_delay: Duration,
    /// How long a stopping server lets running jobs finish before it hands
    /// them back to the queue.
    pub shutdown_grace: Duration,
}

impl Default for Tuning {
    fn default() -> Tuning {
        Tuning {
            poll_interval: Duration::from_secs(1),
            retry_delay: Duration::from_secs(5),
            shutdown_grace: Duration::from_secs(10),
        }
    }
}

/// State shared by handlers, the worker and the sweeper. Cheap to clone.
#[derive(Clone, Debug)]
pub struct AppState(Arc<Inner>);

/// The shared parts.
#[derive(Debug)]
pub struct Inner {
    /// The configuration.
    pub config: Config,
    /// The database.
    pub db: Db,
    /// The blob store.
    pub store: Arc<dyn BlobStore>,
    /// Background timing.
    pub tuning: Tuning,
    /// Wakes the worker when a job is created or a slot frees up.
    pub jobs_notify: Notify,
    /// Admission of synchronous diffs (`max_parallel_diffs` and
    /// `sync_memory_bytes`). What a diff is given is held until its request
    /// is done and its engine has returned, even when the request timed out.
    pub sync: Admission,
    /// Per-key request limits, when on.
    pub limiter: Option<DefaultKeyedRateLimiter<Uuid>>,
    /// Prefix of this process's job leases.
    pub worker_id: String,
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    /// Builds the state.
    pub fn new(config: Config, db: Db, store: Arc<dyn BlobStore>, tuning: Tuning) -> AppState {
        let sync = Admission::new(
            config.limits.max_parallel_diffs,
            config.limits.sync_memory_bytes,
        );
        let limiter = NonZeroU32::new(config.limits.rate_limit_per_minute)
            .map(|n| RateLimiter::keyed(Quota::per_minute(n)));
        let worker_id = format!(
            "w-{}-{}",
            std::process::id(),
            &Uuid::new_v4().simple().to_string()[..8]
        );
        AppState(Arc::new(Inner {
            config,
            db,
            store,
            tuning,
            jobs_notify: Notify::new(),
            sync,
            limiter,
            worker_id,
        }))
    }
}
