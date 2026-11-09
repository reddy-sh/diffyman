use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::*;

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn cwd(&self) -> &Path {
        self.dir.path()
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn write_private(&self, name: &str, text: &str) {
        let path = self.cwd().join(name);
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    fn load(&self, vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: BTreeMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Config::load(Sources {
            vars,
            cwd: self.cwd().to_owned(),
            home: Some(self.home()),
            cpus: 6,
        })
    }
}

const PW: (&str, &str) = ("DIFFYMAN_PG_PASSWORD", "pw-123");

#[test]
fn local_defaults() {
    let fx = Fixture::new();
    let config = fx.load(&[PW]).unwrap();
    assert_eq!(config.env, Environment::Local);
    assert_eq!(config.bind, "127.0.0.1:8080".parse().unwrap());
    assert_eq!(
        config.db_url.expose(),
        "postgres://diffyman:pw-123@127.0.0.1:5433/diffyman"
    );
    let blobs = url::Url::from_directory_path(fx.home().join("diffyman-data/blobs")).unwrap();
    assert_eq!(config.store_url, blobs.to_string());
    assert_eq!(config.tmp_dir, fx.home().join("diffyman-data/tmp"));
    assert_eq!(
        config.admin_token_file,
        fx.home().join(".diffyman/admin-token")
    );
    assert_eq!(config.auth_mode, AuthMode::ApiKey);
    assert_eq!(
        config.allowed_hosts,
        ["127.0.0.1:8080", "localhost:8080", "[::1]:8080"]
    );
    assert!(config.allowed_origins.is_empty());
    assert_eq!(config.limits.max_body_bytes, 24 << 20);
    assert_eq!(config.limits.max_sync_input_bytes, 8 << 20);
    assert_eq!(config.limits.max_blob_bytes, 1 << 30);
    assert_eq!(config.limits.max_parallel_jobs, 3, "half of 6 cores");
    assert_eq!(config.limits.max_parallel_diffs, 6, "one per core");
    assert_eq!(config.limits.sync_memory_bytes, 1 << 30);
    assert_eq!(config.limits.max_job_input_bytes(), 512 << 20);
    assert_eq!(config.limits.rate_limit_per_minute, 600);
    assert_eq!(config.log_level, LogLevel::Info);
    assert_eq!(config.limits.job_lease, Duration::from_secs(60));
    assert_eq!(config.limits.header_read_timeout, Duration::from_secs(10));
    assert_eq!(config.limits.idle_timeout, Duration::from_secs(60));
    assert_eq!(config.retention.diff_ttl, Duration::from_hours(30 * 24));
    assert_eq!(config.retention.blob_ttl, config.retention.diff_ttl);
    assert_eq!(config.db_max_connections, 10);
    assert!(config.unknown_vars.is_empty());
}

#[test]
fn parallel_jobs_default_to_half_the_cores_between_one_and_four() {
    for (cpus, jobs) in [(0, 1), (1, 1), (2, 1), (4, 2), (8, 4), (64, 4)] {
        assert_eq!(default_parallel_jobs(cpus), jobs, "{cpus} cores");
    }
}

#[test]
fn log_level_and_rate_limit() {
    let fx = Fixture::new();
    let config = fx
        .load(&[
            PW,
            ("DIFFYMAN_LOG", "debug"),
            ("DIFFYMAN_RATE_LIMIT_PER_MINUTE", "0"),
            ("DIFFYMAN_MAX_SYNC_INPUT_BYTES", "1024"),
        ])
        .unwrap();
    assert_eq!(config.log_level, LogLevel::Debug);
    assert_eq!(config.limits.rate_limit_per_minute, 0);
    assert_eq!(config.limits.max_sync_input_bytes, 1024);
    let err = fx.load(&[PW, ("DIFFYMAN_LOG", "trace")]).unwrap_err();
    assert!(err.to_string().starts_with("DIFFYMAN_LOG"), "{err}");
}

#[test]
fn secrets_never_show_in_debug_output() {
    let fx = Fixture::new();
    let config = fx.load(&[PW]).unwrap();
    let shown = format!("{config:?}");
    assert!(!shown.contains("pw-123"), "{shown}");
    assert!(shown.contains("(redacted)"));
}

#[test]
fn env_local_fills_unset_variables_only() {
    let fx = Fixture::new();
    fx.write_private(
        ".env.local",
        "# local secrets\nDIFFYMAN_PG_PASSWORD=from-file\nexport DIFFYMAN_BIND='127.0.0.1:9999'\n\nDIFFYMAN_S3_TEST_KEY=\"k\"\n",
    );
    let config = fx.load(&[]).unwrap();
    assert!(config.db_url.expose().contains(":from-file@"));
    assert_eq!(config.bind.port(), 9999);
    assert!(
        config.unknown_vars.is_empty(),
        "tooling variables are known"
    );
    let config = fx.load(&[("DIFFYMAN_PG_PASSWORD", "from-env")]).unwrap();
    assert!(config.db_url.expose().contains(":from-env@"));
}

#[cfg(unix)]
#[test]
fn env_local_must_be_private() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fixture::new();
    fx.write_private(".env.local", "DIFFYMAN_PG_PASSWORD=x\n");
    let path = fx.cwd().join(".env.local");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let err = fx.load(&[]).unwrap_err();
    assert!(err.to_string().contains("chmod 600"), "{err}");
}

#[test]
fn env_local_syntax_errors() {
    let fx = Fixture::new();
    fx.write_private(".env.local", "JUST TEXT\n");
    assert!(fx.load(&[]).unwrap_err().to_string().contains("line 1"));
    fx.write_private(".env.local", "lower=x\n");
    assert!(
        fx.load(&[])
            .unwrap_err()
            .to_string()
            .contains("invalid name")
    );
}

#[test]
fn prod_ignores_env_local_and_needs_explicit_settings() {
    let fx = Fixture::new();
    fx.write_private(".env.local", "DIFFYMAN_PG_PASSWORD=x\n");
    let prod = ("DIFFYMAN_ENV", "prod");
    let err = fx.load(&[prod]).unwrap_err();
    assert_eq!(err.to_string(), "DIFFYMAN_DB_URL must be set");
    let url = (
        "DIFFYMAN_DB_URL",
        "postgres://u:p@db:5432/diffyman?sslmode=verify-full",
    );
    let err = fx.load(&[prod, url]).unwrap_err();
    assert_eq!(err.to_string(), "DIFFYMAN_ALLOWED_HOSTS must be set");
    let hosts = (
        "DIFFYMAN_ALLOWED_HOSTS",
        "diff.example.com, diff.example.com:443",
    );
    let config = fx
        .load(&[prod, url, hosts, ("DIFFYMAN_BIND", "0.0.0.0:8080")])
        .unwrap();
    assert_eq!(config.env, Environment::Prod);
    assert_eq!(
        config.allowed_hosts,
        ["diff.example.com", "diff.example.com:443"]
    );
    assert!(!config.bind.ip().is_loopback());
    let err = fx
        .load(&[prod, url, hosts, ("DIFFYMAN_AUTH_MODE", "none")])
        .unwrap_err();
    assert!(err.to_string().contains("local environment only"));
}

#[test]
fn local_refuses_non_loopback_bind() {
    let fx = Fixture::new();
    for bind in ["0.0.0.0:8080", "192.168.1.10:8080", "[::]:8080"] {
        let err = fx.load(&[PW, ("DIFFYMAN_BIND", bind)]).unwrap_err();
        assert!(err.to_string().contains("loopback"), "{bind}: {err}");
    }
    let config = fx.load(&[PW, ("DIFFYMAN_BIND", "[::1]:8181")]).unwrap();
    assert_eq!(config.allowed_hosts[2], "[::1]:8181");
}

#[test]
fn file_layer_is_overridden_by_variables() {
    let fx = Fixture::new();
    std::fs::write(
        fx.cwd().join("diffyman.toml"),
        "bind = \"127.0.0.1:7000\"\nallowed_origins = [\"http://localhost:5173\"]\n\
         [limits]\nmax_parallel_jobs = 4\njob_lease_secs = 30\nmax_parallel_diffs = 3\n\
         sync_memory_bytes = 1048576\nidle_timeout_secs = 90\n[retention]\ndiff_ttl_days = 7\n",
    )
    .unwrap();
    let config = fx
        .load(&[
            PW,
            ("DIFFYMAN_MAX_PARALLEL_JOBS", "8"),
            ("DIFFYMAN_MAX_PARALLEL_DIFFS", "2"),
            ("DIFFYMAN_HEADER_READ_TIMEOUT_SECS", "3"),
        ])
        .unwrap();
    assert_eq!(config.limits.header_read_timeout, Duration::from_secs(3));
    assert_eq!(config.limits.idle_timeout, Duration::from_secs(90));
    assert_eq!(config.bind.port(), 7000);
    assert_eq!(config.limits.max_parallel_jobs, 8);
    assert_eq!(config.limits.max_parallel_diffs, 2);
    assert_eq!(config.limits.sync_memory_bytes, 1 << 20);
    for (name, value) in [
        ("DIFFYMAN_MAX_PARALLEL_DIFFS", "0"),
        ("DIFFYMAN_SYNC_MEMORY_BYTES", "1024"),
    ] {
        let err = fx.load(&[PW, (name, value)]).unwrap_err();
        assert!(err.to_string().starts_with(name), "{err}");
    }
    assert_eq!(config.limits.job_lease, Duration::from_secs(30));
    assert_eq!(config.retention.diff_ttl, Duration::from_hours(7 * 24));
    assert_eq!(config.allowed_origins, ["http://localhost:5173"]);

    std::fs::write(fx.cwd().join("other.toml"), "typo_key = 1\n").unwrap();
    let err = fx
        .load(&[PW, ("DIFFYMAN_CONFIG", "other.toml")])
        .unwrap_err();
    assert!(err.to_string().contains("typo_key"), "{err}");
    let err = fx
        .load(&[PW, ("DIFFYMAN_CONFIG", "missing.toml")])
        .unwrap_err();
    assert!(
        err.to_string()
            .starts_with("cannot read the configuration file")
    );
}

#[test]
fn invalid_values_name_the_variable() {
    let fx = Fixture::new();
    for (name, value) in [
        ("DIFFYMAN_ENV", "staging"),
        ("DIFFYMAN_BIND", "localhost"),
        ("DIFFYMAN_DB_URL", "mysql://x"),
        ("DIFFYMAN_AUTH_MODE", "oidc"),
        ("DIFFYMAN_STORE_URL", "ftp://x/"),
        ("DIFFYMAN_STORE_URL", "file://relative/blobs"),
        ("DIFFYMAN_TMP_DIR", "relative/tmp"),
        ("DIFFYMAN_MAX_BODY_BYTES", "lots"),
        ("DIFFYMAN_MAX_BODY_BYTES", "0"),
        ("DIFFYMAN_JOB_MAX_ATTEMPTS", "101"),
        ("DIFFYMAN_JOB_LEASE_SECS", "1"),
        ("DIFFYMAN_HEADER_READ_TIMEOUT_SECS", "0"),
        ("DIFFYMAN_IDLE_TIMEOUT_SECS", "3601"),
        ("DIFFYMAN_DIFF_TTL_DAYS", "0"),
        ("DIFFYMAN_ALLOWED_HOSTS", "Example.COM"),
        ("DIFFYMAN_ALLOWED_ORIGINS", "http://localhost:5173/"),
        ("DIFFYMAN_ALLOWED_ORIGINS", "localhost:5173"),
    ] {
        let err = fx.load(&[PW, (name, value)]).unwrap_err();
        assert!(err.to_string().starts_with(name), "{name}={value}: {err}");
    }
    let err = fx.load(&[]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "DIFFYMAN_DB_URL or DIFFYMAN_PG_PASSWORD must be set"
    );
}

#[test]
fn store_urls_and_auth_none() {
    let fx = Fixture::new();
    let config = fx.load(&[PW, ("DIFFYMAN_STORE_URL", "memory://")]).unwrap();
    assert_eq!(config.store_url, "memory://");
    let config = fx
        .load(&[PW, ("DIFFYMAN_STORE_URL", "s3://bucket/prefix")])
        .unwrap();
    assert_eq!(config.store_url, "s3://bucket/prefix");
    let config = fx.load(&[PW, ("DIFFYMAN_AUTH_MODE", "none")]).unwrap();
    assert_eq!(config.auth_mode, AuthMode::None);
}

#[test]
fn unknown_variables_are_reported() {
    let fx = Fixture::new();
    let config = fx
        .load(&[
            PW,
            ("DIFFYMAN_MAX_PARALEL_JOBS", "3"),
            ("DIFFYMAN_TEST_DB_URL", "x"),
            ("OTHER", "y"),
        ])
        .unwrap();
    assert_eq!(config.unknown_vars, ["DIFFYMAN_MAX_PARALEL_JOBS"]);
}
