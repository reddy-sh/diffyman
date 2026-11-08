"""Spike C speed run: WASM in Node.js against the native CLI.

For each input size, runs the release CLI (a new process each time, `--json`
to a pipe) and each given `--target nodejs` package (in process, see
node_speed.mjs). Reports the median of N runs after one warm-up, and checks
that every surface returns the same document once `provenance` is removed.

    uv run --no-project --python 3.12 python bindings/wasm/bench/speed.py \
        --cli target/release/diffyman --pkg o3=bindings/wasm/pkg-node [--pkg NAME=DIR ...]

Inputs come from gen_inputs.py (default ~/diffyman-data/tmp/spikec).
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def canonical(doc: dict) -> str:
    doc = dict(doc)
    doc.pop("provenance", None)
    return json.dumps(doc, sort_keys=True, ensure_ascii=False, separators=(",", ":"))


def run_cli(cli: Path, left: Path, right: Path, runs: int) -> dict:
    cmd = [str(cli), "diff", "--json", str(left), str(right)]
    subprocess.run(cmd, capture_output=True)  # warm the page cache
    times = []
    engine = []
    out = b""
    for _ in range(runs):
        start = time.perf_counter()
        proc = subprocess.run(cmd, capture_output=True)
        times.append((time.perf_counter() - start) * 1000)
        out = proc.stdout
        if proc.returncode not in (0, 1):
            raise RuntimeError(proc.stderr.decode())
        # Engine time measured inside the process (decode, diff, id; not the final serialization).
        engine.append(json.loads(out)["provenance"]["timing"]["totalMicros"] / 1000)
    return {"timesMs": times, "medianMs": statistics.median(times), "engineMs": engine,
            "engineMedianMs": statistics.median(engine), "documentBytes": len(out),
            "document": json.loads(out)}


def run_node(pkg: Path, left: Path, right: Path, runs: int) -> dict:
    proc = subprocess.run(
        ["node", str(HERE / "node_speed.mjs"), "--pkg", str(pkg), "--runs", str(runs), str(left), str(right)],
        capture_output=True, check=True, text=True,
    )
    return json.loads(proc.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--pkg", action="append", default=[], help="NAME=DIR of a nodejs package")
    parser.add_argument("--data", type=Path, default=Path.home() / "diffyman-data" / "tmp" / "spikec")
    parser.add_argument("--sizes", default="1,10")
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()

    ok = True
    rows = []
    for mb in args.sizes.split(","):
        left, right = args.data / f"{mb}mb-left.txt", args.data / f"{mb}mb-right.txt"
        cli = run_cli(args.cli, left, right, args.runs)
        reference = canonical(cli["document"])
        rows.append((f"{mb} MB", "native CLI, whole process", cli["medianMs"], None, cli["documentBytes"], True))
        rows.append((f"{mb} MB", "native engine, in process (totalMicros)", cli["engineMedianMs"], None,
                     cli["documentBytes"], True))
        for spec in args.pkg:
            name, _, pkg = spec.partition("=")
            node = run_node(Path(pkg), left, right, args.runs)
            same = canonical(node["document"]) == reference
            ok &= same
            rows.append((f"{mb} MB", f"WASM in Node, {name}", node["medianMs"], node["coldMs"],
                         node["documentBytes"], same))
            print(json.dumps({"size": mb, "surface": name, "loadMs": node["loadMs"],
                              "coldMs": node["coldMs"], "timesMs": node["timesMs"]}), file=sys.stderr)
        print(json.dumps({"size": mb, "surface": "cli", "timesMs": cli["timesMs"], "engineMs": cli["engineMs"],
                          "status": cli["document"]["status"]}), file=sys.stderr)

    print(f"| Input | Surface | Median of {args.runs} | Cold first call | Document | Same document as CLI |")
    print("|---|---|---|---|---|---|")
    for size, surface, median, cold, doc_bytes, same in rows:
        cold_text = f"{cold:.1f} ms" if cold is not None else "n/a"
        print(f"| {size} | {surface} | {median:.1f} ms | {cold_text} | {doc_bytes / 1000:.0f} KB | {'yes' if same else 'NO'} |")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
