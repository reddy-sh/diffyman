//! Runs ONE line diff (one input pair, one algorithm) and prints a flat JSON object.
//! Run it under `/usr/bin/time -l` (see `run-all`) so peak memory is per case.
//!
//! Usage: bench --left L --right R --algo A [--io mmap|read] [--deadline-ms N] [--timeout-s N]
//! Algorithms: similar-myers similar-myers-raw similar-patience similar-histogram
//!             imara-histogram imara-myers imara-myers-minimal imara-histogram-native

use imara_diff::{Algorithm as IAlg, Diff, InternedInput, Token};
use line_diff_bench::{count_lines, trim, Hunks, Interner, SimilarCollector};
use similar::Algorithm as SAlg;
use std::time::{Duration, Instant};

enum Data {
    Map(memmap2::Mmap),
    Buf(Vec<u8>),
}

impl std::ops::Deref for Data {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Data::Map(m) => m,
            Data::Buf(b) => b,
        }
    }
}

fn load(path: &str, io: &str) -> Data {
    if io == "read" {
        return Data::Buf(std::fs::read(path).expect("read input"));
    }
    let f = std::fs::File::open(path).expect("open input");
    if f.metadata().expect("stat").len() == 0 {
        return Data::Buf(Vec::new());
    }
    // SAFETY: the corpus files are not modified while the benchmark runs.
    let m = unsafe { memmap2::Mmap::map(&f) }.expect("mmap input");
    let _ = m.advise(memmap2::Advice::Sequential);
    Data::Map(m)
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args[i + 1].clone())
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let left = arg(&args, "--left").expect("--left");
    let right = arg(&args, "--right").expect("--right");
    let algo = arg(&args, "--algo").expect("--algo");
    let io = arg(&args, "--io").unwrap_or_else(|| "mmap".into());
    let deadline_ms: Option<u64> = arg(&args, "--deadline-ms").map(|s| s.parse().unwrap());
    let timeout_s: u64 = arg(&args, "--timeout-s").map_or(300, |s| s.parse().unwrap());

    // Watchdog: report a timeout and exit, so a quadratic case cannot hang the run.
    {
        let algo = algo.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(timeout_s));
            println!("{{\"algo\":\"{algo}\",\"status\":\"timeout\",\"timeout_s\":{timeout_s}}}");
            std::process::exit(124);
        });
    }

    let t0 = Instant::now();
    let a = load(&left, &io);
    let b = load(&right, &io);
    let t_load = t0.elapsed();

    let t1 = Instant::now();
    let tr = trim(&a, &b);
    let t_trim = t1.elapsed();
    let am = &a[tr.prefix_bytes..a.len() - tr.suffix_bytes];
    let bm = &b[tr.prefix_bytes..b.len() - tr.suffix_bytes];

    let mut hunks = Hunks::default();
    let off = tr.prefix_lines;
    let (t_intern, t_diff, t_post, mid_l, mid_r, uniq, valid, mut deadline_hit);
    deadline_hit = false;

    if algo == "imara-histogram-native" {
        // imara-diff's own pipeline: its interner, Histogram, then the indent-aware
        // slider post-processing (`postprocess_lines`).
        let t2 = Instant::now();
        let input = InternedInput::new(am, bm);
        t_intern = t2.elapsed();
        let t3 = Instant::now();
        let mut diff = Diff::compute(IAlg::Histogram, &input);
        t_diff = t3.elapsed();
        let t4 = Instant::now();
        diff.postprocess_lines(&input);
        t_post = t4.elapsed();
        for h in diff.hunks() {
            hunks.push_change(
                off + h.before.start as usize,
                off + h.before.end as usize,
                off + h.after.start as usize,
                off + h.after.end as usize,
            );
        }
        mid_l = input.before.len();
        mid_r = input.after.len();
        uniq = input.interner.num_tokens();
        valid = hunks.is_valid_script(off, &input.before, &input.after);
    } else {
        let t2 = Instant::now();
        let est = count_lines(am);
        let mut interner = Interner::with_capacity(est + est / 8);
        let mut lt = Vec::with_capacity(est);
        let mut rt = Vec::with_capacity(est + est / 8);
        interner.intern_all(am, &mut lt);
        interner.intern_all(bm, &mut rt);
        t_intern = t2.elapsed();
        mid_l = lt.len();
        mid_r = rt.len();
        uniq = interner.num_tokens();

        let t3 = Instant::now();
        match algo.as_str() {
            "similar-myers" | "similar-myers-raw" | "similar-patience" | "similar-histogram" => {
                let alg = match algo.as_str() {
                    "similar-myers" => SAlg::Myers,
                    "similar-myers-raw" => SAlg::RawMyers,
                    "similar-patience" => SAlg::Patience,
                    _ => SAlg::Histogram,
                };
                let deadline = deadline_ms.map(|d| Instant::now() + Duration::from_millis(d));
                let mut c = SimilarCollector { hunks: Hunks::default() };
                similar::algorithms::diff_deadline(alg, &mut c, &lt[..], 0..lt.len(), &rt[..], 0..rt.len(), deadline)
                    .unwrap();
                if let Some(d) = deadline {
                    deadline_hit = Instant::now() >= d;
                }
                for h in c.hunks.list {
                    hunks.push_change(
                        off + h[0] as usize,
                        off + h[1] as usize,
                        off + h[2] as usize,
                        off + h[3] as usize,
                    );
                }
            }
            "imara-histogram" | "imara-myers" | "imara-myers-minimal" => {
                let alg = match algo.as_str() {
                    "imara-histogram" => IAlg::Histogram,
                    "imara-myers" => IAlg::Myers,
                    _ => IAlg::MyersMinimal,
                };
                // SAFETY: `Token` is `#[repr(transparent)]` over `u32`.
                let (lt_t, rt_t): (&[Token], &[Token]) = unsafe {
                    (
                        std::slice::from_raw_parts(lt.as_ptr().cast(), lt.len()),
                        std::slice::from_raw_parts(rt.as_ptr().cast(), rt.len()),
                    )
                };
                let mut diff = Diff::default();
                diff.compute_with(alg, lt_t, rt_t, uniq);
                for h in diff.hunks() {
                    hunks.push_change(
                        off + h.before.start as usize,
                        off + h.before.end as usize,
                        off + h.after.start as usize,
                        off + h.after.end as usize,
                    );
                }
            }
            "gix-histogram" | "gix-myers" | "gix-myers-minimal" => {
                use gix_imara_diff::{Algorithm as GAlg, Diff as GDiff, Token as GToken};
                let alg = match algo.as_str() {
                    "gix-histogram" => GAlg::Histogram,
                    "gix-myers" => GAlg::Myers,
                    _ => GAlg::MyersMinimal,
                };
                // SAFETY: `Token` is `#[repr(transparent)]` over `u32`.
                let (lt_t, rt_t): (&[GToken], &[GToken]) = unsafe {
                    (
                        std::slice::from_raw_parts(lt.as_ptr().cast(), lt.len()),
                        std::slice::from_raw_parts(rt.as_ptr().cast(), rt.len()),
                    )
                };
                let mut diff = GDiff::default();
                diff.compute_with(alg, lt_t, rt_t, uniq);
                for h in diff.hunks() {
                    hunks.push_change(
                        off + h.before.start as usize,
                        off + h.before.end as usize,
                        off + h.after.start as usize,
                        off + h.after.end as usize,
                    );
                }
            }
            other => panic!("unknown algo {other}"),
        }
        t_diff = t3.elapsed();
        t_post = Duration::ZERO;
        valid = hunks.is_valid_script(off, &lt, &rt);
    }
    let total = t0.elapsed();

    println!(
        concat!(
            "{{\"algo\":\"{}\",\"status\":\"ok\",\"io\":\"{}\",\"deadline_ms\":{},\"deadline_hit\":{},",
            "\"left_bytes\":{},\"right_bytes\":{},\"prefix_lines\":{},\"suffix_lines\":{},",
            "\"mid_left_lines\":{},\"mid_right_lines\":{},\"unique_tokens\":{},",
            "\"t_load_ms\":{},\"t_trim_ms\":{},\"t_intern_ms\":{},\"t_diff_ms\":{},\"t_post_ms\":{},\"t_total_ms\":{},",
            "\"hunks\":{},\"deleted\":{},\"inserted\":{},\"valid\":{},\"fingerprint\":\"{:016x}\"}}"
        ),
        algo,
        io,
        deadline_ms.map_or("null".to_string(), |d| d.to_string()),
        deadline_hit,
        a.len(),
        b.len(),
        tr.prefix_lines,
        tr.suffix_lines,
        mid_l,
        mid_r,
        uniq,
        ms(t_load),
        ms(t_trim),
        ms(t_intern),
        ms(t_diff),
        ms(t_post),
        ms(total),
        hunks.list.len(),
        hunks.deleted(),
        hunks.inserted(),
        valid,
        hunks.fingerprint()
    );
    // Skip tearing down large allocations; the measurement is already printed.
    std::process::exit(0);
}
