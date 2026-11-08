"""Static server for the browser bench page.

Serves bindings/wasm at / (so the page can load ../pkg-web/) and a data folder
with the generated inputs at /data/. Standard library only.

    uv run --no-project --python 3.12 python bindings/wasm/bench/serve.py \
        [--port 8765] [--data ~/diffyman-data/tmp/spikec]

Then open http://127.0.0.1:8765/bench/index.html?size=1
"""

from __future__ import annotations

import argparse
import functools
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WASM_DIR = Path(__file__).resolve().parent.parent


class Handler(SimpleHTTPRequestHandler):
    data_dir: Path = Path()

    def translate_path(self, path: str) -> str:
        clean = path.split("?", 1)[0].split("#", 1)[0]
        if clean.startswith("/data/"):
            name = Path(clean[len("/data/"):]).name  # no sub-folders, no `..`
            return str(self.data_dir / name)
        return super().translate_path(path)

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--data", type=Path, default=Path.home() / "diffyman-data" / "tmp" / "spikec")
    args = parser.parse_args()
    Handler.data_dir = args.data.resolve()
    Handler.extensions_map[".wasm"] = "application/wasm"
    handler = functools.partial(Handler, directory=str(WASM_DIR))
    server = ThreadingHTTPServer(("127.0.0.1", args.port), handler)
    print(f"serving {WASM_DIR} and /data/ -> {Handler.data_dir} on http://127.0.0.1:{args.port}/", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
