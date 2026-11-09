# 052-yaml-workflow-steps

A CI workflow in YAML (`mediaType: "application/yaml"` from the extension): the trigger list grows, a `timeout-minutes` key is added and the lint and test steps swap. A correct result has one hunk: `replace` L2/R2, `insert` R6, and the swap reported as the lint step deleted at L8-9 and inserted again at R11-12 below the unit tests, which stay equal (L10-11/R9-10), as git's histogram diff gives. Stats: `linesAdded: 4`, `linesRemoved: 3`, `linesUnchanged: 10`.
