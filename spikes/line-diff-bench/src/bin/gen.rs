//! Deterministic corpus generator for Spike A.
//!
//! Usage: gen <out-dir> [case ...]
//! Cases: log-1g log-100m source-50m identical histo-lowfreq swapped-unique huge-line all-different
//! With no case names every case is generated. Output is byte-for-byte reproducible.

use line_diff_bench::Rng;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const ALL: &[&str] = &[
    "log-1g",
    "log-100m",
    "source-50m",
    "identical",
    "histo-lowfreq",
    "swapped-unique",
    "huge-line",
    "all-different",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: gen <out-dir> [case ...]; cases: {}", ALL.join(" "));
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[0]);
    std::fs::create_dir_all(&dir).expect("create out dir");
    let cases: Vec<String> = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        ALL.iter().map(|s| s.to_string()).collect()
    };
    for case in cases {
        let t = std::time::Instant::now();
        let (l, r) = (dir.join(format!("{case}.left")), dir.join(format!("{case}.right")));
        let stats = match case.as_str() {
            "log-1g" => gen_log(&l, &r, 1_000_000_000, 0xA11CE),
            "log-100m" => gen_log(&l, &r, 100_000_000, 0xB0B),
            "source-50m" => gen_source(&l, &r, 50_000_000, 0xC0DE),
            "identical" => gen_identical(&l, &r, 5_000_000, 0x1D),
            "histo-lowfreq" => gen_swapped(&l, &r, 200_000, 16, 0x415),
            "swapped-unique" => gen_swapped(&l, &r, 200_000, 1, 0x5A9),
            "huge-line" => gen_huge_line(&l, &r, 50_000_000, 0x6E),
            "all-different" => gen_all_different(&l, &r, 1_000_000, 0xD1F),
            other => panic!("unknown case {other}"),
        };
        println!("{case}: {stats} ({:.1}s)", t.elapsed().as_secs_f64());
    }
}

fn writer(p: &Path) -> BufWriter<File> {
    BufWriter::with_capacity(1 << 20, File::create(p).expect("create file"))
}

/// Changes one ASCII digit (or, failing that, appends a token): a minimal realistic edit.
fn tweak(line: &str, rng: &mut Rng) -> String {
    let digits: Vec<usize> = line
        .bytes()
        .enumerate()
        .filter(|(_, b)| b.is_ascii_digit())
        .map(|(i, _)| i)
        .collect();
    let body_end = line.strip_suffix('\n').map_or(line.len(), |s| s.len());
    if digits.is_empty() {
        let mut s = line[..body_end].to_string();
        s.push_str(" retry=1\n");
        return s;
    }
    // Prefer digits after the timestamp so edits look like changed values.
    let start = digits.len() / 2;
    let i = digits[start + rng.below((digits.len() - start) as u64) as usize];
    let old = line.as_bytes()[i] - b'0';
    let new = (old + 1 + rng.below(9) as u8) % 10;
    let mut bytes = line.as_bytes().to_vec();
    bytes[i] = b'0' + new;
    String::from_utf8(bytes).unwrap()
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

const SERVICES: &[&str] = &["api", "auth", "orders", "billing", "search", "cache", "worker", "gateway"];
const PATHS: &[&str] = &["/api/v1/items", "/api/v1/orders", "/api/v1/users", "/api/v2/search", "/api/v1/cart", "/api/v1/payments"];
const TABLES: &[&str] = &["orders", "users", "items", "payments", "sessions", "audit_log"];
const FRAMES: &[&str] = &[
    "    at com.example.orders.OrderService.place(OrderService.java:214)",
    "    at com.example.orders.OrderService.validate(OrderService.java:131)",
    "    at com.example.orders.OrderController.create(OrderController.java:88)",
    "    at com.example.billing.InvoiceService.charge(InvoiceService.java:402)",
    "    at com.example.billing.PaymentGateway.authorize(PaymentGateway.java:77)",
    "    at com.example.common.db.Repository.findById(Repository.java:59)",
    "    at com.example.common.db.Repository.query(Repository.java:120)",
    "    at com.example.common.http.Handler.dispatch(Handler.java:33)",
    "    at com.example.common.http.Router.route(Router.java:145)",
    "    at org.eclipse.jetty.server.HttpChannel.handle(HttpChannel.java:516)",
    "    at org.eclipse.jetty.server.Server.handle(Server.java:563)",
    "    at org.eclipse.jetty.util.thread.QueuedThreadPool.runJob(QueuedThreadPool.java:883)",
    "    at java.base/java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1144)",
    "    at java.base/java.lang.Thread.run(Thread.java:1583)",
    "Caused by: java.sql.SQLTransientConnectionException: pool timeout after 30000ms",
    "    at com.zaxxer.hikari.pool.HikariPool.getConnection(HikariPool.java:181)",
    "    ... 12 more",
];

struct LogGen {
    rng: Rng,
    ts_us: u64,
    frames_left: u32,
    frame_pos: usize,
}

/// One log line: optional timestamp (stack frames have none) and the rest of the line.
struct Entry {
    ts: Option<u64>,
    body: String,
}

impl Entry {
    fn render(&self, drift_us: u64, out: &mut String) {
        out.clear();
        if let Some(ts) = self.ts {
            fmt_ts(ts + drift_us, out);
            out.push(' ');
        }
        out.push_str(&self.body);
        out.push('\n');
    }
}

fn fmt_ts(us: u64, out: &mut String) {
    // Base: 2026-11-07T06:00:00Z.
    let total = us + 6 * 3_600_000_000;
    let day = total / 86_400_000_000;
    let rem = total % 86_400_000_000;
    let (h, m, s, frac) = (
        rem / 3_600_000_000,
        rem / 60_000_000 % 60,
        rem / 1_000_000 % 60,
        rem % 1_000_000,
    );
    let _ = write!(out, "2026-11-{:02}T{h:02}:{m:02}:{s:02}.{frac:06}Z", 7 + day);
}

impl LogGen {
    fn new(seed: u64) -> Self {
        LogGen { rng: Rng::new(seed), ts_us: 0, frames_left: 0, frame_pos: 0 }
    }

    fn next(&mut self) -> Entry {
        let r = &mut self.rng;
        if self.frames_left > 0 {
            self.frames_left -= 1;
            let f = FRAMES[self.frame_pos % FRAMES.len()];
            self.frame_pos += 1;
            return Entry { ts: None, body: f.to_string() };
        }
        self.ts_us += r.range(10, 1500);
        let svc = *r.pick(SERVICES);
        let req = r.next_u64();
        let roll = r.below(1000);
        let mut body = String::with_capacity(120);
        let level = match roll {
            0..=799 => "INFO ",
            800..=919 => "DEBUG",
            920..=979 => "WARN ",
            _ => "ERROR",
        };
        let _ = write!(body, "{level} [{svc}] req={req:016x} ");
        match level {
            "INFO " => match r.below(5) {
                0 | 1 => {
                    let method = if r.chance(0.75) { "GET" } else { *r.pick(&["POST", "PUT", "DELETE"]) };
                    let status = *r.pick(&[200, 200, 200, 200, 201, 204, 304, 404]);
                    let _ = write!(
                        body,
                        "{method} {}/{} status={status} bytes={} latency_ms={}.{}",
                        r.pick(PATHS),
                        r.below(100_000),
                        r.range(80, 64_000),
                        r.below(250),
                        r.below(10)
                    );
                }
                2 => {
                    let _ = write!(body, "cache {} key=user:{}", if r.chance(0.8) { "hit" } else { "miss" }, r.below(50_000));
                }
                3 => {
                    let _ = write!(body, "db query table={} rows={} took_ms={}", r.pick(TABLES), r.below(500), r.below(80));
                }
                _ => {
                    let _ = write!(body, "user {} logged in from 10.{}.{}.{}", r.below(1_000_000), r.below(256), r.below(256), r.below(256));
                }
            },
            "DEBUG" => match r.below(3) {
                0 => body.push_str("heartbeat ok"),
                1 => {
                    let _ = write!(body, "pool stats active={} idle={} waiting=0", r.below(32), r.below(32));
                }
                _ => body.push_str("config reload skipped: unchanged"),
            },
            "WARN " => {
                if r.chance(0.5) {
                    let _ = write!(body, "slow request GET {} latency_ms={}", r.pick(PATHS), r.range(1000, 9000));
                } else {
                    let _ = write!(body, "retrying upstream call attempt={} backoff_ms={}", r.range(1, 5), 100 << r.below(5));
                }
            }
            _ => {
                let _ = write!(body, "unhandled exception: java.lang.IllegalStateException: order {} not found", r.below(10_000_000));
                self.frames_left = r.range(4, 12) as u32;
                self.frame_pos = r.below(FRAMES.len() as u64) as usize;
            }
        }
        Entry { ts: Some(self.ts_us), body }
    }
}

#[derive(Default)]
struct LogStats {
    left_lines: u64,
    right_lines: u64,
    changed: u64,
    deleted: u64,
    del_blocks: u64,
    inserted: u64,
    ins_blocks: u64,
    moved: u64,
    moves: u64,
    drifted: u64,
    drifts: u64,
}

fn gen_log(lp: &Path, rp: &Path, target: u64, seed: u64) -> String {
    let mut left = writer(lp);
    let mut right = writer(rp);
    let mut gen = LogGen::new(seed);
    let mut extra = LogGen::new(seed ^ 0xFFFF);
    let mut m = Rng::new(seed ^ 0x5EED);
    let mut st = LogStats::default();
    let (mut skip, mut capture, mut drift_left, mut drift_us) = (0u64, 0u64, 0u64, 0u64);
    let mut capture_buf: Vec<String> = Vec::new();
    let mut pending: VecDeque<(u64, Vec<String>)> = VecDeque::new();
    let mut written = 0u64;
    let mut line = String::new();
    let mut rline = String::new();
    let mut i = 0u64;
    while written < target {
        let e = gen.next();
        e.render(0, &mut line);
        left.write_all(line.as_bytes()).unwrap();
        written += line.len() as u64;
        st.left_lines += 1;
        i += 1;

        while pending.front().is_some_and(|(at, _)| *at <= i) {
            let (_, block) = pending.pop_front().unwrap();
            for l in block {
                right.write_all(l.as_bytes()).unwrap();
                st.right_lines += 1;
            }
        }

        if drift_left > 0 {
            drift_left -= 1;
        } else if m.chance(0.000_001_5) {
            drift_left = m.range(500, 5000);
            drift_us = m.range(1, 2000) * 1000;
            st.drifts += 1;
        }
        let drift = if drift_left > 0 { drift_us } else { 0 };
        if drift > 0 && e.ts.is_some() {
            st.drifted += 1;
        }

        if skip > 0 {
            skip -= 1;
            st.deleted += 1;
            continue;
        }
        e.render(drift, &mut rline);
        if capture > 0 {
            capture -= 1;
            capture_buf.push(rline.clone());
            if capture == 0 {
                let at = i + m.range(1_000, 100_000);
                pending.push_back((at, std::mem::take(&mut capture_buf)));
                pending.make_contiguous().sort_by_key(|(at, _)| *at);
            }
            continue;
        }
        if m.chance(0.000_01) {
            skip = m.range(5, 200);
            st.del_blocks += 1;
            st.deleted += 1;
            skip -= 1;
            continue;
        }
        if m.chance(0.000_001) {
            capture = m.range(20, 300);
            st.moves += 1;
            st.moved += capture;
            capture -= 1;
            capture_buf.push(rline.clone());
            continue;
        }
        if m.chance(0.000_01) {
            let k = m.range(5, 200);
            st.ins_blocks += 1;
            extra.ts_us = gen.ts_us;
            for _ in 0..k {
                let x = extra.next();
                let mut s = String::new();
                x.render(drift, &mut s);
                right.write_all(s.as_bytes()).unwrap();
                st.inserted += 1;
                st.right_lines += 1;
            }
        }
        if m.chance(0.005) {
            rline = tweak(&rline, &mut m);
            st.changed += 1;
        }
        right.write_all(rline.as_bytes()).unwrap();
        st.right_lines += 1;
    }
    for (_, block) in pending.drain(..) {
        for l in block {
            right.write_all(l.as_bytes()).unwrap();
            st.right_lines += 1;
        }
    }
    for l in capture_buf.drain(..) {
        right.write_all(l.as_bytes()).unwrap();
        st.right_lines += 1;
    }
    left.flush().unwrap();
    right.flush().unwrap();
    format!(
        "left_lines={} right_lines={} changed={} deleted={} in {} blocks, inserted={} in {} blocks, moved={} in {} blocks, drifted={} in {} regions",
        st.left_lines, st.right_lines, st.changed, st.deleted, st.del_blocks, st.inserted, st.ins_blocks, st.moved, st.moves, st.drifted, st.drifts
    )
}

// ---------------------------------------------------------------------------
// "Large source tree": concatenated generated Rust-like files.
// ---------------------------------------------------------------------------

const MODS: &[&str] = &["core", "net", "store", "auth", "billing", "search", "ui", "util", "proto", "sched"];
const NOUNS: &[&str] = &[
    "order", "user", "item", "cart", "price", "token", "session", "index", "buffer", "cursor", "range", "entry",
    "node", "edge", "graph", "queue", "batch", "frame", "chunk", "block", "record", "field", "value", "limit",
    "offset", "count", "total", "state", "config", "handle", "client", "server", "request", "response", "header",
    "payload", "digest", "key", "slot", "page",
];
const VERBS: &[&str] = &[
    "load", "store", "parse", "render", "update", "insert", "remove", "find", "merge", "split", "flush", "apply",
    "build", "check", "encode", "decode", "resolve", "compute", "fetch", "push", "pop", "scan", "sort", "hash",
];
const TYPES: &[&str] = &["u32", "u64", "usize", "i64", "bool", "String", "Vec<u8>", "Option<u32>", "&str", "f64"];
const USES: &[&str] = &[
    "use std::collections::HashMap;",
    "use std::collections::BTreeMap;",
    "use std::sync::Arc;",
    "use std::io::{self, Read, Write};",
    "use std::fmt;",
    "use crate::error::Error;",
    "use crate::util::hash::Hasher;",
    "use serde::{Deserialize, Serialize};",
    "use tracing::{debug, info, warn};",
];

fn camel(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
}

fn gen_fn(r: &mut Rng, ty: &str) -> Vec<String> {
    let v = *r.pick(VERBS);
    let n = *r.pick(NOUNS);
    let n2 = *r.pick(NOUNS);
    let t = *r.pick(TYPES);
    let mut f = vec![format!("    pub fn {v}_{n}(&mut self, {n2}: {t}) -> Result<{t}, Error> {{")];
    for _ in 0..r.range(2, 12) {
        let a = *r.pick(NOUNS);
        let b = *r.pick(NOUNS);
        match r.below(8) {
            0 => f.push(format!("        let {a} = self.{b}.{}({n2})?;", r.pick(VERBS))),
            1 => {
                f.push(format!("        if {a}.len() > {} {{", r.below(1024)));
                f.push(format!("            return Err(Error::{}{});", camel(a), camel(r.pick(&["overflow", "missing", "invalid", "busy"]))));
                f.push("        }".to_string());
            }
            2 => {
                f.push(format!("        for {a} in self.{b}s.iter() {{"));
                f.push(format!("            {}(&{a});", r.pick(VERBS)));
                f.push("        }".to_string());
            }
            3 => f.push(format!("        debug!(\"{v} {n}: {{}}\", {a});")),
            4 => f.push(format!("        self.{a}_{b} += {};", r.below(64))),
            5 => {
                f.push(format!("        match self.{a} {{"));
                f.push(format!("            Some(x) => x.{}(),", r.pick(VERBS)));
                f.push("            None => {}".to_string());
                f.push("        }".to_string());
            }
            6 => f.push(String::new()),
            _ => f.push(format!("        let {a}: {} = {ty}::{}_{b}(&self.{a});", r.pick(TYPES), r.pick(VERBS))),
        }
    }
    f.push(format!("        Ok({n2})"));
    f.push("    }".to_string());
    f.push(String::new());
    f
}

/// A file as chunks: [header, struct, impl-open, fn..., impl-close].
fn gen_file(r: &mut Rng, idx: u64) -> Vec<Vec<String>> {
    let m = *r.pick(MODS);
    let ty = format!("{}{}", camel(r.pick(NOUNS)), camel(r.pick(NOUNS)));
    let mut header = vec![
        format!("// ===== file: src/{m}/{}_{idx}.rs =====", r.pick(NOUNS)),
        "// SPDX-License-Identifier: MIT OR Apache-2.0".to_string(),
        String::new(),
    ];
    for _ in 0..r.range(2, 6) {
        header.push(r.pick(USES).to_string());
    }
    header.push(String::new());
    let mut st = vec!["#[derive(Debug, Clone, Default)]".to_string(), format!("pub struct {ty} {{")];
    for _ in 0..r.range(2, 8) {
        st.push(format!("    {}: {},", r.pick(NOUNS), r.pick(TYPES)));
    }
    st.push("}".to_string());
    st.push(String::new());
    let mut chunks = vec![header, st, vec![format!("impl {ty} {{")]];
    for _ in 0..r.range(3, 15) {
        chunks.push(gen_fn(r, &ty));
    }
    chunks.push(vec!["}".to_string(), String::new()]);
    chunks
}

fn write_lines(w: &mut BufWriter<File>, lines: &[String]) -> u64 {
    let mut n = 0;
    for l in lines {
        w.write_all(l.as_bytes()).unwrap();
        w.write_all(b"\n").unwrap();
        n += l.len() as u64 + 1;
    }
    n
}

fn gen_source(lp: &Path, rp: &Path, target: u64, seed: u64) -> String {
    let mut left = writer(lp);
    let mut right = writer(rp);
    let mut r = Rng::new(seed);
    let mut m = Rng::new(seed ^ 0x5EED);
    let mut written = 0u64;
    let mut idx = 0u64;
    let mut pending: VecDeque<(u64, Vec<String>)> = VecDeque::new();
    let (mut files, mut edited, mut fn_ins, mut fn_del, mut renamed, mut moved, mut f_del, mut f_ins) = (0, 0, 0, 0, 0, 0, 0, 0);
    while written < target {
        idx += 1;
        files += 1;
        let mut chunks = gen_file(&mut r, idx);
        let flat_left: Vec<String> = chunks.iter().flatten().cloned().collect();
        written += write_lines(&mut left, &flat_left);

        while pending.front().is_some_and(|(at, _)| *at <= idx) {
            let (_, f) = pending.pop_front().unwrap();
            write_lines(&mut right, &f);
        }
        if m.chance(0.003) {
            f_del += 1;
            continue;
        }
        if m.chance(0.003) {
            f_ins += 1;
            let extra = gen_file(&mut m, 1_000_000 + idx);
            write_lines(&mut right, &extra.concat());
        }
        let nfn = chunks.len() - 4;
        if m.chance(0.03) {
            let at = 3 + m.below(nfn as u64 + 1) as usize;
            let ty = chunks[2][0].trim_start_matches("impl ").trim_end_matches(" {").to_string();
            chunks.insert(at, gen_fn(&mut m, &ty));
            fn_ins += 1;
        }
        if m.chance(0.03) && nfn > 1 {
            chunks.remove(3 + m.below(nfn as u64) as usize);
            fn_del += 1;
        }
        let mut flat: Vec<String> = chunks.concat();
        if m.chance(0.08) {
            edited += 1;
            for _ in 0..m.range(1, 3) {
                let k = m.below(flat.len() as u64) as usize;
                flat[k] = tweak(&flat[k], &mut m);
            }
        }
        if m.chance(0.01) {
            renamed += 1;
            let from = *m.pick(NOUNS);
            let to = format!("{from}_v2");
            for l in flat.iter_mut() {
                if l.contains(from) {
                    *l = l.replace(from, &to);
                }
            }
        }
        if m.chance(0.005) {
            moved += 1;
            let at = idx + m.range(20, 200);
            pending.push_back((at, flat));
            pending.make_contiguous().sort_by_key(|(at, _)| *at);
            continue;
        }
        write_lines(&mut right, &flat);
    }
    for (_, f) in pending.drain(..) {
        write_lines(&mut right, &f);
    }
    left.flush().unwrap();
    right.flush().unwrap();
    format!("files={files} edited={edited} fn_inserted={fn_ins} fn_deleted={fn_del} renamed={renamed} moved={moved} files_deleted={f_del} files_inserted={f_ins}")
}

// ---------------------------------------------------------------------------
// Pathological inputs
// ---------------------------------------------------------------------------

/// Millions of identical lines; the right side has sparse unique replacements and
/// extra copies of the repeated line (ambiguous "slider" positions).
fn gen_identical(lp: &Path, rp: &Path, n: u64, seed: u64) -> String {
    let mut left = writer(lp);
    let mut right = writer(rp);
    let mut r = Rng::new(seed);
    let line = b"INFO heartbeat ok\n";
    let (mut repl, mut ins) = (0, 0);
    for i in 0..n {
        left.write_all(line).unwrap();
        if r.chance(0.000_2) {
            repl += 1;
            writeln!(right, "WARN heartbeat late seq={i}").unwrap();
            continue;
        }
        if r.chance(0.000_2) {
            ins += 1;
            right.write_all(line).unwrap();
        }
        right.write_all(line).unwrap();
    }
    left.flush().unwrap();
    right.flush().unwrap();
    format!("lines={n} replaced={repl} extra_copies={ins}")
}

/// Every adjacent pair of lines is swapped on the right. With `repeats` = 1 all lines are
/// unique; with `repeats` = 16 each line occurs 16 times (low frequency, below the
/// 63/64-occurrence histogram cut-off), shuffled. Anchors are tiny and land at the region
/// edges, which drives histogram diff towards quadratic time.
fn gen_swapped(lp: &Path, rp: &Path, n: u64, repeats: u64, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let vocab = (n / repeats).max(1);
    let mut seq: Vec<u64> = (0..n).map(|i| i % vocab).collect();
    if repeats > 1 {
        for i in (1..seq.len()).rev() {
            let j = r.below(i as u64 + 1) as usize;
            seq.swap(i, j);
        }
    }
    let mut left = writer(lp);
    let mut right = writer(rp);
    let text = |v: u64| format!("fn_{v:07}(ctx, {}, {});\n", v * 7 % 1000, v % 13);
    for v in &seq {
        left.write_all(text(*v).as_bytes()).unwrap();
    }
    for pair in seq.chunks(2) {
        for v in pair.iter().rev() {
            right.write_all(text(*v).as_bytes()).unwrap();
        }
    }
    left.flush().unwrap();
    right.flush().unwrap();
    format!("lines={n} distinct={vocab} repeats={repeats}")
}

/// One ~50 MB line (no newlines inside) between a few normal lines; the right side
/// changes a handful of bytes in the middle of the huge line.
fn gen_huge_line(lp: &Path, rp: &Path, size: usize, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let words = ["alpha", "bravo", "charlie", "delta", "echo", "fox", "golf", "hotel", "india", "juliet"];
    let mut huge = String::with_capacity(size + 16);
    while huge.len() < size {
        huge.push_str(r.pick(&words));
        huge.push(if r.chance(0.1) { ',' } else { ' ' });
    }
    let mut huge_r = huge.clone().into_bytes();
    for k in 0..8 {
        let at = size / 2 + k * 4096;
        huge_r[at] = b'X';
    }
    let head = "{\"version\": 1,\n\"generated\": \"2026-11-07\",\n\"payload\": \"";
    let tail = "\"\n}\n";
    let mut left = writer(lp);
    let mut right = writer(rp);
    left.write_all(head.as_bytes()).unwrap();
    left.write_all(huge.as_bytes()).unwrap();
    left.write_all(tail.as_bytes()).unwrap();
    right.write_all(head.as_bytes()).unwrap();
    right.write_all(&huge_r).unwrap();
    right.write_all(tail.as_bytes()).unwrap();
    left.flush().unwrap();
    right.flush().unwrap();
    format!("huge_line_bytes={}", huge.len())
}

/// Every line is distinct and the two sides share no line at all.
fn gen_all_different(lp: &Path, rp: &Path, n: u64, seed: u64) -> String {
    let mut r = Rng::new(seed);
    let mut left = writer(lp);
    let mut right = writer(rp);
    for i in 0..n {
        writeln!(left, "L{i:08} {:016x} {:016x}", r.next_u64(), r.next_u64()).unwrap();
        writeln!(right, "R{i:08} {:016x} {:016x}", r.next_u64(), r.next_u64()).unwrap();
    }
    left.flush().unwrap();
    right.flush().unwrap();
    format!("lines={n} per side, 0 shared")
}
