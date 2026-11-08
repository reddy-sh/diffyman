# 017-code-refactor

A small Python refactor: a loader function is extracted, a dictionary becomes a `defaultdict`, and a loop becomes a generator expression. Several lines (the `open` and `DictReader` lines, the `region =` line, blank lines) recur in new positions, so this exercises histogram's choice of anchors on real code. A correct result is a valid tiling of both files under `text.histogram@1`, with `import csv` and the final `__main__` block equal, and no move information (`moves: []`).
