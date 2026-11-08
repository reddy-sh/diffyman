//! `diffyman-conformance`: runs the golden corpus.
//!
//! ```text
//! diffyman-conformance [--root <DIR>] [--bless] [--strict]
//! ```
//!
//! Exit codes: 0 when every case passes (cases without `expected.json` count
//! as `new` and pass unless `--strict`) and the moved-block gate holds, 1 when
//! a case fails or errors or the gate is missed, 2 on bad arguments.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use diffyman_conformance::{
    Evaluation, Outcome, RunOptions, category, fmt_counts, fmt_ratio, gate_failures, list_cases,
    run_case,
};

#[derive(Debug, Parser)]
#[command(
    name = "diffyman-conformance",
    version,
    about = "Run the diffyman golden corpus"
)]
struct Cli {
    /// Corpus root holding `text/` and `moved/` (default: the repository's `tests/corpus`).
    #[arg(long, value_name = "DIR")]
    root: Option<PathBuf>,
    /// Write `expected.json` (exact canonical bytes, no trailing newline) for every case.
    #[arg(long)]
    bless: bool,
    /// Treat cases without `expected.json` as failures.
    #[arg(long)]
    strict: bool,
}

fn default_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = cli.root.unwrap_or_else(default_root);
    let cases = match list_cases(&root) {
        Ok(cases) => cases,
        Err(e) => {
            eprintln!("diffyman-conformance: {e}");
            return ExitCode::from(1);
        }
    };
    let options = RunOptions { bless: cli.bless };
    let (mut pass, mut fail, mut new, mut blessed, mut errors) = (0, 0, 0, 0, 0);
    let mut moves = Evaluation::default();
    let mut by_category: BTreeMap<String, (u64, Evaluation)> = BTreeMap::new();
    let mut negatives_with_moves = Vec::new();
    let mut moved_cases = 0;
    for (name, dir) in &cases {
        let result = run_case(name, dir, &options);
        match &result.outcome {
            Outcome::Pass => {
                pass += 1;
                println!("PASS  {name}");
            }
            Outcome::Fail(why) => {
                fail += 1;
                println!("FAIL  {name}: {why}");
            }
            Outcome::New => {
                new += 1;
                println!("NEW   {name} (no expected.json)");
            }
            Outcome::Blessed => {
                blessed += 1;
                println!("BLESS {name}");
            }
            Outcome::Error(why) => {
                errors += 1;
                println!("ERROR {name}: {why}");
            }
        }
        if let Some(eval) = result.metrics {
            moved_cases += 1;
            moves.add(&eval);
            let entry = by_category.entry(category(name).to_owned()).or_default();
            entry.0 += 1;
            entry.1.add(&eval);
            if eval.labels == 0 && eval.predicted > 0 {
                negatives_with_moves.push(name.clone());
            }
            println!("      moves: {}", fmt_counts(&eval.all));
        }
    }
    println!(
        "\n{} cases: {pass} passed, {fail} failed, {new} new, {blessed} blessed, {errors} errors",
        cases.len()
    );
    let mut gate_missed = false;
    if moved_cases > 0 {
        println!("\nmoved-block metrics (all labels):");
        for (cat, (n, eval)) in &by_category {
            println!("  {cat:<8} {n:>3} cases  {}", fmt_counts(&eval.all));
        }
        println!(
            "  total    {moved_cases:>3} cases  {}",
            fmt_counts(&moves.all)
        );
        println!("  pure labels only:    {}", fmt_counts(&moves.pure_only));
        let ratio = |num: u64, den: u64| {
            #[allow(clippy::cast_precision_loss)]
            fmt_ratio((den > 0).then(|| num as f64 / den as f64))
        };
        println!(
            "  kind accuracy {} and exact boundaries {} over {} matched pairs",
            ratio(moves.kind_ok, moves.all.tp),
            ratio(moves.exact, moves.all.tp),
            moves.all.tp
        );
        let failures = gate_failures(&moves, &negatives_with_moves);
        if failures.is_empty() {
            println!("  gate: pass (precision >= 0.95, recall >= 0.90, negatives empty)");
        } else {
            gate_missed = true;
            for f in failures {
                println!("  gate: FAIL {f}");
            }
        }
    }
    if fail > 0 || errors > 0 || (cli.strict && new > 0) || gate_missed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
