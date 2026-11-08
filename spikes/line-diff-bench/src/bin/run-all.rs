//! Runs every (case, algorithm) pair in its own process under `/usr/bin/time -l`, so
//! peak memory is per case, and appends one JSON line per run to the results file.
//!
//! Run:    run-all --data DIR --out FILE [--cases a,b] [--algos x,y] [--timeout-s N] [--repeat N]
//! Report: run-all --report FILE     (prints Markdown tables, median of repeats)

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CASES: &[&str] = &[
    "log-100m",
    "log-1g",
    "source-50m",
    "identical",
    "histo-lowfreq",
    "swapped-unique",
    "huge-line",
    "all-different",
];

/// (label, kind, extra args). kind is a bench algorithm or "git:<algo>".
fn algos(case: &str) -> Vec<(String, String, Vec<String>)> {
    let mut v: Vec<(String, String, Vec<String>)> = [
        ("similar-myers", "similar-myers", vec![]),
        ("similar-myers+dl5s", "similar-myers", vec!["--deadline-ms", "5000"]),
        ("similar-myers-raw", "similar-myers-raw", vec![]),
        ("similar-myers-raw+dl5s", "similar-myers-raw", vec!["--deadline-ms", "5000"]),
        ("similar-patience", "similar-patience", vec![]),
        ("similar-patience+dl5s", "similar-patience", vec!["--deadline-ms", "5000"]),
        ("similar-histogram", "similar-histogram", vec![]),
        ("similar-histogram+dl5s", "similar-histogram", vec!["--deadline-ms", "5000"]),
        ("imara-histogram", "imara-histogram", vec![]),
        ("imara-myers", "imara-myers", vec![]),
        ("imara-myers-minimal", "imara-myers-minimal", vec![]),
        ("imara-histogram-native", "imara-histogram-native", vec![]),
        ("gix-histogram", "gix-histogram", vec![]),
        ("gix-myers", "gix-myers", vec![]),
        ("gix-myers-minimal", "gix-myers-minimal", vec![]),
        ("git-histogram", "git:histogram", vec![]),
        ("git-myers", "git:myers", vec![]),
    ]
    .into_iter()
    .map(|(l, k, a)| (l.to_string(), k.to_string(), a.into_iter().map(String::from).collect()))
    .collect();
    if case == "log-1g" {
        v.push(("imara-histogram/read".into(), "imara-histogram".into(), vec!["--io".into(), "read".into()]));
        v.push(("similar-myers/read".into(), "similar-myers".into(), vec!["--io".into(), "read".into()]));
    }
    v
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(f) = arg(&args, "--report") {
        report(Path::new(&f));
        return;
    }
    let data = PathBuf::from(arg(&args, "--data").expect("--data"));
    let out = PathBuf::from(arg(&args, "--out").expect("--out"));
    let timeout_s: u64 = arg(&args, "--timeout-s").map_or(300, |s| s.parse().unwrap());
    let repeat: usize = arg(&args, "--repeat").map_or(1, |s| s.parse().unwrap());
    let cases: Vec<String> = arg(&args, "--cases")
        .map(|s| s.split(',').map(String::from).collect())
        .unwrap_or_else(|| CASES.iter().map(|s| s.to_string()).collect());
    let only: Option<Vec<String>> = arg(&args, "--algos").map(|s| s.split(',').map(String::from).collect());
    let bench = std::env::current_exe().unwrap().with_file_name("bench");

    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    let mut outf = std::fs::OpenOptions::new().create(true).append(true).open(&out).unwrap();

    for case in &cases {
        let left = data.join(format!("{case}.left"));
        let right = data.join(format!("{case}.right"));
        warm(&left);
        warm(&right);
        for (label, kind, extra) in algos(case) {
            if only.as_ref().is_some_and(|o| !o.contains(&label)) {
                continue;
            }
            for rep in 0..repeat {
                let mut rec = if let Some(galg) = kind.strip_prefix("git:") {
                    run_git(galg, &left, &right, timeout_s)
                } else {
                    let mut a = vec![
                        bench.to_string_lossy().to_string(),
                        "--left".into(),
                        left.to_string_lossy().to_string(),
                        "--right".into(),
                        right.to_string_lossy().to_string(),
                        "--algo".into(),
                        kind.clone(),
                        "--timeout-s".into(),
                        timeout_s.to_string(),
                    ];
                    a.extend(extra.iter().cloned());
                    run_timed(&a, timeout_s + 30, |stdout| {
                        stdout
                            .lines()
                            .rev()
                            .find_map(|l| serde_json::from_str::<Map<String, Value>>(l).ok())
                    })
                };
                rec.insert("case".into(), json!(case));
                rec.insert("label".into(), json!(label));
                rec.insert("rep".into(), json!(rep));
                let line = serde_json::to_string(&rec).unwrap();
                println!("{line}");
                writeln!(outf, "{line}").unwrap();
                outf.flush().unwrap();
            }
        }
    }
}

/// Reads the file once so every algorithm sees a warm page cache.
fn warm(p: &Path) {
    let mut f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut buf = vec![0u8; 8 << 20];
    while f.read(&mut buf).unwrap() > 0 {}
}

/// Runs `cmd` under `/usr/bin/time -l` in its own process group with a hard timeout,
/// parses the resource usage, and lets `parse` extract fields from stdout.
fn run_timed(
    cmd: &[String],
    timeout_s: u64,
    parse: impl FnOnce(&str) -> Option<Map<String, Value>>,
) -> Map<String, Value> {
    let mut child = Command::new("/usr/bin/time")
        .arg("-l")
        .args(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn /usr/bin/time");
    let pid = child.id();
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let ho = std::thread::spawn(move || {
        let mut s = String::new();
        so.read_to_string(&mut s).ok();
        s
    });
    let he = std::thread::spawn(move || {
        let mut s = String::new();
        se.read_to_string(&mut s).ok();
        s
    });
    let start = Instant::now();
    let mut killed = false;
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if start.elapsed() > Duration::from_secs(timeout_s) {
            let _ = Command::new("/bin/kill").args(["-KILL", "--", &format!("-{pid}")]).status();
            killed = true;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let wall = start.elapsed().as_secs_f64();
    let stdout = ho.join().unwrap();
    let stderr = he.join().unwrap();
    let mut rec = parse(&stdout).unwrap_or_default();
    if killed {
        rec.insert("status".into(), json!("timeout"));
    } else if !rec.contains_key("status") {
        rec.insert("status".into(), json!(format!("error(exit {:?})", status.code())));
        let tail: String = stderr.lines().take(5).collect::<Vec<_>>().join(" | ");
        rec.insert("stderr".into(), json!(tail));
    }
    rec.insert("wall_s".into(), json!((wall * 100.0).round() / 100.0));
    for l in stderr.lines() {
        let t = l.trim();
        if let Some(n) = t.strip_suffix("maximum resident set size") {
            rec.insert("max_rss_bytes".into(), json!(n.trim().parse::<u64>().unwrap_or(0)));
        } else if let Some(n) = t.strip_suffix("peak memory footprint") {
            rec.insert("peak_footprint_bytes".into(), json!(n.trim().parse::<u64>().unwrap_or(0)));
        } else if t.ends_with(" sys") && t.contains(" real") {
            if let Some(r) = t.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()) {
                rec.insert("real_s".into(), json!(r));
            }
        }
    }
    rec
}

/// Baseline: `git diff --no-index -U0` with the given algorithm; counts hunks and lines.
fn run_git(alg: &str, left: &Path, right: &Path, timeout_s: u64) -> Map<String, Value> {
    let cmd: Vec<String> = vec![
        "git".into(),
        "-c".into(),
        "core.bigFileThreshold=8g".into(),
        "diff".into(),
        "--no-index".into(),
        "--no-color".into(),
        "-U0".into(),
        format!("--diff-algorithm={alg}"),
        left.to_string_lossy().to_string(),
        right.to_string_lossy().to_string(),
    ];
    let mut rec = run_timed(&cmd, timeout_s, |stdout| {
        let (mut hunks, mut ins, mut del) = (0u64, 0u64, 0u64);
        let mut binary = false;
        for l in stdout.lines() {
            if l.starts_with("@@") {
                hunks += 1;
            } else if l.starts_with("+++") || l.starts_with("---") {
            } else if l.starts_with('+') {
                ins += 1;
            } else if l.starts_with('-') {
                del += 1;
            } else if l.starts_with("Binary files") {
                binary = true;
            }
        }
        let mut m = Map::new();
        m.insert("algo".into(), json!(format!("git-{alg}")));
        m.insert("status".into(), json!(if binary { "binary" } else { "ok" }));
        m.insert("hunks".into(), json!(hunks));
        m.insert("inserted".into(), json!(ins));
        m.insert("deleted".into(), json!(del));
        Some(m)
    });
    rec.insert("note".into(), json!("includes writing -U0 patch text"));
    rec
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

fn report(path: &Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let mut by: BTreeMap<(usize, String), Vec<Map<String, Value>>> = BTreeMap::new();
    let mut label_order: Vec<String> = Vec::new();
    for l in text.lines().filter(|l| !l.trim().is_empty()) {
        let m: Map<String, Value> = serde_json::from_str(l).unwrap();
        let case = m["case"].as_str().unwrap().to_string();
        let label = m["label"].as_str().unwrap().to_string();
        if !label_order.contains(&label) {
            label_order.push(label.clone());
        }
        let ci = CASES.iter().position(|c| *c == case).unwrap_or(99);
        by.entry((ci, case.clone())).or_default().push(m);
    }
    let f = |m: &Map<String, Value>, k: &str| m.get(k).and_then(|v| v.as_f64());
    let mb = |v: Option<f64>| v.map_or("-".to_string(), |b| format!("{:.0}", b / 1_048_576.0));
    for ((_, case), recs) in by {
        let first = &recs[0];
        println!("\n### {case}\n");
        if let (Some(lb), Some(rb)) = (f(first, "left_bytes"), f(first, "right_bytes")) {
            println!(
                "Left {:.1} MB, right {:.1} MB; common prefix {} lines, suffix {} lines; middle {} / {} lines; {} distinct lines.\n",
                lb / 1e6,
                rb / 1e6,
                first.get("prefix_lines").map_or(Value::Null, |v| v.clone()),
                first.get("suffix_lines").map_or(Value::Null, |v| v.clone()),
                first.get("mid_left_lines").map_or(Value::Null, |v| v.clone()),
                first.get("mid_right_lines").map_or(Value::Null, |v| v.clone()),
                first.get("unique_tokens").map_or(Value::Null, |v| v.clone()),
            );
        }
        // Smallest edit script seen for this case (exact when similar-myers-raw finished
        // without a deadline), used as the minimality baseline.
        let best = recs
            .iter()
            .filter(|r| r.get("valid").and_then(|v| v.as_bool()) == Some(true))
            .filter_map(|r| Some(f(r, "deleted")? + f(r, "inserted")?))
            .fold(f64::INFINITY, f64::min);
        println!("| Algorithm | Status | Wall s | Intern s | Diff s | Max RSS MB | Footprint MB | Hunks | Deleted | Inserted | Del+Ins | vs best | Valid | Output |");
        println!("|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|:-:|:-:|");
        // Median by wall time per label.
        let mut groups: BTreeMap<usize, Vec<&Map<String, Value>>> = BTreeMap::new();
        for r in &recs {
            let li = label_order.iter().position(|l| l == r["label"].as_str().unwrap()).unwrap();
            groups.entry(li).or_default().push(r);
        }
        let mut fps: Vec<String> = Vec::new();
        for (li, mut rs) in groups {
            rs.sort_by(|a, b| f(a, "wall_s").partial_cmp(&f(b, "wall_s")).unwrap());
            let r = rs[rs.len() / 2];
            let status = r["status"].as_str().unwrap_or("?").to_string();
            let status = if r.get("deadline_hit").and_then(|v| v.as_bool()) == Some(true) {
                "deadline".to_string()
            } else {
                status
            };
            let group = match r.get("fingerprint").and_then(|v| v.as_str()) {
                Some(fp) => {
                    if !fps.iter().any(|x| x == fp) {
                        fps.push(fp.to_string());
                    }
                    let i = fps.iter().position(|x| x == fp).unwrap();
                    ((b'A' + (i % 26) as u8) as char).to_string()
                }
                None => "-".to_string(),
            };
            let sec = |k: &str| f(r, k).map_or("-".to_string(), |v| format!("{:.2}", v / 1000.0));
            let int = |k: &str| f(r, k).map_or("-".to_string(), |v| format!("{v:.0}"));
            let (di, vs) = match (f(r, "deleted"), f(r, "inserted")) {
                (Some(d), Some(i)) if best.is_finite() && best > 0.0 => {
                    (format!("{:.0}", d + i), format!("{:+.1}%", ((d + i) / best - 1.0) * 100.0))
                }
                (Some(d), Some(i)) => (format!("{:.0}", d + i), "-".into()),
                _ => ("-".into(), "-".into()),
            };
            let valid = match r.get("valid").and_then(|v| v.as_bool()) {
                Some(true) => "yes",
                Some(false) => "NO",
                None => "-",
            };
            let reps = if rs.len() > 1 { format!(" (median of {})", rs.len()) } else { String::new() };
            println!(
                "| {}{} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                label_order[li],
                reps,
                status,
                f(r, "wall_s").map_or("-".into(), |v| format!("{v:.2}")),
                sec("t_intern_ms"),
                sec("t_diff_ms"),
                mb(f(r, "max_rss_bytes")),
                mb(f(r, "peak_footprint_bytes")),
                int("hunks"),
                int("deleted"),
                int("inserted"),
                di,
                vs,
                valid,
                group
            );
        }
    }
}
