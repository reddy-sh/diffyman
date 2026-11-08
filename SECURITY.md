# Security policy

## Reporting a vulnerability

Please report security issues **privately** through GitHub's private vulnerability reporting:

1. Go to <https://github.com/reddy-sh/diffyman/security/advisories/new>.
2. Describe the issue, the affected version or commit, and how to reproduce it (a minimal input pair is ideal).

Do not open a public issue, pull request or discussion for a suspected vulnerability.

You can expect an acknowledgement within 7 days. Once the issue is confirmed, a fix is prepared in a private advisory, and the advisory is published with credit to you (unless you prefer otherwise) when the fix is released.

## Scope

diffyman parses untrusted files. Crashes, hangs, unbounded memory use, or output that does not match the spec on crafted inputs are all in scope, as are issues in the build and release pipeline.

## Supported versions

diffyman is in early development (0.x) and has no released versions yet. Fixes go to the `main` branch.
