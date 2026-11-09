//! Server configuration ([ADR 0012](../../../docs/adr/0012-local-service-architecture.md)).
//!
//! Layers, later ones winning: built-in defaults, then `diffyman.toml` (the
//! path in `DIFFYMAN_CONFIG`, or `./diffyman.toml` when it exists), then
//! `DIFFYMAN_*` environment variables. With `DIFFYMAN_ENV=local` (the
//! default) `./.env.local` also fills in variables that are not set, and the
//! database URL defaults to the local container built from
//! `DIFFYMAN_PG_PASSWORD`. With `DIFFYMAN_ENV=prod`, `.env.local` is never
//! read and `DIFFYMAN_DB_URL` and `DIFFYMAN_ALLOWED_HOSTS` are required.
//!
//! | Variable | Default |
//! |---|---|
//! | `DIFFYMAN_ENV` | `local` (or `prod`) |
//! | `DIFFYMAN_CONFIG` | `./diffyman.toml` if present |
//! | `DIFFYMAN_BIND` | `127.0.0.1:8080`; `local` accepts loopback addresses only |
//! | `DIFFYMAN_DB_URL` | `local`: `postgres://diffyman:$DIFFYMAN_PG_PASSWORD@127.0.0.1:5433/diffyman` |
//! | `DIFFYMAN_DB_MAX_CONNECTIONS` | `10` |
//! | `DIFFYMAN_STORE_URL` | `file://~/diffyman-data/blobs` |
//! | `DIFFYMAN_TMP_DIR` | `~/diffyman-data/tmp` |
//! | `DIFFYMAN_AUTH_MODE` | `api_key` (`none` only in `local`) |
//! | `DIFFYMAN_ADMIN_TOKEN_FILE` | `~/.diffyman/admin-token` |
//! | `DIFFYMAN_ALLOWED_HOSTS` | `local`: `127.0.0.1`, `localhost` and `[::1]` with the bound port |
//! | `DIFFYMAN_ALLOWED_ORIGINS` | none |
//! | `DIFFYMAN_MAX_BODY_BYTES` | 24 MiB (a synchronous `/v1/diff` request body) |
//! | `DIFFYMAN_MAX_SYNC_INPUT_BYTES` | 8 MiB (each input of a synchronous diff) |
//! | `DIFFYMAN_MAX_BLOB_BYTES` | 1 GiB |
//! | `DIFFYMAN_REQUEST_TIMEOUT_SECS` | `30` |
//! | `DIFFYMAN_HEADER_READ_TIMEOUT_SECS` | `10` (to send a request head: from the accept, or from its first byte on a kept-alive connection) |
//! | `DIFFYMAN_IDLE_TIMEOUT_SECS` | `60` (a kept-alive connection silent between requests is closed) |
//! | `DIFFYMAN_MAX_PARALLEL_DIFFS` | the CPU cores (synchronous `/v1/diff` requests at once) |
//! | `DIFFYMAN_SYNC_MEMORY_BYTES` | 1 GiB (shared by synchronous diffs, 4 bytes per body byte) |
//! | `DIFFYMAN_JOB_TIMEOUT_SECS` | `600` |
//! | `DIFFYMAN_JOB_MEMORY_BYTES` | 2 GiB |
//! | `DIFFYMAN_MAX_PARALLEL_JOBS` | half the CPU cores, between 1 and 4 |
//! | `DIFFYMAN_JOB_LEASE_SECS` | `60` |
//! | `DIFFYMAN_JOB_MAX_ATTEMPTS` | `3` |
//! | `DIFFYMAN_RATE_LIMIT_PER_MINUTE` | `600` requests per API key (`0` turns it off) |
//! | `DIFFYMAN_DIFF_TTL_DAYS`, `DIFFYMAN_BLOB_TTL_DAYS`, `DIFFYMAN_JOB_TTL_DAYS` | `30` |
//! | `DIFFYMAN_SWEEP_INTERVAL_SECS` | `3600` |
//! | `DIFFYMAN_ORPHAN_GRACE_SECS` | `86400` (objects without a row are removed after this) |
//! | `DIFFYMAN_LOG` | `info` (or `error`, `warn`, `debug`) |
//!
//! List values are comma-separated. `~/` at the start of a path (or after
//! `file://`) is the home folder.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;
const DAY_SECS: u64 = 86_400;

/// Where the server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Environment {
    /// A developer machine: loopback only, `.env.local` allowed.
    Local,
    /// A deployment: every secret comes from the environment.
    Prod,
}

/// How clients authenticate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// API keys created with the admin token.
    ApiKey,
    /// No authentication: `local` only, for experiments.
    None,
}

/// A secret value: `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Sensitive(String);

impl Sensitive {
    /// The value, for the one place that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Sensitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(redacted)")
    }
}

impl fmt::Display for Sensitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(redacted)")
    }
}

/// How much the server logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Errors and warnings.
    Warn,
    /// Also one line per request, job and sweep.
    Info,
    /// Also internal steps.
    Debug,
}

/// Size, time and concurrency limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest request body of a synchronous diff.
    pub max_body_bytes: u64,
    /// Largest input (each side) of a synchronous diff.
    pub max_sync_input_bytes: u64,
    /// Largest uploaded blob.
    pub max_blob_bytes: u64,
    /// Time limit of one request.
    pub request_timeout: Duration,
    /// Time a client has to send a whole request head: from the accept on
    /// a new connection, from the head's first byte on a kept-alive one.
    pub header_read_timeout: Duration,
    /// Time a kept-alive connection may stay silent between requests.
    pub idle_timeout: Duration,
    /// Synchronous diffs admitted at once in this process, body reading
    /// included.
    pub max_parallel_diffs: u32,
    /// Memory budget shared by the synchronous diffs in flight; each one
    /// reserves four bytes per byte of its request body.
    pub sync_memory_bytes: u64,
    /// Time limit of one job attempt.
    pub job_timeout: Duration,
    /// Memory budget of one job, admitted by input size.
    pub job_memory_bytes: u64,
    /// Jobs running at once in this process.
    pub max_parallel_jobs: u32,
    /// Lease of a claimed job, renewed by heartbeats.
    pub job_lease: Duration,
    /// Attempts per job.
    pub job_max_attempts: u32,
    /// Requests per minute and API key; `0` means no limit.
    pub rate_limit_per_minute: u32,
}

impl Limits {
    /// The largest total input size of one job: the engine is budgeted at
    /// four bytes of memory per input byte (the inputs, their line index and
    /// the document).
    pub fn max_job_input_bytes(&self) -> u64 {
        self.job_memory_bytes / 4
    }
}

/// How long content is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    /// Saved diffs.
    pub diff_ttl: Duration,
    /// Uploaded blobs that nothing references.
    pub blob_ttl: Duration,
    /// Finished jobs.
    pub job_ttl: Duration,
    /// Time between sweeps.
    pub sweep_interval: Duration,
    /// Age after which a stored object without a row is removed.
    pub orphan_grace: Duration,
}

/// The resolved configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// `local` or `prod`.
    pub env: Environment,
    /// Listening address.
    pub bind: SocketAddr,
    /// Database URL (contains the password).
    pub db_url: Sensitive,
    /// Pool size.
    pub db_max_connections: u32,
    /// Blob store URL: `file://`, `memory://` or `s3://`.
    pub store_url: String,
    /// Folder for temporary files.
    pub tmp_dir: PathBuf,
    /// Client authentication.
    pub auth_mode: AuthMode,
    /// Where the admin token is written on first start.
    pub admin_token_file: PathBuf,
    /// Accepted `Host` header values (`host:port`, lowercase).
    pub allowed_hosts: Vec<String>,
    /// Accepted `Origin` header values; requests with any other `Origin` are
    /// refused.
    pub allowed_origins: Vec<String>,
    /// Limits.
    pub limits: Limits,
    /// Retention.
    pub retention: Retention,
    /// Log level.
    pub log_level: LogLevel,
    /// Unknown `DIFFYMAN_*` variable names, to report at start.
    pub unknown_vars: Vec<String>,
}

/// A configuration error. Messages name the setting, never a secret value.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A setting has a value that cannot be used.
    #[error("{name}: {reason}")]
    Invalid {
        /// The variable or key.
        name: String,
        /// What is wrong.
        reason: String,
    },
    /// A required setting is missing.
    #[error("{0} must be set")]
    Missing(&'static str),
    /// A file could not be read.
    #[error("cannot read {what}: {source}")]
    Read {
        /// Which file.
        what: &'static str,
        /// The error.
        #[source]
        source: std::io::Error,
    },
    /// `diffyman.toml` is not valid.
    #[error("diffyman.toml: {0}")]
    File(String),
}

fn invalid(name: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        name: name.to_owned(),
        reason: reason.into(),
    }
}

/// Inputs of [`Config::load`], so tests need not touch the process.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// Environment variables.
    pub vars: BTreeMap<String, String>,
    /// Working folder (for `.env.local` and `diffyman.toml`).
    pub cwd: PathBuf,
    /// Home folder, for `~/` and the defaults.
    pub home: Option<PathBuf>,
    /// CPU cores available, for the default number of parallel jobs and
    /// synchronous diffs (`0` when unknown).
    pub cpus: usize,
}

impl Sources {
    /// The real process environment.
    pub fn from_process() -> Sources {
        let vars: BTreeMap<String, String> = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        let home = vars
            .get("HOME")
            .or_else(|| vars.get("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let cpus = std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get);
        Sources {
            vars,
            cwd,
            home,
            cpus,
        }
    }
}

/// `diffyman.toml`: every key optional; unknown keys are errors.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<String>,
    db_url: Option<String>,
    db_max_connections: Option<u32>,
    store_url: Option<String>,
    tmp_dir: Option<String>,
    auth_mode: Option<String>,
    admin_token_file: Option<String>,
    allowed_hosts: Option<Vec<String>>,
    allowed_origins: Option<Vec<String>>,
    log: Option<String>,
    #[serde(default)]
    limits: FileLimits,
    #[serde(default)]
    retention: FileRetention,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLimits {
    max_body_bytes: Option<u64>,
    max_sync_input_bytes: Option<u64>,
    max_blob_bytes: Option<u64>,
    request_timeout_secs: Option<u64>,
    header_read_timeout_secs: Option<u64>,
    idle_timeout_secs: Option<u64>,
    max_parallel_diffs: Option<u32>,
    sync_memory_bytes: Option<u64>,
    job_timeout_secs: Option<u64>,
    job_memory_bytes: Option<u64>,
    max_parallel_jobs: Option<u32>,
    job_lease_secs: Option<u64>,
    job_max_attempts: Option<u32>,
    rate_limit_per_minute: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileRetention {
    diff_ttl_days: Option<u64>,
    blob_ttl_days: Option<u64>,
    job_ttl_days: Option<u64>,
    sweep_interval_secs: Option<u64>,
    orphan_grace_secs: Option<u64>,
}

/// Every variable the server reads. Others starting with `DIFFYMAN_` are
/// reported as unknown, except the test and tooling ones.
const KNOWN_VARS: &[&str] = &[
    "DIFFYMAN_ENV",
    "DIFFYMAN_CONFIG",
    "DIFFYMAN_BIND",
    "DIFFYMAN_DB_URL",
    "DIFFYMAN_DB_MAX_CONNECTIONS",
    "DIFFYMAN_PG_PASSWORD",
    "DIFFYMAN_STORE_URL",
    "DIFFYMAN_TMP_DIR",
    "DIFFYMAN_AUTH_MODE",
    "DIFFYMAN_ADMIN_TOKEN_FILE",
    "DIFFYMAN_ALLOWED_HOSTS",
    "DIFFYMAN_ALLOWED_ORIGINS",
    "DIFFYMAN_MAX_BODY_BYTES",
    "DIFFYMAN_MAX_SYNC_INPUT_BYTES",
    "DIFFYMAN_MAX_BLOB_BYTES",
    "DIFFYMAN_REQUEST_TIMEOUT_SECS",
    "DIFFYMAN_HEADER_READ_TIMEOUT_SECS",
    "DIFFYMAN_IDLE_TIMEOUT_SECS",
    "DIFFYMAN_MAX_PARALLEL_DIFFS",
    "DIFFYMAN_SYNC_MEMORY_BYTES",
    "DIFFYMAN_JOB_TIMEOUT_SECS",
    "DIFFYMAN_JOB_MEMORY_BYTES",
    "DIFFYMAN_MAX_PARALLEL_JOBS",
    "DIFFYMAN_JOB_LEASE_SECS",
    "DIFFYMAN_JOB_MAX_ATTEMPTS",
    "DIFFYMAN_RATE_LIMIT_PER_MINUTE",
    "DIFFYMAN_DIFF_TTL_DAYS",
    "DIFFYMAN_BLOB_TTL_DAYS",
    "DIFFYMAN_JOB_TTL_DAYS",
    "DIFFYMAN_SWEEP_INTERVAL_SECS",
    "DIFFYMAN_ORPHAN_GRACE_SECS",
    "DIFFYMAN_LOG",
];
const TOOLING_PREFIXES: &[&str] = &["DIFFYMAN_TEST_", "DIFFYMAN_S3_TEST_"];

impl Config {
    /// Loads the configuration of the running process.
    pub fn from_process() -> Result<Config, ConfigError> {
        Config::load(Sources::from_process())
    }

    /// Resolves the configuration from `sources`.
    pub fn load(sources: Sources) -> Result<Config, ConfigError> {
        let Sources {
            mut vars,
            cwd,
            home,
            cpus,
        } = sources;
        let env = match vars.get("DIFFYMAN_ENV").map(String::as_str) {
            None | Some("local") => Environment::Local,
            Some("prod") => Environment::Prod,
            Some(_) => return Err(invalid("DIFFYMAN_ENV", "expected `local` or `prod`")),
        };
        if env == Environment::Local {
            let path = cwd.join(".env.local");
            if path.exists() {
                for (key, value) in read_env_file(&path)? {
                    vars.entry(key).or_insert(value);
                }
            }
        }
        let file = load_file(&vars, &cwd)?;
        let r = Resolver {
            vars: &vars,
            home: home.as_deref(),
        };

        let bind_text = r.text("DIFFYMAN_BIND", file.bind, "127.0.0.1:8080");
        let bind: SocketAddr = bind_text
            .parse()
            .map_err(|_| invalid("DIFFYMAN_BIND", "expected ip:port"))?;
        if env == Environment::Local && !bind.ip().is_loopback() {
            return Err(invalid(
                "DIFFYMAN_BIND",
                "the local environment listens on loopback addresses only",
            ));
        }

        let db_url = match vars.get("DIFFYMAN_DB_URL").cloned().or(file.db_url) {
            Some(url) => url,
            None if env == Environment::Local => match vars.get("DIFFYMAN_PG_PASSWORD") {
                Some(password) => {
                    format!("postgres://diffyman:{password}@127.0.0.1:5433/diffyman")
                }
                None => {
                    return Err(ConfigError::Missing(
                        "DIFFYMAN_DB_URL or DIFFYMAN_PG_PASSWORD",
                    ));
                }
            },
            None => return Err(ConfigError::Missing("DIFFYMAN_DB_URL")),
        };
        if !(db_url.starts_with("postgres://") || db_url.starts_with("postgresql://")) {
            return Err(invalid("DIFFYMAN_DB_URL", "expected a postgres:// URL"));
        }

        let store_url = r.store_url(file.store_url)?;
        let tmp_dir = r.path("DIFFYMAN_TMP_DIR", file.tmp_dir, "~/diffyman-data/tmp")?;
        let admin_token_file = r.path(
            "DIFFYMAN_ADMIN_TOKEN_FILE",
            file.admin_token_file,
            "~/.diffyman/admin-token",
        )?;

        let auth_mode = match r
            .text("DIFFYMAN_AUTH_MODE", file.auth_mode, "api_key")
            .as_str()
        {
            "api_key" => AuthMode::ApiKey,
            "none" if env == Environment::Local => AuthMode::None,
            "none" => {
                return Err(invalid(
                    "DIFFYMAN_AUTH_MODE",
                    "`none` is for the local environment only",
                ));
            }
            _ => {
                return Err(invalid(
                    "DIFFYMAN_AUTH_MODE",
                    "expected `api_key` or `none`",
                ));
            }
        };

        let allowed_hosts = match r.list("DIFFYMAN_ALLOWED_HOSTS", file.allowed_hosts) {
            Some(hosts) if !hosts.is_empty() => hosts,
            _ if env == Environment::Local => {
                let port = bind.port();
                vec![
                    format!("127.0.0.1:{port}"),
                    format!("localhost:{port}"),
                    format!("[::1]:{port}"),
                ]
            }
            _ => return Err(ConfigError::Missing("DIFFYMAN_ALLOWED_HOSTS")),
        };
        for host in &allowed_hosts {
            check_host(host)?;
        }
        let allowed_origins = r
            .list("DIFFYMAN_ALLOWED_ORIGINS", file.allowed_origins)
            .unwrap_or_default();
        for origin in &allowed_origins {
            check_origin(origin)?;
        }

        let l = file.limits;
        let limits = Limits {
            max_body_bytes: r.number(
                "DIFFYMAN_MAX_BODY_BYTES",
                l.max_body_bytes,
                24 * MIB,
                1..=GIB,
            )?,
            max_sync_input_bytes: r.number(
                "DIFFYMAN_MAX_SYNC_INPUT_BYTES",
                l.max_sync_input_bytes,
                8 * MIB,
                1..=GIB,
            )?,
            max_blob_bytes: r.number(
                "DIFFYMAN_MAX_BLOB_BYTES",
                l.max_blob_bytes,
                GIB,
                1..=64 * GIB,
            )?,
            request_timeout: r.secs(
                "DIFFYMAN_REQUEST_TIMEOUT_SECS",
                l.request_timeout_secs,
                30,
                1..=3600,
            )?,
            header_read_timeout: r.secs(
                "DIFFYMAN_HEADER_READ_TIMEOUT_SECS",
                l.header_read_timeout_secs,
                10,
                1..=600,
            )?,
            idle_timeout: r.secs(
                "DIFFYMAN_IDLE_TIMEOUT_SECS",
                l.idle_timeout_secs,
                60,
                1..=3600,
            )?,
            max_parallel_diffs: r.small(
                "DIFFYMAN_MAX_PARALLEL_DIFFS",
                l.max_parallel_diffs,
                u32::try_from(cpus.clamp(1, 1024)).unwrap_or(1),
                1..=1024,
            )?,
            sync_memory_bytes: r.number(
                "DIFFYMAN_SYNC_MEMORY_BYTES",
                l.sync_memory_bytes,
                GIB,
                MIB..=256 * GIB,
            )?,
            job_timeout: r.secs(
                "DIFFYMAN_JOB_TIMEOUT_SECS",
                l.job_timeout_secs,
                600,
                1..=DAY_SECS,
            )?,
            job_memory_bytes: r.number(
                "DIFFYMAN_JOB_MEMORY_BYTES",
                l.job_memory_bytes,
                2 * GIB,
                MIB..=256 * GIB,
            )?,
            max_parallel_jobs: r.small(
                "DIFFYMAN_MAX_PARALLEL_JOBS",
                l.max_parallel_jobs,
                default_parallel_jobs(cpus),
                1..=256,
            )?,
            job_lease: r.secs("DIFFYMAN_JOB_LEASE_SECS", l.job_lease_secs, 60, 5..=3600)?,
            job_max_attempts: r.small(
                "DIFFYMAN_JOB_MAX_ATTEMPTS",
                l.job_max_attempts,
                3,
                1..=100,
            )?,
            rate_limit_per_minute: r.small(
                "DIFFYMAN_RATE_LIMIT_PER_MINUTE",
                l.rate_limit_per_minute,
                600,
                0..=1_000_000,
            )?,
        };
        let t = file.retention;
        let retention = Retention {
            diff_ttl: r.days("DIFFYMAN_DIFF_TTL_DAYS", t.diff_ttl_days)?,
            blob_ttl: r.days("DIFFYMAN_BLOB_TTL_DAYS", t.blob_ttl_days)?,
            job_ttl: r.days("DIFFYMAN_JOB_TTL_DAYS", t.job_ttl_days)?,
            sweep_interval: r.secs(
                "DIFFYMAN_SWEEP_INTERVAL_SECS",
                t.sweep_interval_secs,
                3600,
                10..=DAY_SECS,
            )?,
            orphan_grace: r.secs(
                "DIFFYMAN_ORPHAN_GRACE_SECS",
                t.orphan_grace_secs,
                DAY_SECS,
                3600..=30 * DAY_SECS,
            )?,
        };
        let log_level = match r.text("DIFFYMAN_LOG", file.log, "info").as_str() {
            "error" => LogLevel::Error,
            "warn" => LogLevel::Warn,
            "info" => LogLevel::Info,
            "debug" => LogLevel::Debug,
            _ => {
                return Err(invalid(
                    "DIFFYMAN_LOG",
                    "expected `error`, `warn`, `info` or `debug`",
                ));
            }
        };
        let db_max_connections = r.small(
            "DIFFYMAN_DB_MAX_CONNECTIONS",
            file.db_max_connections,
            10,
            1..=1000,
        )?;

        let unknown_vars = vars
            .keys()
            .filter(|k| k.starts_with("DIFFYMAN_"))
            .filter(|k| !KNOWN_VARS.contains(&k.as_str()))
            .filter(|k| !TOOLING_PREFIXES.iter().any(|p| k.starts_with(p)))
            .cloned()
            .collect();

        Ok(Config {
            env,
            bind,
            db_url: Sensitive(db_url),
            db_max_connections,
            store_url,
            tmp_dir,
            auth_mode,
            admin_token_file,
            allowed_hosts,
            allowed_origins,
            limits,
            retention,
            log_level,
            unknown_vars,
        })
    }
}

/// Half the cores, between 1 and 4: jobs are CPU-bound, and the other half
/// serves requests and synchronous diffs.
fn default_parallel_jobs(cpus: usize) -> u32 {
    u32::try_from((cpus / 2).clamp(1, 4)).unwrap_or(1)
}

/// Reads `diffyman.toml`.
fn load_file(vars: &BTreeMap<String, String>, cwd: &Path) -> Result<FileConfig, ConfigError> {
    let path = if let Some(path) = vars.get("DIFFYMAN_CONFIG") {
        cwd.join(path)
    } else {
        let default = cwd.join("diffyman.toml");
        if !default.exists() {
            return Ok(FileConfig::default());
        }
        default
    };
    let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
        what: "the configuration file",
        source,
    })?;
    toml::from_str(&text).map_err(|e| ConfigError::File(e.message().to_owned()))
}

/// Reads `KEY=VALUE` lines. Blank lines and `#` comments are skipped, an
/// `export ` prefix and matching quotes around the value are removed. The
/// file must not be readable by group or others.
pub fn read_env_file(path: &Path) -> Result<Vec<(String, String)>, ConfigError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path).map_err(|source| ConfigError::Read {
            what: ".env.local",
            source,
        })?;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err(invalid(
                ".env.local",
                "must not be readable by group or others (chmod 600)",
            ));
        }
    }
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        what: ".env.local",
        source,
    })?;
    let mut pairs = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return Err(invalid(
                ".env.local",
                format!("line {} is not KEY=VALUE", number + 1),
            ));
        };
        let key = key.trim();
        let valid_key = !key.is_empty()
            && key
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        if !valid_key {
            return Err(invalid(
                ".env.local",
                format!("line {} has an invalid name", number + 1),
            ));
        }
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(value);
        pairs.push((key.to_owned(), value.to_owned()));
    }
    Ok(pairs)
}

fn check_host(host: &str) -> Result<(), ConfigError> {
    let valid = !host.is_empty()
        && host == host.to_ascii_lowercase()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b));
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "DIFFYMAN_ALLOWED_HOSTS",
            "expected lowercase host[:port] values",
        ))
    }
}

fn check_origin(origin: &str) -> Result<(), ConfigError> {
    let parsed = url::Url::parse(origin).ok();
    let valid = parsed.is_some_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some()
            && u.path() == "/"
            && u.query().is_none()
            && u.username().is_empty()
            && !origin.ends_with('/')
    });
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "DIFFYMAN_ALLOWED_ORIGINS",
            "expected scheme://host[:port] values",
        ))
    }
}

/// Applies the variable over the file value over the default.
struct Resolver<'a> {
    vars: &'a BTreeMap<String, String>,
    home: Option<&'a Path>,
}

impl Resolver<'_> {
    fn text(&self, name: &str, file: Option<String>, default: &str) -> String {
        self.vars
            .get(name)
            .cloned()
            .or(file)
            .unwrap_or_else(|| default.to_owned())
    }

    fn list(&self, name: &str, file: Option<Vec<String>>) -> Option<Vec<String>> {
        match self.vars.get(name) {
            Some(text) => Some(
                text.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            None => file,
        }
    }

    fn number(
        &self,
        name: &str,
        file: Option<u64>,
        default: u64,
        range: std::ops::RangeInclusive<u64>,
    ) -> Result<u64, ConfigError> {
        let value = match self.vars.get(name) {
            Some(text) => text
                .parse()
                .map_err(|_| invalid(name, "expected a whole number"))?,
            None => file.unwrap_or(default),
        };
        if range.contains(&value) {
            Ok(value)
        } else {
            Err(invalid(
                name,
                format!("must be between {} and {}", range.start(), range.end()),
            ))
        }
    }

    fn small(
        &self,
        name: &str,
        file: Option<u32>,
        default: u32,
        range: std::ops::RangeInclusive<u32>,
    ) -> Result<u32, ConfigError> {
        let wide = (u64::from(*range.start()))..=(u64::from(*range.end()));
        let value = self.number(name, file.map(u64::from), u64::from(default), wide)?;
        Ok(u32::try_from(value).unwrap_or(u32::MAX))
    }

    fn secs(
        &self,
        name: &str,
        file: Option<u64>,
        default: u64,
        range: std::ops::RangeInclusive<u64>,
    ) -> Result<Duration, ConfigError> {
        self.number(name, file, default, range)
            .map(Duration::from_secs)
    }

    fn days(&self, name: &str, file: Option<u64>) -> Result<Duration, ConfigError> {
        let days = self.number(name, file, 30, 1..=3650)?;
        Ok(Duration::from_secs(days * DAY_SECS))
    }

    fn expand(&self, name: &str, text: &str) -> Result<PathBuf, ConfigError> {
        let path = match text.strip_prefix("~/") {
            Some(rest) => match self.home {
                Some(home) => home.join(rest),
                None => return Err(invalid(name, "`~/` needs a home folder (HOME)")),
            },
            None => PathBuf::from(text),
        };
        if path.is_absolute() {
            Ok(path)
        } else {
            Err(invalid(
                name,
                "expected an absolute path or one starting with ~/",
            ))
        }
    }

    fn path(
        &self,
        name: &str,
        file: Option<String>,
        default: &str,
    ) -> Result<PathBuf, ConfigError> {
        self.expand(name, &self.text(name, file, default))
    }

    fn store_url(&self, file: Option<String>) -> Result<String, ConfigError> {
        const NAME: &str = "DIFFYMAN_STORE_URL";
        let text = self.text(NAME, file, "file://~/diffyman-data/blobs");
        if let Some(rest) = text.strip_prefix("file://") {
            let folder = self.expand(NAME, rest)?;
            let url = url::Url::from_directory_path(&folder)
                .map_err(|()| invalid(NAME, "expected an absolute folder"))?;
            return Ok(url.to_string());
        }
        let parsed = url::Url::parse(&text).map_err(|_| invalid(NAME, "not a URL"))?;
        match parsed.scheme() {
            "memory" | "s3" => Ok(text),
            _ => Err(invalid(NAME, "expected file://, memory:// or s3://")),
        }
    }
}

#[cfg(test)]
mod tests;
