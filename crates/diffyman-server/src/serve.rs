//! `diffyman-server serve`: start-up and graceful shutdown.
//!
//! Start-up, in order: load the configuration (done by the caller), install
//! the log allowlist, create the temporary folder (mode 0700), write the
//! admin token file if it is missing (and say where it is, never what it
//! holds), connect to the database and run its migrations, open the blob
//! store (creating its folder), bind the listener, then start the worker and
//! the sweeper. Connections keep a header-read and an idle time limit
//! ([`conn`]). On SIGINT or SIGTERM the listener stops accepting, requests
//! in flight finish, the worker gives running jobs the shutdown grace
//! period, and the database pool closes.

use std::future::Future;

use axum::middleware::from_fn;
use diffyman_db::{Db, DbError, DbOptions};
use diffyman_storage::{StorageError, StoreOptions};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::admin_token::{self, TokenError};
use crate::config::{Config, Environment};
use crate::conn::{self, Connection, TimedListener, Timeouts};
use crate::state::{AppState, Tuning};
use crate::{api, logging, sweeper, worker};

/// Why the server could not start or stopped with an error. Messages never
/// contain secrets or the database URL.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The admin token file.
    #[error(transparent)]
    Token(#[from] TokenError),
    /// The database.
    #[error("{0}")]
    Db(#[from] DbError),
    /// The blob store.
    #[error("blob store: {0}")]
    Storage(#[from] StorageError),
    /// The listener or a folder.
    #[error("{what}: {source}")]
    Io {
        /// What failed.
        what: &'static str,
        /// The error.
        #[source]
        source: std::io::Error,
    },
}

fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> ServeError {
    move |source| ServeError::Io { what, source }
}

fn private_dir(path: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// Starts the server and runs it until SIGINT or SIGTERM.
pub async fn serve(config: Config) -> Result<(), ServeError> {
    logging::init(config.log_level);
    private_dir(&config.tmp_dir).map_err(io("cannot create the temporary folder"))?;
    let token = admin_token::ensure(&config.admin_token_file)?;
    // The location goes to the terminal once; it is never logged.
    eprintln!(
        "diffyman-server: admin token {} {} (keep it private; it creates API keys)",
        if token.created {
            "written to"
        } else {
            "kept in"
        },
        config.admin_token_file.display()
    );
    let db = Db::connect(
        config.db_url.expose(),
        DbOptions {
            max_connections: config.db_max_connections,
            ..DbOptions::default()
        },
    )
    .await?;
    db.migrate().await?;
    let store = diffyman_storage::open(
        &config.store_url,
        StoreOptions {
            max_blob_bytes: config.limits.max_blob_bytes,
            ..StoreOptions::default()
        },
    )?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(io("cannot listen"))?;
    let state = AppState::new(config, db, store, Tuning::default());
    run(listener, state, shutdown_signal()).await
}

/// Serves on `listener` until `shutdown` completes, with the worker and the
/// sweeper running alongside.
pub async fn run(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let addr = listener.local_addr().map_err(io("cannot listen"))?;
    let env = match state.config.env {
        Environment::Local => "local",
        Environment::Prod => "prod",
    };
    let store = state
        .config
        .store_url
        .split_once(':')
        .map_or("?", |(scheme, _)| scheme);
    tracing::info!(
        addr = %addr,
        env,
        store,
        parallel_jobs = state.config.limits.max_parallel_jobs,
        "listening"
    );
    eprintln!("diffyman-server: listening on http://{addr}");
    let (stop, stopped) = watch::channel(false);
    let worker = tokio::spawn(worker::run(state.clone(), stopped.clone()));
    let sweeper = tokio::spawn(sweeper::run(state.clone(), stopped));
    let limits = state.config.limits;
    let listener = TimedListener::new(
        listener,
        Timeouts {
            header_read: limits.header_read_timeout,
            idle: limits.idle_timeout,
        },
    );
    let app = api::router(state.clone())
        .layer(from_fn(conn::track))
        .into_make_service_with_connect_info::<Connection>();
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    let _ = stop.send(true);
    let _ = worker.await;
    let _ = sweeper.await;
    state.db.close().await;
    tracing::info!("stopped");
    eprintln!("diffyman-server: stopped");
    served.map_err(io("serving failed"))
}

/// Completes on SIGINT (Ctrl-C) or, on Unix, SIGTERM.
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
