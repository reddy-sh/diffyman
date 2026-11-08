//! Golden-corpus conformance runner (Diff Document spec §3; moves spec §5).
//!
//! Each case is a folder `tests/corpus/<group>/<case>/` with `left.*`,
//! `right.*`, `options.json` and `expected.json`; the groups are `text` and
//! `moved`. For every case the runner diffs the inputs (passing the file names,
//! such as `left.txt`, as input names), removes the top-level `provenance` key
//! and compares the RFC 8785 bytes with `expected.json`, which is canonicalized
//! the same way first (so it may be pretty-printed).
//!
//! `--bless` writes `expected.json` as the exact canonical bytes **without a
//! trailing newline**. A missing `expected.json` is reported as `new`, which
//! fails only under `--strict`. A document with an `approximate` warning is
//! never a valid expected result.
//!
//! `moved` cases also carry `labels.json` (human ground truth). The runner
//! matches predicted moves against labels (moves spec §5.3), prints precision
//! and recall per case, per category and in total, and checks the gate of
//! moves spec §5.4 ([`gate_failures`]).

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use diffyman::model::{DiffDocument, MoveKind, WarningCode, jcs};
use diffyman::{Input, model};
use serde_json::Value;

/// Groups of cases, in run order.
pub const GROUPS: [&str; 2] = ["text", "moved"];

/// How a case ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Output equals `expected.json`.
    Pass,
    /// Output differs from `expected.json`.
    Fail(String),
    /// No `expected.json` yet.
    New,
    /// `expected.json` was written.
    Blessed,
    /// The case could not be run.
    Error(String),
}

/// The result of one case.
#[derive(Debug, Clone)]
pub struct CaseResult {
    /// `group/case`.
    pub name: String,
    /// Outcome.
    pub outcome: Outcome,
    /// Move metrics, for `moved` cases with valid labels.
    pub metrics: Option<Evaluation>,
}

/// Runner settings.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Write `expected.json` for every case.
    pub bless: bool,
}

/// Lists case folders of `root/<group>/`, sorted by name.
pub fn list_cases(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut cases = Vec::new();
    for group in GROUPS {
        let dir = root.join(group);
        if !dir.is_dir() {
            continue;
        }
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        for path in entries {
            let case = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            cases.push((format!("{group}/{case}"), path));
        }
    }
    Ok(cases)
}

/// Runs one case folder.
pub fn run_case(name: &str, dir: &Path, options: &RunOptions) -> CaseResult {
    let mut result = CaseResult {
        name: name.to_owned(),
        outcome: Outcome::Pass,
        metrics: None,
    };
    let doc = match diff_case(dir) {
        Ok(doc) => doc,
        Err(e) => {
            result.outcome = Outcome::Error(e);
            return result;
        }
    };
    if name.starts_with("moved/") {
        match load_labels(&dir.join("labels.json")) {
            Ok(labels) => result.metrics = Some(evaluate(&predictions(&doc), &labels)),
            Err(e) => {
                result.outcome = Outcome::Error(format!("labels.json: {e}"));
                return result;
            }
        }
    }
    result.outcome = compare_or_bless(dir, &doc, options);
    result
}

fn find_input(dir: &Path, stem: &str) -> Result<PathBuf, String> {
    let prefix = format!("{stem}.");
    let mut found: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("no {stem}.* file")),
        _ => Err(format!("more than one {stem}.* file")),
    }
}

/// Diffs a case's inputs with its options.
pub fn diff_case(dir: &Path) -> Result<DiffDocument, String> {
    let left = find_input(dir, "left")?;
    let right = find_input(dir, "right")?;
    let options_path = dir.join("options.json");
    let options_text =
        fs::read_to_string(&options_path).map_err(|e| format!("options.json: {e}"))?;
    let options =
        diffyman::parse_options(&options_text).map_err(|e| format!("options.json: {e}"))?;
    let left_bytes = fs::read(&left).map_err(|e| e.to_string())?;
    let right_bytes = fs::read(&right).map_err(|e| e.to_string())?;
    let left_name = file_name(&left);
    let right_name = file_name(&right);
    diffyman::diff_document(
        Input::named(&left_bytes, &left_name),
        Input::named(&right_bytes, &right_name),
        &options,
    )
    .map_err(|e| e.to_string())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn compare_or_bless(dir: &Path, doc: &DiffDocument, options: &RunOptions) -> Outcome {
    if doc
        .warnings
        .iter()
        .any(|w| w.code == WarningCode::Approximate)
    {
        return Outcome::Fail("the result has an `approximate` warning".to_owned());
    }
    let actual = match model::to_conformance_json(doc) {
        Ok(text) => text,
        Err(e) => return Outcome::Error(e.to_string()),
    };
    let path = dir.join("expected.json");
    if options.bless {
        return match fs::write(&path, actual.as_bytes()) {
            Ok(()) => Outcome::Blessed,
            Err(e) => Outcome::Error(format!("cannot write expected.json: {e}")),
        };
    }
    let Ok(expected_text) = fs::read_to_string(&path) else {
        return Outcome::New;
    };
    let expected = match canonical_expected(&expected_text) {
        Ok(text) => text,
        Err(e) => return Outcome::Error(format!("expected.json: {e}")),
    };
    if expected == actual {
        Outcome::Pass
    } else {
        Outcome::Fail(first_difference(&expected, &actual))
    }
}

/// Canonicalizes an expected document, dropping `provenance` if present.
pub fn canonical_expected(text: &str) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Value::Object(map) = &mut value {
        map.remove("provenance");
    }
    jcs::canonicalize(&value).map_err(|e| e.to_string())
}

/// Describes where two canonical texts first differ.
pub fn first_difference(expected: &str, actual: &str) -> String {
    let at = expected
        .bytes()
        .zip(actual.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let snippet = |s: &str| -> String {
        let start = floor_char_boundary(s, at.saturating_sub(40));
        let end = floor_char_boundary(s, (at + 40).min(s.len()));
        s[start..end].to_owned()
    };
    format!(
        "first difference at byte {at}\n    expected: …{}…\n    actual:   …{}…",
        snippet(expected),
        snippet(actual)
    )
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A line range `{start, count}` (1-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelRange {
    /// First line.
    pub start: u64,
    /// Number of lines.
    pub count: u64,
}

/// A labeled or predicted move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveSpec {
    /// Left range.
    pub left: LabelRange,
    /// Right range.
    pub right: LabelRange,
    /// `true` for `edited`, `false` for `pure`.
    pub edited: bool,
}

/// Lines as the Diff Document counts them (spec §7.2): LF, CRLF and a lone CR
/// each end a line. Labels use the same numbering.
pub fn label_line_count(bytes: &[u8]) -> u64 {
    let text = diffyman::decode_text(bytes).unwrap_or_default();
    diffyman::split_lines(text).len() as u64
}

/// Loads and validates `labels.json` (moves spec §5.2).
pub fn load_labels(path: &Path) -> Result<Vec<MoveSpec>, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let count_of = |stem: &str| -> Result<u64, String> {
        let file = find_input(dir, stem)?;
        Ok(label_line_count(
            &fs::read(file).map_err(|e| e.to_string())?,
        ))
    };
    parse_labels(&text, count_of("left")?, count_of("right")?)
}

/// Parses and validates labels against the files' line counts.
pub fn parse_labels(
    text: &str,
    left_lines: u64,
    right_lines: u64,
) -> Result<Vec<MoveSpec>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let top = value.as_object().ok_or("top level must be an object")?;
    for key in top.keys() {
        if key != "moves" && key != "notes" {
            return Err(format!("unknown key `{key}`"));
        }
    }
    if let Some(notes) = top.get("notes")
        && !notes.is_string()
    {
        return Err("`notes` must be a string".to_owned());
    }
    let moves = top
        .get("moves")
        .and_then(Value::as_array)
        .ok_or("`moves` must be an array")?;
    let mut out = Vec::with_capacity(moves.len());
    for (i, m) in moves.iter().enumerate() {
        let obj = m
            .as_object()
            .ok_or(format!("moves[{i}] must be an object"))?;
        if obj.len() != 3
            || !["left", "right", "kind"]
                .iter()
                .all(|k| obj.contains_key(*k))
        {
            return Err(format!("moves[{i}] must have exactly left, right and kind"));
        }
        let range = |key: &str, lines: u64| -> Result<LabelRange, String> {
            let r = obj[key]
                .as_object()
                .ok_or(format!("moves[{i}].{key} must be an object"))?;
            if r.len() != 2 {
                return Err(format!(
                    "moves[{i}].{key} must have exactly start and count"
                ));
            }
            let get = |k: &str| {
                r.get(k)
                    .and_then(Value::as_u64)
                    .ok_or(format!("moves[{i}].{key}.{k} must be an integer"))
            };
            let (start, count) = (get("start")?, get("count")?);
            if start < 1 || count < 1 || start + count - 1 > lines {
                return Err(format!("moves[{i}].{key} is out of range"));
            }
            Ok(LabelRange { start, count })
        };
        let edited = match obj["kind"].as_str() {
            Some("pure") => false,
            Some("edited") => true,
            _ => return Err(format!("moves[{i}].kind must be \"pure\" or \"edited\"")),
        };
        out.push(MoveSpec {
            left: range("left", left_lines)?,
            right: range("right", right_lines)?,
            edited,
        });
    }
    for pair in out.windows(2) {
        if (pair[0].left.start, pair[0].right.start) > (pair[1].left.start, pair[1].right.start) {
            return Err("moves must be sorted by left.start, then right.start".to_owned());
        }
    }
    for (i, a) in out.iter().enumerate() {
        for b in &out[i + 1..] {
            if overlap(a.left, b.left) > 0 || overlap(a.right, b.right) > 0 {
                return Err("label ranges overlap on one side".to_owned());
            }
        }
    }
    Ok(out)
}

/// Predicted moves from a document's `moves[]`.
pub fn predictions(doc: &DiffDocument) -> Vec<MoveSpec> {
    doc.moves
        .iter()
        .map(|m| MoveSpec {
            left: LabelRange {
                start: m.from.range.start,
                count: m.from.range.count,
            },
            right: LabelRange {
                start: m.to.range.start,
                count: m.to.range.count,
            },
            edited: m.kind == MoveKind::Edited,
        })
        .collect()
}

fn overlap(a: LabelRange, b: LabelRange) -> u64 {
    let start = a.start.max(b.start);
    let end = (a.start + a.count).min(b.start + b.count);
    end.saturating_sub(start)
}

/// Jaccard index of two ranges as a fraction `(intersection, union)`.
fn jaccard(a: LabelRange, b: LabelRange) -> (u64, u64) {
    let inter = overlap(a, b);
    (inter, a.count + b.count - inter)
}

/// Which labels count (moves spec §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    /// `edited` labels are out of scope.
    Pure,
    /// All labels count.
    Edited,
}

/// True-positive, false-positive and false-negative counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Matched pairs.
    pub tp: u64,
    /// Unmatched predictions.
    pub fp: u64,
    /// Unmatched labels.
    pub fn_: u64,
}

impl Counts {
    /// Adds another case's counts.
    pub fn add(&mut self, other: Counts) {
        self.tp += other.tp;
        self.fp += other.fp;
        self.fn_ += other.fn_;
    }

    /// TP / (TP + FP), or `None` without predictions.
    pub fn precision(&self) -> Option<f64> {
        ratio(self.tp, self.tp + self.fp)
    }

    /// TP / (TP + FN), or `None` without labels.
    pub fn recall(&self) -> Option<f64> {
        ratio(self.tp, self.tp + self.fn_)
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(num: u64, den: u64) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

/// Matches predictions to labels one-to-one (Jaccard ≥ 0.5 on both sides,
/// greedy by the larger minimum-side Jaccard; ties by label order, then
/// prediction order) and counts the result.
pub fn score(predicted: &[MoveSpec], labels: &[MoveSpec], mode: MoveMode) -> Counts {
    let label_match = match_moves(predicted, labels);
    let in_scope = |label: &MoveSpec| mode == MoveMode::Edited || !label.edited;
    let mut counts = Counts::default();
    let mut pred_matched = vec![false; predicted.len()];
    for (label, m) in labels.iter().zip(&label_match) {
        if let Some(p) = m {
            pred_matched[*p] = true;
        }
        match m {
            Some(_) if in_scope(label) => counts.tp += 1,
            None if in_scope(label) => counts.fn_ += 1,
            _ => {}
        }
    }
    counts.fp = pred_matched.iter().filter(|m| !**m).count() as u64;
    counts
}

/// Move metrics of one case or a sum of cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Evaluation {
    /// All labels count (moves spec §5.3, edited moves shipped).
    pub all: Counts,
    /// `edited` labels left out (the pure-only scope).
    pub pure_only: Counts,
    /// Matched pairs whose predicted kind equals the label's kind.
    pub kind_ok: u64,
    /// Matched pairs whose ranges equal the label's on both sides.
    pub exact: u64,
    /// Number of predictions.
    pub predicted: u64,
    /// Number of labels.
    pub labels: u64,
}

impl Evaluation {
    /// Adds another case's metrics.
    pub fn add(&mut self, other: &Evaluation) {
        self.all.add(other.all);
        self.pure_only.add(other.pure_only);
        self.kind_ok += other.kind_ok;
        self.exact += other.exact;
        self.predicted += other.predicted;
        self.labels += other.labels;
    }
}

/// Scores one case in both scopes, with kind accuracy and exact boundaries.
pub fn evaluate(predicted: &[MoveSpec], labels: &[MoveSpec]) -> Evaluation {
    let label_match = match_moves(predicted, labels);
    let mut eval = Evaluation {
        all: score(predicted, labels, MoveMode::Edited),
        pure_only: score(predicted, labels, MoveMode::Pure),
        predicted: predicted.len() as u64,
        labels: labels.len() as u64,
        ..Evaluation::default()
    };
    for (label, m) in labels.iter().zip(&label_match) {
        if let Some(p) = m {
            let pred = &predicted[*p];
            eval.kind_ok += u64::from(pred.edited == label.edited);
            eval.exact += u64::from(pred.left == label.left && pred.right == label.right);
        }
    }
    eval
}

/// The category of a `moved` case: the word after the number in its folder
/// name, such as `pure` for `moved/001-pure-function-down`.
pub fn category(name: &str) -> &str {
    let case = name.rsplit('/').next().unwrap_or(name);
    case.split('-').nth(1).unwrap_or("other")
}

/// Gate of moves spec §5.4 over all labels: precision >= 0.95, recall >= 0.90,
/// and no predictions in a case without labels. Returns the failures.
pub fn gate_failures(total: &Evaluation, negatives_with_moves: &[String]) -> Vec<String> {
    let mut failures = Vec::new();
    let c = total.all;
    if 100 * c.tp < 95 * (c.tp + c.fp) {
        failures.push(format!("precision {} < 0.95", fmt_ratio(c.precision())));
    }
    if 10 * c.tp < 9 * (c.tp + c.fn_) {
        failures.push(format!("recall {} < 0.90", fmt_ratio(c.recall())));
    }
    for name in negatives_with_moves {
        failures.push(format!("negative case {name} has moves"));
    }
    failures
}

/// The prediction matched to each label, if any (moves spec §5.3).
pub fn match_moves(predicted: &[MoveSpec], labels: &[MoveSpec]) -> Vec<Option<usize>> {
    // Candidate pairs (label index, prediction index, min-side Jaccard).
    let mut pairs: Vec<(usize, usize, (u64, u64))> = Vec::new();
    for (g, label) in labels.iter().enumerate() {
        for (p, pred) in predicted.iter().enumerate() {
            let jl = jaccard(pred.left, label.left);
            let jr = jaccard(pred.right, label.right);
            if 2 * jl.0 >= jl.1 && 2 * jr.0 >= jr.1 {
                let min =
                    if u128::from(jl.0) * u128::from(jr.1) <= u128::from(jr.0) * u128::from(jl.1) {
                        jl
                    } else {
                        jr
                    };
                pairs.push((g, p, min));
            }
        }
    }
    pairs.sort_by(|a, b| {
        let lhs = u128::from(b.2.0) * u128::from(a.2.1);
        let rhs = u128::from(a.2.0) * u128::from(b.2.1);
        lhs.cmp(&rhs).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1))
    });
    let mut label_match: Vec<Option<usize>> = vec![None; labels.len()];
    let mut pred_match: Vec<Option<usize>> = vec![None; predicted.len()];
    for (g, p, _) in pairs {
        if label_match[g].is_none() && pred_match[p].is_none() {
            label_match[g] = Some(p);
            pred_match[p] = Some(g);
        }
    }
    label_match
}

/// Formats an optional ratio with 4 decimals.
pub fn fmt_ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.4}"))
}

/// Formats a one-line metrics summary.
pub fn fmt_counts(c: &Counts) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "tp={} fp={} fn={} precision={} recall={}",
        c.tp,
        c.fp,
        c.fn_,
        fmt_ratio(c.precision()),
        fmt_ratio(c.recall())
    );
    s
}

#[cfg(test)]
mod tests;
