//! Measures the `text.histogram@1` work budget on a pair of files.
//!
//! ```text
//! cargo run --release -p diffyman-algo --example line_diff_work -- LEFT RIGHT [--unguarded]
//! ```
//!
//! Prints one JSON line: trimmed line counts, Histogram work units and their
//! ratio to the trimmed line count, the budget, whether it fell back, timings
//! for guarded Histogram and Myers, and (with `--unguarded`) whether the
//! guarded result equals unguarded Histogram. Lines are compared byte for byte
//! with their terminators (whitespace mode `none`). Timing is for reading
//! only; nothing here affects a Diff Document.

use std::time::Instant;

use diffyman_algo::key::{Interner, KeyBuilder, KeyOptions, intern_sides};
use diffyman_algo::line_diff::{self, LineAlgorithm};
use diffyman_algo::lines::split_lines;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let unguarded = args.iter().any(|a| a == "--unguarded");
    let [left_path, right_path] = paths.as_slice() else {
        eprintln!("usage: line_diff_work LEFT RIGHT [--unguarded]");
        std::process::exit(2);
    };
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let (left_text, right_text) = (read(left_path), read(right_path));
    let (left, right) = (split_lines(&left_text), split_lines(&right_text));
    let builder = KeyBuilder::new(&KeyOptions::default()).expect("default key options");
    let mut interner = Interner::new();
    let (lt, rt) = intern_sides(
        &builder,
        &mut interner,
        &left_text,
        &left,
        &right_text,
        &right,
    );
    let n = interner.num_tokens();

    let prefix = lt.iter().zip(&rt).take_while(|(a, b)| a == b).count();
    let suffix = lt[prefix..]
        .iter()
        .rev()
        .zip(rt[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let trimmed = (lt.len() - prefix - suffix) + (rt.len() - prefix - suffix);

    let started = Instant::now();
    let (guarded, report) = line_diff::align_with_report(LineAlgorithm::Histogram, &lt, &rt, n);
    let histogram_s = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let myers = line_diff::align(LineAlgorithm::Myers, &lt, &rt, n);
    let myers_s = started.elapsed().as_secs_f64();

    let mut extra = String::new();
    if unguarded {
        let started = Instant::now();
        let (free, free_report) = line_diff::align_histogram_with_budget(&lt, &rt, n, u64::MAX);
        let free_s = started.elapsed().as_secs_f64();
        extra = format!(
            ",\"unguardedWork\":{},\"unguardedS\":{free_s:.3},\"sameAsUnguarded\":{}",
            free_report.histogram_work,
            free == guarded
        );
    }
    let changed =
        |a: &line_diff::Alignment| a.removed.iter().chain(&a.added).filter(|x| **x).count();
    #[allow(clippy::cast_precision_loss)]
    let ratio = report.histogram_work as f64 / trimmed.max(1) as f64;
    println!(
        "{{\"leftLines\":{},\"rightLines\":{},\"trimmedLines\":{trimmed},\"work\":{},\
         \"workPerLine\":{ratio:.2},\"budget\":{},\"fellBack\":{},\"approximate\":{},\
         \"histogramChanged\":{},\"myersChanged\":{},\"histogramS\":{histogram_s:.3},\
         \"myersS\":{myers_s:.3}{extra}}}",
        lt.len(),
        rt.len(),
        report.histogram_work,
        report.histogram_budget,
        report.histogram_fell_back,
        line_diff::is_approximate(&lt, &rt, &guarded),
        changed(&guarded),
        changed(&myers),
    );
}
