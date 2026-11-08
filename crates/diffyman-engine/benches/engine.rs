//! Engine benchmarks on generated inputs.
//!
//! Each benchmark calls [`diffyman::diff`], the JSON-in/JSON-out entry point
//! every binding uses, so the time includes type detection, the diff itself
//! and canonical JSON serialization.
//!
//! - `text/<lines>`: `<lines>` generated lines per side (10k, 100k, 1M), with
//!   about 1% of lines replaced, 0.5% deleted and 0.5% inserted.
//! - `moves-off/<lines>` and `moves-on/<lines>`: the same edits plus blocks of
//!   lines moved elsewhere, with `detectMoves` false and true.
//! - `binary/<case>`: 10 MiB of pseudo-random bytes per side (detected as
//!   binary), with scattered one-byte edits (`edits`) or an insertion and a
//!   deletion that shift the rest of the input (`shift`).
//!
//! Before timing each case, the benchmark prints its hunk and move counts and
//! warning codes to stderr, because a limit warning changes what is measured.
//! With moved blocks, 100k and 1M lines get `approximate` (more than 500
//! changed lines have a key that also occurs on the other side) and
//! `moves-skipped-limit` (the edited-move search stops at its work limit; the
//! pure moves are still found). The 10 MiB `edits` case gets
//! `bytes-coarse-limit`.
//!
//! Inputs come from a fixed-seed generator, so every run and every machine
//! diffs the same bytes. Run all of them with
//! `cargo bench -p diffyman --bench engine`, or a quick pass with
//! `cargo bench -p diffyman --bench engine -- --quick`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main};

/// Line counts per side for the text benchmarks.
const SIZES: [(&str, usize); 3] = [("10k", 10_000), ("100k", 100_000), ("1m", 1_000_000)];

/// Size of each binary input.
const BINARY_BYTES: usize = 10 * 1024 * 1024;

/// The `SplitMix64` generator: small and deterministic, so the benchmark needs no
/// extra dependency and its inputs never change.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..n` (`n > 0`).
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).expect("usize fits in u64");
        usize::try_from(self.next() % n).expect("below n")
    }
}

const WORDS: [&str; 32] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau", "upsilon", "phi", "chi",
    "psi", "omega", "value", "count", "result", "index", "buffer", "offset", "length", "status",
];

/// One generated line: a sequence number, so most lines are unique as in
/// source code or logs, then three to eight words.
fn line(rng: &mut Rng, number: usize) -> String {
    use std::fmt::Write as _;
    let mut text = format!("{number:08}");
    for _ in 0..3 + rng.below(6) {
        text.push(' ');
        text.push_str(WORDS[rng.below(WORDS.len())]);
    }
    let _ = write!(text, " = {};", rng.below(1000));
    text
}

/// A text pair of `lines` lines per side. The right side replaces, deletes
/// and inserts single lines, and with `moves` also moves blocks of 6 to 20
/// lines (one block per 2000 lines) to another place.
fn text_pair(lines: usize, moves: bool) -> (Vec<u8>, Vec<u8>) {
    let mut rng = Rng(0x00D1_FF00 ^ u64::try_from(lines).expect("fits"));
    let left: Vec<String> = (0..lines).map(|i| line(&mut rng, i)).collect();
    let mut right: Vec<String> = Vec::with_capacity(lines + lines / 100);
    for (i, text) in left.iter().enumerate() {
        match rng.below(200) {
            0 | 1 => right.push(format!("{text} // edited")),
            2 => {}
            3 => {
                right.push(text.clone());
                right.push(line(&mut rng, lines + i));
            }
            _ => right.push(text.clone()),
        }
    }
    if moves {
        for _ in 0..lines / 2000 {
            let len = 6 + rng.below(15);
            if right.len() <= 2 * len {
                break;
            }
            let from = rng.below(right.len() - len);
            let block: Vec<String> = right.drain(from..from + len).collect();
            let to = rng.below(right.len());
            right.splice(to..to, block);
        }
    }
    (join(&left), join(&right))
}

fn join(lines: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for text in lines {
        out.extend_from_slice(text.as_bytes());
        out.push(b'\n');
    }
    out
}

/// A binary pair of [`BINARY_BYTES`] per side. `shift` inserts 4 KiB at one
/// third and deletes 4 KiB at two thirds; otherwise 256 single bytes change
/// in place.
fn binary_pair(shift: bool) -> (Vec<u8>, Vec<u8>) {
    let mut rng = Rng(0xB1_0A_27);
    let mut left = Vec::with_capacity(BINARY_BYTES);
    while left.len() < BINARY_BYTES {
        left.extend_from_slice(&rng.next().to_le_bytes());
    }
    let mut right = left.clone();
    if shift {
        let insert: Vec<u8> = (0..4096).map(|_| rng.next().to_le_bytes()[0]).collect();
        let at = BINARY_BYTES / 3;
        right.splice(at..at, insert);
        let at = 2 * right.len() / 3;
        right.drain(at..at + 4096);
    } else {
        for _ in 0..256 {
            let at = rng.below(right.len());
            right[at] = !right[at];
        }
    }
    (left, right)
}

/// Runs one benchmark after checking, once and outside the timing, that the
/// engine gives the expected kind of document (so a benchmark never measures
/// an error path by accident).
fn bench_pair<M: criterion::measurement::Measurement>(
    group: &mut BenchmarkGroup<'_, M>,
    name: &str,
    (left, right): &(Vec<u8>, Vec<u8>),
    options: &str,
    expect: &[&str],
) {
    let json = diffyman::diff(left, right, options).expect("engine accepts the options");
    for needle in expect {
        assert!(json.contains(needle), "{name}: document lacks {needle}");
    }
    // Limit warnings change what a run measures, so show them next to it.
    let doc: serde_json::Value = serde_json::from_str(&json).expect("document parses");
    let codes: Vec<&str> = doc["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["code"].as_str())
        .collect();
    let moves = doc["moves"].as_array().map_or(0, Vec::len);
    let hunks = doc["hunks"].as_array().map_or(0, Vec::len);
    eprintln!("{name}: {hunks} hunks, {moves} moves, warnings {codes:?}");
    group.throughput(Throughput::Bytes(
        u64::try_from(left.len() + right.len()).expect("fits"),
    ));
    group.bench_function(name, |b| {
        b.iter(|| diffyman::diff(black_box(left), black_box(right), black_box(options)));
    });
}

/// Fewer samples and a longer window for inputs that take more than a few
/// milliseconds per run.
fn tune<M: criterion::measurement::Measurement>(group: &mut BenchmarkGroup<'_, M>, lines: usize) {
    if lines >= 100_000 {
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(10));
    }
}

const TEXT: &str = r#""kind":"text""#;
const STATUS_DIFFERENT: &str = r#""status":"different""#;

fn text(c: &mut Criterion) {
    let mut group = c.benchmark_group("text");
    for (label, lines) in SIZES {
        tune(&mut group, lines);
        let pair = text_pair(lines, false);
        bench_pair(&mut group, label, &pair, "{}", &[TEXT, STATUS_DIFFERENT]);
    }
    group.finish();
}

fn moves(c: &mut Criterion) {
    for (group_name, options, expect) in [
        ("moves-off", r#"{"detectMoves":false}"#, r#""moves":[]"#),
        ("moves-on", r#"{"detectMoves":true}"#, r#""moves":[{"#),
    ] {
        let mut group = c.benchmark_group(group_name);
        for (label, lines) in SIZES {
            tune(&mut group, lines);
            let pair = text_pair(lines, true);
            bench_pair(&mut group, label, &pair, options, &[TEXT, expect]);
        }
        group.finish();
    }
}

fn binary(c: &mut Criterion) {
    let mut group = c.benchmark_group("binary");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for (label, shift) in [("edits-10mib", false), ("shift-10mib", true)] {
        let pair = binary_pair(shift);
        bench_pair(
            &mut group,
            label,
            &pair,
            "{}",
            &[r#""kind":"binary""#, STATUS_DIFFERENT],
        );
    }
    group.finish();
}

criterion_group!(benches, text, moves, binary);
criterion_main!(benches);
