#!/usr/bin/env python3
"""Generate the labeled moved-block cases under tests/corpus/moved/.

Each case is built from named segments. The left and right files are the
concatenation of their segments, and every label is computed from segment
positions, so line numbers never need to be counted by hand. The labels are
the human ground truth described in docs/spec/text-granularity-and-moves.md
section 5; check them with tools/check_labels.py after generating.

Usage (from the repository root):

    uv run --python 3.12 python tests/corpus/tools/make_moved_cases.py

The script rewrites left.txt, right.txt, options.json and labels.json of
every case and never touches expected.json. All content is original and
written for the diffyman corpus.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "moved"

OPTIONS = {
    "algorithm": "histogram",
    "granularity": "line",
    "ignore": {"whitespace": "none", "case": False, "blankLines": False, "regex": []},
    "context": 3,
    "positionEncoding": "utf-8",
    "detectMoves": True,
}


def b(text: str) -> list[str]:
    """Turn a triple-quoted block into a list of lines (no terminators).

    A single leading newline is dropped so blocks can start on the line after
    the opening quotes. Use BLANK for an empty line segment.
    """
    if text.startswith("\n"):
        text = text[1:]
    if not text.endswith("\n"):
        raise ValueError("block must end with a newline")
    return text[:-1].split("\n")


BLANK = [""]

CASES: list[dict] = []


def case(slug, left, right, moves, notes, *, crlf=False):
    """Register a case.

    left/right: lists of (segment name, lines). Names must be unique per side.
    moves: list of (left segment names, right segment names, kind); a tuple of
    names spans consecutive segments.
    """
    CASES.append(
        {"slug": slug, "left": left, "right": right, "moves": moves, "notes": notes, "crlf": crlf}
    )


# ---------------------------------------------------------------------------
# Pure multi-line moves
# ---------------------------------------------------------------------------

PY_HEADER = b("""
import json
import os
import sys
""")
PY_PARSE = b("""
def parse_config(path):
  with open(path, encoding="utf-8") as handle:
    raw = json.load(handle)
  if "version" not in raw:
    raise ValueError("config file has no version field")
  raw.setdefault("workers", os.cpu_count())
  raw.setdefault("log_level", "info")
  return raw
""")
PY_LOAD = b("""
def load_jobs(directory):
  jobs = []
  for name in sorted(os.listdir(directory)):
    if not name.endswith(".job"):
      continue
    full = os.path.join(directory, name)
    with open(full, encoding="utf-8") as handle:
      jobs.append(handle.read().strip())
  print(f"loaded {len(jobs)} jobs from {directory}")
  return jobs
""")
PY_MAIN = b("""
def main(argv):
  if len(argv) < 3:
    print("usage: runner CONFIG JOBDIR", file=sys.stderr)
    return 2
  config = parse_config(argv[1])
  jobs = load_jobs(argv[2])
  for index, job in enumerate(jobs, start=1):
    if index > config["workers"] * 4:
      print("queue limit reached, stopping early")
      break
    print(f"[{index}] {job}")
  return 0
""")
PY_TAIL = b("""

if __name__ == "__main__":
  sys.exit(main(sys.argv))
""")
case(
    "001-pure-function-down",
    [("hdr", PY_HEADER), ("b0", BLANK), ("parse", PY_PARSE), ("b1", BLANK), ("load", PY_LOAD),
     ("b2", BLANK), ("main", PY_MAIN), ("tail", PY_TAIL)],
    [("hdr", PY_HEADER), ("b0", BLANK), ("load", PY_LOAD), ("b2", BLANK), ("main", PY_MAIN),
     ("b1", BLANK), ("parse", PY_PARSE), ("tail", PY_TAIL)],
    [("parse", "parse", "pure")],
    "parse_config() moved from the top of the module to just above the __main__ guard; "
    "the blank separator lines are not part of the label.",
)

RS_USE = b("""
use std::collections::HashMap;
use std::fmt::Write;
""")
RS_STRUCT = b("""
pub struct Page {
  title: String,
  sections: Vec<String>,
  meta: HashMap<String, String>,
}
""")
RS_IMPL = b("""
impl Page {
  pub fn new(title: &str) -> Self {
    Page {
      title: title.to_string(),
      sections: Vec::new(),
      meta: HashMap::new(),
    }
  }

  pub fn add_section(&mut self, body: &str) {
    self.sections.push(body.to_string());
  }

  pub fn render(&self) -> String {
    let mut out = String::new();
    writeln!(out, "<h1>{}</h1>", escape_html(&self.title)).unwrap();
    for section in &self.sections {
      writeln!(out, "<section>{}</section>", escape_html(section)).unwrap();
    }
    out
  }
}
""")
RS_ESCAPE = b("""
fn escape_html(input: &str) -> String {
  let mut escaped = String::with_capacity(input.len());
  for ch in input.chars() {
    match ch {
      '<' => escaped.push_str("&lt;"),
      '>' => escaped.push_str("&gt;"),
      '&' => escaped.push_str("&amp;"),
      _ => escaped.push(ch),
    }
  }
  escaped
}
""")
case(
    "002-pure-function-up",
    [("use", RS_USE), ("b0", BLANK), ("struct", RS_STRUCT), ("b1", BLANK), ("impl", RS_IMPL),
     ("b2", BLANK), ("escape", RS_ESCAPE)],
    [("use", RS_USE), ("b0", BLANK), ("escape", RS_ESCAPE), ("b2", BLANK), ("struct", RS_STRUCT),
     ("b1", BLANK), ("impl", RS_IMPL)],
    [("escape", "escape", "pure")],
    "escape_html() moved from the end of the file to just below the use lines, above "
    "the struct and impl that call it.",
)

MD_TITLE = b("""
# Harbour Street community garden
""")
MD_P1 = b("""
The garden opened in the spring of 2019 on a lot that had been empty for
eleven years. Volunteers cleared the rubble by hand over six weekends and
built the first twelve raised beds from reclaimed scaffold boards.
""")
MD_P2 = b("""
Plots are shared between two households each season. A household may keep
the same plot for three seasons in a row, after which it moves to the end
of the waiting list so that newcomers get a fair chance.
""")
MD_P3 = b("""
Tools live in the green shed by the east gate. Please clean the spades and
forks before you hang them up, and write any broken tool on the board so
the maintenance team can repair it on the first Saturday of the month.
""")
MD_P4 = b("""
Water comes from two rain barrels and a standpipe. In dry spells the
standpipe is opened from seven to nine in the morning only, and hoses must
not be left running unattended.
""")
MD_P5 = b("""
New members are always welcome. Come to the open morning on the second
Sunday of each month, meet the coordinators, and put your name down for a
plot or for one of the shared herb beds.
""")
case(
    "003-pure-paragraph-prose",
    [("title", MD_TITLE), ("b0", BLANK), ("p1", MD_P1), ("b1", BLANK), ("p2", MD_P2), ("b2", BLANK),
     ("p3", MD_P3), ("b3", BLANK), ("p4", MD_P4), ("b4", BLANK), ("p5", MD_P5)],
    [("title", MD_TITLE), ("b0", BLANK), ("p1", MD_P1), ("b4", BLANK), ("p5", MD_P5), ("b1", BLANK),
     ("p2", MD_P2), ("b2", BLANK), ("p3", MD_P3), ("b3", BLANK), ("p4", MD_P4)],
    [("p5", "p5", "pure")],
    "The welcome paragraph moved from the end of the page to second position, right "
    "after the history paragraph.",
)

_routes_names = [
    ("get", "/api/v1/users", "listUsers", "true"),
    ("post", "/api/v1/users", "createUser", "true"),
    ("get", "/api/v1/users/:id", "getUser", "true"),
    ("patch", "/api/v1/users/:id", "updateUser", "true"),
    ("delete", "/api/v1/users/:id", "deleteUser", "true"),
    ("get", "/api/v1/teams", "listTeams", "true"),
    ("post", "/api/v1/teams", "createTeam", "true"),
    ("get", "/api/v1/teams/:id/members", "listMembers", "true"),
    ("post", "/api/v1/teams/:id/members", "addMember", "true"),
    ("delete", "/api/v1/teams/:id/members/:uid", "removeMember", "true"),
    ("get", "/api/v1/projects", "listProjects", "true"),
    ("post", "/api/v1/projects", "createProject", "true"),
    ("get", "/api/v1/projects/:id", "getProject", "true"),
    ("put", "/api/v1/projects/:id", "replaceProject", "true"),
    ("delete", "/api/v1/projects/:id", "archiveProject", "true"),
    ("get", "/api/v1/projects/:id/tasks", "listTasks", "true"),
    ("post", "/api/v1/projects/:id/tasks", "createTask", "true"),
    ("patch", "/api/v1/tasks/:id", "updateTask", "true"),
    ("post", "/api/v1/tasks/:id/comments", "addComment", "true"),
    ("get", "/api/v1/tasks/:id/comments", "listComments", "true"),
    ("get", "/api/v1/search", "searchAll", "true"),
    ("get", "/api/v1/reports/weekly", "weeklyReport", "true"),
    ("get", "/api/v1/reports/monthly", "monthlyReport", "true"),
    ("post", "/api/v1/exports", "startExport", "true"),
    ("get", "/api/v1/exports/:id", "exportStatus", "true"),
    ("post", "/api/v1/login", "login", "false"),
    ("post", "/api/v1/logout", "logout", "true"),
    ("post", "/api/v1/password/reset", "resetPassword", "false"),
    ("get", "/healthz", "health", "false"),
    ("get", "/version", "version", "false"),
]
JS_ROUTES = (
    ["const routes = ["]
    + [f'  {{ method: "{m}", path: "{p}", handler: "{h}", auth: {a} }},' for m, p, h, a in _routes_names]
    + ["];"]
)
_handler_specs = [
    ("listUsers", "users", "list"), ("createUser", "users", "insert"),
    ("listTeams", "teams", "list"), ("createTeam", "teams", "insert"),
    ("listMembers", "members", "list"), ("addMember", "members", "insert"),
    ("listProjects", "projects", "list"), ("createProject", "projects", "insert"),
    ("listTasks", "tasks", "list"), ("createTask", "tasks", "insert"),
    ("listComments", "comments", "list"), ("addComment", "comments", "insert"),
    ("searchAll", "search_index", "list"), ("weeklyReport", "report_weekly", "list"),
    ("monthlyReport", "report_monthly", "list"), ("startExport", "exports", "insert"),
]
JS_HANDLERS: list[str] = []
for _name, _table, _kind in _handler_specs:
    if _kind == "list":
        JS_HANDLERS += [
            f"function {_name}(req, res) {{",
            f'  const rows = db.select("{_table}", req.query);',
            "  res.json({ ok: true, count: rows.length, rows });",
            "}",
            "",
        ]
    else:
        JS_HANDLERS += [
            f"function {_name}(req, res) {{",
            f'  const row = db.insert("{_table}", req.body);',
            "  res.status(201).json({ ok: true, row });",
            "}",
            "",
        ]
JS_HANDLERS = JS_HANDLERS[:-1]
JS_HEAD = b("""
"use strict";

const db = require("./db");
""")
JS_TAIL = b("""

module.exports = { routes };
""")
case(
    "004-pure-large-block",
    [("head", JS_HEAD), ("b0", BLANK), ("routes", JS_ROUTES), ("b1", BLANK), ("handlers", JS_HANDLERS),
     ("tail", JS_TAIL)],
    [("head", JS_HEAD), ("b0", BLANK), ("handlers", JS_HANDLERS), ("b1", BLANK), ("routes", JS_ROUTES),
     ("tail", JS_TAIL)],
    [("routes", "routes", "pure")],
    "A 32-line route table moved from above the 79 lines of handler functions to below "
    "them. The handler bodies repeat line for line (the same res.json and closing-brace "
    "lines occur many times), which must not disturb the single large move.",
)

TODO_TOP = b("""
Errands for the week
- pick up the dry cleaning on Elm Road
- return the library books before Friday
- buy stamps and envelopes for the invitations
- drop the old phone at the recycling point
- top up the travel card
- order a new filter for the kitchen tap
- call the plumber about the slow drain
- water the neighbour's plants on Wednesday
- print the boarding passes
""")
TODO_MOVED = b("""
- renew the parking permit online
- book the dentist for March
""")
case(
    "005-pure-small-block",
    [("top", TODO_TOP), ("moved", TODO_MOVED)],
    [("top_title", TODO_TOP[:1]), ("moved", TODO_MOVED), ("top_rest", TODO_TOP[1:])],
    [("moved", "moved", "pure")],
    "Two short list items moved from the bottom of the list to the top. Two non-trivial "
    "lines is the smallest block a multi-line move can have, and together they have 49 "
    "alphanumeric characters, so they must still be detected.",
)

INI_SERVER = b("""
[server]
host = 0.0.0.0
port = 8443
read_timeout = 15s
write_timeout = 30s
max_body_size = 8MB
""")
INI_DATABASE = b("""
[database]
url = postgres://app@db.internal:5432/orders
pool_min = 2
pool_max = 20
statement_timeout = 5s
migrations_dir = ./migrations
""")
INI_CACHE = b("""
[cache]
backend = redis
address = cache.internal:6379
default_ttl = 10m
key_prefix = orders:
""")
INI_LOGGING = b("""
[logging]
level = info
format = json
output = stdout
include_caller = false
""")
INI_METRICS = b("""
[metrics]
enabled = true
listen = 0.0.0.0:9100
namespace = orders
""")
case(
    "006-pure-config-section",
    [("server", INI_SERVER), ("b0", BLANK), ("cache", INI_CACHE), ("b1", BLANK), ("database", INI_DATABASE),
     ("b2", BLANK), ("logging", INI_LOGGING), ("b3", BLANK), ("metrics", INI_METRICS)],
    [("server", INI_SERVER), ("b0", BLANK), ("database", INI_DATABASE), ("b2", BLANK), ("logging", INI_LOGGING),
     ("b3", BLANK), ("metrics", INI_METRICS), ("b1", BLANK), ("cache", INI_CACHE)],
    [("cache", "cache", "pure")],
    "The [cache] section moved from second place to the end of the file.",
)

GO_PKG = b("""
package store

import (
  "errors"
  "sync"
)

var ErrNotFound = errors.New("store: key not found")

type Store struct {
  mu   sync.RWMutex
  data map[string][]byte
}
""")
GO_GET = b("""
func (s *Store) Get(key string) ([]byte, error) {
  s.mu.RLock()
  defer s.mu.RUnlock()
  value, ok := s.data[key]
  if !ok {
    return nil, ErrNotFound
  }
  return value, nil
}
""")
GO_PUT = b("""
func (s *Store) Put(key string, value []byte) {
  s.mu.Lock()
  defer s.mu.Unlock()
  if s.data == nil {
    s.data = make(map[string][]byte)
  }
  s.data[key] = value
}
""")
GO_DELETE = b("""
func (s *Store) Delete(key string) bool {
  s.mu.Lock()
  defer s.mu.Unlock()
  if _, ok := s.data[key]; !ok {
    return false
  }
  delete(s.data, key)
  return true
}
""")
GO_LEN = b("""
func (s *Store) Len() int {
  s.mu.RLock()
  defer s.mu.RUnlock()
  return len(s.data)
}
""")
case(
    "007-pure-with-trivial-edges",
    [("pkg", GO_PKG), ("b0", BLANK), ("get", GO_GET), ("b1", BLANK), ("put", GO_PUT), ("b2", BLANK),
     ("delete", GO_DELETE), ("b3", BLANK), ("len", GO_LEN)],
    [("pkg", GO_PKG), ("b0", BLANK), ("put", GO_PUT), ("b2", BLANK), ("delete", GO_DELETE), ("b3", BLANK),
     ("len", GO_LEN), ("b1", BLANK), ("get", GO_GET)],
    [("get", "get", "pure")],
    "Get() moved below Len(). The label includes the function's inner and final closing "
    "braces, which are trivial lines inside and at the edge of the block, and excludes "
    "the blank separator.",
)

SQL_Q1 = b("""
-- name: active_customers
SELECT c.id, c.name, c.email
FROM customers AS c
WHERE c.deleted_at IS NULL
  AND c.last_order_at > now() - interval '90 days'
ORDER BY c.last_order_at DESC;
""")
SQL_Q2 = b("""
-- name: revenue_by_month
SELECT date_trunc('month', o.created_at) AS month,
  sum(o.total_cents) / 100.0 AS revenue
FROM orders AS o
WHERE o.status = 'paid'
GROUP BY 1
ORDER BY 1;
""")
SQL_Q3 = b("""
-- name: top_products
SELECT p.sku, p.title, count(*) AS units
FROM order_items AS i
JOIN products AS p ON p.id = i.product_id
GROUP BY p.sku, p.title
ORDER BY units DESC
LIMIT 20;
""")
SQL_Q4 = b("""
-- name: stale_carts
SELECT k.id, k.customer_id, k.updated_at
FROM carts AS k
WHERE k.checked_out = false
  AND k.updated_at < now() - interval '14 days';
""")
case(
    "008-pure-sql-query",
    [("q1", SQL_Q1), ("b0", BLANK), ("q2", SQL_Q2), ("b1", BLANK), ("q3", SQL_Q3), ("b2", BLANK), ("q4", SQL_Q4)],
    [("q2", SQL_Q2), ("b1", BLANK), ("q3", SQL_Q3), ("b2", BLANK), ("q1", SQL_Q1), ("b0", BLANK), ("q4", SQL_Q4)],
    [("q1", "q1", "pure")],
    "The active_customers query moved from first to third place in a file of named "
    "queries.",
)

PYC_HEAD = b("""
class Invoice:
  def __init__(self, number, customer):
    self.number = number
    self.customer = customer
    self.lines = []
""")
PYC_ADD_L = b("""
  def add_line(self, description, quantity, unit_price):
    if quantity <= 0:
      raise ValueError("quantity must be positive")
    self.lines.append((description, quantity, unit_price))
""")
PYC_ADD_R = b("""
  def add_line(self, description, quantity, unit_price):
    if quantity <= 0:
      raise ValueError(f"quantity must be positive, got {quantity}")
    self.lines.append((description, quantity, unit_price))
""")
PYC_TOTAL = b("""
  def total(self):
    return sum(quantity * price for _, quantity, price in self.lines)
""")
PYC_RENDER = b("""
  def render(self):
    rows = [f"Invoice {self.number} for {self.customer}"]
    for description, quantity, price in self.lines:
      rows.append(f"{description:30} {quantity:>4} x {price:>8.2f}")
    rows.append(f"{'Total':30} {self.total():>17.2f}")
    return "\\n".join(rows)
""")
PYC_TAX_L = b("""
  def tax(self, rate):
    return round(self.total() * rate, 2)
""")
PYC_TAX_R = b("""
  def tax(self, rate=0.2):
    return round(self.total() * rate, 2)

  def total_with_tax(self, rate=0.2):
    return self.total() + self.tax(rate)
""")
case(
    "009-pure-move-plus-edits",
    [("head", PYC_HEAD), ("b0", BLANK), ("total", PYC_TOTAL), ("b1", BLANK), ("add", PYC_ADD_L), ("b2", BLANK),
     ("render", PYC_RENDER), ("b3", BLANK), ("tax", PYC_TAX_L)],
    [("head", PYC_HEAD), ("b0", BLANK), ("add", PYC_ADD_R), ("b2", BLANK), ("render", PYC_RENDER), ("b1", BLANK),
     ("total", PYC_TOTAL), ("b3", BLANK), ("tax", PYC_TAX_R)],
    [("total", "total", "pure")],
    "total() moved from above add_line() to below render(). Unrelated edits elsewhere: "
    "the ValueError message in add_line() changed, tax() gained a default rate, and a "
    "new total_with_tax() method was added at the end. None of those is a move.",
)

SW_INTRO = b("""
# Onboarding notes
""")
SW_A = b("""
## Accounts
Every new starter gets a laptop, a badge and an email account on day one.
The service desk creates the accounts the Friday before the start date.
Ask your manager which shared drives and chat channels you need.
Two-factor sign-in is required for email and for the expense system.
Passwords are rotated by the identity team, never by hand.
If you lose your badge, report it to reception the same day.
The visitor desk can print a temporary badge valid for one week.
Laptops are replaced every four years or after a hardware failure.
Personal devices may join the guest network only.
""")
SW_B = b("""
## First week
Shadow a teammate for the first two days and read the team handbook.
Pick one small ticket from the starter board by Wednesday.
Book a thirty minute chat with each person on your team.
""")
SW_OUTRO = b("""
Questions go to the onboarding channel.
""")
case(
    "010-pure-swapped-blocks",
    [("intro", SW_INTRO), ("b0", BLANK), ("a", SW_A), ("b1", BLANK), ("b", SW_B), ("b2", BLANK), ("outro", SW_OUTRO)],
    [("intro", SW_INTRO), ("b0", BLANK), ("b", SW_B), ("b1", BLANK), ("a", SW_A), ("b2", BLANK), ("outro", SW_OUTRO)],
    [("b", "b", "pure")],
    "Two adjacent sections swapped places. The larger Accounts section is matched in "
    "place by the line diff, so only the shorter First week section is reported as "
    "moved; a reader may see a swap, but one side of a swap is always an equal op.",
)

SH_HEAD = b("""
#!/bin/sh
set -eu

LOG_DIR=/var/log/backup
TARGET=/mnt/archive
""")
SH_LOG = b("""
log() {
  printf '%s %s\\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >> "$LOG_DIR/run.log"
}
""")
SH_MID1 = b("""
check_target() {
  if [ ! -d "$TARGET" ]; then
    echo "archive target $TARGET is not mounted" >&2
    exit 1
  fi
  free_kb=$(df -k "$TARGET" | awk 'NR == 2 { print $4 }')
  if [ "$free_kb" -lt 1048576 ]; then
    echo "less than 1 GB free on $TARGET" >&2
    exit 1
  fi
}
""")
SH_ROTATE = b("""
rotate_logs() {
  find "$LOG_DIR" -name 'run.log.*' -mtime +30 -delete
  mv "$LOG_DIR/run.log" "$LOG_DIR/run.log.$(date +%Y%m%d)"
}
""")
SH_MID2 = b("""
copy_home() {
  for dir in /home/*; do
    name=$(basename "$dir")
    tar -czf "$TARGET/home-$name.tar.gz" -C /home "$name"
    log "archived home directory $name"
  done
}

copy_etc() {
  tar -czf "$TARGET/etc.tar.gz" -C / etc
  log "archived /etc"
}
""")
SH_MAIN = b("""
check_target
copy_home
copy_etc
rotate_logs
""")
case(
    "011-pure-two-moves-adjacent",
    [("head", SH_HEAD), ("b0", BLANK), ("log", SH_LOG), ("b1", BLANK), ("mid1", SH_MID1), ("b2", BLANK),
     ("rotate", SH_ROTATE), ("b3", BLANK), ("mid2", SH_MID2), ("b4", BLANK), ("main", SH_MAIN)],
    [("head", SH_HEAD), ("b0", BLANK), ("mid1", SH_MID1), ("b2", BLANK), ("mid2", SH_MID2), ("b1", BLANK),
     ("log", SH_LOG), ("rotate", SH_ROTATE), ("b4", BLANK), ("main", SH_MAIN)],
    [("log", "log", "pure"), ("rotate", "rotate", "pure")],
    "Two helper functions, log() and rotate_logs(), moved from separate places to "
    "back-to-back positions just above the main sequence (the blank line between them "
    "was dropped). They are two moves, not one, because they are not contiguous on the "
    "left. The second destination directly follows the first, so renderers give them "
    "different zebra stripes.",
)

_glossary = {
    "A": ("Anchor", "A fixed point in a document that a link can jump to.",
          "Anchors are usually headings or explicitly named elements."),
    "B": ("Baseline", "The invisible line that most letters sit on.",
          "Descenders such as g and y drop below the baseline."),
    "C": ("Caption", "Short text that describes a figure, table or photo.",
          "Captions sit directly above or below the item they describe."),
    "D": ("Drop cap", "A large first letter that spans several lines of a paragraph.",
          "Drop caps are common in books and long magazine features."),
    "E": ("Em dash", "A dash about as wide as the letter M, used to set off a phrase.",
          "Some style guides put thin spaces on either side of it."),
    "F": ("Footnote", "A note at the bottom of a page, linked by a superscript number.",
          "Endnotes collect the same notes at the end of a chapter instead."),
    "G": ("Gutter", "The inner margin where two facing pages meet at the binding.",
          "A wide gutter keeps text from disappearing into the spine."),
    "H": ("Hanging indent", "A paragraph whose first line starts further left than the rest.",
          "Bibliographies and reference lists often use hanging indents."),
    "I": ("Italic", "A slanted style of a typeface used for emphasis and titles.",
          "True italics are drawn separately rather than simply slanted."),
    "J": ("Justified text", "Text aligned to both the left and the right margin.",
          "Justification needs good hyphenation to avoid wide word gaps."),
}


def _entry(key: str) -> list[str]:
    term, first, second = _glossary[key]
    return [f"{term}:", f"  {first}", f"  {second}"]


_left12: list = [("title", ["# Typesetting glossary"])]
for _i, _key in enumerate("JACDBEFIGH"):
    _left12 += [(f"bl{_i}", BLANK), (_key, _entry(_key))]
_right12: list = [("title", ["# Typesetting glossary"])]
for _i, _key in enumerate("ABCDEFGHIJ"):
    _right12 += [(f"br{_i}", BLANK), (_key, _entry(_key))]
case(
    "012-pure-three-moves",
    _left12,
    _right12,
    [("J", "J", "pure"), ("B", "B", "pure"), ("I", "I", "pure")],
    "A glossary was put back into alphabetical order: Justified text moved from first "
    "to last, Baseline moved up to second, and Italic moved down to ninth. Every other "
    "entry stays in place. The Italic and Justified text destinations are consecutive.",
)

JS_FN = {
    "alpha": b("""
function alpha(items) {
  return items.filter((item) => item.enabled);
}
"""),
    "bravo": b("""
function bravo(items, key) {
  const seen = new Set();
  return items.filter((item) => {
    if (seen.has(item[key])) return false;
    seen.add(item[key]);
    return true;
  });
}
"""),
    "charlie": b("""
function charlie(items) {
  const groups = {};
  for (const item of items) {
    (groups[item.kind] ||= []).push(item);
  }
  return groups;
}
"""),
    "delta": b("""
function delta(before, after) {
  const added = after.filter((x) => !before.includes(x));
  const removed = before.filter((x) => !after.includes(x));
  return { added, removed };
}
"""),
    "echo": b("""
function echo(items, size) {
  const pages = [];
  for (let i = 0; i < items.length; i += size) {
    pages.push(items.slice(i, i + size));
  }
  return pages;
}
"""),
}
_left13: list = [("head", ['"use strict";'])]
for _i, _name in enumerate(["delta", "alpha", "echo", "bravo", "charlie"]):
    _left13 += [(f"bl{_i}", BLANK), (_name, JS_FN[_name])]
_left13 += [("bx", BLANK), ("exp", ["module.exports = { alpha, bravo, charlie, delta, echo };"])]
_right13: list = [("head", ['"use strict";'])]
for _i, _name in enumerate(["alpha", "bravo", "charlie", "delta", "echo"]):
    _right13 += [(f"br{_i}", BLANK), (_name, JS_FN[_name])]
_right13 += [("bx", BLANK), ("exp", ["module.exports = { alpha, bravo, charlie, delta, echo };"])]
case(
    "013-pure-functions-reordered",
    _left13,
    _right13,
    [("delta", "delta", "pure"), ("echo", "echo", "pure")],
    "Five functions sorted into alphabetical order. alpha, bravo and charlie keep their "
    "relative order and are matched in place by the line diff; delta and echo are the "
    "two moved functions, and their destinations are consecutive.",
)

CS_HEAD = b("""
using System;
using System.Collections.Generic;

namespace Billing
{
  public static class Rates
  {
""")
CS_VAT = b("""
    public static decimal Vat(decimal net, string country)
    {
      var rate = country switch
      {
        "DE" => 0.19m,
        "FR" => 0.20m,
        "NL" => 0.21m,
        _ => throw new ArgumentException($"no VAT rate for {country}"),
      };
      return Math.Round(net * rate, 2);
    }
""")
CS_DISCOUNT = b("""
    public static decimal Discount(decimal net, int quantity)
    {
      if (quantity >= 100) return net * 0.10m;
      if (quantity >= 20) return net * 0.05m;
      return 0m;
    }
""")
CS_SHIPPING = b("""
    public static decimal Shipping(decimal weightKg, bool express)
    {
      var basePrice = weightKg <= 2 ? 4.90m : 4.90m + (weightKg - 2) * 1.10m;
      return express ? basePrice * 2 : basePrice;
    }
""")
CS_ROUND = b("""
    public static decimal RoundToCash(decimal amount)
    {
      return Math.Round(amount * 20, MidpointRounding.AwayFromZero) / 20;
    }
""")
CS_TAIL = b("""
  }
}
""")
case(
    "014-pure-crlf-function",
    [("head", CS_HEAD), ("vat", CS_VAT), ("b0", BLANK), ("discount", CS_DISCOUNT), ("b1", BLANK),
     ("shipping", CS_SHIPPING), ("b2", BLANK), ("round", CS_ROUND), ("tail", CS_TAIL)],
    [("head", CS_HEAD), ("discount", CS_DISCOUNT), ("b1", BLANK), ("shipping", CS_SHIPPING), ("b2", BLANK),
     ("round", CS_ROUND), ("b0", BLANK), ("vat", CS_VAT), ("tail", CS_TAIL)],
    [("vat", "vat", "pure")],
    "CRLF line endings on both sides. The Vat() method moved from first to last place "
    "inside the class. Line numbers follow the LF/CRLF line rule; the CR is never part "
    "of the compared content.",
    crlf=True,
)

STEPS_A = b("""
Laboratory shutdown procedure
1. Tell the duty officer that the shutdown has started.
2. Close the gas valves in the order written on the panel.
3. Switch the fume hoods to night mode.
4. Log the freezer temperatures on the clipboard by the door.
5. Turn off the centrifuges and leave the lids open to dry.
6. Empty the waste bins into the yellow containers.
""")
STEPS_MOVED = b("""
7. Check that the eyewash station has been flushed this week.
8. Sign the station sheet with the date and time.
""")
STEPS_B = b("""
9. Switch off the bench lights, leaving the emergency lights on.
10. Lock the outer door and return the key to security.
""")
case(
    "015-pure-crlf-list",
    [("a", STEPS_A), ("moved", STEPS_MOVED), ("b", STEPS_B)],
    [("a_title", STEPS_A[:2]), ("moved", STEPS_MOVED), ("a_rest", STEPS_A[2:]), ("b", STEPS_B)],
    [("moved", "moved", "pure")],
    "CRLF line endings on both sides. Two checklist steps moved from near the end of "
    "the list to right after the first step; their numbers were not updated, which "
    "keeps the move pure.",
    crlf=True,
)

ZH_P1 = b("""
这个项目的目标是让任何人都能比较两个文件的差异。
它支持纯文本、代码、表格和文档等多种格式。
""")
ZH_P2 = b("""
安装非常简单，只需要下载一个可执行文件。
不需要额外的运行环境，也不会修改系统设置。
在命令行中输入两个文件的路径即可开始比较。
""")
ZH_P3 = b("""
结果会以统一的格式输出，方便其他程序读取。
每个结果都有一个稳定的标识符，相同的输入总是得到相同的标识符。
""")
ZH_P4 = b("""
如果遇到问题，请先阅读常见问题解答。
仍然无法解决时，可以在讨论区提出问题，并附上出错的文件。
""")
case(
    "016-pure-cjk-paragraph",
    [("p1", ZH_P1), ("b0", BLANK), ("p2", ZH_P2), ("b1", BLANK), ("p3", ZH_P3), ("b2", BLANK), ("p4", ZH_P4)],
    [("p1", ZH_P1), ("b1", BLANK), ("p3", ZH_P3), ("b2", BLANK), ("p4", ZH_P4), ("b0", BLANK), ("p2", ZH_P2)],
    [("p2", "p2", "pure")],
    "Chinese text. The three-line installation paragraph moved from second place to the "
    "end. Han ideographs count as alphanumeric for the move size rules.",
)

HE_A = b("""
שעות הפתיחה של הספרייה
ימים א עד ה: משמונה בבוקר עד שמונה בערב.
יום ו: משמונה בבוקר עד אחת בצהריים.
""")
HE_MOVED = b("""
החזרת ספרים אפשרית גם בתיבה שליד הכניסה הראשית.
ספרים שהוחזרו בתיבה נרשמים ביום העבודה הבא.
""")
HE_B = b("""
בחגים הספרייה סגורה.
""")
AR_C = b("""
مواعيد المكتبة
من الأحد إلى الخميس: من الثامنة صباحا حتى الثامنة مساء.
يوم الجمعة: من الثامنة صباحا حتى الواحدة ظهرا.
المكتبة مغلقة في الأعياد.
""")
case(
    "017-pure-rtl-hebrew",
    [("a", HE_A), ("moved", HE_MOVED), ("b", HE_B), ("b0", BLANK), ("c", AR_C)],
    [("a", HE_A), ("b", HE_B), ("b0", BLANK), ("c", AR_C), ("b1", BLANK), ("moved", HE_MOVED)],
    [("moved", "moved", "pure")],
    "Right-to-left text. Two Hebrew lines about returning books moved from the Hebrew "
    "opening-hours block to the end of the file, after the Arabic block. Line numbers "
    "are logical (storage) order.",
)

PY_DUP_HEAD = b("""
\"\"\"Helpers shared by the reporting jobs.\"\"\"
""")
PY_IMPORTS_TOP = b("""
import datetime
import decimal
import logging
import os
import pathlib
import re
""")
PY_LONG_IMPORT = [
    "from reporting.formatting import format_currency, format_percentage, format_short_date, format_table"
]
PY_DUP_BODY = b("""

log = logging.getLogger(__name__)


def output_dir():
  return pathlib.Path(os.environ.get("REPORT_DIR", "/tmp/reports"))
""")
case(
    "018-pure-two-destinations",
    [("head", PY_DUP_HEAD), ("b0", BLANK), ("imports", PY_IMPORTS_TOP),
     ("moved", b("""
def week_label(day):
  monday = day - datetime.timedelta(days=day.weekday())
  return f"week of {monday:%d %b %Y}"
""")),
     ("body", PY_DUP_BODY)],
    [("head", PY_DUP_HEAD), ("b0", BLANK), ("imports", PY_IMPORTS_TOP), ("body", PY_DUP_BODY), ("bx", BLANK),
     ("bx2", BLANK),
     ("moved", b("""
def week_label(day):
  monday = day - datetime.timedelta(days=day.weekday())
  return f"week of {monday:%d %b %Y}"
""")),
     ("by", BLANK), ("by2", BLANK),
     ("copy", b("""
def week_label(day):
  monday = day - datetime.timedelta(days=day.weekday())
  return f"week of {monday:%d %b %Y}"
"""))],
    [("moved", "moved", "pure")],
    "week_label() was deleted after the imports and inserted twice near the end of the "
    "file. A deleted block pairs with at most one destination; the selection order "
    "(left start, then right start) picks the first copy, and the second copy stays a "
    "plain insertion.",
)

# ---------------------------------------------------------------------------
# Long single-line moves
# ---------------------------------------------------------------------------

case(
    "019-single-long-import",
    [("head", PY_DUP_HEAD), ("b0", BLANK), ("imports", PY_IMPORTS_TOP), ("long", PY_LONG_IMPORT),
     ("body", PY_DUP_BODY)],
    [("head", PY_DUP_HEAD), ("b0", BLANK), ("long", PY_LONG_IMPORT), ("imports", PY_IMPORTS_TOP),
     ("body", PY_DUP_BODY)],
    [("long", "long", "pure")],
    "One long from-import line (85 alphanumeric characters) moved from below the plain "
    "imports to above them. A single non-trivial line qualifies because it has at "
    "least 40 alphanumeric characters.",
)

PY_WORKER = b("""
def drain(queue, handler, batch_size=50):
  processed = 0
  while True:
    batch = queue.take(batch_size)
    if not batch:
      break
    for message in batch:
      handler(message)
      processed += 1
    queue.ack(batch)
""")
PY_WARN = ['  log.warning("queue drain stopped early because the handler raised an exception for message %s", message.id)']
PY_WORKER_END = b("""
  return processed
""")
PY_OTHER = b("""


def requeue(queue, messages):
  for message in messages:
    queue.put(message, delay_seconds=30)
  log.info("requeued %d messages", len(messages))
""")
case(
    "020-single-long-log-message",
    [("worker", PY_WORKER), ("warn", PY_WARN), ("end", PY_WORKER_END), ("other", PY_OTHER)],
    [("worker", PY_WORKER), ("end", PY_WORKER_END), ("other", PY_OTHER), ("warn", PY_WARN)],
    [("warn", "warn", "pure")],
    "A single long log.warning() line moved from the end of drain() to the end of "
    "requeue(). The indentation is the same in both places.",
)

ESSAY_START = b("""
Notes on the walking route
""")
ESSAY_BODY = b("""
Start at the old station car park and follow the river path upstream.
After the second footbridge, take the steep track on the left.
The track climbs through beech woods for about two kilometres.
At the top there is a stone bench with a view over the valley.
Turn right along the ridge and follow the waymarks back down.
""")
ESSAY_LONG = ["The whole loop is roughly eleven kilometres and takes most walkers between three and four hours including a lunch stop."]
case(
    "021-single-long-sentence",
    [("start", ESSAY_START), ("b0", BLANK), ("body", ESSAY_BODY), ("long", ESSAY_LONG)],
    [("start", ESSAY_START), ("b0", BLANK), ("long", ESSAY_LONG), ("body", ESSAY_BODY)],
    [("long", "long", "pure")],
    "A one-line summary sentence moved from the end of the route description to the "
    "start. It is a single line, but long enough (over 40 alphanumeric characters) to "
    "count as a move.",
)

# ---------------------------------------------------------------------------
# Indentation changes (moveWhitespace "allowIndentationChange", the default)
# ---------------------------------------------------------------------------

case(
    "022-indent-into-block",
    [("a", b("""
import time


def sync(items, dry_run, uploader):
  total = len(items)
""")),
     ("moved_l", b("""
  print(f"syncing {total} items to {uploader.name}")
  started = time.monotonic()
""")),
     ("b", b("""
  for item in items:
    uploader.upload(item)
  if not dry_run:
    uploader.commit()
  return total
"""))],
    [("a", b("""
import time


def sync(items, dry_run, uploader):
  total = len(items)
""")),
     ("b1", b("""
  for item in items:
    uploader.upload(item)
  if not dry_run:
""")),
     ("moved_r", b("""
    print(f"syncing {total} items to {uploader.name}")
    started = time.monotonic()
""")),
     ("b2", b("""
    uploader.commit()
  return total
"""))],
    [("moved_l", "moved_r", "pure")],
    "Two lines moved from the top of sync() into the body of the if statement further "
    "down, gaining two spaces of indentation. Every line of the block has the same "
    "indentation delta, so the move is pure under allowIndentationChange.",
)

case(
    "023-indent-out-of-block",
    [("a", b("""
def read_rows(path):
  rows = []
  with open(path, encoding="utf-8") as handle:
    for raw_line in handle:
""")),
     ("moved_l", b("""
      line = raw_line.strip()
      if line and not line.startswith("#"):
        rows.append(line.split(","))
""")),
     ("b", b("""
  return rows


def count_rows(path):
  return len(read_rows(path))
"""))],
    [("a", b("""
def read_rows(path):
  with open(path, encoding="utf-8") as handle:
    return parse_rows(handle)


def count_rows(path):
  return len(read_rows(path))


def parse_rows(lines):
  rows = []
  for raw_line in lines:
""")),
     ("moved_r", b("""
    line = raw_line.strip()
    if line and not line.startswith("#"):
      rows.append(line.split(","))
""")),
     ("b", b("""
  return rows
"""))],
    [("moved_l", "moved_r", "pure")],
    "The body of the parsing loop moved out of read_rows() into a new parse_rows() "
    "function at the end of the file, losing two spaces of indentation. The new loop "
    "header and the return lines around it are not part of the move: the header "
    "changed, and the return line has a different indentation delta.",
)

case(
    "024-indent-yaml",
    [("a", b("""
version: 3
defaults:
""")),
     ("moved_l", b("""
  retries: 3
  timeout_seconds: 20
  log_format: json
""")),
     ("b", b("""
services:
  api:
    image: registry.example/api:2.4.1
    replicas: 3
    ports:
      - 8080
  worker:
    image: registry.example/worker:2.4.1
    replicas: 2
    queue: jobs-main
""")),
     ("c", b("""
monitoring:
  scrape_interval: 30s
  alert_channel: ops-alerts
"""))],
    [("a", b("""
version: 3
""")),
     ("b", b("""
services:
  api:
    image: registry.example/api:2.4.1
    replicas: 3
    ports:
      - 8080
  worker:
    image: registry.example/worker:2.4.1
    replicas: 2
    queue: jobs-main
""")),
     ("moved_r", b("""
    retries: 3
    timeout_seconds: 20
    log_format: json
""")),
     ("c", b("""
monitoring:
  scrape_interval: 30s
  alert_channel: ops-alerts
"""))],
    [("moved_l", "moved_r", "pure")],
    "Three settings moved from a top-level defaults map into the worker service, two "
    "levels deeper. The emptied `defaults:` key is deleted and is not part of the move.",
)

case(
    "025-indent-html",
    [("a", b("""
<nav>
  <ul class="main">
    <li><a href="/">Home</a></li>
    <li><a href="/pricing">Pricing</a></li>
""")),
     ("moved_l", b("""
    <li><a href="/docs/install">Install guide</a></li>
    <li><a href="/docs/api">API reference</a></li>
""")),
     ("b", b("""
    <li><a href="/blog">Blog</a></li>
    <li>
      <span>Docs</span>
      <ul class="sub">
""")),
     ("c", b("""
        <li><a href="/docs/faq">FAQ</a></li>
      </ul>
    </li>
  </ul>
</nav>
"""))],
    [("a", b("""
<nav>
  <ul class="main">
    <li><a href="/">Home</a></li>
    <li><a href="/pricing">Pricing</a></li>
""")),
     ("b", b("""
    <li><a href="/blog">Blog</a></li>
    <li>
      <span>Docs</span>
      <ul class="sub">
""")),
     ("moved_r", b("""
        <li><a href="/docs/install">Install guide</a></li>
        <li><a href="/docs/api">API reference</a></li>
""")),
     ("c", b("""
        <li><a href="/docs/faq">FAQ</a></li>
      </ul>
    </li>
  </ul>
</nav>
"""))],
    [("moved_l", "moved_r", "pure")],
    "Two navigation links moved from the main menu into the nested Docs submenu, "
    "gaining four spaces of indentation.",
)

# ---------------------------------------------------------------------------
# Edited moves (labels of kind "edited"; scored only with moveDetection "edited")
# Each block is designed so that no unchanged sub-run meets the pure minimum size,
# so the edited label is the only sensible answer.
# ---------------------------------------------------------------------------

JS_UTIL_A = b("""
"use strict";

function clamp(value, low, high) {
  return Math.min(Math.max(value, low), high);
}
""")
JS_UTIL_B = b("""
function chunk(list, size) {
  const out = [];
  for (let i = 0; i < list.length; i += size) {
    out.push(list.slice(i, i + size));
  }
  return out;
}

function uniqueBy(list, key) {
  const seen = new Map();
  for (const item of list) {
    if (!seen.has(item[key])) seen.set(item[key], item);
  }
  return [...seen.values()];
}
""")
JS_UTIL_END = b("""

module.exports = { checksum, chunk, clamp, uniqueBy };
""")
case(
    "026-edited-renamed-variable",
    [("a", JS_UTIL_A), ("b0", BLANK), ("moved_l", b("""
function checksum(data) {
  let acc = 0;
  for (const byte of data) {
    acc = (acc * 31 + byte) >>> 0;
  }
  return acc.toString(16).padStart(8, "0");
}
""")), ("b1", BLANK), ("b", JS_UTIL_B), ("end", JS_UTIL_END)],
    [("a", JS_UTIL_A), ("b1", BLANK), ("b", JS_UTIL_B), ("b0", BLANK), ("moved_r", b("""
function checksum(data) {
  let hash = 0;
  for (const byte of data) {
    hash = (hash * 31 + byte) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}
""")), ("end", JS_UTIL_END)],
    [("moved_l", "moved_r", "edited")],
    "checksum() moved from below clamp() to the end of the module, and its accumulator "
    "variable was renamed from acc to hash on every line that uses it. The unchanged "
    "lines in between are single short lines, so no part of the block is a pure move.",
)

PROSE_HEAD = b("""
Volunteer handbook: kitchen shifts
""")
PROSE_A = b("""
Shifts start at half past five. Sign in on the sheet by the door and put on
an apron and a hair net before you touch any food. The shift lead tells you
which station you are on.
""")
PROSE_B = b("""
At the end of the shift every station is wiped down, the floor is swept and
mopped, and the bins are taken to the yard. Nobody leaves until the shift lead
has checked the cold room door is shut.
""")
case(
    "027-edited-typo-fixes",
    [("head", PROSE_HEAD), ("b0", BLANK), ("a", PROSE_A), ("b1", BLANK), ("b", PROSE_B), ("b2", BLANK),
     ("moved_l", b("""
Allergens are lisetd on the whiteboard for every dish. If a guest asks about
an ingredient and you are not sure, ask the shift laed rather than guesing.
Never swap an ingredient without updating the board.
"""))],
    [("head", PROSE_HEAD), ("b0", BLANK), ("a", PROSE_A), ("b2", BLANK), ("moved_r", b("""
Allergens are listed on the whiteboard for every dish. If a guest asks about
an ingredient and you are not sure, ask the shift lead rather than guessing.
Never swap an ingredient without updating the whiteboard.
""")), ("b1", BLANK), ("b", PROSE_B)],
    [("moved_l", "moved_r", "edited")],
    "The allergens paragraph moved up above the end-of-shift paragraph, and every one "
    "of its three lines was edited on the way: two typos fixed on the first two lines "
    "and one word changed on the last.",
)

RECIPE_HEAD = b("""
Shortbread
Bake at 160 C for about 25 minutes.
""")
RECIPE_STEPS = b("""
Method:
  1. Cream the butter and sugar until pale.
  2. Work in the flour and a pinch of salt.
  3. Press into a tin and prick with a fork.
  4. Bake until just golden at the edges.
  5. Cut into fingers while still warm.
""")
case(
    "028-edited-extra-line",
    [("head", RECIPE_HEAD), ("b0", BLANK), ("steps", RECIPE_STEPS), ("b1", BLANK),
     ("moved_l", b("""
  - butter: 250 g
  - flour: 360 g
  - egg: 1
  - salt: 2 g
"""))],
    [("head", RECIPE_HEAD), ("b1", BLANK), ("moved_r", b("""
  - butter: 250 g
  - flour: 360 g
  - sugar: 120 g
  - egg: 1
  - salt: 2 g
""")), ("b0", BLANK), ("steps", RECIPE_STEPS)],
    [("moved_l", "moved_r", "edited")],
    "The ingredient list moved from below the method to above it, and a sugar line was "
    "added in the middle. The two-line runs before and after the new line are each "
    "below 20 alphanumeric characters (19 and 10), so the move is only found as an "
    "edited block. A stationary heading above the list would join the first run and "
    "make it a pure move, which is why the list has none.",
)

PY_NET_A = b("""
import random


def jitter(seconds):
  return seconds * random.uniform(0.8, 1.2)
""")
PY_NET_B = b("""
def should_retry(status):
  return status in (408, 429, 500, 502, 503, 504)


def parse_retry_after(header):
  if header is None:
    return None
  try:
    return max(0, int(header))
  except ValueError:
    return None
""")
case(
    "029-edited-changed-constant",
    [("a", PY_NET_A), ("b0", BLANK), ("b00", BLANK), ("moved_l", b("""
def retry_delay(attempt):
  base = 0.25
  return min(base * 2 ** attempt, 30)
""")), ("b1", BLANK), ("b11", BLANK), ("b", PY_NET_B)],
    [("a", PY_NET_A), ("b1", BLANK), ("b11", BLANK), ("b", PY_NET_B), ("b0", BLANK), ("b00", BLANK),
     ("moved_r", b("""
def retry_delay(attempt):
  base = 0.5
  return min(base * 2 ** attempt, 30)
"""))],
    [("moved_l", "moved_r", "edited")],
    "retry_delay() moved to the end of the file and its base delay changed from 0.25 to "
    "0.5. Its unchanged first and last lines are single lines under 40 alphanumeric "
    "characters, so only the edited block qualifies.",
)

PY_HTTP_HEAD = b("""
from transport import Request, send_raw


def build_request(url, raw_headers, body):
  request = Request(url)
""")
PY_HTTP_TAIL_L = b("""
  request.headers = headers
  request.body = body
  return request


def send(request, timeout=10):
  response = send_raw(request, timeout=timeout)
  response.raise_for_status()
  return response
""")
case(
    "030-edited-comment-removed",
    [("head", PY_HTTP_HEAD), ("moved_l", b("""
  headers = dict(raw_headers)
  # hop-by-hop headers must not be forwarded
  headers.pop("connection", None)
  headers.pop("keep-alive", None)
""")), ("tail", PY_HTTP_TAIL_L)],
    [("head", PY_HTTP_HEAD), ("tail", b("""
  request.headers = clean_headers(raw_headers)
  request.body = body
  return request


def send(request, timeout=10):
  response = send_raw(request, timeout=timeout)
  response.raise_for_status()
  return response


def clean_headers(raw_headers):
""")), ("moved_r", b("""
  headers = dict(raw_headers)
  headers.pop("Connection", None)
  headers.pop("keep-alive", None)
""")), ("ret", ["  return headers"])],
    [("moved_l", "moved_r", "edited")],
    "Header clean-up lines moved out of build_request() into a new clean_headers() "
    "helper at the end of the file. On the way the comment line was dropped and one "
    "header name changed case. The new def and return lines are plain insertions.",
)

PY_FETCH_HEAD = b("""
import time

from errors import ClientError, ServerError

MAX_BACKOFF = 60


def fetch(session, url, host, log):
  backoff = 1
  while True:
    response = session.get(url)
""")
PY_FETCH_MID = b("""
    if response.status >= 500:
      log.error("server error %s from %s", response.status, host)
      raise ServerError(response.status)
    if response.ok:
      return response.json()
""")
PY_FETCH_END = b("""
    raise ClientError(response.status)
""")
case(
    "031-edited-string-change",
    [("head", PY_FETCH_HEAD), ("moved_l", b("""
    if response.status == 429:
      log.info("rate limited, backing off")
      time.sleep(backoff)
      backoff = min(backoff * 2, 60)
      continue
""")), ("mid", PY_FETCH_MID), ("end", PY_FETCH_END)],
    [("head", PY_FETCH_HEAD), ("mid", PY_FETCH_MID), ("moved_r", b("""
    if response.status == 429:
      log.warning("rate limited by %s, backing off", host)
      time.sleep(backoff)
      backoff = min(backoff * 2, MAX_BACKOFF)
      continue
""")), ("end", PY_FETCH_END)],
    [("moved_l", "moved_r", "edited")],
    "The rate-limit branch moved below the server-error and success checks. Its log "
    "call changed level and message, and the backoff cap now uses the MAX_BACKOFF "
    "constant. Edited and unchanged lines alternate, so no unchanged run is big enough "
    "to be a pure move.",
)

# ---------------------------------------------------------------------------
# Negative cases: nothing here is a move (labels have moves: [])
# ---------------------------------------------------------------------------

NOTES_1 = b("""
## 2.4.0
- faster startup on large projects
- new dark theme for the settings window
""")
NOTES_2 = b("""
## 2.3.2
- fixed a crash when the config file was empty
""")
NOTES_3 = b("""
## 2.3.1
- fixed the window position on multi-monitor setups
- the status bar shows the current branch again
""")
NOTES_4 = b("""
## 2.3.0
- added keyboard shortcuts for every menu item
""")
case(
    "032-neg-separators-blank-lines",
    [("title", ["# Release notes"]), ("b0", BLANK), ("n1", NOTES_1), ("sep_l", b("""

---

* * *
""")), ("b1", BLANK), ("n2", NOTES_2), ("b2", BLANK), ("n3", NOTES_3), ("b3", BLANK), ("n4", NOTES_4)],
    [("title", ["# Release notes"]), ("b0", BLANK), ("n1", NOTES_1), ("b1", BLANK), ("n2", NOTES_2), ("b2", BLANK),
     ("n3", NOTES_3), ("sep_r", b("""

---

* * *
""")), ("b3", BLANK), ("n4", NOTES_4)],
    [],
    "A group of separator lines (blank lines, a --- rule and a * * * rule) moved from "
    "after 2.4.0 to after 2.3.1. All of them are trivial lines, and a block made only "
    "of trivial lines is never a move.",
)

case(
    "033-neg-closing-braces",
    [("a", b("""
function render(list) {
  if (list.length > 0) {
    drawHeader();
""")), ("brace_l", ["  }"]), ("b", b("""
  drawRows(list);
  drawFooter();
}

function bind(button) {
  button.addEventListener("click", () => {
    toggle(button);
""")), ("close_l", ["  });"]), ("c", b("""
  button.focus();
}
"""))],
    [("a", b("""
function render(list) {
  if (list.length > 0) {
    drawHeader();
""")), ("b", b("""
  drawRows(list);
  drawFooter();
""")), ("brace_r", ["  }"]), ("b2", b("""
}

function bind(button) {
  button.addEventListener("click", () => {
    toggle(button);
""")), ("c1", b("""
  button.focus();
""")), ("close_r", ["  });"]), ("c2", ["}"])],
    [],
    "Only closing-brace lines changed position: the `}` that ends the if block and the "
    "`});` that ends the event listener each moved down past two or one line. Both "
    "are trivial lines (at most two alphanumeric characters), so nothing is a move.",
)

case(
    "034-neg-short-block",
    [("a", b("""
def report(orders):
""")), ("moved_l", b("""
  count = 0
  total = 0
""")), ("b", b("""
  lines = []
  for order in orders:
    lines.append(f"{order.id}: {order.amount:.2f}")
""")), ("c", b("""
  for order in orders:
    count += 1
    total += order.amount
  lines.append(f"{count} orders, {total:.2f} in total")
  return lines
"""))],
    [("a", b("""
def report(orders):
""")), ("b", b("""
  lines = []
  for order in orders:
    lines.append(f"{order.id}: {order.amount:.2f}")
""")), ("moved_r", b("""
  count = 0
  total = 0
""")), ("c", b("""
  for order in orders:
    count += 1
    total += order.amount
  lines.append(f"{count} orders, {total:.2f} in total")
  return lines
"""))],
    [],
    "A reader would call this a move: two counter initialisations moved down to just "
    "before the loop that uses them. Together they have only 12 alphanumeric "
    "characters, below the 20 needed, so the spec says no move is reported.",
)

case(
    "035-neg-single-short-line",
    [("a", b("""
def warm_cache(cache, keys, log):
  started = time.monotonic()
""")), ("moved_l", ['  log.debug("cache warmed in %d ms", elapsed)']), ("b", b("""
  for key in keys:
    cache.load(key)
  elapsed = int((time.monotonic() - started) * 1000)
  return elapsed
"""))],
    [("a", b("""
def warm_cache(cache, keys, log):
  started = time.monotonic()
  for key in keys:
    cache.load(key)
  elapsed = int((time.monotonic() - started) * 1000)
""")), ("moved_r", ['  log.debug("cache warmed in %d ms", elapsed)']), ("b", b("""
  return elapsed
"""))],
    [],
    "A single log line moved below the code that computes the value it prints. A reader "
    "may call it a move, but it is one non-trivial line with fewer than 40 "
    "alphanumeric characters, so it is below the single-line minimum.",
)

case(
    "036-neg-delete-unrelated-insert",
    [("a", b("""
Trip plan
Day 1: train to the coast, check in at the guest house by six.
""")), ("deleted", b("""
Day 2: morning ferry to the island, walk the lighthouse loop.
Pack lunch, the island cafe is closed on Mondays.
""")), ("b", b("""
Day 3: hire bikes and ride the old railway path inland.
Day 4: museum in the morning, train home after lunch.
"""))],
    [("a", b("""
Trip plan
Day 1: train to the coast, check in at the guest house by six.
""")), ("b", b("""
Day 3: hire bikes and ride the old railway path inland.
Day 4: museum in the morning, train home after lunch.
""")), ("inserted", b("""
Budget: about 420 for the guest house, 60 for bike hire.
Book the train tickets at least two weeks ahead.
"""))],
    [],
    "Two lines are deleted in the middle and two unrelated lines are inserted at the "
    "end. Deleted and inserted text share no lines, so nothing is a move.",
)

GO_COPY_HEAD = b("""
package mathx

func Abs(x int) int {
  if x < 0 {
    return -x
  }
  return x
}
""")
GO_COPY_SRC = b("""
func Clamp(x, low, high int) int {
  if x < low {
    return low
  }
  if x > high {
    return high
  }
  return x
}
""")
GO_COPY_TAIL = b("""
func Sign(x int) int {
  switch {
  case x > 0:
    return 1
  case x < 0:
    return -1
  }
  return 0
}
""")
case(
    "037-neg-copy-function",
    [("head", GO_COPY_HEAD), ("b0", BLANK), ("src", GO_COPY_SRC), ("b1", BLANK), ("tail", GO_COPY_TAIL)],
    [("head", GO_COPY_HEAD), ("b0", BLANK), ("src", GO_COPY_SRC), ("b1", BLANK), ("tail", GO_COPY_TAIL),
     ("b2", BLANK), ("copy", b("""
func Clamp(x, low, high int) int {
  if x < low {
    return low
  }
  if x > high {
    return high
  }
  return x
}
"""))],
    [],
    "Clamp() was copied to the end of the file while the original stays where it was. "
    "A copy whose source still exists unchanged is not a move: the original lines are "
    "equal, so nothing on the left is deleted.",
)

DUP_P1 = b("""
The reading room is open to everyone with a library card. Bags and coats go in
the lockers by the stairs, and drinks other than water are not allowed.
""")
DUP_P2 = b("""
Rare books are requested at the front desk at least one day ahead. They are
read at the supervised table and never leave the room.
""")
DUP_P3 = b("""
Photography without flash is allowed for personal study. Ask the desk for a
permit if you plan to publish any images.
""")
case(
    "038-neg-duplicated-paragraph",
    [("title", ["Reading room rules"]), ("b0", BLANK), ("p1", DUP_P1), ("b1", BLANK), ("p2", DUP_P2), ("b2", BLANK),
     ("p3", DUP_P3)],
    [("title", ["Reading room rules"]), ("b0", BLANK), ("p1", DUP_P1), ("b1", BLANK), ("p2", DUP_P2), ("b2", BLANK),
     ("p3", DUP_P3), ("b3", BLANK), ("dup", b("""
Rare books are requested at the front desk at least one day ahead. They are
read at the supervised table and never leave the room.
"""))],
    [],
    "The rare-books paragraph is repeated at the end of the page, and the original "
    "stays in place. Duplicated text is a copy, not a move.",
)

case(
    "039-neg-in-place-rewrite",
    [("a", b("""
Parking at the office
""")), ("old", b("""
Staff parking is in the basement and opens at seven in the morning.
Visitors should use the short stay bays in front of the main entrance.
Bicycles can be locked to the racks beside the loading dock.
""")), ("b", b("""
Lost parking cards are replaced by reception for a fee of ten euros.
"""))],
    [("a", b("""
Parking at the office
""")), ("new", b("""
Staff parking is in the basement and opens at half past six in the morning.
Visitors should use the short stay bays next to the main entrance.
Bicycles can be locked to the covered racks beside the loading dock.
""")), ("b", b("""
Lost parking cards are replaced by reception for a fee of ten euros.
"""))],
    [],
    "Three lines are reworded in place: they form one replace op, and the edited lines "
    "are very similar to the originals. A deletion and an insertion inside the same op "
    "are an in-place rewrite, never a move, in pure and edited mode alike.",
)

case(
    "040-neg-repeated-short-lines",
    [("a", b("""
func Sync(src, dst Store) error {
  items, err := src.List()
""")), ("check_l", b("""
  if err != nil {
    return err
  }
""")), ("b", b("""
  for _, item := range items {
""")), ("put_l", b("""
    dst.Put(item)
""")), ("c", b("""
  }
  log.Printf("synced %d items", len(items))
  return nil
}
"""))],
    [("a", b("""
func Sync(src, dst Store) error {
  items := src.ListCached()
""")), ("b", b("""
  for _, item := range items {
""")), ("put_r", b("""
    err := dst.Put(item)
    if err != nil {
      return err
    }
""")), ("c", b("""
  }
  log.Printf("synced %d items", len(items))
  return nil
}
"""))],
    [],
    "The error check after List() is removed and an identical-looking check is added "
    "inside the loop. The repeated `if err != nil {` / `return err` / `}` lines have "
    "only 17 alphanumeric characters, so even with the indentation change allowed "
    "they are below the move minimum; this guards against reporting boilerplate as "
    "moved code.",
)


# ---------------------------------------------------------------------------
# Writer
# ---------------------------------------------------------------------------


def assemble(segments):
    lines: list[str] = []
    spans: dict[str, tuple[int, int]] = {}
    for name, seg_lines in segments:
        if name in spans:
            raise SystemExit(f"duplicate segment name {name!r}")
        spans[name] = (len(lines) + 1, len(seg_lines))
        lines.extend(seg_lines)
    return lines, spans


def write_case(entry: dict) -> dict:
    left_lines, left_spans = assemble(entry["left"])
    right_lines, right_spans = assemble(entry["right"])
    moves = []
    for left_name, right_name, kind in entry["moves"]:
        ls, lc = left_spans[left_name]
        rs, rc = right_spans[right_name]
        moves.append({"left": {"start": ls, "count": lc}, "right": {"start": rs, "count": rc}, "kind": kind})
    moves.sort(key=lambda m: (m["left"]["start"], m["right"]["start"]))
    terminator = "\r\n" if entry["crlf"] else "\n"
    directory = ROOT / entry["slug"]
    directory.mkdir(parents=True, exist_ok=True)
    for side, lines in (("left", left_lines), ("right", right_lines)):
        data = "".join(line + terminator for line in lines).encode("utf-8")
        (directory / f"{side}.txt").write_bytes(data)
    (directory / "options.json").write_text(json.dumps(OPTIONS, indent=2) + "\n", encoding="utf-8")
    labels = {"moves": moves, "notes": entry["notes"]}
    (directory / "labels.json").write_text(
        json.dumps(labels, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    return labels


def main() -> int:
    slugs = [entry["slug"] for entry in CASES]
    if len(set(slugs)) != len(slugs):
        raise SystemExit("duplicate case slug")
    total_moves = 0
    negatives = 0
    for entry in CASES:
        labels = write_case(entry)
        total_moves += len(labels["moves"])
        negatives += not labels["moves"]
    print(f"wrote {len(CASES)} moved cases ({total_moves} labeled moves, {negatives} negative cases)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

