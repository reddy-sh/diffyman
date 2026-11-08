use super::*;
use crate::key::Interner;
use crate::line_diff::{self, LineAlgorithm};
use crate::ops::{self, OpKind};

/// Full op list of a histogram line diff over exact line contents.
fn diff_ops(left: &[&str], right: &[&str]) -> Vec<LineOp> {
    let mut interner = Interner::new();
    let lt: Vec<u32> = left
        .iter()
        .map(|s| interner.intern(s.as_bytes().to_vec()))
        .collect();
    let rt: Vec<u32> = right
        .iter()
        .map(|s| interner.intern(s.as_bytes().to_vec()))
        .collect();
    let alignment = line_diff::align(LineAlgorithm::Histogram, &lt, &rt, interner.num_tokens());
    ops::full_ops(&alignment)
}

fn detect_with(left: &[&str], right: &[&str], options: MoveOptions) -> Moves {
    detect_moves(left, right, &diff_ops(left, right), &options)
}

fn detect(left: &[&str], right: &[&str]) -> Moves {
    detect_with(left, right, MoveOptions::default())
}

/// (left start, left len, right start, right len, kind) per move, 0-based.
fn spans(m: &Moves) -> Vec<(usize, usize, usize, usize, MoveKind)> {
    m.moves
        .iter()
        .map(|m| (m.left_start, m.left_len, m.right_start, m.right_len, m.kind))
        .collect()
}

/// Splits test text into lines; the text is leaked so the lines outlive it.
fn lines(text: &str) -> Vec<&'static str> {
    let text: &'static str = Box::leak(text.to_owned().into_boxed_str());
    text.lines().collect()
}

const PARSE: &str = "def parse_config(path):\n    with open(path) as handle:\n        data = json.load(handle)\n    return Config(**data)";
const OTHER: &str = "def main():\n    config = parse_config(sys.argv[1])\n    run(config)";

#[test]
fn pure_move_down_and_up() {
    let left = lines(&format!("import json\n\n{PARSE}\n\n{OTHER}\n"));
    let right = lines(&format!("import json\n\n{OTHER}\n\n{PARSE}\n"));
    // The line diff keeps the longer PARSE block in place and moves OTHER.
    let m = detect(&left, &right);
    assert!(!m.skipped_limit);
    assert_eq!(m.moves.len(), 1, "{m:?}");
    let mv = m.moves[0];
    assert_eq!(mv.kind, MoveKind::Pure);
    assert_eq!(mv.similarity_bp, 10_000);
    assert_eq!(mv.left_len, 3);
    assert_eq!(left[mv.left_start], "def main():");
    assert_eq!(right[mv.right_start], "def main():");
    // Reversed direction finds the mirror move.
    let back = detect(&right, &left);
    assert_eq!(back.moves.len(), 1);
    assert_eq!(back.moves[0].left_len, 3);
}

#[test]
fn blank_edges_are_trimmed_and_braces_kept() {
    let block = "func (s *Set) Get(key string) bool {\n\tif s.items == nil {\n\t\treturn false\n\t}\n\treturn s.items[key]\n}";
    let stable = "var first = 1\nvar second = 2\nvar third = 3\nvar fourth = 4\nvar fifth = 5\nvar sixth = 6\nvar seventh = 7\nvar eighth = 8";
    let left = lines(&format!("package set\n\n{block}\n\n{stable}\n// end\n"));
    let right = lines(&format!("package set\n\n{stable}\n\n{block}\n// end\n"));
    let m = detect(&left, &right);
    assert_eq!(m.moves.len(), 1, "{m:?}");
    let mv = m.moves[0];
    assert_eq!(mv.left_len, 6);
    assert_eq!(left[mv.left_start + 5], "}");
    assert!(!left[mv.left_start].is_empty());
}

#[test]
fn trivial_only_and_small_blocks_are_not_moves() {
    // Only braces and blank lines change place.
    let left = [
        "a = compute_value()",
        "}",
        "",
        "}",
        "b = other_value()",
        "c = third()",
    ];
    let right = [
        "a = compute_value()",
        "b = other_value()",
        "}",
        "",
        "}",
        "c = third()",
    ];
    assert!(detect(&left, &right).moves.is_empty());
    // Two short lines: 12 alphanumerics in total, below 20.
    let left = [
        "let i = 0;",
        "let j = 0;",
        "for (;;) {",
        "  work(i, j);",
        "}",
    ];
    let right = [
        "for (;;) {",
        "let i = 0;",
        "let j = 0;",
        "  work(i, j);",
        "}",
    ];
    assert!(detect(&left, &right).moves.is_empty());
    // One non-trivial line under 40 alphanumerics.
    let left = ["log.info(\"total\", total)", "x = 1", "y = 2", "z = 3"];
    let right = ["x = 1", "y = 2", "z = 3", "log.info(\"total\", total)"];
    assert!(detect(&left, &right).moves.is_empty());
}

#[test]
fn single_long_line_moves() {
    let long = "from package.module import alpha, bravo, charlie, delta, echo, foxtrot";
    let left = ["import os", "import sys", "x = 1", long, "y = 2"];
    let right = [long, "import os", "import sys", "x = 1", "y = 2"];
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(3, 1, 0, 1, MoveKind::Pure)]);
}

#[test]
fn copies_and_in_place_rewrites_are_not_moves() {
    // The source stays (equal op), so the second copy is a plain insertion.
    let left = lines(&format!("{PARSE}\n\n{OTHER}\n"));
    let right = lines(&format!("{PARSE}\n\n{OTHER}\n\n{PARSE}\n"));
    assert!(detect(&left, &right).moves.is_empty());
    // A deletion and an insertion of equal lines inside one replace op.
    let left = [
        "start",
        "alpha bravo charlie delta",
        "echo foxtrot golf hotel",
        "end",
    ];
    let right = [
        "start",
        "alpha bravo charlie delta",
        "echo foxtrot golf hotel!",
        "end",
    ];
    let ops = diff_ops(&left, &right);
    assert!(ops.iter().any(|o| o.kind == OpKind::Replace));
    assert!(
        detect_moves(&left, &right, &ops, &MoveOptions::default())
            .moves
            .is_empty()
    );
}

#[test]
fn one_source_pairs_with_the_first_destination() {
    let left = lines(&format!("{PARSE}\n\n{OTHER}\nkeep_one()\nkeep_two()\n"));
    let right = lines(&format!(
        "{OTHER}\nkeep_one()\n{PARSE}\nkeep_two()\n{PARSE}\n"
    ));
    let m = detect(&left, &right);
    assert_eq!(m.moves.len(), 1, "{m:?}");
    assert_eq!(m.moves[0].right_start, 4);
}

#[test]
fn indentation_change_is_allowed_when_constant() {
    let left = [
        "def sync():",
        "    load_everything_now()",
        "    refresh_all_the_caches()",
        "    if ready:",
        "        go()",
    ];
    let right = [
        "def sync():",
        "    if ready:",
        "        load_everything_now()",
        "        refresh_all_the_caches()",
        "        go()",
    ];
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(1, 2, 2, 2, MoveKind::Pure)]);
    // Different deltas per line: no pure move under whitespace "none" ...
    let right_mixed = [
        "def sync():",
        "    if ready:",
        "        load_everything_now()",
        "          refresh_all_the_caches()",
        "        go()",
    ];
    let m = detect(&left, &right_mixed);
    assert!(m.moves.iter().all(|mv| mv.kind != MoveKind::Pure), "{m:?}");
    // ... but whitespace "change" does not check the delta.
    let change = MoveOptions {
        whitespace: WhitespaceMode::Change,
        ignore_case: false,
    };
    let ops = diff_ops(&left, &right_mixed);
    let m = detect_moves(&left, &right_mixed, &ops, &change);
    assert_eq!(spans(&m), [(1, 2, 2, 2, MoveKind::Pure)]);
}

#[test]
fn case_folding_shapes_the_key() {
    let left = [
        "SELECT name, email FROM customers",
        "WHERE active = TRUE",
        "x",
        "y",
    ];
    let right = [
        "x",
        "y",
        "select name, email from customers",
        "where active = true",
    ];
    assert!(
        detect(&left, &right)
            .moves
            .iter()
            .all(|m| m.kind != MoveKind::Pure)
    );
    let fold = MoveOptions {
        whitespace: WhitespaceMode::None,
        ignore_case: true,
    };
    let m = detect_with(&left, &right, fold);
    assert_eq!(spans(&m), [(0, 2, 2, 2, MoveKind::Pure)]);
}

#[test]
fn edited_move_with_renamed_variable() {
    let left = lines(
        "function clamp(v, lo, hi) {\n  return Math.min(Math.max(v, lo), hi);\n}\n\nfunction checksum(data) {\n  let acc = 0;\n  for (const byte of data) {\n    acc = (acc * 31 + byte) >>> 0;\n  }\n  return acc.toString(16).padStart(8, \"0\");\n}\n\nfunction chunk(list, size) {\n  return list.slice(0, size);\n}\n",
    );
    let right = lines(
        "function clamp(v, lo, hi) {\n  return Math.min(Math.max(v, lo), hi);\n}\n\nfunction chunk(list, size) {\n  return list.slice(0, size);\n}\n\nfunction checksum(data) {\n  let hash = 0;\n  for (const byte of data) {\n    hash = (hash * 31 + byte) >>> 0;\n  }\n  return hash.toString(16).padStart(8, \"0\");\n}\n",
    );
    let m = detect(&left, &right);
    // The line diff may match the final `}` with the one of chunk(); the
    // move then ends one line earlier.
    assert_eq!(m.moves.len(), 1, "{m:?}");
    let mv = m.moves[0];
    assert_eq!(
        (mv.left_start, mv.right_start, mv.kind),
        (4, 8, MoveKind::Edited)
    );
    assert!(mv.left_len >= 6 && mv.left_len == mv.right_len, "{m:?}");
    let sim = m.moves[0].similarity_bp;
    assert!((MOVE_EDITED_MIN_SIM_BP..10_000).contains(&sim), "{sim}");
}

#[test]
fn edited_move_with_an_extra_line() {
    let left = [
        "Shortbread",
        "Method:",
        "  1. Cream the butter and sugar.",
        "  2. Bake until golden.",
        "",
        "  - butter: 250 g",
        "  - flour: 360 g",
        "  - egg: 1",
        "  - salt: 2 g",
    ];
    let right = [
        "Shortbread",
        "  - butter: 250 g",
        "  - flour: 360 g",
        "  - sugar: 120 g",
        "  - egg: 1",
        "  - salt: 2 g",
        "",
        "Method:",
        "  1. Cream the butter and sugar.",
        "  2. Bake until golden.",
    ];
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(5, 4, 1, 5, MoveKind::Edited)], "{m:?}");
    // 4 perfect pairs over 4 + 5 non-trivial lines.
    assert_eq!(m.moves[0].similarity_bp, 2 * 40_000 / 9);
}

const TOTALS: [&str; 5] = [
    "def compute_totals(orders, tax_rate):",
    "    subtotal = sum(o.amount for o in orders)",
    "    tax = subtotal * tax_rate",
    "    discount = apply_discounts(orders, subtotal)",
    "    return subtotal + tax - discount",
];

fn context(name: &str) -> Vec<String> {
    (0..6)
        .map(|i| format!("{name} unchanged context line {i}"))
        .collect()
}

/// A moved block whose middle line was edited: `a + block + b + c` on the
/// left, `a + b + edited + c` on the right.
fn moved_with_middle(middle: &[&str]) -> (Vec<&'static str>, Vec<&'static str>) {
    let (a, b, c) = (context("alpha"), context("beta"), context("gamma"));
    let mut edited: Vec<String> = TOTALS.iter().map(|s| (*s).to_owned()).collect();
    edited.splice(2..3, middle.iter().map(|s| (*s).to_owned()));
    let left = [
        a.clone(),
        TOTALS.map(str::to_owned).to_vec(),
        b.clone(),
        c.clone(),
    ]
    .concat();
    let right = [a, b, edited, c].concat();
    (
        lines(&(left.join("\n") + "\n")),
        lines(&(right.join("\n") + "\n")),
    )
}

#[test]
fn edited_middle_line_joins_the_pure_pieces() {
    // Lines 1-2 and 4-5 are pure pieces; the edited line 3 joins them into
    // one edited move instead of two pure ones.
    let (left, right) = moved_with_middle(&["    tax = round(subtotal * tax_rate, 2)"]);
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(6, 5, 12, 5, MoveKind::Edited)], "{m:?}");
    assert!((MOVE_EDITED_MIN_SIM_BP..10_000).contains(&m.moves[0].similarity_bp));
    // Trailing whitespace on the middle line under `none`.
    let (left, right) = moved_with_middle(&["    tax = subtotal * tax_rate  "]);
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(6, 5, 12, 5, MoveKind::Edited)], "{m:?}");
    // A rewritten middle line is an unpaired line on each side: 4 of 5
    // lines kept is a block similarity of exactly 0.80, still one move.
    let (left, right) = moved_with_middle(&["    raise NotImplementedError('totals moved')"]);
    let m = detect(&left, &right);
    assert_eq!(spans(&m), [(6, 5, 12, 5, MoveKind::Edited)], "{m:?}");
    assert_eq!(m.moves[0].similarity_bp, MOVE_EDITED_MIN_SIM_BP);
    // Two unrelated lines in its place: 2 * 40000 / 11 < 8000, two pure pieces.
    let (left, right) = moved_with_middle(&[
        "    raise NotImplementedError('totals moved')",
        "    log.warning('compute_totals is deprecated')",
    ]);
    let m = detect(&left, &right);
    assert_eq!(
        spans(&m),
        [(6, 2, 12, 2, MoveKind::Pure), (9, 2, 16, 2, MoveKind::Pure)],
        "{m:?}"
    );
}

#[test]
fn pieces_split_only_by_indentation_stay_apart() {
    // The second half of the block gains two more spaces than the first: the
    // indentation rule splits it, and joining needs a real edit.
    let left = [
        "alpha context line zero",
        "first_moved_call(alpha, beta)",
        "second_moved_call(gamma, delta)",
        "third_moved_call(alpha, beta)",
        "fourth_moved_call(gamma, delta)",
        "beta context line zero",
        "beta context line one",
    ];
    let right = [
        "alpha context line zero",
        "beta context line zero",
        "beta context line one",
        "  first_moved_call(alpha, beta)",
        "  second_moved_call(gamma, delta)",
        "    third_moved_call(alpha, beta)",
        "    fourth_moved_call(gamma, delta)",
    ];
    let m = detect(&left, &right);
    assert_eq!(
        spans(&m),
        [(1, 2, 3, 2, MoveKind::Pure), (3, 2, 5, 2, MoveKind::Pure)],
        "{m:?}"
    );
}

#[test]
fn large_unrelated_churn_does_not_switch_off_edited_moves() {
    // 1100 changed lines per side in their own ops: 1.2 M line pairs in
    // total, but the small segment pairs holding the move are compared first.
    let ed: Vec<String> = TOTALS
        .iter()
        .map(|l| l.replace("orders", "items"))
        .collect();
    let churn_l: Vec<String> = (0..1_100)
        .map(|i| format!("left churn {i} {}", i * 7919))
        .collect();
    let churn_r: Vec<String> = (0..1_100)
        .map(|i| format!("right churn {i} {}", i * 104_729))
        .collect();
    let (a, b, c, d) = (
        context("alpha"),
        context("beta"),
        context("gamma"),
        context("delta"),
    );
    let left = [
        a.clone(),
        TOTALS.map(str::to_owned).to_vec(),
        b.clone(),
        churn_l,
        c.clone(),
        d.clone(),
    ]
    .concat();
    let right = [a, b, c, churn_r, d, ed].concat();
    let left: Vec<&str> = left.iter().map(String::as_str).collect();
    let right: Vec<&str> = right.iter().map(String::as_str).collect();
    let m = detect(&left, &right);
    assert!(m.skipped_limit);
    assert_eq!(m.moves.len(), 1, "{m:?}");
    let mv = m.moves[0];
    assert_eq!(
        (mv.left_start, mv.left_len, mv.kind),
        (6, 5, MoveKind::Edited)
    );
    assert_eq!(
        right[mv.right_start],
        "def compute_totals(items, tax_rate):"
    );
}

#[test]
fn move_plus_unrelated_edits() {
    let render = "    def render(self):\n        lines = [str(item) for item in self.items]\n        header = self.title.upper()\n        footer = self.summary()\n        return header + footer";
    let left = lines(&format!(
        "class Cart:\n    def add(self, item):\n        raise ValueError('bad item')\n{PARSE}\n{render}\n    def tax(self, rate):\n        return rate * 2\n"
    ));
    let right = lines(&format!(
        "class Cart:\n    def add(self, item):\n        raise ValueError('item is not valid')\n{render}\n{PARSE}\n    def tax(self, rate=0.2):\n        return rate * 2\n    def total_with_tax(self):\n        return self.total() * 3\n"
    ));
    let m = detect(&left, &right);
    assert_eq!(m.moves.len(), 1, "{m:?}");
    assert_eq!(m.moves[0].kind, MoveKind::Pure);
    assert_eq!(left[m.moves[0].left_start], "def parse_config(path):");
}

#[test]
fn several_moves_are_sorted_and_one_to_one() {
    let a = "alpha_function_one(first_argument)\nalpha_function_two(second_argument)";
    let b = "bravo_function_one(first_argument)\nbravo_function_two(second_argument)";
    let c = "charlie_function_one(first_argument)\ncharlie_function_two(second_argument)";
    let left = lines(&format!("{a}\nstay_1\n{b}\nstay_2\n{c}\nstay_3\n"));
    let right = lines(&format!("stay_1\n{c}\nstay_2\n{a}\n{b}\nstay_3\n"));
    let m = detect(&left, &right);
    assert!(
        m.moves
            .windows(2)
            .all(|w| (w[0].left_start, w[0].right_start) < (w[1].left_start, w[1].right_start))
    );
    let mut used_left = vec![false; left.len()];
    let mut used_right = vec![false; right.len()];
    for mv in &m.moves {
        for used in &mut used_left[mv.left_start..mv.left_start + mv.left_len] {
            assert!(!*used);
            *used = true;
        }
        for used in &mut used_right[mv.right_start..mv.right_start + mv.right_len] {
            assert!(!*used);
            *used = true;
        }
    }
    assert!(m.moves.len() >= 2, "{m:?}");
}

#[test]
fn many_identical_short_lines_stay_fast_and_move_free() {
    // 20000 copies of the same short lines, swapped around a marker so the
    // line diff deletes and re-inserts one half.
    let mut left = Vec::new();
    let mut right = Vec::new();
    for _ in 0..5_000 {
        left.extend(["if err != nil {", "  return err", "}", ""]);
    }
    left.push("marker line that stays in place");
    for _ in 0..5_000 {
        right.extend(["x = x + 1", "y = y + 1"]);
    }
    let mut right_full = right.clone();
    right_full.push("marker line that stays in place");
    right_full.extend(left[..20_000].iter().copied());
    left.extend(right.iter().copied());
    let start = std::time::Instant::now();
    let m = detect(&left, &right_full);
    assert!(start.elapsed().as_secs() < 20);
    // Every key occurs more than MOVE_MAX_KEY_OCCURRENCES times: no seeds.
    assert!(m.moves.is_empty(), "{:?}", m.moves.len());
}

#[test]
fn many_small_candidate_blocks() {
    // 3000 small distinct blocks, each two lines with 11 + 11 alphanumerics,
    // moved as a whole: each block alone qualifies, every one is found once.
    let mut block = Vec::new();
    for i in 0..3_000 {
        block.push(format!("value_{i:05} = 1"));
        block.push(format!("other_{i:05} = 2"));
    }
    let head: Vec<String> = (0..10).map(|i| format!("head line {i}")).collect();
    let mut left: Vec<&str> = block.iter().map(String::as_str).collect();
    left.extend(head.iter().map(String::as_str));
    let mut right: Vec<&str> = head.iter().map(String::as_str).collect();
    right.extend(block.iter().map(String::as_str));
    let start = std::time::Instant::now();
    let m = detect(&left, &right);
    assert!(start.elapsed().as_secs() < 20);
    // The line diff keeps the larger block in place and moves the head.
    let moved: usize = m.moves.iter().map(|m| m.left_len).sum();
    assert!(moved >= 10, "{m:?}");
    assert!(!m.skipped_limit);
}

#[test]
fn similar_lines_everywhere_hit_the_edited_budget_not_the_clock() {
    // 1500 near-identical long lines on both sides in different ops: every
    // pair is similar. The edited pass is skipped by its pair budget.
    let left_block: Vec<String> = (0..1_500)
        .map(|i| format!("logger.info(\"processing record number {i:06} of the batch\")"))
        .collect();
    let right_block: Vec<String> = (0..1_500)
        .map(|i| format!("logger.info(\"processing record number {i:06} of the batches\")"))
        .collect();
    let mut left: Vec<&str> = left_block.iter().map(String::as_str).collect();
    left.push("anchor");
    let mut right = vec!["anchor"];
    right.extend(right_block.iter().map(String::as_str));
    let start = std::time::Instant::now();
    let m = detect(&left, &right);
    assert!(start.elapsed().as_secs() < 20);
    assert!(m.skipped_limit);
    // Smaller: 300 x 300 runs the edited pass within budget.
    let mut left: Vec<&str> = left_block[..300].iter().map(String::as_str).collect();
    left.push("anchor");
    let mut right = vec!["anchor"];
    right.extend(right_block[..300].iter().map(String::as_str));
    let start = std::time::Instant::now();
    let m = detect(&left, &right);
    assert!(start.elapsed().as_secs() < 20);
    assert!(!m.skipped_limit);
    assert!(!m.moves.is_empty());
}

#[test]
fn too_many_changed_lines_skip_detection() {
    let left: Vec<String> = (0..=MOVE_MAX_CANDIDATE_LINES)
        .map(|i| format!("l{i}"))
        .collect();
    let left: Vec<&str> = left.iter().map(String::as_str).collect();
    let ops = vec![LineOp {
        kind: OpKind::Delete,
        left_start: 0,
        left_len: left.len(),
        right_start: 0,
        right_len: 0,
    }];
    let m = detect_moves(&left, &[], &ops, &MoveOptions::default());
    assert!(m.skipped_limit);
    assert!(m.moves.is_empty());
}

#[test]
fn deterministic_output() {
    let left = lines(&format!("{PARSE}\n\n{OTHER}\nx\n"));
    let right = lines(&format!("{OTHER}\nx\n\n{PARSE}\n"));
    let first = detect(&left, &right);
    for _ in 0..5 {
        assert_eq!(detect(&left, &right), first);
    }
}

/// Plain dynamic-programming Levenshtein distance for cross-checking.
fn naive(a: &[u32], b: &[u32]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(x != y))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[test]
fn bit_parallel_distance_matches_dynamic_programming() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = |n: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % n
    };
    let alphabet = 6;
    let mut peq = vec![0u64; alphabet * WORDS];
    for round in 0..400 {
        let la = 1 + usize::try_from(next(if round % 4 == 0 { 256 } else { 80 })).unwrap();
        let lb = usize::try_from(next(260)).unwrap();
        let a: Vec<u32> = (0..la).map(|_| u32::try_from(next(6)).unwrap()).collect();
        let mut b: Vec<u32> = (0..lb).map(|_| u32::try_from(next(6)).unwrap()).collect();
        if round % 3 == 0 {
            // A near copy, to exercise small distances too.
            b = a.clone();
            for _ in 0..next(5) {
                let at = usize::try_from(next(b.len() as u64 + 1)).unwrap();
                b.insert(at.min(b.len()), u32::try_from(next(6)).unwrap());
            }
        }
        for (p, &g) in a.iter().enumerate() {
            peq[g as usize * WORDS + p / 64] |= 1 << (p % 64);
        }
        assert_eq!(
            myers_distance(&peq, a.len(), &b),
            naive(&a, &b),
            "round {round}"
        );
        peq.fill(0);
    }
}

#[test]
fn delta_rules() {
    assert_eq!(delta("  ", "    "), Some(Delta::Add("  ")));
    assert_eq!(delta("\t  ", "\t\t  "), Some(Delta::Add("\t")));
    assert_eq!(delta("    ", "  "), Some(Delta::Remove("  ")));
    assert_eq!(delta("", ""), Some(Delta::Add("")));
    assert_eq!(delta("\t", "  "), None);
}

#[test]
fn many_edited_blocks_in_one_op_pair() {
    // 80 three-line functions, each edited on its middle line and shuffled
    // into one big insertion: every block is an edited move. Selecting one
    // block must not force a re-search of the whole op pair.
    let block = |i: usize, verb: &str| {
        vec![
            format!("def handler_{i}(request, response):"),
            format!("    value_{i} = {verb}(request.args['key_{i}'])"),
            format!("    return response.send(value_{i})"),
            format!("noise {}", i * 7919 % 1000),
        ]
    };
    let mut left_owned = Vec::new();
    for i in 0..80 {
        left_owned.extend(block(i, "compute"));
    }
    left_owned.push("anchor".to_owned());
    let mut right_owned = vec!["anchor".to_owned()];
    for k in 0..80 {
        let i = (k * 37) % 80;
        let mut b = block(i, "calculate");
        b[3] = format!("other {k}");
        right_owned.extend(b);
    }
    let left: Vec<&str> = left_owned.iter().map(String::as_str).collect();
    let right: Vec<&str> = right_owned.iter().map(String::as_str).collect();
    let m = detect(&left, &right);
    assert!(!m.skipped_limit);
    assert_eq!(m.moves.len(), 80);
    assert!(
        m.moves
            .iter()
            .all(|mv| mv.kind == MoveKind::Edited && mv.left_len == 3)
    );
}
