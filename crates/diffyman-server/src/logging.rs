//! Logs with a field allowlist.
//!
//! Only events from this crate are written, one `key=value` line each on
//! standard error, and only the fields in [`FIELDS`] are kept: IDs, sizes,
//! counts, timings, routes and error codes. Anything else on an event, such
//! as a field that could hold file content, a file name, a path or a secret,
//! is dropped before formatting, so a careless log statement cannot leak it.
//! Events of other crates (the HTTP stack, the database driver, the object
//! store) are dropped too, because their messages may carry paths or SQL.

use std::fmt::{self, Write as _};
use std::io::Write;
use std::sync::Mutex;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::config::LogLevel;

/// The only fields a log line may carry.
pub const FIELDS: &[&str] = &[
    "message",
    "request_id",
    "method",
    "route",
    "status",
    "code",
    "duration_ms",
    "bytes_in",
    "bytes_out",
    "size",
    "key_id",
    "job_id",
    "diff_id",
    "blob_id",
    "attempt",
    "outcome",
    "count",
    "requeued",
    "failed",
    "cancelled",
    "diffs",
    "jobs",
    "blobs",
    "orphans",
    "staging",
    "worker",
    "addr",
    "env",
    "store",
    "parallel_jobs",
];

/// Writes allow-listed fields of this crate's events.
pub struct AllowlistLayer<W> {
    max: Level,
    out: Mutex<W>,
}

impl<W> AllowlistLayer<W> {
    /// A layer writing events up to `level` to `out`.
    pub fn new(level: LogLevel, out: W) -> AllowlistLayer<W> {
        let max = match level {
            LogLevel::Error => Level::ERROR,
            LogLevel::Warn => Level::WARN,
            LogLevel::Info => Level::INFO,
            LogLevel::Debug => Level::DEBUG,
        };
        AllowlistLayer {
            max,
            out: Mutex::new(out),
        }
    }
}

fn own_target(target: &str) -> bool {
    target == "diffyman_server" || target.starts_with("diffyman_server::")
}

impl<S: Subscriber, W: Write + Send + 'static> Layer<S> for AllowlistLayer<W> {
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        own_target(metadata.target()) && *metadata.level() <= self.max
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        if !own_target(metadata.target()) || *metadata.level() > self.max {
            return;
        }
        let mut line = Line::default();
        event.record(&mut line);
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let text = format!(
            "{now} {:<5} {}{}\n",
            metadata.level(),
            line.message,
            line.fields
        );
        if let Ok(mut out) = self.out.lock() {
            let _ = out.write_all(text.as_bytes());
            let _ = out.flush();
        }
    }
}

/// The allow-listed fields of one event.
#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl Line {
    fn push(&mut self, field: &Field, value: &str) {
        let name = field.name();
        if !FIELDS.contains(&name) {
            return;
        }
        if name == "message" {
            self.message = clean(value);
            return;
        }
        let quoted = value.is_empty()
            || value
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '=');
        if quoted {
            let _ = write!(self.fields, " {name}={value:?}");
        } else {
            let _ = write!(self.fields, " {name}={value}");
        }
    }
}

/// The message on one line.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field, value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field, &value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field, &value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field, if value { "true" } else { "false" });
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field, &value.to_string());
    }

    fn record_error(&mut self, _field: &Field, _value: &(dyn std::error::Error + 'static)) {
        // Error chains may name paths; log the stable code instead.
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if FIELDS.contains(&field.name()) {
            self.push(field, &format!("{value:?}"));
        }
    }
}

/// Installs the allowlist layer on standard error as the global subscriber.
/// Later calls do nothing.
pub fn init(level: LogLevel) {
    use tracing_subscriber::prelude::*;
    let subscriber =
        tracing_subscriber::registry().with(AllowlistLayer::new(level, std::io::stderr()));
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tracing_subscriber::prelude::*;

    use super::*;

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn only_allowed_fields_of_own_events_are_written() {
        let sink = Sink::default();
        let subscriber =
            tracing_subscriber::registry().with(AllowlistLayer::new(LogLevel::Info, sink.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(
                route = "/v1/diff",
                status = 200u16,
                duration_ms = 3u64,
                path = "/home/someone/secret.txt",
                content = "top secret line",
                name = "secret.txt",
                "request"
            );
            tracing::debug!(job_id = "x", "too verbose");
            tracing::info!(target: "sqlx::query", statement = "SELECT 1", "query");
            tracing::warn!(code = "storage-unavailable", error = %"/private/blobs", "store failed");
        });
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("INFO  request route=/v1/diff status=200 duration_ms=3"));
        assert!(lines[1].contains("WARN  store failed code=storage-unavailable"));
        for secret in ["secret", "/home", "SELECT", "private", "too verbose"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }

    #[test]
    fn values_with_spaces_are_quoted() {
        let mut line = Line::default();
        let fields = tracing::field::FieldSet::new(
            &["message", "route", "code"],
            tracing::callsite::Identifier(&CALLSITE),
        );
        let route = fields.field("route").unwrap();
        let message = fields.field("message").unwrap();
        line.push(&route, "a b");
        line.push(&message, "two\nlines");
        assert_eq!(line.fields, " route=\"a b\"");
        assert_eq!(line.message, "two lines");
    }

    struct Callsite;
    static CALLSITE: Callsite = Callsite;
    impl tracing::callsite::Callsite for Callsite {
        fn set_interest(&self, _: tracing::subscriber::Interest) {}
        fn metadata(&self) -> &Metadata<'_> {
            unreachable!()
        }
    }
}
