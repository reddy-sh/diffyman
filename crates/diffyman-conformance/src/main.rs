//! `diffyman-conformance`: runs the golden corpus.
//!
//! ```text
//! diffyman-conformance [--root <DIR>] [--bless] [--strict]
//! ```
//!
//! Exit codes: 0 when every case passes (cases without `expected.json` count
//! as `new` and pass unless `--strict`), 1 when a case fails or errors, 2 on
//! bad arguments.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use diffyman_conformance::{Counts, Outcome, RunOptions, fmt_counts, list_cases, run_case};

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
    let mut moves = Counts::default();
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
        if let Some(counts) = result.metrics {
            moved_cases += 1;
            moves.add(counts);
            println!("      moves: {}", fmt_counts(&counts));
        }
    }
    println!(
        "\n{} cases: {pass} passed, {fail} failed, {new} new, {blessed} blessed, {errors} errors",
        cases.len()
    );
    if moved_cases > 0 {
        println!(
            "moved-block metrics over {moved_cases} cases (mode pure; detection is a Phase 0 stub, not gating): {}",
            fmt_counts(&moves)
        );
    }
    if fail > 0 || errors > 0 || (cli.strict && new > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
